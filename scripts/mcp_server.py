#!/usr/bin/env python3
"""
AOA-TaskBoard MCP Server
讓 Claude Desktop、Claude Code、agy 或任何支援 MCP 的工具直接操作 Agent Board。

傳輸方式：stdio (JSON-RPC 2.0 / MCP 2024-11-05 protocol)
依賴：僅使用 Python 標準庫 (urllib, json, sys, os, pathlib)，無需安裝額外套件。

API Token 搜尋順序：
1. 命令列參數 --token
2. 環境變數 AGENT_BOARD_TOKEN
3. --token-file 指定的檔案
4. 預設 App 資料夾內的 api_token
"""
import os
import sys
import json
import platform
import argparse
import urllib.request
import urllib.error
from pathlib import Path
from typing import Any, Dict, Optional

DEFAULT_SERVER_URL = "http://127.0.0.1:18833"
APP_IDENTIFIER = "com.naxo.agent-board"
SERVER_NAME = "agent-board"
SERVER_VERSION = "0.1.0"


def default_token_file() -> Path:
    system = platform.system()
    if system == "Darwin":
        base = Path.home() / "Library" / "Application Support"
    elif system == "Windows":
        base = Path(os.environ.get("APPDATA", Path.home()))
    else:
        base = Path(os.environ.get("XDG_DATA_HOME", Path.home() / ".local" / "share"))
    return base / APP_IDENTIFIER / "api_token"


class BoardClient:
    def __init__(self, server_url: str, token: str):
        self.server_url = server_url.rstrip("/")
        self.token = token

    def _request(self, method: str, path: str, payload: Optional[Dict[str, Any]] = None) -> Any:
        url = f"{self.server_url}{path}"
        headers = {
            "Authorization": f"Bearer {self.token}",
            "Content-Type": "application/json",
            "User-Agent": f"{SERVER_NAME}-mcp/{SERVER_VERSION}",
        }
        data = json.dumps(payload).encode("utf-8") if payload is not None else None
        req = urllib.request.Request(url, data=data, headers=headers, method=method)

        try:
            with urllib.request.urlopen(req, timeout=30) as resp:
                status = resp.status
                body = resp.read().decode("utf-8")
                if not body:
                    return {"status": status}
                try:
                    return json.loads(body)
                except json.JSONDecodeError:
                    return body
        except urllib.error.HTTPError as e:
            err_body = e.read().decode("utf-8")
            raise RuntimeError(f"HTTP {e.code}: {err_body or e.reason}")
        except urllib.error.URLError as e:
            raise RuntimeError(f"連線 Agent Board 失敗: {e.reason}")

    def list_tasks(self, query: Optional[str] = None):
        path = "/api/tasks"
        if query:
            import urllib.parse
            path += f"?q={urllib.parse.quote(query)}"
        return self._request("GET", path)

    def get_task(self, task_id: str):
        return self._request("GET", f"/api/tasks/{task_id}")

    def create_task(self, data: Dict[str, Any]):
        return self._request("POST", "/api/tasks", data)

    def dispatch_task(self, task_id: str, opts: Dict[str, Any]):
        return self._request("POST", f"/api/tasks/{task_id}/dispatch", opts)

    def spawn_task(self, task_id: str, opts: Dict[str, Any]):
        return self._request("POST", f"/api/tasks/{task_id}/spawn", opts)

    def cancel_task(self, task_id: str):
        return self._request("POST", f"/api/tasks/{task_id}/cancel")

    def requeue_task(self, task_id: str, fresh: bool = False):
        return self._request("POST", f"/api/tasks/{task_id}/requeue", {"fresh": fresh})

    def followup_task(self, task_id: str, prompt: str, creator: str = "mcp"):
        return self._request("POST", f"/api/tasks/{task_id}/followup", {"prompt": prompt, "creator": creator})

    def list_agents(self):
        return self._request("GET", "/api/agents")

    def list_runners(self):
        return self._request("GET", "/api/runners")

    def add_artifact(self, task_id: str, agent_id: str, file_path: str, summary: str):
        return self._request("POST", f"/api/tasks/{task_id}/artifacts", {
            "agent_id": agent_id,
            "file_path": file_path,
            "summary": summary
        })

    def list_artifacts(self, task_id: str):
        return self._request("GET", f"/api/tasks/{task_id}/artifacts")

    def send_message(self, sender: str, receiver: str, payload: str):
        return self._request("POST", "/api/messages", {
            "sender": sender,
            "receiver": receiver,
            "payload": payload
        })


