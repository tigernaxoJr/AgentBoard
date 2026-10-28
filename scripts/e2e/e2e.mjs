// 端對端測試：用真的 server、Runner、MCP 與假 Agent（fake-agent.rs）驗證任務生命週期。
// 用法：node scripts/e2e/e2e.mjs   （需要 cargo 與 rustc；不需要真的 claude / agy）
import { spawn, execSync } from "node:child_process";
import fs from "node:fs";
import path from "node:path";
import { fileURLToPath } from "node:url";

const SP = path.dirname(fileURLToPath(import.meta.url));
const TAURI = path.resolve(SP, "../../src-tauri");
const ROOT = path.join(TAURI, "target/debug");
const EXE = process.platform === "win32" ? ".exe" : "";
const SERVER = `${ROOT}/examples/headless${EXE}`;
const CLI = `${ROOT}/agent-board${EXE}`;

console.log("building server, CLI and fake agent...");
execSync("cargo build --bin agent-board --example headless", { cwd: TAURI, stdio: "inherit" });
const binDir = path.join(TAURI, "target/e2e/bin");
fs.mkdirSync(binDir, { recursive: true });
for (const name of ["agy", "claude"]) {
  execSync(`rustc -O -o "${path.join(binDir, name + EXE)}" "${path.join(SP, "fake-agent.rs")}"`, { stdio: "inherit" });
}
const PORT = 18899;
const BASE = `http://127.0.0.1:${PORT}`;
const work = path.join(TAURI, "target/e2e/work");
fs.rmSync(work, { recursive: true, force: true });
fs.mkdirSync(work, { recursive: true });
const dbPath = path.join(work, "board.db");
const tokenPath = path.join(work, "api_token");
const pidFile = path.join(work, "pids.txt");

const sleep = (ms) => new Promise((r) => setTimeout(r, ms));
let failures = 0;
const check = (name, ok, extra = "") => {
  console.log(`${ok ? "PASS" : "FAIL"}  ${name}${extra ? "  -> " + extra : ""}`);
  if (!ok) failures++;
};

let token = "";
const api = async (method, p, body) => {
  const res = await fetch(BASE + p, {
    method,
    headers: { "Content-Type": "application/json", Authorization: `Bearer ${token}` },
    body: body === undefined ? undefined : JSON.stringify(body),
  });
  const text = await res.text();
  let json;
  try { json = JSON.parse(text); } catch { json = text; }
  return { status: res.status, json };
};

const startServer = async (db, lease) => {
  const proc = spawn(SERVER, [db, tokenPath], {
    env: { ...process.env, PORT: String(PORT), LEASE_TIMEOUT_SECS: String(lease) },
    stdio: ["ignore", "pipe", "pipe"],
  });
  proc.stdout.on("data", () => {});
  proc.stderr.on("data", () => {});
  for (let i = 0; i < 50; i++) {
    await sleep(200);
    if (fs.existsSync(tokenPath)) {
      token = fs.readFileSync(tokenPath, "utf8").trim();
      try { if ((await api("GET", "/api/tasks")).status === 200) return proc; } catch {}
    }
  }
  throw new Error("server did not start");
};
const killProc = (proc) => {
  try {
    if (process.platform === "win32") execSync(`taskkill /PID ${proc.pid} /T /F`, { stdio: "ignore" });
    else proc.kill("SIGKILL");
  } catch {}
};

const waitFor = async (id, pred, ms = 30000) => {
  const end = Date.now() + ms;
  let t;
  while (Date.now() < end) {
    t = (await api("GET", `/api/tasks/${id}`)).json;
    if (pred(t)) return t;
    await sleep(300);
  }
  return t;
};
const mk = async (prompt, extra = {}) =>
  (await api("POST", "/api/tasks", { title: prompt.slice(0, 20), prompt, assigned_agent: "agy", ...extra })).json;
const alive = (pid) => { try { process.kill(pid, 0); return true; } catch { return false; } };

// ───────────────────────── Phase A: runner scenarios ─────────────────────────
let server = await startServer(dbPath, 60);
console.log("server up, token length", token.length);

let runnerOut = "";
const runner = spawn(CLI, ["runner", "--agent", "agy", "--server", BASE, "--token-file", tokenPath, "--interval", "1"], {
  env: { ...process.env, PATH: `${binDir}${path.delimiter}${process.env.PATH}`, FAKE_PID_FILE: pidFile },
  stdio: ["ignore", "pipe", "pipe"],
});
runner.stdout.on("data", (d) => (runnerOut += d));
runner.stderr.on("data", (d) => (runnerOut += d));

// T1 success
let t = await mk("OK please");
t = await waitFor(t.id, (x) => x.status === "completed" || x.status === "failed");
check("T1 success -> completed", t.status === "completed" && t.result === "done", `${t.status} result=${t.result} err=${t.error}`);

