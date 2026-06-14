# edgeplane TUI — In-App Authentication

**Date:** 2026-05-11
**Status:** Design — pending review
**Owner:** mc-engineer
**Context:** Today the TUI silently shows empty panels when a session expires (only anonymous data is returned). The auth panel just instructs the user to exit and run `edgeplane auth login` from a shell. This is a critical gap — auth is the front door, and the front door currently has no handle.

---

## Goals

1. **Zero shell-out for auth.** Login, refresh, switch identity, and inspect status all happen inside the TUI.
2. **Fail loudly, recover gracefully.** Expired or missing auth surfaces immediately and offers a one-keystroke recovery — never a silently empty panel.
3. **Long-lived by default.** Operators authenticate once and stay logged in for the practical lifetime of a workstation (30+ days), with proactive refresh before expiry.
4. **Service token escape hatch.** For headless nodes, scripts, and long-running daemons, a true non-expiring or year-scale token must be available without per-call OIDC.
5. **Polish.** First-time login should feel guided. Returning logins should feel invisible.

---

## What exists today (audit)

- Session tokens (`mcs_*`) issued by `POST /auth/sessions`, stored at `~/.edgeplane/session.json` (chmod 600), keyed per controlplane context.
- Two login methods in `edgeplane auth login`:
  - **OIDC/SSO** — browser flow, polls `/auth/oidc/cli-poll/{nonce}` for up to 60s, falls back to paste-the-code.
  - **API token** — paste a long-lived bearer, exchanged for a session token.
- Session TTL: default 8h, max 720h (30 days). Hard-capped server-side at `args.ttl_hours.clamp(1, 720)`.
- `load_saved_session()` validates expiry + URL match before returning the token.
- `EP_TOKEN` env var bypasses session storage entirely — closest thing to a "service token" today, but its lifetime is whoever-issued-it's choice.
- TUI builds `RemoteDataClient` once at startup with `Option<String>` token. There is **no** 401 handler — non-2xx responses bubble up as `anyhow::bail!("backend returned 401 ...")` and become an error toast at best, empty panel at worst.
- Config screen auth panel (`tui/screens/config.rs:527-549`) is read-only instruction text.
- F2 binding is labeled "toggle auth mode" but only swaps between token/anonymous on the existing client — it does not authenticate.

---

## UX Flow

### First launch (no session)

TUI opens on the **Missions** tab as usual. The auth state is detected at startup:

- `load_saved_session(base_url)` returns `None` → token = `None` → anonymous mode

Behavior:

```
┌─ Missions ─────────────────────────────────────────────┐
│                                                        │
│   You're not signed in.                                │
│                                                        │
│   Some panels (private domains, missions, agents)     │
│   require authentication.                              │
│                                                        │
│        [  Sign in  ]   or press  L                     │
│                                                        │
│   Continue as anonymous — public data only:  Esc       │
│                                                        │
└────────────────────────────────────────────────────────┘
```

- Empty panels are never silently empty when auth is the cause. The reason is named.
- `L` (login) is the universal shortcut from anywhere in the TUI. It opens the login modal.
- `Esc` dismisses the prompt and lets the user browse anonymously (current behavior).

### Login modal

A single modal, three steps, keyboard-driven:

```
┌─ Sign in to Edgeplane ────────────────────────────┐
│                                                        │
│   Server  http://localhost:8008                ▾ change│
│                                                        │
│   Method  ● OIDC / SSO  (recommended)                  │
│           ○ API token                                  │
│           ○ Service token   (long-lived, headless)     │
│                                                        │
│   Session  ▾ 30 days  (max)                            │
│                                                        │
│           [  Continue  ]                          Enter│
│                                                        │
│   ←/→ change field   ↑/↓ navigate   Esc cancel         │
│                                                        │
└────────────────────────────────────────────────────────┘
```

**OIDC / SSO** (default):
1. Modal becomes a status panel:
   ```
   Opening browser…  Waiting for completion.
   If the browser doesn't open: <copy URL>
   ```
2. Poll `/auth/oidc/cli-poll/{nonce}` every 2s (same as CLI flow).
3. On success: spinner becomes a green checkmark, session details flash, modal auto-dismisses after 1s. Data refetches.
4. On 60s timeout: modal switches to "Paste code from browser" with a text input. Same fallback the CLI uses.

**API token**:
1. Masked input field (`*`-rendered). Tab to confirm, Enter to submit.
2. Posts to `/auth/sessions` with the chosen TTL.