TOOLS = [
    {
        "name": "list_tasks",
        "description": "列出任務摘要（不含 prompt/result 全文，需要細節請用 get_task）",
        "inputSchema": {
            "type": "object",
            "properties": {
                "query": {"type": "string", "description": "搜尋標題/Prompt/ID/結果"},
                "status": {"type": "string", "enum": ["queued", "running", "completed", "failed"]},
                "limit": {"type": "integer", "description": "最多幾筆，預設 10"}
            }
        }
    },
    {
        "name": "get_task",
        "description": "取得任務詳情；prompt/result 預設截斷至 1500 字且不含 prompt",
        "inputSchema": {
            "type": "object",
            "properties": {
                "task_id": {"type": "string"},
                "max_chars": {"type": "integer", "description": "prompt/result 各自上限，預設 1500"},
                "full": {"type": "boolean", "description": "true = 不截斷且含 prompt"},
                "include_prompt": {"type": "boolean", "description": "附上 prompt（預設不附）"}
            },
            "required": ["task_id"]
        }
    },
    {
        "name": "create_task",
        "description": "建立看板任務",
        "inputSchema": {
            "type": "object",
            "properties": {
                "title": {
                    "type": "string",
                    "description": "任務標題"
                },
                "prompt": {
                    "type": "string",
                    "description": "提示詞；可用 {{dep.result}} 引用前置結果"
                },
                "assigned_agent": {
                    "type": "string",
                    "description": "指定執行的 Agent（例如 'agy' 或 'claude'，不填代表未指派，任何 Agent 可搶單）"
                },
                "creator": {
                    "type": "string",
                    "description": "建立者標記（預設 'mcp'）",
                    "default": "mcp"
                },
                "dependency_ids": {
                    "type": "array",
                    "items": {"type": "string"},
                    "description": "前置任務 ID 陣列，全部完成才執行；個別引用 {{dep.<ID前8碼>.result}}"
                },
                "priority": {
                    "type": "integer",
                    "description": "優先級（越大越先被領取，預設 0）",
                    "default": 0
                },
                "model": {
                    "type": "string",
                    "description": "指定模型名稱（例如 'sonnet', 'haiku', 'pro', 'flash' 等）"
                },
                "workdir": {
                    "type": "string",
                    "description": "任務執行時的工作目錄路徑"
                },
                "auto_approve": {
                    "type": "boolean",
                    "description": "略過權限確認",
                    "default": True
                },
                "isolate": {
                    "type": "boolean",
                    "description": "用獨立 git worktree 隔離",
                    "default": False
                }
            },
            "required": ["title", "prompt"]
        }
    },
    {
        "name": "dispatch_task",
        "description": "排入佇列等 Runner 領取",
        "inputSchema": {
            "type": "object",
            "properties": {
                "task_id": {
                    "type": "string",
                    "description": "任務 ID"
                },
                "agent": {
                    "type": "string",
                    "description": "指定 Agent（例如 'agy', 'claude'）"
                },
                "model": {
                    "type": "string",
                    "description": "指定模型"
                },
                "workdir": {
                    "type": "string",
                    "description": "指定工作目錄"
                },
                "auto_approve": {
                    "type": "boolean",
                    "description": "是否略過授權"
                },
                "isolate": {
                    "type": "boolean",
                    "description": "是否以 worktree 隔離"
                }
            },
            "required": ["task_id"]
        }
    },
    {
        "name": "spawn_task",
        "description": "由看板立即啟動本機子程序執行",
        "inputSchema": {
            "type": "object",
            "properties": {
                "task_id": {
                    "type": "string",
                    "description": "任務 ID"
                },
                "agent": {
                    "type": "string",
                    "description": "指定 Agent（例如 'agy', 'claude'）"
                },
                "model": {
                    "type": "string",
                    "description": "指定模型"
                },
                "workdir": {
                    "type": "string",
                    "description": "指定工作目錄"
                },
                "auto_approve": {
                    "type": "boolean",
                    "description": "是否略過授權"
                },
                "isolate": {
                    "type": "boolean",
                    "description": "是否以 worktree 隔離"
                }
            },
            "required": ["task_id"]
        }
    },
    {
        "name": "cancel_task",
        "description": "取消正在執行或佇列中的任務",
        "inputSchema": {
            "type": "object",
            "properties": {
                "task_id": {
                    "type": "string",
                    "description": "任務 ID"
                }
            },
            "required": ["task_id"]
        }
    },
    {
        "name": "requeue_task",
        "description": "重排失敗任務",
        "inputSchema": {
            "type": "object",
            "properties": {
                "task_id": {
                    "type": "string",
                    "description": "任務 ID"
                },
                "fresh": {
                    "type": "boolean",
                    "description": "true = 捨棄原 session 從頭跑",
                    "default": False
                }
            },
            "required": ["task_id"]
        }
    },
    {
        "name": "followup_task",
        "description": "對已結束任務追加指示（沿用原 session，建立新任務）",
        "inputSchema": {
            "type": "object",
            "properties": {
                "task_id": {
                    "type": "string",
                    "description": "原任務 ID"
                },
                "prompt": {
                    "type": "string",
                    "description": "追加的指令內容"
                },
                "creator": {
                    "type": "string",
                    "description": "建立者標記（預設 'mcp'）",
                    "default": "mcp"
                }
            },
            "required": ["task_id", "prompt"]
        }
    },
    {
        "name": "add_artifact",
        "description": "登記任務所產生的成果檔案（Artifact）",
        "inputSchema": {
            "type": "object",
            "properties": {
                "task_id": {
                    "type": "string",
                    "description": "任務 ID"
                },
                "agent_id": {
                    "type": "string",
                    "description": "登記者 Agent ID"
                },
                "file_path": {
                    "type": "string",
                    "description": "成果檔案絕對或相對路徑"
                },
                "summary": {
                    "type": "string",
                    "description": "成果簡述"
                }
            },
            "required": ["task_id", "agent_id", "file_path", "summary"]
        }
    },
    {
        "name": "list_artifacts",
        "description": "列出指定任務登記的成果檔案清單",
        "inputSchema": {
            "type": "object",
            "properties": {
                "task_id": {
                    "type": "string",
                    "description": "任務 ID"
                }
            },
            "required": ["task_id"]
        }
    },
    {
        "name": "list_runners",
        "description": "列出目前在線的背景 Runner 及其狀態",
        "inputSchema": {
            "type": "object",
            "properties": {}
        }
    },
    {
        "name": "list_agents",
        "description": "列出系統支援的 Agent 類型及可用模型",
        "inputSchema": {
            "type": "object",
            "properties": {}
        }
    },
    {
        "name": "send_message",
        "description": "發送 Agent 間的內部訊息留言",
        "inputSchema": {
            "type": "object",
            "properties": {
                "sender": {
                    "type": "string",
                    "description": "發送者 ID"
                },
                "receiver": {
                    "type": "string",
                    "description": "接收者 ID"
                },
                "payload": {
                    "type": "string",
                    "description": "訊息文字或 JSON 內容"
                }
            },
            "required": ["sender", "receiver", "payload"]
        }
    }
]


