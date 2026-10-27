#!/usr/bin/env python3
"""
AOA-TaskBoard Runner for `agy` and other agents.
支援輪詢機制 (Polling) 自主領取任務並喚醒本機 Session 執行。

API token 讀取順序：--token、環境變數 AGENT_BOARD_TOKEN、--token-file（預設為 App 資料夾內的 api_token）
"""
import os
import sys
import time
import argparse
import platform
import signal
import subprocess
import threading
from pathlib import Path

import requests

DEFAULT_SERVER_URL = "http://127.0.0.1:18833"
APP_IDENTIFIER = "com.naxo.agent-board"
# 需小於後端的 LEASE_TIMEOUT_SECS（預設 60 秒）
HEARTBEAT_INTERVAL = float(os.environ.get("AGENT_BOARD_HEARTBEAT_SECS", 15))


def default_token_file() -> Path:
    system = platform.system()
    if system == "Darwin":
        base = Path.home() / "Library" / "Application Support"
    elif system == "Windows":
        base = Path(os.environ.get("APPDATA", Path.home()))
    else:
        base = Path(os.environ.get("XDG_DATA_HOME", Path.home() / ".local" / "share"))
    return base / APP_IDENTIFIER / "api_token"


def make_session(args) -> requests.Session:
    token = args.token or os.environ.get("AGENT_BOARD_TOKEN")
    if not token:
        try:
            token = Path(args.token_file).read_text().strip()
        except OSError as e:
            sys.exit(f"讀取 API token 失敗（{e}）。請先啟動 Agent Board，或用 --token / AGENT_BOARD_TOKEN 指定。")
    session = requests.Session()
    session.headers["Authorization"] = f"Bearer {token}"
    return session


class Board:
    def __init__(self, session: requests.Session, server_url: str, agent_id: str):
        self.http = session
        self.server = server_url
        self.agent_id = agent_id

    def claim(self):
        try:
            resp = self.http.post(f"{self.server}/api/tasks/claim", json={"agent_id": self.agent_id}, timeout=5)
            if resp.status_code == 401:
                sys.exit("API token 無效，請確認 token 檔是否為目前看板產生的。")
            if resp.status_code == 200:
                return resp.json()
        except requests.RequestException as e:
            print(f"[{self.agent_id}] 連線看板失敗: {e}", file=sys.stderr)
        return None

    def heartbeat(self, task) -> bool:
        """回傳 False 代表租約已失效（任務被回收或轉給其他 Agent），應停止執行"""
        try:
            resp = self.http.post(f"{self.server}/api/tasks/{task['id']}/heartbeat", json={"lease_id": task["lease_id"]}, timeout=3)
            return resp.status_code != 409
        except requests.RequestException:
            return True  # 暫時連不上不代表租約失效，下次再試

    def log(self, task, message: str, level="INFO"):
        try:
            self.http.post(
                f"{self.server}/api/tasks/{task['id']}/logs",
                json={"agent_id": self.agent_id, "level": level, "message": message},
                timeout=3,
            )
        except requests.RequestException:
            pass

    def output(self, task, line: str):
        """Agent 的原始輸出；伺服器解析 stream-json 後寫成日誌並記錄結果"""
        try:
            self.http.post(
                f"{self.server}/api/tasks/{task['id']}/output",
                json={"agent_id": self.agent_id, "lease_id": task["lease_id"], "lines": [line]},
                timeout=3,
            )
        except requests.RequestException:
            pass

    def finish(self, task, ok: bool):
        action = "complete" if ok else "fail"
        try:
            resp = self.http.post(f"{self.server}/api/tasks/{task['id']}/{action}", json={"lease_id": task["lease_id"]}, timeout=5)
            if resp.status_code == 409:
                print(f"[{self.agent_id}] 租約已失效，無法回報 {action}（任務可能已被回收）", file=sys.stderr)
        except requests.RequestException as e:
            print(f"回報 {action} 失敗: {e}", file=sys.stderr)


class HeartbeatThread(threading.Thread):
    """獨立於任務輸出定時送心跳；租約失效時設定 lost 並呼叫 on_lost"""

    def __init__(self, board: Board, task, on_lost=lambda: None):
        super().__init__(daemon=True)
        self.board, self.task, self.on_lost = board, task, on_lost
        self.stopped = threading.Event()
        self.lost = False

    def run(self):
        while not self.stopped.wait(HEARTBEAT_INTERVAL):
            if not self.board.heartbeat(self.task):
                self.lost = True
                self.on_lost()
                return

    def stop(self):
        self.stopped.set()


