#!/usr/bin/env python3
"""
AOA-TaskBoard Runner for `agy` and other agents.
支援輪詢機制 (Polling) 自主領取任務並喚醒本機 Session 執行。
"""
import sys
import time
import argparse
import subprocess
import requests

DEFAULT_SERVER_URL = "http://127.0.0.1:3000"

def claim_task(server_url: str, agent_id: str):
    url = f"{server_url}/api/tasks/claim"
    try:
        resp = requests.post(url, json={"agent_id": agent_id}, timeout=5)
        if resp.status_code == 200:
            return resp.json()
    except Exception as e:
        print(f"[{agent_id}] 連線看板失敗: {e}", file=sys.stderr)
    return None

def send_heartbeat(server_url: str, task_id: str, agent_id: str):
    url = f"{server_url}/api/tasks/{task_id}/heartbeat"
    try:
        requests.post(url, json={"agent_id": agent_id}, timeout=3)
    except Exception:
        pass

def post_log(server_url: str, task_id: str, agent_id: str, message: str, level="INFO"):
    url = f"{server_url}/api/tasks/{task_id}/logs"
    try:
        requests.post(url, json={"agent_id": agent_id, "level": level, "message": message}, timeout=3)
    except Exception:
        pass

def complete_task(server_url: str, task_id: str):
    url = f"{server_url}/api/tasks/{task_id}/complete"
    try:
        requests.post(url, timeout=5)
    except Exception as e:
        print(f"回報完成失敗: {e}", file=sys.stderr)

def fail_task(server_url: str, task_id: str):
    url = f"{server_url}/api/tasks/{task_id}/fail"
    try:
        requests.post(url, timeout=5)
    except Exception as e:
        print(f"回報失敗失敗: {e}", file=sys.stderr)

def execute_agy(task, server_url: str, dry_run=False):
    task_id = task["id"]
    prompt = task["prompt"]
    agent_id = task.get("assigned_agent") or "agy"

    print(f"\n==========================================")
    print(f"🚀 領取任務 #{task_id[:8]}: {task['title']}")
    print(f"Prompt: {prompt}")
    print(f"==========================================\n")

    post_log(server_url, task_id, agent_id, f"開始執行任務: {task['title']}")

    if dry_run:
        print("[Dry-run] 模擬任務執行中...")
        for i in range(1, 4):
            time.sleep(1)
            send_heartbeat(server_url, task_id, agent_id)
            post_log(server_url, task_id, agent_id, f"執行步驟 {i}/3 完成...")
        complete_task(server_url, task_id)
        post_log(server_url, task_id, agent_id, "任務模擬執行完成！")
        return

    # 本機開啟獨立 agy session
    try:
        post_log(server_url, task_id, agent_id, "喚醒本機 agy session...")
        # 呼叫 agy 執行任務
        proc = subprocess.Popen(
            ["agy", prompt],
            stdout=subprocess.PIPE,
            stderr=subprocess.STDOUT,
            text=True,
            bufsize=1
        )

        while True:
            line = proc.stdout.readline()
            if not line and proc.poll() is not None:
                break
            if line:
                cleaned = line.strip()
                if cleaned:
                    print(f"[{agent_id}] {cleaned}")
                    post_log(server_url, task_id, agent_id, cleaned)
            send_heartbeat(server_url, task_id, agent_id)

        ret = proc.poll()
        if ret == 0:
            complete_task(server_url, task_id)
            post_log(server_url, task_id, agent_id, "任務已成功完成！")
        else:
            fail_task(server_url, task_id)
            post_log(server_url, task_id, agent_id, f"任務異常退出，代碼: {ret}", level="ERROR")
    except Exception as e:
        fail_task(server_url, task_id)
        post_log(server_url, task_id, agent_id, f"執行過程拋出異常: {str(e)}", level="ERROR")

def main():
    parser = argparse.ArgumentParser(description="Agent Board Runner")
    parser.add_argument("--agent", default="agy", help="Agent 識別碼 (例如: agy, claude)")
    parser.add_argument("--server", default=DEFAULT_SERVER_URL, help="看板 API 伺服器網址")
    parser.add_argument("--interval", type=int, default=3, help="輪詢間隔 (秒)")
    parser.add_argument("--dry-run", action="store_true", help="以模擬模式執行")
    args = parser.parse_args()

    print(f"[*] Agent Board Runner 啟動")
    print(f"[*] Agent ID: {args.agent}")
    print(f"[*] Server: {args.server}")
    print(f"[*] Polling 間隔: {args.interval} 秒")
    print(f"[*] 等待領取任務中...\n")

    while True:
        task = claim_task(args.server, args.agent)
        if task:
            execute_agy(task, args.server, dry_run=args.dry_run)
        time.sleep(args.interval)

if __name__ == "__main__":
    main()