# --- 精簡輸出：避免大量 token 消耗 ---
LIST_DEFAULT_LIMIT = 10
TEXT_DEFAULT_MAX = 1500
LIST_FIELDS = ("id", "title", "status", "assigned_agent", "priority", "dependency_ids", "parent_task_id", "cost_usd")
DROP_FIELDS = {"last_heartbeat_at", "lease_id", "base_commit", "worktree_path", "depends_on_task_id", "updated_at"}


def clip(text: Any, limit: int) -> Any:
    """超過 limit 的字串保留頭尾並註明被截掉的字數（結果的結論常在結尾）"""
    if not isinstance(text, str) or limit <= 0 or len(text) <= limit:
        return text
    head = limit * 2 // 3
    tail = limit - head
    return f"{text[:head]}\n…（省略 {len(text) - limit} 字，傳 full=true 或調高 max_chars 取得完整內容）…\n{text[-tail:]}"


def slim_task(task: Dict[str, Any], max_chars: int) -> Dict[str, Any]:
    out = {k: v for k, v in task.items() if v not in (None, "", []) and k not in DROP_FIELDS}
    for k in ("prompt", "result"):
        if k in out:
            out[k] = clip(out[k], max_chars)
    return out


def list_summary(task: Dict[str, Any]) -> Dict[str, Any]:
    out = {k: task[k] for k in LIST_FIELDS if task.get(k) not in (None, "", [])}
    result = task.get("result")
    if result:
        out["result_preview"] = clip(result, 120)
    return out