**Service token** (long-lived):
1. Same masked input UI.
2. Stored differently — see "Long-lived tokens" below.

### Already signed in — silent

If `load_saved_session()` returns a valid session, the TUI starts with that token. No prompt. Status bar shows identity.

### Status bar identity badge

Bottom-right of the status bar shows current identity at all times:

```
… │ admin@edgeplane-local │ 28d left │
```

Coloring:
- Green: >7 days remaining
- Yellow: 1-7 days remaining (auto-refresh attempted in background — see below)
- Red: <24h remaining or expired (will trigger interception on next API call)
- Dim grey "anonymous" if no session

Click target / `F2` opens the **Identity** modal:

```
┌─ Identity ─────────────────────────────────────────────┐
│  Signed in as     admin@edgeplane-local                       │
│  Email            -                                    │
│  Token expires    2026-06-10T10:02:13Z   (29d 23h)     │
│  Session ID       4837                                 │
│  Server           http://localhost:8008                │
│  Context          local                                │
│                                                        │
│  [ Refresh now ]   [ Switch context ]   [ Sign out ]   │
│                                                        │
└────────────────────────────────────────────────────────┘
```

`F2` replaces the current "toggle auth mode" binding (which is more confusing than useful). The old toggle moves to the Identity modal as an explicit checkbox.

### Mid-session expiry / 401 interception

If any backend call returns 401:

1. `RemoteDataClient` wraps the response — surfaces a typed `AuthError::SessionExpired { path }` instead of generic `bail!`.
2. The app dispatches an `AuthExpired` event to the central event loop.
3. The current operation pauses (in-flight queries cancelled or marked stale).
4. The login modal opens automatically, pre-filled with the last used method.
5. On success, the cancelled operations re-run automatically. The user sees a brief "Re-authenticated — reloading" toast.

The user does **not** lose their place in the TUI. They land back where they were.

---

## Long-lived tokens

Three distinct token classes, each with a clear purpose:

| Class | Issued by | TTL | Stored at | Use case |
|-------|-----------|-----|-----------|----------|
| **Session** | OIDC or API-token exchange via `POST /auth/sessions` | ≤720h (30d) | `~/.edgeplane/sessions/<context>.json` | Interactive TUI/CLI |
| **API token** | Server admin issues out-of-band; passed by user to exchange for a session | Caller's choice (existing) | Not persisted by `edgeplane` — caller's responsibility | Bootstrap, scripts |
| **Service token** *(new)* | `POST /auth/service-tokens` (admin-only) | Year-scale or non-expiring; scoped capabilities | `~/.edgeplane/service-token.json` (chmod 600), separate from session.json | Headless agents, edgeplaned daemons, CI |

**Why a new class:** The current model conflates two needs. A 30-day session is fine for a workstation but wrong for an edgeplaned node that runs for 6 months. Forcing daemons to refresh tokens or rely on plaintext `EP_TOKEN` env vars produces leak-prone configs and 3am pager alerts.

**Server changes for service tokens:**
- `POST /auth/service-tokens` — admin-gated. Body: `{ name, scopes, ttl_days? }`. Returns the token once; never again.
- `GET /auth/service-tokens` — list (admin only). Shows name, scopes, created_at, last_used_at, expires_at, **not** the token value.
- `DELETE /auth/service-tokens/{id}` — revoke.
- `/auth/me` recognises a service-token bearer just like a session token; principal resolution unchanged.

**TUI surface:**
- Identity modal → Admin sub-screen (visible only when `principal.is_admin`) lists service tokens with revoke buttons.
- Login modal → "Service token" choice paste-and-store. Stored at the separate path so it isn't trampled by `edgeplane auth logout`.

**Boundary:** The Service Token path replaces — does not augment — the use of bare `EP_TOKEN` env vars in long-running daemons. The env var path stays for one-shot scripts and bootstrap.

---

## Phase 1.5 — remove `anonymous` as a principal (server-side)

Phase 1 surfaced unauthenticated state in the TUI but left a subtle hole: the controlplane still synthesises an `anonymous` principal for callers without valid credentials and lets them through to private endpoints. The endpoints filter by `owners = principal.subject`, so anonymous gets back `[]` instead of 401 — the same silent-empty failure mode Phase 1 worked around on the client.

Phase 1.5 closes the loop on the server.

