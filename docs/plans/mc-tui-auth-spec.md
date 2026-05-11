# mc TUI — In-App Authentication

**Date:** 2026-05-11
**Status:** Design — pending review
**Owner:** mc-engineer
**Context:** Today the TUI silently shows empty panels when a session expires (only anonymous data is returned). The auth panel just instructs the user to exit and run `mc auth login` from a shell. This is a critical gap — auth is the front door, and the front door currently has no handle.

---

## Goals

1. **Zero shell-out for auth.** Login, refresh, switch identity, and inspect status all happen inside the TUI.
2. **Fail loudly, recover gracefully.** Expired or missing auth surfaces immediately and offers a one-keystroke recovery — never a silently empty panel.
3. **Long-lived by default.** Operators authenticate once and stay logged in for the practical lifetime of a workstation (30+ days), with proactive refresh before expiry.
4. **Service token escape hatch.** For headless nodes, scripts, and long-running daemons, a true non-expiring or year-scale token must be available without per-call OIDC.
5. **Polish.** First-time login should feel guided. Returning logins should feel invisible.

---

## What exists today (audit)

- Session tokens (`mcs_*`) issued by `POST /auth/sessions`, stored at `~/.missioncontrol/session.json` (chmod 600), keyed per controlplane context.
- Two login methods in `mc auth login`:
  - **OIDC/SSO** — browser flow, polls `/auth/oidc/cli-poll/{nonce}` for up to 60s, falls back to paste-the-code.
  - **API token** — paste a long-lived bearer, exchanged for a session token.
- Session TTL: default 8h, max 720h (30 days). Hard-capped server-side at `args.ttl_hours.clamp(1, 720)`.
- `load_saved_session()` validates expiry + URL match before returning the token.
- `MC_TOKEN` env var bypasses session storage entirely — closest thing to a "service token" today, but its lifetime is whoever-issued-it's choice.
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
│   Some panels (private missions, klusters, agents)     │
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
┌─ Sign in to MissionControl ────────────────────────────┐
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
… │ admin@mc-local │ 28d left │
```

Coloring:
- Green: >7 days remaining
- Yellow: 1-7 days remaining (auto-refresh attempted in background — see below)
- Red: <24h remaining or expired (will trigger interception on next API call)
- Dim grey "anonymous" if no session

Click target / `F2` opens the **Identity** modal:

```
┌─ Identity ─────────────────────────────────────────────┐
│  Signed in as     admin@mc-local                       │
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
| **Session** | OIDC or API-token exchange via `POST /auth/sessions` | ≤720h (30d) | `~/.missioncontrol/sessions/<context>.json` | Interactive TUI/CLI |
| **API token** | Server admin issues out-of-band; passed by user to exchange for a session | Caller's choice (existing) | Not persisted by `mc` — caller's responsibility | Bootstrap, scripts |
| **Service token** *(new)* | `POST /auth/service-tokens` (admin-only) | Year-scale or non-expiring; scoped capabilities | `~/.missioncontrol/service-token.json` (chmod 600), separate from session.json | Headless agents, mc-mesh daemons, CI |

**Why a new class:** The current model conflates two needs. A 30-day session is fine for a workstation but wrong for an mc-mesh node that runs for 6 months. Forcing daemons to refresh tokens or rely on plaintext `MC_TOKEN` env vars produces leak-prone configs and 3am pager alerts.

**Server changes for service tokens:**
- `POST /auth/service-tokens` — admin-gated. Body: `{ name, scopes, ttl_days? }`. Returns the token once; never again.
- `GET /auth/service-tokens` — list (admin only). Shows name, scopes, created_at, last_used_at, expires_at, **not** the token value.
- `DELETE /auth/service-tokens/{id}` — revoke.
- `/auth/me` recognises a service-token bearer just like a session token; principal resolution unchanged.

**TUI surface:**
- Identity modal → Admin sub-screen (visible only when `principal.is_admin`) lists service tokens with revoke buttons.
- Login modal → "Service token" choice paste-and-store. Stored at the separate path so it isn't trampled by `mc auth logout`.

**Boundary:** The Service Token path replaces — does not augment — the use of bare `MC_TOKEN` env vars in long-running daemons. The env var path stays for one-shot scripts and bootstrap.

