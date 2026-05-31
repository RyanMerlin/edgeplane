# edgeplaned Windows Build Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Make `edgeplaned-bin` compile for `x86_64-pc-windows-msvc` and run as a Windows console app, with the Linux/systemd path byte-for-byte unchanged.

**Architecture:** Target-cfg gating (`#[cfg(unix)]` / `#[cfg(windows)]` + `[target.'cfg(...)'.dependencies]`). On Windows the mgmt gateway serves TCP-loopback only (session-token AUTH handshake — no EP_TOKEN, eradicated in PR #4); the secrets gateway and `get-secret` are deferred (unavailable); `--kill-existing` uses `TerminateProcess` via `windows-sys`.

**Tech Stack:** Rust, tokio, `nix` (Unix-gated), `windows-sys` (Windows-gated), `fs2` (already cross-platform), GitHub Actions.

**Design:** `docs/superpowers/specs/2026-05-31-edgeplaned-windows-build-design.md` (approved 2026-05-31).
**Decisions locked (Merlin):** real `TerminateProcess`; defer secrets gateway on Windows; `windows-sys`.
**Crate path (abbrev `EPB`):** `crates/edgeplaned/crates/edgeplaned-bin`.

---

## Verification model (read first)

This is a cross-compilation port, not feature work — the canonical "did it work" gate is the **compiler for the Windows target**, not new unit tests:

- **Windows gate:** `cargo check --target x86_64-pc-windows-msvc -p edgeplaned-bin` (and `cargo build --target ...` at the end).
- **Linux regression gate:** `cargo nextest run -p edgeplaned` must keep its current pass count (197/197 at last measurement) — Unix code is only being annotated, not changed (except the loopback bind, which has existing tests).

There are no new behaviors to TDD on Linux. The one piece of genuinely new logic — the Windows `kill_holder` — cannot be unit-tested from Linux (it's `#[cfg(windows)]`); its correctness is gated by the Windows cross-compile plus the documented behavior note. Where a step adds code, the full code is shown.

---

## File structure

| File | Responsibility | Change |
|------|----------------|--------|
| `EPB/Cargo.toml` | dependency manifest | Gate `nix` to `cfg(unix)`; add `windows-sys` for `cfg(windows)` |
| `EPB/src/singleton.rs` | singleton lock + stale-daemon kill | Split `kill_holder` into Unix (existing) + Windows (new) |
| `EPB/src/mgmt_gateway.rs` | CLI↔daemon JSON-RPC gateway | Gate Unix accept loop; bind TCP to loopback |
| `EPB/src/secrets_gateway.rs` | secrets broker (Unix socket) | Gate to `cfg(unix)` |
| `EPB/src/main.rs` | CLI entrypoint, `get-secret` | Gate `get_secret` handler to `cfg(unix)` |
| `.github/workflows/release-edgeplane.yml` | release/extras build matrix | Add `edgeplaned` Windows cross-compile entry |
| `docs/guides/edgeplaned-windows.md` (new) | Windows run + limitations | Document MVP scope |

---

## Task 0: Rebase onto PR #4

**Files:** none (git operation)

The branch `feat/edgeplaned-windows-build` is based on pre-#4 `main` (`a814bc1`), where the mgmt-gateway session-token field is still `ep_token` and the doc comment still says "EP_TOKEN". Rebasing first ensures this work assumes the post-#4 `session_token` naming and never reintroduces EP_TOKEN.

- [ ] **Step 1: Rebase**

```bash
cd /tmp/ep-windows
git fetch origin
# After #4 merges to main:
git rebase origin/main
# If #4 not yet merged, rebase onto its branch instead:
# git rebase origin/fix/nuke-ep-token-complete
```

- [ ] **Step 2: Verify post-#4 naming present, no EP_TOKEN**

Run:
```bash
grep -n "session_token" crates/edgeplaned/crates/edgeplaned-bin/src/mgmt_gateway.rs
grep -rn "EP_TOKEN" crates/edgeplaned/crates/edgeplaned-bin/src/
```
Expected: `session_token` matches present; `EP_TOKEN` returns nothing.

- [ ] **Step 3: Confirm Linux baseline green**

Run: `cargo nextest run -p edgeplaned`
Expected: PASS (197/197 or current count — record it).

---

## Task 1: Install the Windows target and capture the baseline failures

**Files:** none (toolchain)

- [ ] **Step 1: Add the MSVC target**

Run: `rustup target add x86_64-pc-windows-msvc`
Expected: target installed (or "up to date").

- [ ] **Step 2: Capture the baseline cross-compile errors**

Run: `cargo check --target x86_64-pc-windows-msvc -p edgeplaned-bin 2>&1 | tee /tmp/win-baseline.txt`
Expected: FAIL. Errors should reference `nix` (unresolved for target), `UnixListener`, `std::os::unix::net`, `PermissionsExt`. This is the blocker list Tasks 2–6 clear.

---

## Task 2: Gate `nix` to Unix; add `windows-sys`

**Files:**
- Modify: `EPB/Cargo.toml` (the `nix` line, currently `nix = { version = "0.29", features = ["signal"] }`)

- [ ] **Step 1: Move `nix` under cfg(unix) and add `windows-sys`**

Remove the existing top-level `nix = { version = "0.29", features = ["signal"] }` from `[dependencies]`. Add these target tables (place after `[dependencies]`):

```toml
[target.'cfg(unix)'.dependencies]
nix = { version = "0.29", features = ["signal"] }

[target.'cfg(windows)'.dependencies]
windows-sys = { version = "0.59", features = [
    "Win32_System_Threading",
    "Win32_Foundation",
] }
```

- [ ] **Step 2: Verify Linux still resolves `nix`**

Run: `cargo check -p edgeplaned-bin`
Expected: PASS (nix resolves via the cfg(unix) table). `singleton.rs` still compiles on Linux.

- [ ] **Step 3: Verify dep graph per target**

Run:
```bash
cargo tree -p edgeplaned-bin --target x86_64-pc-windows-msvc 2>/dev/null | grep -E "nix|windows-sys" || true
cargo tree -p edgeplaned-bin --target x86_64-unknown-linux-gnu 2>/dev/null | grep -E "nix|windows-sys" || true
```
Expected: Windows target shows `windows-sys`, NOT `nix`. Linux target shows `nix`, NOT `windows-sys`.

- [ ] **Step 4: Commit**

```bash
git add crates/edgeplaned/crates/edgeplaned-bin/Cargo.toml
git commit -m "build(edgeplaned): gate nix to cfg(unix); add windows-sys for windows"
```

---

## Task 3: Split `kill_holder` into Unix + Windows

**Files:**
- Modify: `EPB/src/singleton.rs` (the `kill_holder` fn at ~L199-235)

The existing Unix body (SIGTERM → poll 5s → SIGKILL) stays, gated `#[cfg(unix)]`. Add a Windows sibling with the same signature using `windows-sys`.

- [ ] **Step 1: Annotate the existing fn `#[cfg(unix)]`**

Find `fn kill_holder(pid: i32) -> Result<()> {` (~L199) and add the attribute immediately above it:

```rust
#[cfg(unix)]
fn kill_holder(pid: i32) -> Result<()> {
    use nix::sys::signal::{Signal, kill};
    use nix::unistd::Pid;
    // ... existing body unchanged ...
}
```

- [ ] **Step 2: Add the Windows sibling directly below the Unix fn**

```rust
/// Windows equivalent of `kill_holder`. Windows has no SIGTERM/SIGKILL
/// distinction — `TerminateProcess` is a single, immediate hard kill (no
/// graceful phase). We open the process, terminate it, then wait up to 5s for
/// the handle to signal exit so the caller can safely retry the lock.
#[cfg(windows)]
fn kill_holder(pid: i32) -> Result<()> {
    use windows_sys::Win32::Foundation::{CloseHandle, WAIT_OBJECT_0};
    use windows_sys::Win32::System::Threading::{
        OpenProcess, TerminateProcess, WaitForSingleObject, PROCESS_SYNCHRONIZE,
        PROCESS_TERMINATE,
    };

    // SAFETY: all calls are standard Win32 process APIs with checked return
    // values; the handle is closed on every path before returning.
    unsafe {
        let handle = OpenProcess(
            PROCESS_TERMINATE | PROCESS_SYNCHRONIZE,
            0, // bInheritHandles = FALSE
            pid as u32,
        );
        if handle.is_null() {
            // Process already gone (or no rights to it). Treat "gone" as success;
            // the lock retry will confirm. We can't cheaply distinguish ACCESS_DENIED
            // here, so surface a clear error only if termination is genuinely needed.
            return Ok(());
        }

        if TerminateProcess(handle, 1) == 0 {
            CloseHandle(handle);
            return Err(anyhow!("TerminateProcess PID {pid} failed"));
        }

        // Mirror the Unix 5s grace: wait for the process to actually exit.
        let wait = WaitForSingleObject(handle, 5000);
        CloseHandle(handle);
        if wait != WAIT_OBJECT_0 {
            eprintln!("--kill-existing: PID {pid} did not exit within 5s after TerminateProcess");
        }
    }

    // Give the OS a moment to release the lock file before the caller retries.
    std::thread::sleep(Duration::from_millis(200));
    Ok(())
}
```

(`anyhow!` and `Duration` are already imported in this file — confirm; if `Duration` is only imported inside the Unix fn, add `use std::time::Duration;` at module scope or inside the Windows fn.)

- [ ] **Step 3: Verify Windows cross-compiles `singleton.rs`**

Run: `cargo check --target x86_64-pc-windows-msvc -p edgeplaned-bin 2>&1 | grep -A3 singleton.rs || echo "no singleton errors"`
Expected: no errors in `singleton.rs` (other files may still error until Tasks 4–6).

- [ ] **Step 4: Verify Linux unchanged**

Run: `cargo nextest run -p edgeplaned`
Expected: PASS, same count as Task 0.

- [ ] **Step 5: Commit**

```bash
git add crates/edgeplaned/crates/edgeplaned-bin/src/singleton.rs
git commit -m "feat(edgeplaned): windows kill_holder via TerminateProcess (windows-sys)"
```

---

## Task 4: Gate the mgmt gateway's Unix accept loop

**Files:**
- Modify: `EPB/src/mgmt_gateway.rs` (`run_unix` at ~L118, `handle_connection`'s Unix caller; `run()` at ~L96 unchanged)

On Windows, `run()` still spawns both tasks; the Unix task becomes an inert no-op so only TCP serves.

- [ ] **Step 1: Annotate the real `run_unix` `#[cfg(unix)]`**

Find `async fn run_unix(self: &Arc<Self>) -> Result<()> {` (~L118) and add above it:

```rust
#[cfg(unix)]
async fn run_unix(self: &Arc<Self>) -> Result<()> {
    // ... existing body unchanged ...
}
```

- [ ] **Step 2: Add the Windows no-op sibling directly below**

```rust
/// Windows has no Unix-domain sockets; the mgmt gateway serves TCP-loopback
/// only (see `run_tcp`). This task exists so `run()` is platform-agnostic but
/// simply idles.
#[cfg(not(unix))]
async fn run_unix(self: &Arc<Self>) -> Result<()> {
    tracing::debug!("mgmt unix socket unavailable on this platform; TCP-only");
    std::future::pending::<()>().await;
    Ok(())
}
```

- [ ] **Step 3: Verify Windows cross-compiles the gateway's unix path**

Run: `cargo check --target x86_64-pc-windows-msvc -p edgeplaned-bin 2>&1 | grep -A3 mgmt_gateway.rs || echo "no mgmt_gateway errors"`
Expected: no `UnixListener`/`PermissionsExt` errors from `run_unix` (the TCP bind is fixed in Task 5; it may still flag the `0.0.0.0` line as fine — that's not a Windows error).

- [ ] **Step 4: Verify Linux unchanged**

Run: `cargo nextest run -p edgeplaned`
Expected: PASS, same count.

- [ ] **Step 5: Commit**

```bash
git add crates/edgeplaned/crates/edgeplaned-bin/src/mgmt_gateway.rs
git commit -m "feat(edgeplaned): gate mgmt unix accept loop; windows serves TCP-only"
```

---

## Task 5: Bind the mgmt TCP listener to loopback (all platforms)

**Files:**
- Modify: `EPB/src/mgmt_gateway.rs` (`run_tcp` at ~L158; module doc header at ~L4)

Security tightening that lands on every platform: stop binding `0.0.0.0`.

- [ ] **Step 1: Change the bind address**

At ~L158, replace:

```rust
        let addr = format!("0.0.0.0:{}", self.tcp_port);
```
with:
```rust
        let addr = format!("127.0.0.1:{}", self.tcp_port);
```

- [ ] **Step 2: Fix the stale module doc header**

At ~L4, change the line `/// TCP socket:  \`0.0.0.0:<EP_MESH_MGMT_PORT>\` (default 7731)` to `127.0.0.1:<EP_MESH_MGMT_PORT>`. Confirm the AUTH-handshake doc references `session_token` (post-#4), not EP_TOKEN.

- [ ] **Step 3: Verify no `0.0.0.0` remains**

Run: `grep -n "0.0.0.0" crates/edgeplaned/crates/edgeplaned-bin/src/mgmt_gateway.rs`
Expected: no output.

- [ ] **Step 4: Verify Linux tests still pass (incl. the TCP auth tests)**

Run: `cargo nextest run -p edgeplaned -E 'test(mgmt_gateway)'`
Expected: PASS — `tcp_auth_accepts_good_token`, `tcp_auth_rejects_bad_token` etc. (they already bind/connect on `127.0.0.1`).

- [ ] **Step 5: Commit**

```bash
git add crates/edgeplaned/crates/edgeplaned-bin/src/mgmt_gateway.rs
git commit -m "fix(edgeplaned): bind mgmt TCP listener to 127.0.0.1 (loopback-only)"
```

---

## Task 6: Defer the secrets gateway + `get-secret` on Windows

**Files:**
- Modify: `EPB/src/secrets_gateway.rs` (`run` at ~L30; the `mod` declaration in parent)
- Modify: `EPB/src/main.rs` (`get_secret` at ~L162; the `Commands::GetSecret` arm at ~L152)

No secret material over unauthenticated loopback; restored later over named pipes (design §8).

- [ ] **Step 1: Gate the secrets_gateway module declaration to Unix**

Find the `mod secrets_gateway;` (or `pub mod secrets_gateway;`) line in `main.rs`/`lib.rs` and gate it:

```rust
#[cfg(unix)]
mod secrets_gateway;
```
Then gate the daemon startup spawn of `SecretsGateway::run` similarly. Search for where `SecretsGateway::new(...).run()` is spawned (daemon bootstrap) and wrap it:

```rust
#[cfg(unix)]
{
    // existing SecretsGateway spawn
}
#[cfg(windows)]
{
    tracing::warn!("secrets gateway unavailable on Windows (follow-up: named-pipe transport)");
}
```

- [ ] **Step 2: Gate the `get_secret` handler**

In `main.rs`, annotate the fn (~L162):

```rust
#[cfg(unix)]
fn get_secret(name: &str) -> anyhow::Result<()> {
    // ... existing body (uses std::os::unix::net::UnixStream) ...
}

#[cfg(windows)]
fn get_secret(_name: &str) -> anyhow::Result<()> {
    anyhow::bail!("get-secret is not supported on Windows yet (secrets gateway is Unix-only)")
}
```

The `Commands::GetSecret { name } => get_secret(&name),` dispatch arm (~L152) stays unchanged — both cfg variants share the signature, so the subcommand remains visible in `--help` on all platforms.

- [ ] **Step 3: Verify Windows cross-compiles `secrets_gateway.rs` / `main.rs`**

Run: `cargo check --target x86_64-pc-windows-msvc -p edgeplaned-bin 2>&1 | grep -A3 -E "secrets_gateway.rs|main.rs" || echo "no secrets/main errors"`
Expected: no `UnixListener`/`os::unix::net` errors.

- [ ] **Step 4: Verify Linux `get-secret` still works**

Run: `cargo nextest run -p edgeplaned` and `cargo run -p edgeplaned-bin -- --help | grep -i get-secret`
Expected: tests PASS; `get-secret` still listed.

- [ ] **Step 5: Commit**

```bash
git add crates/edgeplaned/crates/edgeplaned-bin/src/secrets_gateway.rs crates/edgeplaned/crates/edgeplaned-bin/src/main.rs
git commit -m "feat(edgeplaned): defer secrets gateway + get-secret on windows"
```

---

## Task 7: Full Windows cross-compile + Linux regression gate

**Files:** none (verification)

- [ ] **Step 1: Windows check + build**

Run:
```bash
cargo check --target x86_64-pc-windows-msvc -p edgeplaned-bin
cargo build --target x86_64-pc-windows-msvc -p edgeplaned-bin
```
Expected: both PASS, zero errors. If any remain, return to the relevant task.

- [ ] **Step 2: Windows clippy (note new findings only)**

Run: `cargo clippy --target x86_64-pc-windows-msvc -p edgeplaned-bin 2>&1 | tail -20`
Expected: no NEW findings from our code (pre-existing workspace clippy debt is out of scope — PR #3). Fix any our-code findings.

- [ ] **Step 3: Linux regression**

Run: `cargo nextest run -p edgeplaned`
Expected: PASS, identical count to Task 0.

- [ ] **Step 4: Confirm zero Unix-side behavior change**

Run:
```bash
cargo tree -p edgeplaned-bin --target x86_64-unknown-linux-gnu 2>/dev/null | grep windows-sys || echo "windows-sys absent on linux (good)"
grep -rn "EP_TOKEN" crates/edgeplaned/crates/edgeplaned-bin/src/ || echo "no EP_TOKEN (good)"
```
Expected: `windows-sys` absent on Linux; no EP_TOKEN.

---

## Task 8: Add the `windows-latest` CI compile lane

**Files:**
- Modify: `.github/workflows/release-edgeplane.yml` (the extras build matrix that already builds the edgeplane CLI on `x86_64-pc-windows-msvc`)

- [ ] **Step 1: Read the existing extras matrix**

Run: `grep -n "windows-msvc\|build-extras\|matrix\|bin:\|target:" .github/workflows/release-edgeplane.yml`
Identify the matrix entry that builds `bin: edgeplane` on `target: x86_64-pc-windows-msvc` / `os: windows-latest`.

- [ ] **Step 2: Add an edgeplaned Windows entry**

Add a sibling matrix entry mirroring the CLI one but for the daemon, compile-only:

```yaml
          - bin: edgeplaned
            target: x86_64-pc-windows-msvc
            os: windows-latest
```

Ensure the build step runs `cargo build -p edgeplaned-bin --target ${{ matrix.target }}` (no run; SCM/runtime tests are out of scope). Reuse the existing checkout/toolchain/cache steps and their pinned action SHAs. If the daemon should be checked on PRs (not just release dispatch), add a `paths`-filtered job in `ci.yml` instead/additionally: trigger on `crates/edgeplaned/**`, run the same `cargo build --target` on `windows-latest`. Prefer the standing PR lane per design §7.

- [ ] **Step 3: Validate the workflow YAML**

Run: `python3 -c "import yaml,sys; yaml.safe_load(open('.github/workflows/release-edgeplane.yml')); print('valid')"` (and `ci.yml` if edited).
Expected: `valid`.

- [ ] **Step 4: Commit**

```bash
git add .github/workflows/
git commit -m "ci(edgeplaned): add windows-latest cross-compile lane"
```

- [ ] **Step 5: Confirm the lane runs green**

After pushing/opening the PR, check the new `edgeplaned` Windows job. Expected: green. Record the run URL. (If it fails on something the local cross-check missed — e.g. a transitive dep — fix and note.)

---

## Task 9: Document Windows console-run + limitations

**Files:**
- Create: `docs/guides/edgeplaned-windows.md`

- [ ] **Step 1: Write the guide**

```markdown
# Running edgeplaned on Windows

edgeplaned compiles and runs on Windows (`x86_64-pc-windows-msvc`) as a
**foreground console application**. This is the MVP from
`docs/superpowers/specs/2026-05-31-edgeplaned-windows-build-design.md`.

## What works
- `edgeplaned run` — console app; Ctrl-C shuts down cleanly.
- mgmt gateway — **TCP loopback** (`127.0.0.1:<EP_MESH_MGMT_PORT>`, default 7731)
  with the session-token AUTH handshake. The `edgeplane` CLI reaches it over TCP.
- `--kill-existing` — terminates a stale edgeplaned via `TerminateProcess`
  (a single hard kill; no graceful SIGTERM phase like on Unix).

## Not available on Windows yet (follow-ups)
- Secrets gateway / `edgeplaned get-secret` (Unix-socket only today).
- ACP attach gateway.
- Running as a managed Windows Service (console only; no SCM registration).
- Named-pipe IPC (loopback TCP is the current transport).

## Security note
On Unix the mgmt/secrets sockets are owner-only via `0600` file perms. On
Windows loopback TCP there is no file-perm equivalent, so the mgmt gateway
relies on the session-token handshake and binds `127.0.0.1` only. Named pipes
with a per-user ACL are the planned follow-up to restore the owner-only property.
```

- [ ] **Step 2: Confirm the limitation list matches design §5**

Cross-check against the design's capability table. Fix any drift.

- [ ] **Step 3: Commit**

```bash
git add docs/guides/edgeplaned-windows.md
git commit -m "docs(edgeplaned): windows console-run guide + limitations"
```

---

## Task 10: Final integration check + flag for review

**Files:** none

- [ ] **Step 1: Clean re-verify both targets**

Run:
```bash
cargo check --target x86_64-pc-windows-msvc -p edgeplaned-bin
cargo nextest run -p edgeplaned
```
Expected: Windows ✓; Linux ✓ (unchanged count).

- [ ] **Step 2: Review the whole diff for Unix-side neutrality**

Run: `git diff origin/main..HEAD -- crates/`
Confirm: every Unix-side change is either a `#[cfg(...)]` annotation or the loopback bind; no logic changes; no EP_TOKEN; no new Linux deps.

- [ ] **Step 3: Push and flag**

```bash
git push origin feat/edgeplaned-windows-build
```
Summarize for Merlin: Windows cross-compile green, Linux untouched, CI lane added. Await approval before merge. Do NOT merge without explicit approval.

---

## Self-review (completed by author)

- **Spec coverage:** design §3 blockers B1–B5 → Tasks 2,3,4,6; §3 loopback → Task 5; §6 flag structure → target-cfg throughout (windows.rs consolidation left optional/inline since `kill_holder` is the only sibling); §7 verification → Tasks 1,7,8; §8 out-of-scope → none attempted; §9 prereq (#4 rebase) → Task 0; §10 decisions → baked into Tasks 3 (TerminateProcess), 6 (defer secrets), 2 (windows-sys). All covered.
- **Placeholder scan:** none — the one new code body (`kill_holder` Windows) is shown in full.
- **Type consistency:** `kill_holder(pid: i32) -> Result<()>` identical across both cfg arms; `run_unix(self: &Arc<Self>) -> Result<()>` identical across both arms; `get_secret(name)` signature shared.

## Out of scope (design §8 — do NOT implement here)

Windows Service / SCM (`windows-service`), named-pipe IPC, secrets gateway on Windows, ACP attach on Windows, runtime (not compile-only) CI test on Windows.
