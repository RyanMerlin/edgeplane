---
title: Overlap Detection
description: Detect duplicate or conflicting work before claiming tasks using EdgePlane's built-in similarity analysis.
---

## What Overlap Detection Does

:::caution[Planned capability]
The `overlapsuggestion` table and the `get_overlap_suggestions` MCP tool both exist today, but the background similarity analysis that would populate the table in a running deployment is **not implemented yet** — currently the table is only ever written by test fixtures. Calling the tool today is safe but will typically return an empty list. The rest of this page documents the intended design; treat it as advisory/roadmap until the analysis pipeline ships.
:::

When agents claim tasks independently, they can unknowingly start work that duplicates something already in progress. The earlier a duplicate is discovered, the cheaper it is to fix — a pre-claim check costs milliseconds; discovering the collision after two agents have done equivalent work costs everything both agents spent.

`get_overlap_suggestions` is an MCP tool available to agents at runtime. Given a task ID, it is designed to return a ranked list of existing tasks that are semantically similar, ordered by `similarity_score` descending. Each entry includes an `evidence` field summarizing why the overlap was detected and a `suggested_action` field with a recommended response.

The tool queries the `overlapsuggestion` table. Nothing populates that table in production yet — there is no background similarity job. You do not drive the analysis, and today there generally isn't one to query results from.

---

## When to Call It

Call `get_overlap_suggestions` in two situations:

**Before claiming a MeshTask.** After you call `submit_mesh_task` to register new work, check for overlaps before calling `claim_mesh_task`. If high-similarity results come back, evaluate whether the work is genuinely distinct before taking the claim.

**Before creating a new task or mission inside a domain.** When an agent is about to create net-new work — a new task, a new mission — checking for overlaps first prevents cluttering the domain with redundant entries.

The general pattern is: submit or identify the task, check overlaps, decide, then claim or reference.

---

## Using the MCP Tool

```
get_overlap_suggestions(task_id: "<your-task-id>", limit: 5)
```

`task_id` is the ID of the task you are about to claim or have just submitted. `limit` caps the result count; the maximum the server enforces is 50. Omitting `limit` defaults to 10.

### Example response

```json
[
  {
    "id": 17,
    "task_id": 42,
    "candidate_task_id": 38,
    "similarity_score": 0.91,
    "evidence": "Both tasks target authentication token refresh logic in the same service; titles share 84% token overlap.",
    "suggested_action": "reference"
  },
  {
    "id": 18,
    "task_id": 42,
    "candidate_task_id": 29,
    "similarity_score": 0.63,
    "evidence": "Partial overlap on error handling in the OAuth flow; different services.",
    "suggested_action": "review"
  }
]
```

Fields returned per entry:

| Field | Type | Description |
|-------|------|-------------|
| `id` | integer | Row ID in the overlap suggestions table |
| `task_id` | integer | The task you queried |
| `candidate_task_id` | integer | The potentially overlapping task |
| `similarity_score` | float (0–1) | How similar the two tasks are; higher is more similar |
| `evidence` | string | Human-readable explanation of the detected overlap |
| `suggested_action` | string | Recommendation: `"reference"`, `"review"`, or `"proceed"` |

---

## Acting on the Results

**No results or all low scores (below ~0.5):** The work looks distinct. Proceed with claiming.

**Medium scores (0.5–0.8) with `suggested_action: "review"`:** Inspect the candidate task. Read its description and current status. The overlap may be incidental — different services, different scopes — or it may mean your task should build on or coordinate with the existing one rather than run in parallel.

**High scores (above 0.8) with `suggested_action: "reference"`:** The candidate task is likely covering the same ground. Before claiming your task:

1. Fetch the candidate with `get_mesh_task(task_id: "<candidate_task_id>")`.
2. Check its `status`. If it is `claimed` or `in_progress`, contact the owning agent before proceeding — use `send_mesh_message` with `recipient_agent_id` set to `claimed_by_agent_id`.
3. If the candidate is `finished`, review its output artifacts before starting your own work. You may be able to reference the result rather than reproduce it.
4. If the candidate is `ready` and unclaimed, consider whether your task and the candidate should be merged, or whether one of them should be cancelled.

**`suggested_action: "proceed"`:** The system found similarity but assessed the work as distinct enough to continue. Treat this as advisory — if the `evidence` field points at meaningful overlap that the score does not capture, investigate anyway.

---

## The Full Workflow

```
submit_mesh_task(mission_id, domain_id, title, description)
→ returns task_id

get_overlap_suggestions(task_id, limit=5)
→ inspect similarity_score + suggested_action for each result

if high overlap found:
    get_mesh_task(candidate_task_id)          # check status + owner
    send_mesh_message(recipient_agent_id)     # coordinate if in progress
    → decide: reference, defer, cancel, or proceed

claim_mesh_task(task_id, agent_id)
→ now execute
```

Running this check consistently keeps the domain clean. Tasks that would have duplicated each other surface before any lease is taken, and agents that might collide can coordinate rather than silently competing.

---

## Notes

**The tool is pre-task by design.** Per ADR 0006, `get_overlap_suggestions` is kept in the runtime MCP surface specifically because it is called before claiming work. It is not a post-hoc audit tool.

**Results depend on analysis currency.** As noted above, no background process populates the `overlapsuggestion` table in production today, so results are currently empty by default. Once the analysis pipeline ships, freshly submitted tasks may still lack overlap candidates until it has run since submission.

**Empty results are not a guarantee of uniqueness.** They mean no overlap candidates were detected at analysis time. If you have reason to believe similar work exists — for example, you know a related task was submitted recently — check `list_mesh_tasks` with a status filter as a secondary signal.
