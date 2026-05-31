# edgeplaned Windows build — design (compile + run, no SCM)

**Status:** Draft for Merlin review · 2026-05-31 · author: Aria (engineer)
**Branch:** `feat/edgeplaned-windows-build`
**Origin:** Merlinlabs flag — "if edgeplaned is ever distributed/installed by others on Windows, the windows-service crate is worth doing properly — behind a `#[cfg(windows)]` feature flag so the Linux/systemd path stays intact."

---

## 1. Goal & scope

Make `edgeplaned` **compile and run on Windows** as a foreground console application (`edgeplaned run`), behind target-cfg gating, with the **Linux/systemd path byte-for-byte unchanged**.

**In scope (this design):**
- `edgeplaned-bin` compiles for `x86_64-pc-windows-msvc`.
- `edgeplaned run` starts, serves its mgmt gateway, and shuts down cleanly on Windows.
- A `windows-latest` CI job proves the build stays green; local `cargo check --target` during implementation.

**Explicitly OUT of scope (named follow-ups, §8):**
- Windows Service Control Manager integration (`windows-service` crate) — the daemon runs as a console app, not a registered Windows service.
- Install/uninstall parity with the systemd path.
- Named-pipe IPC (TCP loopback is the MVP transport).
- Secrets gateway and ACP attach gateway on Windows.

The Merlinlabs flag explicitly anticipates the `windows-service` crate "if ever distributed." This design builds the **foundation that makes that possible** (a daemon that compiles and runs on Windows) without committing to SCM yet. SCM is the natural Phase 2.

**Decisions locked (Merlin, 2026-05-31):** TCP-loopback IPC for the MVP; prove via local cross-check **and** a CI lane.

---

## 2. Current state — why edgeplaned does not compile on Windows today

Established by grep against `origin/main` (line numbers approximate; the authoritative fact is "these sites are ungated"). Most of edgeplaned's Unix code is **already** `#[cfg(unix)]`-gated — the blocker set is small and contained.

### 2a. Hard blockers (ungated Unix surface)

| # | Site | What | Notes |
|---|------|------|-------|
| B1 | `edgeplaned-bin/Cargo.toml:40` (`nix = { version = "0.29", features = ["signal"] }`) | Unconditional Unix-only dependency | Used in exactly one place (B2). |
| B2 | `singleton.rs::kill_holder` | `nix::sys::signal::kill` SIGTERM→SIGKILL of a stale daemon for `--kill-existing` | The only `nix` consumer. The lock itself uses `fs2::try_lock_exclusive` (already cross-platform). |
| B3 | `mgmt_gateway.rs` (`UnixListener::bind` ~L129, `from_mode(0o600)` ~L191) | Unix-socket listener + owner-only perms, ungated | A parallel TCP listener (`TcpListener::bind` ~L181/193) already exists in the same struct. |
| B4 | `secrets_gateway.rs` | `UnixListener::bind` (the perms block is already `cfg(unix)`, but the bind is not) | No TCP path exists for this gateway. |
| B5 | `main.rs` `get-secret` subcommand | `std::os::unix::net::UnixStream::connect` to the secrets socket, ungated | Client side of B4. |

### 2b. Already handled (no work needed)

- `attach_gateway.rs` — fully `#[cfg(unix)]` gated already.
- `register.rs`, `state.rs`, `local_registry.rs` — `PermissionsExt` uses already `#[cfg(unix)]` gated.
- `secrets_gateway.rs` permissions block — already `#[cfg(unix)]`.
- Shutdown: `daemon.rs` uses `tokio::signal::ctrl_c()`, which is **cross-platform** — no change needed.
- `fs2` (singleton lock), `tokio`, `portable-pty`, `dirs`, `reqwest`, etc. — all cross-platform.

**Coupling assessment:** moderate, not deep. Five gated edits + one small Windows sibling for process-kill. An earlier "deeply coupled" framing over-counted by including already-gated sites.

---

## 3. Design — per-blocker resolution

### B1 + B2 — `nix` dependency and `kill_holder`

