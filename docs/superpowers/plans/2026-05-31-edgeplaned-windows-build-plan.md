# Plan: edgeplaned Windows build (compile + run, no SCM)

## Context

- **Design:** `docs/superpowers/specs/2026-05-31-edgeplaned-windows-build-design.md` (approved 2026-05-31).
- **Goal:** `edgeplaned-bin` compiles for `x86_64-pc-windows-msvc` and `edgeplaned run` runs as a Windows console app; Linux/systemd path byte-for-byte unchanged.
- **Approach:** target-cfg gating (`#[cfg(unix)]` / `#[cfg(windows)]`), TCP-loopback IPC, session-token AUTH handshake (no EP_TOKEN — eradicated in PR #4).
- **§10 decisions (Merlin, 2026-05-31):** (1) real `TerminateProcess` for `--kill-existing`; (2) defer secrets gateway on Windows; (3) `windows-sys`.
- **Crate:** `crates/edgeplaned/crates/edgeplaned-bin` (abbreviated `EPB` below).

### Prerequisite — rebase onto PR #4

This branch (`feat/edgeplaned-windows-build`) is based on pre-#4 `main` (`a814bc1`), where the mgmt-gateway session-token field is still named `ep_token` and the doc comment still says "EP_TOKEN". **Step 0 rebases onto a `main` that includes #4** so the field is `session_token` and no EP_TOKEN reference is reintroduced. All steps below assume post-#4 naming (`self.session_token`).

### Starting state (ground truth, verified on branch)

- `EPB/Cargo.toml:40` — `nix = { version = "0.29", features = ["signal"] }` (unconditional).
- `EPB/src/singleton.rs:197` — `fn kill_holder(pid: i32) -> Result<()>` using `nix::sys::signal::kill` (SIGTERM → poll 5s → SIGKILL). The lock itself uses `fs2::FileExt::try_lock_exclusive` (already cross-platform).
- `EPB/src/mgmt_gateway.rs` — `run()` spawns `run_unix()` + `run_tcp()` (`tokio::spawn` → `try_join!`). `run_unix` (≈L100) uses `std::os::unix::fs::PermissionsExt` + `tokio::net::UnixListener` + `from_mode(0o600)`. `run_tcp` (≈L125) binds `format!("0.0.0.0:{}", self.tcp_port)`. `handle_tcp_connection` does the AUTH handshake against `self.session_token`.
- `EPB/src/secrets_gateway.rs:27` — `pub async fn serve(self)` uses `tokio::net::UnixListener::bind`; already has a `#[cfg(unix)]` perms block at L29.
- `EPB/src/main.rs:159` — `async fn run_get_secret(args)` uses `std::os::unix::net::UnixStream`.
- `EPB/src/daemon.rs` — shutdown uses `tokio::signal::ctrl_c()` (already cross-platform; no change).
- Already `#[cfg(unix)]`-gated (no work): `attach_gateway.rs`, `register.rs`, `state.rs`, `local_registry.rs`, the `secrets_gateway` perms block.

### Verification baseline

`cargo check --target x86_64-pc-windows-msvc -p edgeplaned-bin` is the canonical "did Windows compile" gate. Requires `rustup target add x86_64-pc-windows-msvc` once (Step 1). Linux regression gate: `cargo nextest run -p edgeplaned` stays green throughout.

---

## Steps

### Step 0: Rebase branch onto PR #4

**What:** Ensure the working branch includes the EP_TOKEN eradication so the mgmt field is `session_token`.
**Where:** `feat/edgeplaned-windows-build`.
**How:** After #4 merges to `main`: `git rebase origin/main`. If #4 is not yet merged, rebase onto `fix/nuke-ep-token-complete` and re-target later. Resolve any trivial conflicts (the design doc + this plan are new files; no overlap expected).
**Verify:** `grep -rn "ep_token\|EP_TOKEN" crates/edgeplaned/crates/edgeplaned-bin/src/mgmt_gateway.rs` shows only `session_token` (and no `EP_TOKEN`); `cargo check -p edgeplaned-bin` (Linux) passes.

### Step 1: Add the Windows cross-compile target locally

**What:** Install the MSVC target so cross-checks run.
**Where:** Local toolchain.
**How:** `rustup target add x86_64-pc-windows-msvc`.
**Verify:** `rustup target list --installed | grep x86_64-pc-windows-msvc` prints the target. `cargo check --target x86_64-pc-windows-msvc -p edgeplaned-bin` now runs and **fails** with errors pointing at the `nix`/`UnixListener`/`os::unix` sites (this confirms the baseline blockers before fixing them — capture the error list).

### Step 2: Gate the `nix` dependency to Unix; add `windows-sys` for Windows

**What:** Make `nix` Unix-only; add the minimal `windows-sys` surface for process termination.
**Where:** `EPB/Cargo.toml`.
**How:** Remove `nix = { version = "0.29", features = ["signal"] }` from `[dependencies]`. Add:
```toml
[target.'cfg(unix)'.dependencies]
nix = { version = "0.29", features = ["signal"] }

[target.'cfg(windows)'.dependencies]
windows-sys = { version = "0.59", features = ["Win32_System_Threading", "Win32_Foundation"] }
```
(Place near other target-specific deps if any; otherwise at the end of the manifest. Keep existing non-nix deps in `[dependencies]`.)
**Verify:** `cargo check -p edgeplaned-bin` (Linux) still passes (nix resolves via the cfg(unix) table). `cargo tree -p edgeplaned-bin --target x86_64-pc-windows-msvc | grep -E "nix|windows-sys"` shows `windows-sys` present and `nix` absent for the Windows target.

### Step 3: Split `kill_holder` into Unix + Windows implementations

**What:** Keep the existing `nix` body under `#[cfg(unix)]`; add a `#[cfg(windows)]` sibling using `windows-sys` `OpenProcess`/`TerminateProcess`/`WaitForSingleObject`.
**Where:** `EPB/src/singleton.rs` (the `kill_holder` fn, ≈L197).
**How:**
- Annotate the current fn `#[cfg(unix)]`.
- Add a `#[cfg(windows)]` `fn kill_holder(pid: i32) -> Result<()>` with identical signature:
  - `OpenProcess(PROCESS_TERMINATE | SYNCHRONIZE, FALSE, pid as u32)`. If the handle is null, treat `GetLastError()` of `ERROR_INVALID_PARAMETER`/process-not-found as "already gone" → `Ok(())`; otherwise return an error.
  - `TerminateProcess(handle, 1)`.
  - `WaitForSingleObject(handle, 5000)` to mirror the Unix 5s grace, then `CloseHandle(handle)`.
  - Document inline that Windows has no SIGTERM/SIGKILL split — `TerminateProcess` is a single hard kill (no graceful phase).
- All `unsafe` FFI calls wrapped in a single `unsafe { }` block with a `// SAFETY:` note.
**Verify:** `cargo check --target x86_64-pc-windows-msvc -p edgeplaned-bin` no longer errors in `singleton.rs`. `cargo check -p edgeplaned-bin` (Linux) still passes. `cargo clippy --target x86_64-pc-windows-msvc -p edgeplaned-bin -- -D warnings` clean for `singleton.rs` (allowing pre-existing unrelated findings).

### Step 4: Gate the mgmt gateway's Unix accept loop; provide a Windows no-op arm

**What:** Make `run_unix` Unix-only so `run()` compiles on Windows with only the TCP loop active.
**Where:** `EPB/src/mgmt_gateway.rs` — `run_unix` (≈L100) and its spawn in `run()` (≈L96).
**How:**
- Annotate `async fn run_unix(...)` with `#[cfg(unix)]`.
- Add a `#[cfg(not(unix))]` `async fn run_unix(self: &Arc<Self>) -> Result<()>` whose body is `std::future::pending::<()>().await; Ok(())` (the spawned task exists but never resolves; the Unix socket simply isn't served on Windows). Add a one-line `tracing::debug!` noting the Unix mgmt socket is unavailable on this platform.
- `run()` is unchanged — it spawns both; on Windows the unix task idles and `run_tcp` carries traffic.
- Leave `handle_unix_connection` (`tokio::net::UnixStream`) as-is but annotate it `#[cfg(unix)]` (only `run_unix` calls it).
**Verify:** `cargo check --target x86_64-pc-windows-msvc -p edgeplaned-bin` no longer errors in `mgmt_gateway.rs` for the unix-socket path (TCP path + `run()` compile on Windows). Linux `cargo nextest run -p edgeplaned` still green.

### Step 5: Bind the mgmt TCP listener to loopback (all platforms)

**What:** Change the TCP bind from `0.0.0.0` to `127.0.0.1` — a security tightening that lands unconditionally.
**Where:** `EPB/src/mgmt_gateway.rs` — `run_tcp` (≈L128), the `format!("0.0.0.0:{}", self.tcp_port)`.
**How:** Replace `0.0.0.0` with `127.0.0.1`. Update the module doc comment header (the `TCP socket: 0.0.0.0:<port>` line) to `127.0.0.1:<port>`. Confirm the AUTH handshake against `self.session_token` (in `handle_tcp_connection`) is unchanged.
**Where (tests):** Existing mgmt tests bind `127.0.0.1:0` already and connect to `127.0.0.1:{port}` — no test change needed, but confirm.
**Verify:** `cargo nextest run -p edgeplaned` green (incl. `mgmt_gateway::tests::tcp_auth_*`). `grep -n "0.0.0.0" EPB/src/mgmt_gateway.rs` returns nothing.

### Step 6: Gate the secrets gateway and `get-secret` to Unix (defer on Windows)

**What:** Make the secrets gateway + `get-secret` Unix-only with a clear Windows "not supported yet" path.
**Where:** `EPB/src/secrets_gateway.rs` (`serve`, ≈L27, uses `UnixListener`); `EPB/src/main.rs` (`run_get_secret`, ≈L159, uses `os::unix::net::UnixStream`); and the call site in the daemon startup that spawns `SecretsGateway::serve`.
**How:**
- `secrets_gateway.rs`: annotate `serve` (and the `UnixListener` import + `SecretsGateway` impl methods that touch the socket) `#[cfg(unix)]`. If the whole module is Unix-only, gate the `mod secrets_gateway;` declaration with `#[cfg(unix)]` in its parent (cleanest — confirm no Windows code path references the type).
- Daemon startup: wrap the `SecretsGateway::serve` spawn in `#[cfg(unix)]`; add a `#[cfg(windows)]` `tracing::warn!("secrets gateway unavailable on Windows (follow-up)")`.
- `main.rs` `run_get_secret`: annotate `#[cfg(unix)]`; add a `#[cfg(windows)]` arm at the subcommand dispatch that returns `anyhow::bail!("get-secret is not supported on Windows yet")`. Ensure the `GetSecretArgs` struct + subcommand enum variant still compile on Windows (the clap arg can stay; only the handler is gated).
**Verify:** `cargo check --target x86_64-pc-windows-msvc -p edgeplaned-bin` no longer errors in `secrets_gateway.rs` / `main.rs`. Linux `cargo nextest run -p edgeplaned` green. On Linux, `edgeplaned get-secret` still works (manual: subcommand still present in `edgeplaned --help`).

### Step 7: Add the `windows.rs` module for Windows-only siblings (optional consolidation)

**What:** If Step 3's Windows `kill_holder` (or any other `#[cfg(windows)]` helper) grows beyond a few lines, move it into a single discoverable `windows.rs` module per the design's §6.
**Where:** `EPB/src/windows.rs` (new), declared `#[cfg(windows)] mod windows;` in `main.rs`/`lib.rs`.
**How:** Only do this if it improves clarity; for a single ~15-line `kill_holder` sibling, an inline `#[cfg(windows)]` in `singleton.rs` is acceptable and Step 7 can be skipped. Decide during implementation; note the choice in the commit.
**Verify:** Both targets still check; no behavior change. (Skippable step — document if skipped.)

### Step 8: Full cross-compile gate

**What:** Confirm the entire daemon crate compiles for Windows.
**Where:** Whole `EPB`.
**How:** `cargo check --target x86_64-pc-windows-msvc -p edgeplaned-bin` and `cargo build --target x86_64-pc-windows-msvc -p edgeplaned-bin`.
**Verify:** Both succeed with zero errors. Capture the output. Run `cargo clippy --target x86_64-pc-windows-msvc -p edgeplaned-bin` and note any NEW findings (pre-existing workspace clippy debt is out of scope, tracked in PR #3).

### Step 9: Linux regression gate

**What:** Prove Linux is untouched.
**Where:** Workspace.
**How:** `cargo nextest run -p edgeplaned` (and `-p edgeplane -p edgeplane-tower` if the rebase touched shared crates).
**Verify:** Same pass counts as before the branch (197/197 for edgeplaned at last measurement). `cargo build -p edgeplaned-bin` (native Linux) succeeds.

### Step 10: Add the `windows-latest` CI compile lane

**What:** A standing CI guarantee that the Windows build stays green.
**Where:** `.github/workflows/` — extend the existing `release-edgeplane.yml` extras matrix (which already builds the *edgeplane CLI* on `x86_64-pc-windows-msvc`) to add an `edgeplaned` Windows entry, OR add a small job to `ci.yml`. Prefer reusing the existing extras matrix pattern.
**How:** Add a matrix entry `{ bin: edgeplaned, target: x86_64-pc-windows-msvc, os: windows-latest }` running `cargo build -p edgeplaned-bin --target x86_64-pc-windows-msvc` (compile-only; no run — SCM/runtime tests are out of scope per design §7). Pin actions to existing SHAs used in the repo. If gating to avoid burning CI minutes on every push, mirror how the CLI Windows build is gated (e.g. `workflow_dispatch`/extras), but the design calls for a *standing* lane — prefer running on PRs that touch `crates/edgeplaned/**` via a `paths` filter.
**Verify:** Trigger the workflow (or open the PR) and confirm the new `edgeplaned` Windows job goes green. Capture the run URL.

### Step 11: Document the Windows console-run + limitations

**What:** A short note so users know what works on Windows.
**Where:** `docs/guides/` (e.g. a "Running edgeplaned on Windows" section) and/or `AGENTS.md` if appropriate.
**How:** Document: `edgeplaned run` works as a console app; mgmt gateway is TCP-loopback with session-token auth; `--kill-existing` works (hard kill, no graceful phase); **unavailable on Windows:** secrets gateway / `get-secret`, ACP attach, managed-service install (console only). Link the design doc and its §8 follow-ups (SCM, named pipes, secrets-over-pipes).
**Verify:** Doc renders; the limitation table matches the design's §5.

### Step 12: Final integration check

**What:** End-to-end confirmation against the design's success criteria.
**Where:** Whole change.
**How:** Re-run Steps 8 + 9 from a clean `cargo` state; confirm both the Windows cross-build and the Linux native build + tests pass. Review the full `git diff origin/main..HEAD` to confirm: zero behavior change on Unix (only `#[cfg(...)]` annotations + the loopback bind), no `EP_TOKEN` reintroduced, no new dependencies on Unix.
**Verify:** `cargo check --target x86_64-pc-windows-msvc -p edgeplaned-bin` ✓; `cargo nextest run -p edgeplaned` ✓ (unchanged counts); `grep -rn "EP_TOKEN" crates/edgeplaned/crates/edgeplaned-bin/src` returns nothing; `cargo tree --target x86_64-unknown-linux-gnu -p edgeplaned-bin | grep windows-sys` returns nothing (Windows dep absent on Linux).

---

## Commit strategy

One commit per logical group, all on `feat/edgeplaned-windows-build`:
1. Steps 2–3: `feat(edgeplaned): gate nix to unix; windows kill_holder via windows-sys`
2. Steps 4–5: `feat(edgeplaned): windows mgmt gateway (TCP-loopback only); bind 127.0.0.1`
3. Step 6: `feat(edgeplaned): defer secrets gateway + get-secret on windows`
4. Step 10: `ci(edgeplaned): add windows-latest cross-compile lane`
5. Step 11: `docs(edgeplaned): windows console-run + limitations`

Each commit must keep **both** `cargo nextest run -p edgeplaned` (Linux) and `cargo check --target x86_64-pc-windows-msvc -p edgeplaned-bin` green. Co-author trailer: `Co-Authored-By: Claude Opus 4.8 <noreply@anthropic.com>`. Flag for Merlin's review; do not merge without approval.

## Out of scope (design §8 — do NOT do here)

Windows Service (SCM / `windows-service`), named-pipe IPC, secrets gateway on Windows, ACP attach on Windows, runtime CI test on Windows. Each is a named follow-up.
