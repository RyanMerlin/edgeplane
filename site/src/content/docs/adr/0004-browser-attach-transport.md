---
title: "ADR-0004: Browser Attach Transport — Tailscale Egress"
description: Decision to use Tailscale cluster egress for browser-to-node ACP attach transport, deferring reverse-dial to when off-tailnet nodes exist.
---

**Status:** Accepted (2026-06-05)

## Context

Browser → node-agent attach flows through `GET /runtime/nodes/{node_id}/agents/{agent_id}/attach`
on edgeplane-tower (in-cluster). The handler upgrades the browser WebSocket and then dials the
node's attach server (`ws://{tailscale_fqdn}:8009/attach/...`) to proxy ACP frames — see
`docs/plans/edgeplaned-persistent-session-architecture.md`, Gap 2 / "Remote ACP relay".

Three breaks blocked end-to-end attach. Two were fixed before this ADR; the third is the decision
recorded here.

### Break 1 — Carrier (node→tower notify WS): FIXED

The deployed `edgeplaned` was a stale binary built before the `/api` ws_url fix (`ae0eee3`), so it
dialed the bare `/runtime/nodes/{id}/notify` path (the route only exists under `/api` → 404) and
ran federation on the 60-second poll fallback, never self-healing `attach_secret`. Fixed
2026-06-05 by rebuilding and redeploying the daemon from `origin/main` (the fix was already
merged; only the binary was stale).

### Break 2 — Browser attach auth and scope: FIXED (PR #14)

`agent_attach_proxy` read the `Authorization` header only, which a browser cannot set on a
WebSocket (the browser sends the same-origin `ep_session_token` cookie, which the handler
ignored) — real browser attach got `401` before the dial. The handler also never scoped the
caller to the node owner. Both were fixed by switching to the `Principal` extractor (cookie- and
Bearer-capable) plus `require_node_owner`, mirroring `node_notify_ws`.

### Break 3 — Tower → node dial fails from the cluster: THIS DECISION

Cluster pods have Tailscale *ingress* to the tower but no *egress* to the tailnet, so the
tower's dial to the node's tailnet address times out. (The notify WS works only because it is the
reverse direction — the node dials *in*.)

## Options Evaluated

**Reverse-dial.** The node dials back to the tower over the notify channel; the tower pairs the
inbound node socket with the held browser socket. Edge-native — works for NAT'd or off-tailnet
nodes. A 5-agent adversarial red-team identified substantial hardening requirements:

- A reliable (non-broadcast) signal channel to request the dial
- RAII rendezvous cleanup with TTL to avoid leaked goroutines
- Per-node concurrency caps and per-principal rate limits (a tower push causes the node to open
  unbounded outbound dials → node DoS vector)
- Abort-loser proxy fix when either side disconnects mid-stream
- Generalizing the pump-stream abstraction

Reverse-dial also imposes a **single-replica-tower hard constraint**: the pending-attach
rendezvous is in-memory and cannot be shared across replicas without a distributed coordination
layer.

**Tailscale egress.** Give the cluster tailnet routing so the tower's existing dial to
`ws://{node}:8009` succeeds. No reverse-dial machinery, no single-replica constraint, sidesteps
the entire red-team surface. Requires cluster egress infra and that every attachable node is on
the tailnet.

## Decision

Use **Tailscale egress from the cluster**. Every node in the current fleet is on the tailnet, so
egress is sufficient and dramatically cheaper than hardened reverse-dial. The tower keeps its
existing `ws://{node}:8009` dial with no code changes.

Reverse-dial is the documented **future** path for when off-tailnet or NAT'd nodes exist; it is
explicitly not built now.

### Implementation

Two mechanisms are available once the Tailscale operator is confirmed present in the cluster:

1. **Subnet router** — advertise the node's tailnet route from a router pod; cluster pods route
   to `<node-tailscale-ip>:8009` directly. Zero tower code change.
2. **Operator egress Service** — per-node `ExternalName` Service with
   `tailscale.com/tailnet-fqdn` annotation; tower dials the egress Service DNS name. Small tower
   wiring, more explicit per-node config.

The egress manifest is added to gitops and deployed via ArgoCD. Ship together with PR #14 so the
single-replica tower takes one attach blip, not two.

## Consequences

**Pros:**
- Minimal or zero code change to tower
- No single-replica constraint introduced
- Avoids the reverse-dial concurrency and DoS surface entirely
- Works immediately for any fleet where all nodes are on the tailnet

**Cons:**
- Adds a cluster→tailnet routing dependency
- Every attachable node must be on the tailnet; a node that cannot join the tailnet cannot be
  attached to via this path

**Future:** When a node that cannot join the tailnet appears, revisit reverse-dial using the
full red-team analysis preserved in the project design notes.

The auth/authz fix (PR #14) is independent of transport and lands regardless of which transport
path is chosen.