- **Cargo.toml:** move `nix` from `[dependencies]` into `[target.'cfg(unix)'.dependencies]`. Add `[target.'cfg(windows)'.dependencies] windows-sys = { version = "0.59", features = ["Win32_System_Threading", "Win32_Foundation"] }` (only what `OpenProcess`/`TerminateProcess`/`WaitForSingleObject` need).
- **`kill_holder`:** keep the existing body under `#[cfg(unix)]`. Add a `#[cfg(windows)]` sibling that opens the target PID with `OpenProcess(PROCESS_TERMINATE | SYNCHRONIZE)`, calls `TerminateProcess`, and waits briefly for exit. ~15 lines. Rationale: `--kill-existing` is genuinely useful for "daemon already running" recovery; a real implementation is cheap and avoids a confusing "not supported" path. (Windows has no SIGTERM/SIGKILL split; `TerminateProcess` is the single hard-kill — documented as a behavior difference.)

### B3 — mgmt gateway (the IPC core)

The `run()` method spawns two accept loops: a Unix-socket one and a TCP one.

- Wrap the **Unix-socket body** in `#[cfg(unix)]`; provide a `#[cfg(not(unix))]` arm that is an inert `pending` future (the task exists but does nothing). On Windows, only the TCP loop carries traffic.
- The TCP loop already exists. **Change its bind from `0.0.0.0` to `127.0.0.1`** (loopback only) — a security tightening that is correct on *all* platforms, not just Windows (see §4), so it lands unconditionally.
- The CLI side (`crates/edgeplane/src/mgmt_gateway.rs`) already reaches the mgmt gateway over TCP, so the CLI→daemon path needs no Windows-specific change.

### B4 + B5 — secrets gateway + `get-secret` client

For the MVP, the secrets gateway is **Windows-unavailable**:
- `secrets_gateway.rs` bind/serve: `#[cfg(unix)]`; on Windows the gateway is not started (the daemon logs "secrets gateway unavailable on this platform").
- `main.rs` `get-secret` subcommand: `#[cfg(unix)]`; on Windows it returns a clear "not supported on Windows yet" error.

Rationale: the secrets gateway has **no existing TCP path**, and exposing secret material over unauthenticated loopback TCP would regress the `0600`-perms security property. A properly-authenticated secrets transport on Windows is a follow-up (§8), aligned with the named-pipe work. This is the one place where the chosen "TCP loopback" answer doesn't directly apply — flagged as an open decision in §10.

---

## 4. IPC & security posture on Windows

On Unix, the mgmt/secrets/attach sockets are gated by **filesystem permissions** (`0600`, owner-only). Windows loopback TCP has no file-perm equivalent, so the security model differs:

- **Bind `127.0.0.1` only** (never `0.0.0.0`) — keeps the listener off the network. Applied on all platforms.
- **Reuse the existing session-token AUTH handshake.** The mgmt gateway's TCP path already requires an `AUTH <token>` handshake when a token is configured. That token is the **session token loaded from the state file** (`profiles.<active>.auth.token`) — *not* an environment variable. (This field was renamed `ep_token` → `session_token` in PR #4; see §9. An earlier draft of this design incorrectly called it "EP_TOKEN", which never existed as the gateway's auth source — EP_TOKEN is fully eradicated, PR #4.)
- **Residual exposure:** any local process running as the same user can reach `127.0.0.1:<port>` and attempt the handshake. On Unix the `0600` perm prevents even reaching the socket. Closing this gap on Windows requires **named pipes with a per-user ACL** (§8) — the documented follow-up that restores the file-perm-equivalent property.

This is an honest, bounded MVP posture: no new auth mechanism, no secrets exposed (secrets gateway is off on Windows), loopback-only, session-token-gated.

---

## 5. Scope boundaries — what works vs. what's unavailable on Windows (MVP)

| Capability | Linux | Windows MVP |
|------------|-------|-------------|
| `edgeplaned run` (console) | ✓ | ✓ |
| mgmt gateway (CLI control path) | ✓ Unix socket + TCP | ✓ TCP loopback (session-token auth) |
| `--kill-existing` | ✓ SIGTERM→SIGKILL | ✓ TerminateProcess |
| Clean shutdown (Ctrl-C) | ✓ | ✓ (`tokio::signal::ctrl_c`) |
| Secrets gateway / `get-secret` | ✓ | ✗ (follow-up) |
| ACP attach gateway | ✓ | ✗ (already `cfg(unix)`; absent) |
| Run as managed service | ✓ systemd | ✗ console only (SCM is Phase 2) |

---

## 6. Feature-flag structure