# 欄位名稱已自明的參數不需要說明；schema 會常駐每個對話的 context，越短越省
_KEEP_DESC = {"prompt", "dependency_ids", "fresh", "isolate", "auto_approve", "full", "include_prompt", "max_chars", "limit"}


def _compact_schema(tools):
    for tool in tools:
        for name, prop in tool["inputSchema"].get("properties", {}).items():
            prop.pop("default", None)
            if name not in _KEEP_DESC:
                prop.pop("description", None)
    return tools


TOOLS = _compact_schema(TOOLS)


class McpServer:
    def __init__(self, client: BoardClient):
        self.client = client

    def handle_request(self, req: Dict[str, Any]) -> Optional[Dict[str, Any]]:
        msg_id = req.get("id")
        method = req.get("method")
        params = req.get("params", {})

        if method == "initialize":
            return {
                "jsonrpc": "2.0",
                "id": msg_id,
                "result": {
                    "protocolVersion": "2024-11-05",
                    "capabilities": {
                        "tools": {}
                    },
                    "serverInfo": {
                        "name": SERVER_NAME,
                        "version": SERVER_VERSION
                    }
                }
            }

        elif method == "notifications/initialized":
            # 客戶端通知初始化完成，不需回傳
            return None

        elif method == "ping":
            return {
                "jsonrpc": "2.0",
                "id": msg_id,
                "result": {}
            }

        elif method == "tools/list":
            return {
                "jsonrpc": "2.0",
                "id": msg_id,
                "result": {
                    "tools": TOOLS
                }
            }

        elif method == "tools/call":
            tool_name = params.get("name")
            args = params.get("arguments", {})
            try:
                content = self.call_tool(tool_name, args)
                # 寫入類工具會回傳整個任務（含剛送出的 prompt），只保留精簡版
                if tool_name not in ("get_task", "list_tasks") and isinstance(content, dict) and "prompt" in content:
                    content = slim_task(content, 200)
                return {
                    "jsonrpc": "2.0",
                    "id": msg_id,
                    "result": {
                        "content": [
                            {
                                "type": "text",
                                "text": json.dumps(content, ensure_ascii=False, separators=(",", ":")) if not isinstance(content, str) else content
                            }
                        ],
                        "isError": False
                    }
                }
            except Exception as e:
                return {
                    "jsonrpc": "2.0",
                    "id": msg_id,
                    "result": {
                        "content": [
                            {
                                "type": "text",
                                "text": f"Error executing tool {tool_name}: {str(e)}"
                            }
                        ],
                        "isError": True
                    }
                }

        else:
            if msg_id is not None:
                return {
                    "jsonrpc": "2.0",
                    "id": msg_id,
                    "error": {
                        "code": -32601,
                        "message": f"Method not found: {method}"
                    }
                }
            return None

    def call_tool(self, name: str, args: Dict[str, Any]) -> Any:
        if name == "list_tasks":
            tasks = self.client.list_tasks(query=args.get("query"))
            if not isinstance(tasks, list):
                return tasks
            if args.get("status"):
                tasks = [t for t in tasks if t.get("status") == args["status"]]
            limit = int(args.get("limit") or LIST_DEFAULT_LIMIT)
            shown = [list_summary(t) for t in tasks[:limit]]
            if len(tasks) > limit:
                shown.append({"note": f"共 {len(tasks)} 筆，僅顯示前 {limit} 筆；請用 query / status / limit 縮小或擴大範圍"})
            return shown
        elif name == "get_task":
            task = self.client.get_task(task_id=args["task_id"])
            if args.get("full") or not isinstance(task, dict):
                return task
            if not args.get("include_prompt"):
                task = {k: v for k, v in task.items() if k != "prompt"}
            return slim_task(task, int(args.get("max_chars") or TEXT_DEFAULT_MAX))
        elif name == "create_task":
            return self.client.create_task(args)
        elif name == "dispatch_task":
            task_id = args.pop("task_id")
            return self.client.dispatch_task(task_id, args)
        elif name == "spawn_task":
            task_id = args.pop("task_id")
            return self.client.spawn_task(task_id, args)
        elif name == "cancel_task":
            return self.client.cancel_task(task_id=args["task_id"])
        elif name == "requeue_task":
            return self.client.requeue_task(task_id=args["task_id"], fresh=args.get("fresh", False))
        elif name == "followup_task":
            return self.client.followup_task(
                task_id=args["task_id"],
                prompt=args["prompt"],
                creator=args.get("creator", "mcp")
            )
        elif name == "add_artifact":
            return self.client.add_artifact(
                task_id=args["task_id"],
                agent_id=args["agent_id"],
                file_path=args["file_path"],
                summary=args["summary"]
            )
        elif name == "list_artifacts":
            return self.client.list_artifacts(task_id=args["task_id"])
        elif name == "list_runners":
            return self.client.list_runners()
        elif name == "list_agents":
            return self.client.list_agents()
        elif name == "send_message":
            return self.client.send_message(
                sender=args["sender"],
                receiver=args["receiver"],
                payload=args["payload"]
            )
        else:
            raise ValueError(f"Unknown tool: {name}")

    def run_stdio(self):
        """讀取標準輸入的 JSON-RPC 訊息並寫入標準輸出"""
        for line in sys.stdin:
            line = line.strip()
            if not line:
                continue
            try:
                req = json.loads(line)
            except json.JSONDecodeError as e:
                err_resp = {
                    "jsonrpc": "2.0",
                    "id": None,
                    "error": {"code": -32700, "message": f"Parse error: {e}"}
                }
                sys.stdout.write(json.dumps(err_resp) + "\n")
                sys.stdout.flush()
                continue

            resp = self.handle_request(req)
            if resp is not None:
                sys.stdout.write(json.dumps(resp, ensure_ascii=False) + "\n")
                sys.stdout.flush()


def main():
    parser = argparse.ArgumentParser(description="Agent Board MCP Server")
    parser.add_argument("--server", default=DEFAULT_SERVER_URL, help="Agent Board API 網址")
    parser.add_argument("--token", help="API token（預設讀取環境變數或本地 token 檔）")
    parser.add_argument("--token-file", default=default_token_file(), help="API token 檔案路徑")
    args = parser.parse_args()

    token = args.token or os.environ.get("AGENT_BOARD_TOKEN")
    if not token:
        try:
            token = Path(args.token_file).read_text().strip()
        except OSError as e:
            sys.stderr.write(f"Agent Board MCP Server: 讀取 API token 失敗（{e}）。\n")
            sys.stderr.write("請先啟動 Agent Board，或以 --token / AGENT_BOARD_TOKEN 提供。\n")
            sys.exit(1)

    client = BoardClient(server_url=args.server, token=token)
    server = McpServer(client)
    server.run_stdio()


if __name__ == "__main__":
    main()
