# ADR 0004: Browser Attach Transport — Tailscale Egress (not reverse-dial)

## Status
Accepted (2026-06-05). Implementation pending.

## Context
Browser → node-agent attach flows through `GET /runtime/nodes/{node_id}/agents/{agent_id}/attach`
on edgeplane-tower (in-cluster). The handler upgrades the browser WebSocket and then dials the
node's attach server (`ws://{tailscale_fqdn}:8009/attach/...`) to proxy ACP frames — see
`docs/plans/edgeplaned-persistent-session-architecture.md`, Gap 2 / "Remote ACP relay".

Three breaks blocked this end-to-end. Two are now fixed; this ADR decides the third.

1. **Carrier (node→tower notify WS) was down in prod — FIXED.** The deployed `edgeplaned` was a
   stale binary built before the `/api` ws_url fix (`ae0eee3`), so it dialed the bare
   `/runtime/nodes/{id}/notify` path (the route only exists under `/api` → 404) and ran federation
   on the 60s poll fallback, never self-healing `attach_secret`. Fixed 2026-06-05 by rebuilding and
   redeploying the daemon from `origin/main` (the fix was already merged; only the binary was stale).

2. **Browser attach was un-authenticatable and un-scoped — FIXED (PR #14).** `agent_attach_proxy`
   read the `Authorization` header only, which a browser cannot set on a WebSocket (it sends the
   same-origin `ep_session_token` cookie, which the handler ignored) → real browser attach got `401`
   before the dial. It also never scoped the caller to the node owner. Both fixed by switching to the
   `Principal` extractor (cookie- and Bearer-capable) + `require_node_owner`, mirroring `node_notify_ws`.

3. **Tower → node dial fails from the cluster — THIS DECISION.** Cluster pods have tailscale
   *ingress* to the tower but no *egress* to the tailnet, so the tower's dial to the node's tailnet
   address times out. (The notify WS works only because it is the reverse direction.)

Two options were evaluated for break #3:

- **Reverse-dial.** The node dials *back* to the tower over the (now proven) notify channel; the
  tower pairs the inbound node socket with the held browser socket. Edge-native — works for NAT'd /
  off-tailnet nodes. A 5-agent adversarial red-team found it requires substantial hardening: a
  reliable (non-broadcast) signal channel, RAII rendezvous cleanup + TTL, per-node concurrency caps
  and per-principal rate limits (a tower push makes the node open unbounded outbound dials → node
  DoS), an abort-loser proxy fix, and real pump-stream generalization. It also imposes a
  **single-replica-tower hard constraint** (the pending-attach rendezvous is in-memory).
- **Tailscale egress.** Give the cluster tailnet routing so the tower's *existing* dial works. No
  reverse-dial machinery, no single-replica constraint, sidesteps the entire red-team surface.
  Requires cluster egress infra and that every attachable node is on the tailnet.

## Decision
Use **Tailscale egress from the cluster**. Every node in the fleet is on the tailnet today
(excalibur `100.99.148.117`), so egress is sufficient and dramatically cheaper than hardened
reverse-dial. The tower keeps its existing `ws://{node}:8009` dial.

Reverse-dial is the documented **future** path for when off-tailnet / NAT'd nodes exist; it is
explicitly NOT built now. The full red-team analysis is preserved in the project memory note.

Implementation (pending):
1. Inspect the cluster's tailscale operator config (the `ts-edgeplane-tower` ingress proxy confirms
   the operator is installed).
2. Pick the mechanism:
   - **Subnet router** advertising excalibur's tailnet route → cluster pods route to `100.x:8009`
     directly → **zero tower code change**; or
   - **Operator egress Service** (per-node ExternalName with `tailscale.com/tailnet-fqdn`) → tower
     dials the egress Service DNS → small tower wiring.
3. Add the manifest to gitops → argocd. Ship together with PR #14 so the single-replica tower takes
   one attach blip, not two.

## Consequences
- **Pros:** minimal/zero code; no single-replica constraint; avoids the reverse-dial
  concurrency/DoS surface entirely; works for the current fleet immediately.
- **Cons:** adds a cluster→tailnet routing dependency; every attachable node must be on the tailnet.
  When a node that cannot join the tailnet appears, revisit reverse-dial.
- The auth/authz fix (PR #14) is independent of transport and lands regardless.
