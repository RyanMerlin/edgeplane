# EdgePlane CLI — Command Tree, From First Principles (WS-3 Design Gate)

**Date:** 2026-06-11 (decisions resolved 2026-06-13)
**Status:** 🟡 DRAFT — CLI surface decisions resolved; deferred/strategic items in the roadmap.
**Source plan:** `docs/superpowers/plans/2026-06-11-edgeplane-cli-cleanup-and-typed-ids.md` (WS-3)
**Roadmap (deferred + strategic):** `docs/superpowers/specs/2026-06-13-edgeplane-platform-roadmap.md`
**Prior art:** `docs/adr/0003-edgeplane-cli-hierarchy-hard-cutover.md`
**Entities (source of truth):** `docs/architecture/entities.md`

---

## 0. The honest finding — count is the wrong target

Real `--help` of the tools we admire:

| Tool | Top-level commands | Why it's loved |
|---|---|---|
| **incus** | **33** | one grammar (noun→verb) + the hot entity's verbs promoted |
| **gh** | ~30 | one grammar (noun→verb) + **sectioned** `--help` (CORE / ACTIONS / ADDITIONAL) |
| **kubectl** | ~40 verbs | one grammar (verb→noun) — viable only on a uniform dynamic API |

Loved despite large counts. The win is **a single grammar + grouped `--help`**, not a small number.

## 1. Grammar = noun-first

`edgeplane <entity> <verb>`, consistent verbs, grouped `--help`, 1–2 promoted hot-path verbs.
EdgePlane has a **fixed entity set** → gh/incus-shaped, not kubectl-shaped. (Verb-first would require a
declarative reconciled control plane — see roadmap R6.) `--help` groups double as the WS-2 colorized render.

## 2. What today's tree got wrong (and we fix)

Mixed grammars at top level (entity nouns + bare verbs + concern-buckets `system`/`data`/`admin`);
no `session`/entity homes for cross-cutting ops; flat ungrouped `--help`. **Plus** (found 2026-06-13)
a lot of built-but-unused scaffolding — see roadmap §0.

---

## 3. RESOLVED TREE

Grammar `edgeplane <noun> <verb>`; verbs `list show create update delete` + entity verbs. 5 `--help` sections.

```
# ── WORK ─────────────────────────────  (entity spine, entities.md)
domain      list show create update delete · attach detach home · roles · governance   # §Domain, §DomainRoleMembership
                                                                                        #   roles = live secondary RBAC (owners/contributors CSV is the PRIMARY gate); governance mostly dormant → R1d
mission     list show create update delete                                              # §Mission
task        list show create update delete                                              # §Task — human/UI work items
meshtask    submit list show watch cancel retry                                         # §MeshTask — agent work; PROMOTED from `daemon task`
                                                                                        #   claim/heartbeat/complete stay agent-protocol (MCP + supervisor). meshtask→swarmtask = R2.
artifact    list show get put                                                           # §Artifact

# ── FLEET ────────────────────────────  (who runs work, and where)
agent       list show register delete set-status · signal cancel attach · supervise · cron · [runs?]   # §Agent/§MeshAgent
                                                                                        #   `evolve` REMOVED (R1b). `runs` = optional agentrun observability after R1a.
fleet       nodes · jobs · leases                          # runtime fabric (renamed from `runtime`); node SELF-mgmt → `daemon`
workspace   load heartbeat commit release
capability  list describe                                  # `exec` is top-level
approval    list decide                                    # NOTE: unenforced today → R1d

# ── PLATFORM ─────────────────────────  (connect · identity · config)
auth        login logout whoami
context     list current use add remove discover
profile     list use add remove
secret      list resolve set ...
config      show                                           # top-level peer (gh/kubectl/incus convention)
channel     ...                                            # `channel claude missioncontrol` rides on aisession → removed with R1a

# ── OPERATE ──────────────────────────  (hot path · daemon · server)
status      # default action: quick local/runtime/auth context
launch      # was `run` — start an agent runtime (claude/codex/gemini/goose/...)
exec        # run a capability (escape hatch, gh-`api` style)
serve       # MCP server — discourage per R4
tui
daemon      # edgeplaned control + THIS node's self-mgmt (register/doctor/join-token, absorbed from `agent node`)
admin       # doctor health backup compat drift logs + global governance policy (mostly dormant → R1d)

# ── META ─────────────────────────────
version · update · init · completion · discover            # init → world-class onboarding (R3); discover = MCP schema contract (ADR 0006)
```

~28 top-level across 5 sections. The number is incidental; the grammar + grouping is the point.

**Removed vs. today:** `data` (→ `sync` dropped R1c, `tools`→admin, `explorer`→`domain tree`); `runtime`→`fleet`;
`run`→`launch`; `ops`→`domain`; `release`→`workspace`; `exec`/`receipts`→top-level/admin; `doctor`/`health`/`version`/`config`
stay peers; `agent evolve`→dropped (R1b); `ai`/AI-chat→dropped (R1a, so no `session` noun).

---

## 4. Resolved decisions (with evidence + confidence)

| # | Decision | Basis | Confidence |
|---|---|---|---|
| Grammar | noun-first | EdgePlane has a fixed entity set (gh/incus-shaped) | HIGH |
| #2 | `run` → **`launch`** | `run` blocks on a foreground runtime subprocess (the incus-`launch` analog) | HIGH (code) |
| #3 | `exec` stays **top-level** | dispatches a capability via edgeplaned routing; `capabilities` = discovery, `exec` = invoke | HIGH (code) |
| #4 | **two nouns**: `task` + `meshtask` (promote from `daemon task`) | distinct tables/lifecycles (task=CRUD, meshtask=claim/lease/DAG); both already in the CLI today | HIGH (schema+routes+CLI) |
| #5 | `config`/`version` **top-level peers** | gh/kubectl/incus convention | HIGH (docs) |
| #6 | maintenance bucket = **`admin`** | incus precedent; absorbs governance-policy too | HIGH |
| #7 | kill `data`: `sync`→drop (R1c), `tools`→admin, `explorer`→`domain tree`; `domain roles` (not `admin`) | data = 3 unrelated things; skill-sync dormant; roles is domain-scoped | HIGH (code) |
| #8 | **no `session` noun** — AI chat dropped (R1a); optional `agent runs` for `agentrun` | aisession cleanly separable; agents supersede it | HIGH (code) |
| #9 | `runtime` → **`fleet`**; node self-mgmt → `daemon` | "fleet" non-colliding (it's the CLI's own tagline); dissolves the node/agent-node clash | HIGH (code) |
| — | drop `agent evolve` | dead + broken (`evolvemission` table missing) | HIGH (code) |

Deferred / strategic (→ roadmap): mesh→swarm rename (R2), AI-chat removal (R1a), evolve removal (R1b),
skill-sync removal (R1c), governance rethink (R1d), world-class init (R3), MCP-discouragement (R4),
sandbox wiring (R5), declarative/GitOps→verb-first (R6).

---

## 5. PER-NOUN VERB TABLE + 6. CALL-SITE SWEEP

> Filled in next. §6 preliminary: blast radius low — `cron.toml` has zero references to any moved command;
> no scripted `edgeplane <cmd>` call-sites in either repo. Renames ship hidden back-compat aliases.

---

## Out of scope (this workstream)

- Everything in the roadmap (R1–R6) — separate slices.
- WS-6 typed-entity-IDs (`d_`/`m_`/`t_`) — separate gate.
- Behavior change inside a leaf — pure tree reshaping + aliases only.