**Policy:**
- The `Principal` extractor returns `AuthRejection::Unauthenticated` (renders as 401) when no valid credential is presented.
- Every route that extracts `Principal` directly is now auth-required by construction.
- Routes that legitimately accept unauthenticated callers — health, version, OIDC bootstrap (`cli-initiate`, `cli-poll`, `exchange`), the OIDC web callback — do not extract `Principal` at all. The set is short and audited.
- The `"anonymous"` string is no longer a valid `auth_type` value. `auth_type` is one of `static` | `session` | `service_account`.

**Consequences for clients:**
- The TUI's 401 → SessionExpired translation (Phase 1) is now the *primary* signal, not a guess based on empty response inference. Auth state is the truth, not an inference.
- Hooks (Claude Code session-start, tool-audit) without valid auth return 401. Operators must configure hooks with a session or service-account token. The reserved-name guard from agents Phase 1 stays as defence-in-depth.
- The `if principal.auth_type == "anonymous"` branches in `routes/auth.rs`, `routes/ai.rs`, and `routes/missions.rs` become dead code and are removed. `ai.rs`'s `require_auth` helper goes with them.
- Tests that asserted the old behavior (`/mcp/call` returns 200 for anon) are updated to assert 401. Tests that already asserted `!= 200` continue to pass.

**Why this fits between Phase 1 and Phase 2:** Phase 2 (in-TUI login modal) needs a clean signal — "you need to sign in" — to trigger on. Phase 1's 401 translation works around the silent-empty issue at the client. Phase 1.5 makes that 401 the truth. Phase 2 then has a single, reliable trigger to open the login modal automatically.

**Scope of the change:**
- `auth.rs`: extractor returns rejection on no-auth; no synthetic anonymous
- `routes/auth.rs`, `routes/ai.rs`, `routes/missions.rs`: dead `if anonymous` branches removed
- `routes/agents.rs`: every handler now extracts `Principal` (was 0/16). This closes the related hole that all of `/agents/*` was wide-open — the synthetic anonymous removal alone wouldn't have helped because those handlers don't ask for a Principal at all.
- `tests/test_routes.rs`: `test_mcp_call_unknown_tool` → `test_mcp_call_requires_auth`
- No schema changes, no migrations

**Follow-up — Phase 1.6 (broader auth-coverage audit):**

An audit during Phase 1.5 surfaced a wider class of unauthenticated handlers across many route files (ingestion, slack_integrations, mcp partially, runtime partially, work partially, etc.). Each needs individual judgement — some are legitimately public (webhook receivers that verify their own signatures), others are gaps. Quick survey:

| File | Auth-extracted / Total handlers |
|------|---------------------------------|
| ingestion.rs | 0 / 6 |
| mcp.rs | 1 / 6 (the `/mcp/call` body — health and tools list are intentionally public) |
| slack_integrations.rs | 3 / 8 |
| skills.rs | 7 / 11 |
| tasks.rs | 6 / 8 |
| hooks.rs | 6 / 10 |
| ops.rs | 5 / 8 |
| search.rs | 3 / 5 |
| explorer.rs | 2 / 3 |
| persistence.rs | 10 / 12 |
| runs.rs | 10 / 11 |
| runtime.rs | 28 / 40 |
| work.rs | 30 / 38 |
| remotectl.rs | 11 / 12 |
| docs.rs | 6 / 8 |
| artifacts.rs | 8 / 10 |
| approvals.rs | 4 / 7 |
| governance.rs | 9 / 10 |
| oidc_web.rs | 0 / 12 (intentional — OIDC bootstrap) |
| health.rs | 0 / 1 (intentional) |
| webhooks_tailscale.rs | 0 / 1 (signature-verified differently) |

Phase 1.6 walks each unauthenticated handler, decides "auth required" / "publicly OK with signature verification" / "publicly OK by design," and adds Principal extraction or documents the exception. Out of scope for Phase 1.5 — that scope was contained to "kill synthetic anonymous + close the agents.rs gap that the operator already saw."

---

## `EP_TOKEN` — bootstrap-only escape hatch

`EP_TOKEN` as a general-purpose auth mechanism is a liability: plaintext in env, leaks through process listings, no scope or last-used tracking, indistinguishable from a fresh-issued session to the server. With service tokens introduced above, it has no remaining legitimate steady-state use. But it cannot be removed — the first admin on a fresh controlplane needs *something* before OIDC works and before any service token exists.

**Policy: bootstrap-only, hidden from normal usage.**

