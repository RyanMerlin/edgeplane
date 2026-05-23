#!/usr/bin/env python3
"""
Self-contained mesh agent worker for the demo.

Enrolls as a MeshAgent, polls for ready tasks in the domain's missions,
claims and executes each one (sleep 2s), then posts complete or fail.
Demonstrates the full backend work loop without needing mc run.

Usage:
    python3 agent_worker.py <domain_id> [<base_url>] [<token>]
"""
import json
import os
import sys
import time
import urllib.request
import urllib.error


def main():
    if len(sys.argv) < 2:
        print("usage: agent_worker.py <domain_id> [base_url] [token]", file=sys.stderr)
        sys.exit(1)

    domain_id = sys.argv[1]
    base_url = sys.argv[2] if len(sys.argv) > 2 else os.environ.get("MC_BASE_URL", "http://localhost:8008")
    token = sys.argv[3] if len(sys.argv) > 3 else os.environ.get("MC_TOKEN", "")
    worker_id = f"demo-worker-{os.getpid()}"

    def api(method, path, body=None):
        url = f"{base_url}{path}"
        data = json.dumps(body).encode() if body is not None else None
        headers = {"Content-Type": "application/json"}
        if token:
            headers["Authorization"] = f"Bearer {token}"
        req = urllib.request.Request(url, data=data, headers=headers, method=method)
        try:
            with urllib.request.urlopen(req, timeout=10) as r:
                return json.loads(r.read())
        except urllib.error.HTTPError as e:
            return {"_error": e.code, "_msg": e.read().decode()}

    def log(msg):
        print(f"[{worker_id}] {msg}", flush=True)

    # Enroll as a mesh agent
    log(f"enrolling in domain {domain_id}…")
    enroll = api("POST", f"/work/domains/{domain_id}/agents/enroll", {
        "runtime_kind": "demo_python",
        "capabilities": ["demo"],
        "labels": {"worker": worker_id},
        "supervision_mode": "solo",
        "profile": {"name": "demo"},
    })
    if "_error" in enroll:
        log(f"enroll failed: {enroll}")
        sys.exit(1)
    agent_id = enroll.get("id", "")
    log(f"enrolled as agent {agent_id}")

    def heartbeat():
        api("POST", f"/work/agents/{agent_id}/heartbeat", {})

    def get_missions():
        result = api("GET", f"/domains/{domain_id}/m")
        if isinstance(result, list):
            return [m["id"] for m in result if "id" in m]
        return []

    def get_ready_tasks(mission_id):
        result = api("GET", f"/work/missions/{mission_id}/tasks?status=ready")
        if isinstance(result, list):
            return [t["id"] for t in result if t.get("status") == "ready"]
        return []

    def claim_task(task_id):
        result = api("POST", f"/work/tasks/{task_id}/claim", {})
        if "_error" in result:
            return None  # 423/409 = already claimed by another agent
        return result

    def complete_task(task_id, lease_id):
        api("POST", f"/work/tasks/{task_id}/complete", {
            "claim_lease_id": lease_id,
        })

    def fail_task(task_id, lease_id, error):
        api("POST", f"/work/tasks/{task_id}/fail", {
            "claim_lease_id": lease_id,
            "error": error,
        })

    # Work loop: poll every second for up to 90s
    deadline = time.time() + 90
    poll_interval = 1.0
    hb_interval = 20.0
    last_hb = time.time()

    log("entering work loop…")
    while time.time() < deadline:
        # Heartbeat
        if time.time() - last_hb > hb_interval:
            heartbeat()
            last_hb = time.time()

        # Scan missions for ready tasks
        claimed_something = False
        for mid in get_missions():
            for task_id in get_ready_tasks(mid):
                log(f"  attempting claim on task {task_id}…")
                claim = claim_task(task_id)
                if claim is None:
                    log(f"  task {task_id} already taken, skipping")
                    continue

                lease_id = claim.get("claim_lease_id", "")
                title = claim.get("title", task_id)
                log(f"  claimed task {task_id} — {title}")

                # Simulate work
                for _ in range(4):  # 4 × 0.5s = 2s total
                    time.sleep(0.5)
                    heartbeat()
                    last_hb = time.time()

                complete_task(task_id, lease_id)
                log(f"  completed task {task_id}")
                claimed_something = True
                break  # one task at a time per outer scan
            if claimed_something:
                break  # one mission at a time per outer scan

        if not claimed_something:
            time.sleep(poll_interval)

    # Mark offline
    api("POST", f"/work/agents/{agent_id}/status?status=offline", {})
    log("offline — exiting")


if __name__ == "__main__":
    main()