---

## `MC_TOKEN` — bootstrap-only escape hatch

`MC_TOKEN` as a general-purpose auth mechanism is a liability: plaintext in env, leaks through process listings, no scope or last-used tracking, indistinguishable from a fresh-issued session to the server. With service tokens introduced above, it has no remaining legitimate steady-state use. But it cannot be removed — the first admin on a fresh controlplane needs *something* before OIDC works and before any service token exists.

**Policy: bootstrap-only, hidden from normal usage.**

| Path | `MC_TOKEN` accepted? | Behavior |
|------|--------------------|----------|
| `mc auth login --non-interactive` | Yes | Exchanges env token for a session token. Existing behavior. |
| `mc auth bootstrap-service-token` *(new)* | Yes | Issues the first admin service token using `MC_TOKEN` as the bearer. One-time use; admin marks token as "bootstrap-used" after success. |
| `RemoteDataClient` (TUI + CLI steady-state ops) | **No** | Does not read `MC_TOKEN`. Auth precedence: explicit `--token` flag > service token file > session file > anonymous. |
| Server endpoints (general API) | Yes, with deprecation header | Server still accepts the bearer for backward compat but returns `X-Auth-Deprecation: use service tokens` on every response. TUI surfaces this as a one-time toast per session. |

**Documentation:**
- Login modal does not mention `MC_TOKEN`.
- Admin runbook (`docs/runbooks/AUTH-BOOTSTRAP.md`, new) is the only place it appears in public docs, framed explicitly as bootstrap-only.
- `mc auth whoami` reports the auth source: `session` / `service-token` / `env (deprecated)`. The "deprecated" label is the operator's nudge to migrate.

**Migration:**
- Existing deployments that use `MC_TOKEN` in systemd units, docker-compose files, or shell rc files keep working.
- The deprecation toast surfaces the migration path: run `mc auth bootstrap-service-token` once, then remove `MC_TOKEN` from the env.
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

`~/.missioncontrol/config.json` gains:

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
- No login modal yet — `L` still says "press L to sign in" but currently directs to `mc auth login` in another shell as a stopgap, AND lands the user back in the TUI when the session file appears (filesystem watch).

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
- Documentation: runbook for issuing service tokens to mc-mesh nodes.

**Value:** Headless agents stop relying on raw `MC_TOKEN` env. Aligns with the persistent-session architecture for mc-mesh nodes.

Each phase ships independently. Phase 1 is the minimum to stop the silent-empty failure.

---

## Polish details

These are the things that separate "functional" from "delightful":

1. **First-launch context discovery.** If `~/.missioncontrol/contexts.yaml` is empty AND a controlplane is reachable at `http://localhost:8008` (or via `mc discover` mDNS), pre-populate the server field with that. Don't make the user type in their server URL on first run.
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

4. **Service token revocation propagation.** A service token revoked on the server invalidates running daemons immediately. **Proposed:** that's correct behavior, but mc-mesh needs to detect 401 and re-resolve via Infisical (where the token is stored) before crashing. Separate work item for mc-mesh.

5. **TUI auto-launch from `mc tui --login`?** Should `mc tui --login` skip the missions screen and open straight to the login modal? **Proposed:** yes, useful for muscle memory.

6. **Session vs Service token in `~/.missioncontrol/`.** Two files: `session.json` (per context) and `service-token.json` (machine-scoped, not per context). The data layer needs a clear precedence rule. **Proposed:** explicit env var `MC_TOKEN` > service token file > session file. Document this in `mc auth whoami`.

---

## Acceptance criteria

A user with no prior session can:

- [ ] Launch `mc tui`
- [ ] See an explicit "you're not signed in" prompt within 250ms
- [ ] Press `L`, complete OIDC in browser, return to a fully-populated Missions tab in under 30 seconds
- [ ] Close the TUI, reopen days later (within 30 days), see populated panels with no prompt

An admin operator can:

- [ ] Issue a service token from inside the TUI
- [ ] Hand that token to an mc-mesh node config
- [ ] Watch that node go online in the Agents panel within 60 seconds
- [ ] Revoke the token from inside the TUI
- [ ] Watch the node go offline within one heartbeat interval

If any of the above requires shell-outs to `mc auth login`, the spec has failed.
