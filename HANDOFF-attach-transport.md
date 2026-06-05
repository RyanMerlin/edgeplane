# HANDOFF — EdgePlane browser-attach transport (TEMP, delete after pickup)

**Date:** 2026-06-05 · **Repo:** `/home/merlin/code/edgeplane` · **Node:** excalibur (10.0.0.5)

This seeds a fresh session to **finish browser→node-agent attach**. Read
`docs/adr/0004-attach-transport-tailscale-egress.md` and the project memory
`project_attach_transport_reverse_dial` first.

## TL;DR
Making browser attach work had three breaks. Two are fixed. One remains and is decided but not built.

| # | Break | State |
|---|-------|-------|
| 1 | node→tower **notify carrier** down (stale daemon dialed bare `/notify` → 404 → 60s poll) | ✅ FIXED — daemon rebuilt/redeployed from `origin/main` |
| 2 | browser attach **un-authenticatable** (header-only vs cookie) + **not owner-scoped** | ✅ FIXED in **PR #14** (`Principal` + `require_node_owner`) |
| 3 | tower→node **dial fails from cluster** (no tailnet egress) | ⏳ DECIDED (ADR 0004 = Tailscale egress), **not built** |

## The remaining task — implement Tailscale egress (ADR 0004)
Goal: let the in-cluster tower reach a node's `:8009` attach server, so its existing
`ws://{node}:8009/attach/...` dial works. **Do NOT build reverse-dial** (red-teamed, deferred).

First step is **read-only** — inspect the cluster's tailscale operator, then pick:
- **Subnet router** advertising excalibur's tailnet route → cluster pods route to `100.x:8009`
  directly → **zero tower code change** (preferred if simple); OR
- **Operator egress Service** (per-node ExternalName w/ `tailscale.com/tailnet-fqdn`) → tower dials
  the egress Service DNS → small tower wiring.

Then gitops manifest → argocd. The `ts-edgeplane-tower` ingress proxy confirms the operator is installed.
Follow the k8s/ArgoCD pre-flight; mutate the cluster only with Merlin's approval.

## Deploy sequencing
- PR #14 (auth) is **not yet deployed**. It alone does NOT make attach functional (the dial still
  fails) — it just makes browser attach *authenticate* and turns the silent dial failure into a `warn`.
- Tower is **single-replica** → each rollout = a brief attach blip. **Ship PR #14 together with the
  egress change** (one blip, not two): tower image → GHCR (`ghcr.io/ryanmerlin/edgeplane`) → helm/argocd.

## How to VERIFY (avoid the false-signals that burned 3 checks)
- `strings edgeplaned | grep /api/runtime/nodes` is **always 0** (the `/api` prefix is runtime-
  concatenated, never a literal) — useless.
- `state.json` `attach_secret` stays **empty by design** (held in-memory, re-fetched each start).
- WS connect logs at **debug** (unit has no RUST_LOG) — absence proves nothing.
- **Real signals:** daemon log `Fetched attach_secret from controlplane ...; browser attach enabled`
  + persistent ESTABLISHED keepalive sockets from daemon (`100.99.148.117`) → tower (`100.104.126.15:8008`)
  that survive minutes. For egress: from the tower pod, `:8009` on the node becomes reachable
  (today it times out on the tailnet IP, connects on the LAN IP).

## Gotchas (cost real cycles this session)
- **Branch divergence:** the shared working tree is on `feat/zellij-zrpc-plugin` (branched at
  `a814bc1`, BEFORE the attach handler was rewritten). Its `agent_attach_proxy` is STALE (`?ep_token=`
  query auth). The DEPLOYED code is `origin/main` (`7ff60cc`). **Always check `origin/main`, not local
  `main` (stale/diverged) and not the shared tree.** PR #14 was built in an isolated worktree.
- `ep_token` and `ep_ws_token` do **not** exist in `origin/main` — browser attach auth is the
  `ep_session_token` **cookie** via the `Principal` extractor.
- Tower test harness uses a **lazy, un-seeded** Postgres pool — only non-DB / auth-rejection paths are
  unit-testable. DB-backed authZ is verified live + by mirroring audited siblings.

## Artifacts / pointers
- **PR #14:** https://github.com/RyanMerlin/edgeplane/pull/14 (branch `fix/attach-owner-scope`)
- **Isolated worktree:** `~/.cache/ep-carrier-build` (on `fix/attach-owner-scope`; remove after merge)
- **Daemon rollback binary:** `~/.cargo/bin/edgeplaned.bak-pre-carrier-20260605`
- **ADR:** `docs/adr/0004-attach-transport-tailscale-egress.md`
- **Attach architecture:** `docs/plans/edgeplaned-persistent-session-architecture.md` (Gap 2)
- **Memory:** `project_attach_transport_reverse_dial` (full red-team + carrier detail)