| Path | `EP_TOKEN` accepted? | Behavior |
|------|--------------------|----------|
| `edgeplane auth login --non-interactive` | Yes | Exchanges env token for a session token. Existing behavior. |
| `edgeplane auth bootstrap-service-token` *(new)* | Yes | Issues the first admin service token using `EP_TOKEN` as the bearer. One-time use; admin marks token as "bootstrap-used" after success. |
| `RemoteDataClient` (TUI + CLI steady-state ops) | **No** | Does not read `EP_TOKEN`. Auth precedence: explicit `--token` flag > service token file > session file > anonymous. |
| Server endpoints (general API) | Yes, with deprecation header | Server still accepts the bearer for backward compat but returns `X-Auth-Deprecation: use service tokens` on every response. TUI surfaces this as a one-time toast per session. |

**Documentation:**
- Login modal does not mention `EP_TOKEN`.
- Admin runbook (`docs/runbooks/AUTH-BOOTSTRAP.md`, new) is the only place it appears in public docs, framed explicitly as bootstrap-only.
- `edgeplane auth whoami` reports the auth source: `session` / `service-token` / `env (deprecated)`. The "deprecated" label is the operator's nudge to migrate.

**Migration:**
- Existing deployments that use `EP_TOKEN` in systemd units, docker-compose files, or shell rc files keep working.
- The deprecation toast surfaces the migration path: run `edgeplane auth bootstrap-service-token` once, then remove `EP_TOKEN` from the env.
- No timeline for removal in this spec. Revisit after 90 days of telemetry on the deprecation header.

---

## Session refresh

Sessions should renew silently long before they expire — operators should never see a "your session is about to expire" toast unless something is wrong.

### Strategy: refresh-on-use with a halfway threshold

- When a session is loaded, compute `halfway = created_at + (expires_at - created_at) / 2`. (Server returns `created_at` in the session response — small addition.)
- On every successful API call made past `halfway`, the controlplane response includes a `X-Session-Refresh-After: <rfc3339>` header when the token is being approached.
- The TUI background task — already polling for fleet state — calls `POST /auth/sessions/refresh` once when:
  - It's past `halfway`, AND
  - It hasn't refreshed in the last 60 minutes (debounce).
- The refresh endpoint issues a new token with full TTL, returns it, and revokes the old token after a 60s grace window (to absorb concurrent callers).

### What about long absences?

If the TUI is closed and reopened past expiry, `load_saved_session()` returns `None` — same path as first launch. The user sees the sign-in prompt and one-keystroke recovery.

### Configurability

`~/.edgeplane/config.json` gains:

```json
"auth": {
  "default_ttl_hours": 720,
  "refresh_enabled": true,
  "refresh_threshold_fraction": 0.5
}
```

Most users never edit this. Power users can disable refresh (e.g. for short-lived audit accounts).

---

## Implementation plan

### Phase 1 — Detect & surface (no server changes)

- Add `AuthState` enum to TUI app state: `Anonymous | SessionValid(SavedSession) | SessionExpired | SignInRequired`.
- Wire startup: call `load_saved_session()`, set state.
- Status bar identity badge (read-only).
- 401 interception in `RemoteDataClient` — typed `AuthError`.
- Empty-panel "Sign in" prompt in each tab when `AuthState == Anonymous` and the panel requires auth.
- No login modal yet — `L` still says "press L to sign in" but currently directs to `edgeplane auth login` in another shell as a stopgap, AND lands the user back in the TUI when the session file appears (filesystem watch).

**Value:** Stops the silent failure today.

### Phase 2 — Login modal (TUI does it itself)

- Modal widget extending existing `widgets::modal`.
- OIDC sub-flow: same calls `auth.rs` makes from the CLI, but driven by a polling task posted to the app's event loop.
- API token sub-flow: masked input + `POST /auth/sessions`.
- On success: write session via existing `save_session()`, swap the `RemoteDataClient` token, refetch data.
- `L` keybind from anywhere; opens modal regardless of current tab.

**Value:** Removes the shell-out. The TUI is self-contained.

### Phase 3 — Session refresh

- Server: add `POST /auth/sessions/refresh` endpoint.
- Server: add `created_at` to session-issue response and `/auth/me`.
- TUI: background refresh loop in the existing polling task.

**Value:** "Logged in once, forever" experience for daily-driver workstations.

### Phase 4 — Service tokens