// T2 non-zero exit
t = await mk("FAIL now");
t = await waitFor(t.id, (x) => x.status === "failed" || x.status === "completed");
check("T2 exit 3 -> failed with reason", t.status === "failed" && /結束碼：3/.test(t.error ?? ""), `${t.status} err=${t.error}`);

// T3 agent-reported error with exit 0
t = await mk("ERR boom");
t = await waitFor(t.id, (x) => x.status === "failed" || x.status === "completed");
check("T3 agent is_error (exit 0) -> failed", t.status === "failed" && /model exploded/.test(t.error ?? ""), `${t.status} err=${t.error}`);

// T4 cascade
const a = await mk("FAIL parent");
const b = await mk("OK child", { dependency_ids: [a.id] });
await waitFor(a.id, (x) => x.status === "failed");
const b2 = await waitFor(b.id, (x) => x.status === "failed");
check("T4 downstream fails with reason", b2.status === "failed" && b2.error === "前置任務失敗", `${b2.status} err=${b2.error}`);
const rq = await api("POST", `/api/tasks/${a.id}/requeue`, {});
check("T4 requeue clears downstream error", rq.status === 200);
const b3 = (await api("GET", `/api/tasks/${b.id}`)).json;
check("T4 downstream back to queued, error cleared", b3.status === "queued" && b3.error === null, `${b3.status} err=${b3.error}`);
await api("POST", `/api/tasks/${a.id}/cancel`); // stop the loop of FAIL parents
await api("POST", `/api/tasks/${b.id}/cancel`);

// T5 cancel kills the whole process tree
fs.rmSync(pidFile, { force: true });
t = await mk("SLEEP forever");
await waitFor(t.id, (x) => x.status === "running");
for (let i = 0; i < 30 && !fs.existsSync(pidFile); i++) await sleep(200);
const [agentPid, childPid] = fs.readFileSync(pidFile, "utf8").split(" ").map(Number);
check("T5 agent + grandchild running", alive(agentPid) && alive(childPid), `pids ${agentPid} ${childPid}`);
const t0 = Date.now();
await api("POST", `/api/tasks/${t.id}/cancel`);
const cancelled = (await api("GET", `/api/tasks/${t.id}`)).json;
check("T5 cancel records reason", cancelled.status === "failed" && cancelled.error === "已取消", `err=${cancelled.error}`);
for (let i = 0; i < 80 && (alive(agentPid) || alive(childPid)); i++) await sleep(500);
const secs = ((Date.now() - t0) / 1000).toFixed(1);
check("T5 runner killed agent AND grandchild after lease loss", !alive(agentPid) && !alive(childPid), `${secs}s; agent alive=${alive(agentPid)} child alive=${alive(childPid)}`);
if (alive(childPid)) try { process.kill(childPid); } catch {}

// T6 finish() retries while the server is briefly down
runnerOut = "";
t = await mk("SLOWOK wait");
await waitFor(t.id, (x) => x.status === "running");
await sleep(1500);
killProc(server);
await sleep(500);
let reallyDown = false;
try { await api("GET", "/api/tasks"); } catch { reallyDown = true; }
check("T6 server is really down during the outage", reallyDown);
console.log("server killed while task running; restarting in ~7s");
await sleep(7000);
server = await startServer(dbPath, 60);
t = await waitFor(t.id, (x) => x.status === "completed" || x.status === "failed", 20000);
check("T6 task still completes after server outage (finish retried)", t.status === "completed", `${t.status} attempts=${t.attempts}`);
check("T6 result emitted during the outage is not lost", t.result === "done-slow", `result=${t.result}`);
check("T6 task not re-run (attempts == 1)", t.attempts === 1, `attempts=${t.attempts}`);

killProc(runner);

// T7 REST messages
await api("POST", "/api/messages", { sender: "a", receiver: "b", payload: "direct" });
await api("POST", "/api/messages", { sender: "a", receiver: "*", payload: "broadcast" });
let inbox = (await api("GET", "/api/messages?receiver=b&unread=true")).json;
check("T7 b sees direct + broadcast", inbox.map((m) => m.payload).join() === "direct,broadcast", JSON.stringify(inbox.map((m) => m.payload)));
inbox = (await api("GET", "/api/messages?receiver=a&unread=true")).json;
check("T7 sender does not get own broadcast", inbox.length === 0);
await api("POST", "/api/messages/read", { reader: "b" });
check("T7 b inbox empty after read", (await api("GET", "/api/messages?receiver=b&unread=true")).json.length === 0);
check("T7 c still sees broadcast", (await api("GET", "/api/messages?receiver=c&unread=true")).json.length === 1);
check("T7 /api/messages/all", (await api("GET", "/api/messages/all")).json.length === 2);

