# EdgePlane — Platform Roadmap (surfaced by the CLI reorg)

**Date:** 2026-06-13
**Status:** 🟡 DRAFT roadmap — companion to `2026-06-11-edgeplane-cli-tree.md` (the resolved CLI surface).
**Origin:** items deferred or surfaced while designing the WS-3 command tree. Each is its own slice/PR.

---

## 0. The cross-cutting finding — built-but-unused scaffolding

The reorg investigation (5 evidence-backed code deep-dives, 2026-06-13) found that a large fraction of
EdgePlane's surface is half-wired. This is the highest-leverage cleanup available and directly serves
"a clean structure to build on."

| Feature | Status (code evidence) |
|---|---|
| AI chat (`aisession` + `/ai/*` + web console) | cleanly separable; **no FK/coupling to agent execution**; edgeplaned never references it; not in the web sidebar nav |
| `evolve` (`agent evolve`, `/evolve/*`) | **dead + broken** — tower `INSERT INTO evolvemission` (no such table → 500), `run` launches nothing; the `evolve` cron invokes the *aria* `/evolve` skill, not this. Nothing calls `edgeplane agent evolve`. |
| skill-sync delivery (`/skills/sync/*`, `skillsnapshot`, `skilllocalstate`, `data sync`) | **dormant** — producer writes bundles (domain-pack import) but **no runtime fetches/applies a snapshot**; `--allowed-tools` derives from `required_capabilities`. `data sync` is manual-only. |
| GovernancePolicy + GovernancePolicyEvent + approvals | **stored, never enforced** — no mutation path reads policy; approvals never block; `reload` is a no-op. |
| `is_admin` | **hardwired `false`** for every principal → all admin-only + governance-mutation routes are permanently inaccessible dead code. |
| `edgeplaned-sandbox` (isolation jail) | full jail (userns + Landlock + seccomp + cap-drop) **implemented but not wired into any spawn path**. |

---

## R1 — Simplification / deletion epic (do early; mostly removals)

Each sub-item is its own PR. All verified safe via the 2026-06-13 deep-dives.

### R1a — Drop the built-in AI chat (`aisession`)
**Why:** early design; agents supersede it; the operator no longer wants a separate chat surface.
**Verified clean:** no FK from `agentrun`/runtime tables to `aisession`; `agentrun.runtime_session_id` is an *external* runtime handle, not an aisession ref; edgeplaned has zero aisession references; `evolverun.ai_session_id` is always null.
**Scope:** delete `routes/ai.rs`, `models/ai.rs`; migration to drop `aisession`/`aiturn`/`aievent`/`aipendingaction`; delete web `routes/ai.tsx` + `lib/conversation/useRestConversation.ts` + `components/conversation/*` + their tests; remove `web/src/lib/queryKeys.ts` ai block; the `edgeplane channel claude missioncontrol` command rides on `/ai/sessions/{id}/stream` → goes with it. Web schema/routeTree regenerate.
**CLI impact:** removes the proposed `session` noun. Optionally add `agent runs` over `agentrun` (`/runs` already exists with list/get/resume) for run observability.

### R1b — Drop `evolve`
**Why:** dead + broken; superseded by the aria `/evolve` skill.
**Scope:** delete `crates/edgeplane/src/evolve.rs` + CLI wiring (`lib.rs`, `commands.rs` `AgentCommand::Evolve`), `routes/evolve.rs` + router merge, the two auth tests, `web/queryKeys.ts` evolve block; migration to drop `evolvedomain`/`evolverun` (guard `IF EXISTS`; confirm empty first).

### R1c — Drop the skill-sync delivery layer
**Why:** dormant — never delivers skills to a running agent.
**Scope:** remove `/skills/sync/*` + snapshot-resolve consumer machinery, `skilllocalstate`, `data sync` CLI, `skills_home_dir`, `supports_skill_packs` advertisement. **Decision:** keep `skillbundle` blob storage *iff* domain-pack export/import still needs it (it currently writes bundles on import) — or migrate domain-packs to a simpler blob store and drop `skillbundle` too.

### R1d — Governance rethink (strategic)
**Finding:** the real access gate is the `owners`/`contributors` CSV on the `domain` row; `domainrolemembership` is only a *secondary* check (docs/artifacts/skills/search/explorer). Above it, GovernancePolicy + approvals are stored-but-unenforced, and `is_admin` is dead.
**The fork (operator decision):**
- **(a) Single-operator simplification:** keep auth + owners/contributors; drop GovernancePolicy + approvals scaffolding + the dead `is_admin` routes; keep `domainrolemembership` only where it's the sole check, or fold it into owners/contributors.
- **(b) Real multi-tenant collaboration:** wire governance properly — make `is_admin` reachable, consult policy in the mutation path, gate mutations on approvals. Larger build.
Original intent was (b) "orgs with multiple people's agents collaborating"; operator is revisiting whether that's still a goal.

---

## R2 — `mesh` → `swarm` vocabulary migration
**Why:** "swarm" arguably fits the work/agent primitive better than "mesh."
**Cost (verified):** cross-cutting — 5 tables (`meshtask`/`meshagent`/`meshmessage`/`meshprogressevent`/`meshtaskartifact`) + 5 FK columns + 23 indexes + ~235 Rust + ~49 TS occurrences + **11 MCP tool names that are the agent ABI** (the aria fleet calls `claim_mesh_task` etc. by exact string — external consumers in `/home/merlin/code/aria` must be swept too).
**Nuance:** apply "swarm" to the *work/agent* primitive (`swarmtask`/`swarmagent`); "mesh" stays apt for the *message bus* (`send_mesh_message`). Decide per-layer during migration design.
**Do NOT half-rename** (CLI=swarm while ABI=mesh is worse than either-consistent). This is a deliberate migration, not part of WS-3.

---

## R3 — World-class `init` / onboarding
**Why:** `init` is half-stubbed (4 TODOs in `--repo`; silent-localhost + silent-`Ok`-on-failure footguns; nothing validates daemon/bins/secrets/context).
**Target:** one command that gets you zero→ready and *proves* it — create/validate config + context, auth login, mint/store creds, then run a comprehensive `doctor` (grown to cover auth-validity, context reachable, daemon up, agent bins on PATH, secrets resolvable) and print an explicit go/no-go. Fix the silent footguns (real exit codes).

---

## R4 — Discourage MCP (reinforce ADR 0005/0006)
**Finding:** already mostly won — ADR 0005/0006 made MCP minimal (runtime-only) and CLI-first; `exec` subprocesses the CLI; management is CLI-only. The gap is build-time-only enforcement.
**Target:** (1) nudge in the `discover`/`exec` meta-tool descriptions ("use `exec` for management; the dedicated tools are only for the in-flight work loop"); (2) a CI guard that fails if the advertised MCP tool count grows without an accompanying ADR note (operationalizes ADR 0005 rule #3).

---

## R5 — Wire the isolation sandbox
**Why:** `edgeplaned-sandbox` is fully built (userns + mount-ns/pivot_root + Landlock + seccomp + cap-drop + rlimits + egress allowlists) but not wired into any spawn path — so agents run as the bare operator today. Closing this is the one place the "k8s-style" framing currently over-claims.

---

## R6 — Declarative / GitOps control plane (the bigger bet)
**Why:** the backend is imperative RPC today, which is why noun-first fits and verb-first (kubectl-style) doesn't. If EdgePlane wants fleet/domain/mission config as desired-state manifests reconciled by controllers (aligning with the homelab's ArgoCD muscle), build a uniform resource API + `apply`/reconcile — and *then* the CLI flips to verb-first as a consequence. Own RFC; don't invert the order.