def kill_tree(proc: subprocess.Popen):
    try:
        os.killpg(proc.pid, signal.SIGKILL)
    except (AttributeError, ProcessLookupError):  # Windows 沒有 killpg
        proc.kill()


def execute(board: Board, task, dry_run=False, dry_run_seconds=3):
    agent_id = board.agent_id

    print(f"\n==========================================")
    print(f"🚀 領取任務 #{task['id'][:8]}: {task['title']} (第 {task.get('attempts', 1)} 次)")
    print(f"Prompt: {task['prompt']}")
    print(f"==========================================\n")

    board.log(task, f"開始執行任務: {task['title']}")

    if dry_run:
        print(f"[Dry-run] 模擬任務執行中（{dry_run_seconds} 秒，期間不輸出）...")
        hb = HeartbeatThread(board, task)
        hb.start()
        deadline = time.monotonic() + dry_run_seconds
        while time.monotonic() < deadline and not hb.lost:
            time.sleep(0.2)
        hb.stop()
        if hb.lost:
            print(f"[{agent_id}] 租約已失效，放棄任務")
            return
        board.finish(task, ok=True)
        board.log(task, "任務模擬執行完成！")
        return

    # argv 由伺服器的 Agent Registry 組好（含 --model 與權限參數），Runner 不自帶指令表
    command = task.get("command")
    if not command:
        board.log(task, f"不支援的 Agent：{task.get('assigned_agent')}", level="ERROR")
        board.finish(task, ok=False)
        return

    # 隔離執行時 run_dir 指向伺服器建立的 git worktree
    workdir = task.get("run_dir") or task.get("workdir") or None
    try:
        board.log(task, f"執行：{' '.join(command[:-1])}（工作目錄：{workdir or os.getcwd()}）")
        proc = subprocess.Popen(
            command,
            cwd=workdir,
            stdin=subprocess.DEVNULL,
            stdout=subprocess.PIPE,
            stderr=subprocess.STDOUT,
            text=True,
            bufsize=1,
            start_new_session=True,  # 獨立 process group，中止時連同工具子程序一起結束
        )
        # Agent 長時間沒輸出時 readline 會卡住，心跳必須在另一個執行緒送
        hb = HeartbeatThread(board, task, on_lost=lambda: kill_tree(proc))
        hb.start()

        # 輸出是 stream-json，原樣交給伺服器解析成日誌、結果與花費
        for line in proc.stdout:
            cleaned = line.strip()
            if cleaned:
                print(f"[{agent_id}] {cleaned[:200]}")
                board.output(task, cleaned)
        ret = proc.wait()
        hb.stop()

        if hb.lost:
            print(f"[{agent_id}] 租約已失效，已中止 {command[0]}（任務被取消或回收）")
        elif ret == 0:
            board.finish(task, ok=True)
            board.log(task, "任務已成功完成！")
        else:
            board.finish(task, ok=False)
            board.log(task, f"任務異常退出，代碼: {ret}", level="ERROR")
    except Exception as e:
        board.finish(task, ok=False)
        board.log(task, f"執行過程拋出異常: {str(e)}", level="ERROR")


def main():
    parser = argparse.ArgumentParser(description="Agent Board Runner")
    parser.add_argument("--agent", default="agy", help="Agent 識別碼 (例如: agy, claude)")
    parser.add_argument("--server", default=DEFAULT_SERVER_URL, help="看板 API 伺服器網址")
    parser.add_argument("--interval", type=int, default=3, help="輪詢間隔 (秒)")
    parser.add_argument("--token", help="API token（預設從 token 檔讀取）")
    parser.add_argument("--token-file", default=str(default_token_file()), help="API token 檔路徑")
    parser.add_argument("--dry-run", action="store_true", help="以模擬模式執行")
    parser.add_argument("--dry-run-seconds", type=float, default=3, help="模擬模式下每個任務的執行秒數")
    parser.add_argument("--once", action="store_true", help="處理完佇列中的任務後就結束（測試用）")
    args = parser.parse_args()

    board = Board(make_session(args), args.server, args.agent)

    print(f"[*] Agent Board Runner 啟動")
    print(f"[*] Agent ID: {args.agent}")
    print(f"[*] Server: {args.server}")
    print(f"[*] Polling 間隔: {args.interval} 秒")
    print(f"[*] 等待領取任務中...\n")

    while True:
        task = board.claim()
        if task:
            execute(board, task, dry_run=args.dry_run, dry_run_seconds=args.dry_run_seconds)
            continue
        if args.once:
            return
        time.sleep(args.interval)


if __name__ == "__main__":
    main()