- Server: `POST /auth/service-tokens`, `GET`, `DELETE`.
- TUI: third choice in login modal, plus admin-only management sub-screen in Identity modal.
- Documentation: runbook for issuing service tokens to edgeplaned nodes.

**Value:** Headless agents stop relying on raw `EP_TOKEN` env. Aligns with the persistent-session architecture for edgeplaned nodes.

Each phase ships independently. Phase 1 is the minimum to stop the silent-empty failure.

---

## Polish details

These are the things that separate "functional" from "delightful":

1. **First-launch context discovery.** If `~/.edgeplane/contexts.yaml` is empty AND a controlplane is reachable at `http://localhost:8008` (or via `edgeplane discover` mDNS), pre-populate the server field with that. Don't make the user type in their server URL on first run.
2. **OIDC browser fallback.** If `open::that()` fails (SSH session, no DISPLAY), show the URL as a copyable line with `c` to copy to system clipboard. The CLI today prints the URL but doesn't offer copy — the TUI can.
3. **Masked token input + paste-from-clipboard.** Token entry should accept paste cleanly (multi-line tokens with whitespace are stripped). Show length + last-4 chars on confirm: `mcs_…a83f (62 chars)` so the user can verify they pasted the right thing.
4. **Identity badge animation.** When the background refresh fires, briefly pulse the badge from cyan to green and back. The user sees that "something just worked" without a modal interruption.
5. **Error specificity.** "Token rejected" is not enough. Distinguish: `401 invalid token`, `403 token valid but lacks scope`, `connection refused`, `TLS error`, `wrong server URL (got 404 on /auth/sessions)`. Each maps to a different next action.
6. **Sign-out confirmation.** Sign-out is destructive (loses the session). The confirmation prompt should mention "you'll need to sign in again on next launch" — not "are you sure?".
7. **No flashing of expired sessions.** If the session is expired at startup, the TUI should never render the Missions panel with the expired token — it should go straight to sign-in. The current race (where 401s come back after panels start populating) is a polish bug.
8. **Keyboard-only.** Every action must be reachable without a mouse. The mock prompts above use `L`, `F2`, `Esc`, `Enter`, `Tab`, arrow keys — no clicks required.

---

## Edge cases & open questions

1. **Multi-context session swap.** If the user switches context (different controlplane) in the Identity modal, the session file changes (per-context path). Should the TUI auto-prompt for sign-in if the new context has no session? **Proposed:** yes, same empty-state UX as first launch.

2. **Clock skew.** `load_saved_session()` uses `chrono::Utc::now()` to compare expiry. If the user's clock is wrong (containers, VMs), sessions appear expired immediately. **Proposed:** when 401 occurs but local check says "valid", surface a "clock skew?" hint. Low-effort, high-clarity.

3. **Refresh during 401.** Concurrent: 401 arrives while a refresh is in-flight. **Proposed:** refresh resolves first → retry the original request with the new token before falling back to the sign-in modal.

4. **Service token revocation propagation.** A service token revoked on the server invalidates running daemons immediately. **Proposed:** that's correct behavior, but edgeplaned needs to detect 401 and re-resolve via Infisical (where the token is stored) before crashing. Separate work item for edgeplaned.

5. **TUI auto-launch from `edgeplane tui --login`?** Should `edgeplane tui --login` skip the missions screen and open straight to the login modal? **Proposed:** yes, useful for muscle memory.

6. **Session vs Service token in `~/.edgeplane/`.** Two files: `session.json` (per context) and `service-token.json` (machine-scoped, not per context). The data layer needs a clear precedence rule. **Proposed:** explicit env var `EP_TOKEN` > service token file > session file. Document this in `edgeplane auth whoami`.

---

## Acceptance criteria

A user with no prior session can:

- [ ] Launch `edgeplane tui`
- [ ] See an explicit "you're not signed in" prompt within 250ms
- [ ] Press `L`, complete OIDC in browser, return to a fully-populated Missions tab in under 30 seconds
- [ ] Close the TUI, reopen days later (within 30 days), see populated panels with no prompt

An admin operator can:

- [ ] Issue a service token from inside the TUI
- [ ] Hand that token to an edgeplaned node config
- [ ] Watch that node go online in the Agents panel within 60 seconds
- [ ] Revoke the token from inside the TUI
- [ ] Watch the node go offline within one heartbeat interval

If any of the above requires shell-outs to `edgeplane auth login`, the spec has failed.
