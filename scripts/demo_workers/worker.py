#!/usr/bin/env python3
"""
Demo mesh worker — reads TASK.md, sleeps 2s, writes a result line, exits 0.

Demonstrates state-only work loop: claim → heartbeat → complete → unblock-dependents.
Called by `edgeplane run python --mission` for each task (via MC_MESH_TASK_ID / MC_TASK_MD_PATH).
"""
import os
import sys
import time


def main():
    task_id = os.environ.get("MC_MESH_TASK_ID", "unknown")
    task_md_path = os.environ.get("MC_TASK_MD_PATH", "")

    print(f"worker: starting task {task_id}", flush=True)

    if task_md_path and os.path.exists(task_md_path):
        with open(task_md_path) as f:
            content = f.read()
        print(f"worker: TASK.md:\n{content}", flush=True)

    # Simulate work.
    time.sleep(2)

    # Write result into TASK.md body section.
    if task_md_path:
        try:
            with open(task_md_path, "a") as f:
                f.write(f"\n## Result\n\nWorker completed task {task_id} successfully.\n")
        except OSError as e:
            print(f"worker: could not write result: {e}", file=sys.stderr)

    print(f"worker: task {task_id} done", flush=True)
    return 0


if __name__ == "__main__":
    sys.exit(main())