// T8 delete dependency
const pa = await mk("hold", { assigned_agent: "claude" });
const pb = await mk("child", { assigned_agent: "claude", dependency_ids: [pa.id] });
await api("DELETE", `/api/tasks/${pa.id}`);
const pb2 = (await api("GET", `/api/tasks/${pb.id}`)).json;
check("T8 deleting dependency fails downstream (not unblocked)", pb2.status === "failed" && pb2.error === "前置任務已被刪除", `${pb2.status} err=${pb2.error}`);
check("T8 404 for log on missing task", (await api("POST", "/api/tasks/nope/logs", { agent_id: "x", message: "m" })).status === 404);

// ───────────────────────── Phase B: MCP over stdio ─────────────────────────
const mcp = spawn(CLI, ["mcp"], {
  env: { ...process.env, AGENT_BOARD_SERVER: BASE, AGENT_BOARD_TOKEN: token },
  stdio: ["pipe", "pipe", "pipe"],
});
let buf = "";
const pending = new Map();
mcp.stdout.on("data", (d) => {
  buf += d;
  let i;
  while ((i = buf.indexOf("\n")) >= 0) {
    const line = buf.slice(0, i).trim();
    buf = buf.slice(i + 1);
    if (!line) continue;
    try { const m = JSON.parse(line); if (m.id != null && pending.has(m.id)) { pending.get(m.id)(m); pending.delete(m.id); } } catch {}
  }
});
let rid = 0;
const rpc = (method, params) => new Promise((resolve) => {
  const id = ++rid;
  pending.set(id, resolve);
  mcp.stdin.write(JSON.stringify({ jsonrpc: "2.0", id, method, params }) + "\n");
});
const call = async (name, args) => {
  const r = await rpc("tools/call", { name, arguments: args });
  const text = r.result?.content?.[0]?.text;
  let json; try { json = JSON.parse(text); } catch { json = text; }
  return { isError: r.result?.isError, json };
};
const init = await rpc("initialize", { protocolVersion: "2024-11-05", capabilities: {}, clientInfo: { name: "e2e", version: "1" } });
mcp.stdin.write(JSON.stringify({ jsonrpc: "2.0", method: "notifications/initialized" }) + "\n");
check("T9 MCP initialize", !!init.result, JSON.stringify(init.error ?? ""));
const tools = (await rpc("tools/list", {})).result.tools.map((x) => x.name);
check("T9 tools include read_messages + wait_task", tools.includes("read_messages") && tools.includes("wait_task"), tools.join(","));

const created = await call("create_task", { title: "prio", prompt: "x", assigned_agent: "claude", priority: 7 });
const back = (await api("GET", `/api/tasks/${created.json.id}`)).json;
check("T9 create_task priority is honored", back.priority === 7, `priority=${back.priority}`);

await call("send_message", { sender: "claude", receiver: "agy", payload: "hello agy", task_id: created.json.id });
let r1 = await call("read_messages", { receiver: "agy", task_id: created.json.id });
check("T9 read_messages returns + marks read", Array.isArray(r1.json) && r1.json.length === 1 && r1.json[0].task_id === created.json.id, JSON.stringify(r1.json).slice(0, 120));
let r2 = await call("read_messages", { receiver: "agy", task_id: created.json.id });
check("T9 second read_messages is empty", Array.isArray(r2.json) && r2.json.length === 0, JSON.stringify(r2.json));

const w1 = Date.now();
const wt = await call("wait_task", { task_id: created.json.id, timeout_secs: 3 });
check("T9 wait_task times out on a queued task", wt.json.timed_out === true && Date.now() - w1 >= 2500, `timed_out=${wt.json.timed_out} after ${Date.now() - w1}ms`);

await api("POST", `/api/tasks/${created.json.id}/cancel`);
const wt2 = await call("wait_task", { task_id: created.json.id, timeout_secs: 5 });
check("T9 wait_task returns immediately once failed", wt2.json.status === "failed" && wt2.json.timed_out === false && wt2.json.error === "已取消", JSON.stringify(wt2.json).slice(0, 160));
killProc(mcp);

// ───────────────────────── Phase C: retry backoff (short lease) ─────────────────────────
killProc(server);
await sleep(500);
fs.rmSync(tokenPath, { force: true });
const db2 = path.join(work, "board2.db");
server = await startServer(db2, 3);
const bt = await mk("backoff");
const claim1 = await api("POST", "/api/tasks/claim", { agent_id: "agy" });
check("T10 claimed", claim1.json?.id === bt.id);
const requeued = await waitFor(bt.id, (x) => x.status === "queued", 15000);
check("T10 lease timeout requeues with not_before", requeued.status === "queued" && !!requeued.not_before, `${requeued.status} not_before=${requeued.not_before}`);
const claim2 = await api("POST", "/api/tasks/claim", { agent_id: "agy" });
check("T10 immediate reclaim is held back by backoff", claim2.json === null, JSON.stringify(claim2.json)?.slice(0, 80));
killProc(server);

console.log(failures === 0 ? "\nALL PASSED" : `\n${failures} FAILED`);
process.exit(failures === 0 ? 0 : 1);