- **Target-cfg, not a Cargo feature.** Use `#[cfg(unix)]` / `#[cfg(windows)]` and `[target.'cfg(...)'.dependencies]`. The Linux compile is identical to today — zero new code paths on Unix, which is the core of the Merlinlabs ask. (A Cargo `feature` would risk accidental activation and wouldn't gate the `nix` dependency cleanly; target-cfg is the idiomatic choice for platform splits.)
- **Single discoverable module** `windows.rs` (compiled `#[cfg(windows)]`) holds the Windows siblings (`kill_holder`, any future SCM entrypoint), so platform code lives in one place rather than scattered `#[cfg(windows)]` blocks.
- Inline `#[cfg(...)]` only where a function has a small Unix/Windows split (e.g. `kill_holder` dispatch, the Unix accept-loop no-op arm).

---

## 7. Verification

- **Local (during implementation):** `rustup target add x86_64-pc-windows-msvc`; `cargo check --target x86_64-pc-windows-msvc -p edgeplaned-bin`. This cross-compiles (no Windows host needed) and catches the compile blockers. Note: cross-*checking* validates types/cfg but not runtime; actual run-on-Windows behavior is validated by whoever runs it on a Windows box (or a future CI runtime test).
- **CI lane:** add `edgeplaned` `x86_64-pc-windows-msvc` to a `windows-latest` job — `cargo build -p edgeplaned-bin` (compile-only; no run, since SCM/runtime tests are out of scope). This is the standing guarantee the port stays green. The existing release-extras matrix already builds the *edgeplane CLI* on Windows; this adds the *daemon*.
- **Linux regression:** the existing `cargo nextest` suite must stay green unchanged (it will — Unix code is untouched, only newly gated).

---

## 8. Out of scope — named follow-ups

1. **Windows Service (SCM)** via `windows-service` crate — service entrypoint, register/start/stop, install/uninstall parity with systemd. This is the "do it properly" end state the Merlinlabs flag points at; it sits on top of this design.
2. **Named-pipe IPC** with per-user ACL — restores the `0600`-equivalent local-only security property; replaces loopback TCP as the Windows transport.
3. **Secrets gateway on Windows** — over named pipes (depends on #2), with the session-token handshake.
4. **ACP attach on Windows** — currently `cfg(unix)`; needs the pipe transport.
5. **Runtime CI test on Windows** — beyond compile-only, an actual `edgeplaned run` smoke test on a `windows-latest` runner.

---

## 9. Dependencies / prerequisites

- **PR #4 (`fix/nuke-ep-token-complete`)** renamed the mgmt gateway's `ep_token` field → `session_token` and fixed the stale `EP_TOKEN` doc comment. This design's §4 auth description assumes that naming. Implementation should branch from a `main` that includes #4 (or rebase onto it) so the Windows work doesn't reintroduce the old name. (This `feat/edgeplaned-windows-build` branch is currently based on pre-#4 `main`; rebase after #4 merges.)
- No new runtime services; no schema/migration changes; no entity-model changes (this is daemon-binary / IPC / build only — the `entities.md` citation rule does not apply).

---

## 10. Open decisions for spec review

1. **`kill_holder` on Windows:** real `TerminateProcess` (recommended, ~15 lines) vs. a "not supported, stop manually" stub. Design assumes the real impl.
2. **Secrets gateway on Windows:** the locked IPC answer was "TCP loopback," but the secrets gateway has no TCP path and loopback-without-ACL would weaken the secret-material security property. Design **defers** it (Windows-unavailable for MVP) rather than expose secrets over unauthenticated loopback. Confirm this deferral, or accept building a session-token-authenticated loopback secrets path now.
3. **`windows-sys` vs `windows` crate** for `TerminateProcess`: `windows-sys` (raw FFI, lighter, faster compile) recommended over the higher-level `windows` crate for this tiny surface.

---

## 11. Implementation plan (after spec approval)

Per the brainstorming flow, this design is the precursor. On approval, `writing-plans` produces the task breakdown — roughly: (1) gate `nix` + `kill_holder` Windows sibling, (2) gate mgmt Unix accept loop + loopback bind, (3) gate secrets/get-secret, (4) `windows.rs` module, (5) local cross-check, (6) CI windows-latest lane, (7) docs note on Windows console-run + limitations.
