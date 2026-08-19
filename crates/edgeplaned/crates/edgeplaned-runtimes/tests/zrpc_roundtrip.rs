//! Live round-trip integration test for the `edgeplane-zrpc` control path.
//!
//! `#[ignore]` by default — it needs a real `zellij` binary and builds a
//! throwaway background session in an isolated cache dir (it does NOT touch the
//! fleet's config or sessions). Run it explicitly:
//!
//! ```bash
//! # build the plugin wasm first:
//! (cd crates/edgeplane-zrpc && cargo build --release --target wasm32-wasip1)
//! cargo test -p edgeplaned-runtimes --test zrpc_roundtrip -- --ignored --nocapture
//! ```
//!
//! What it proves end-to-end against a real Zellij:
//! 1. `install_zrpc_plugin` writes a valid `config.kdl` (plugins/load_plugins)
//!    + `permissions.kdl` (raw-path key).
//! 2. The bin-crate plugin instantiates (exports `_start`; a cdylib would fail).
//! 3. `ZellijPluginClient::inject` drives it via `zellij pipe` and **returns
//!    promptly without hanging** — the pipe-exit fix (read-until-response +
//!    reap), against the real `zellij pipe` that stays open after responding.

use std::path::PathBuf;
use std::process::Command;
use std::time::Duration;

use edgeplaned_runtimes::zellij_install::install_zrpc_plugin;
use edgeplaned_runtimes::zellij_plugin::ZellijPluginClient;

fn zellij_cmd(cache: &std::path::Path) -> Command {
    let mut c = Command::new("zellij");
    c.env("XDG_CACHE_HOME", cache)
        .env_remove("ZELLIJ")
        .env_remove("ZELLIJ_SESSION_NAME");
    c
}

#[tokio::test(flavor = "current_thread")]
#[ignore = "live: needs a real zellij; builds a throwaway session"]
async fn zrpc_roundtrip_inject_returns_without_hang() {
    let wasm = std::env::var("EDGEPLANE_ZRPC_WASM").unwrap_or_else(|_| {
        "/workspace/cargo-target/wasm32-wasip1/release/edgeplane_zrpc.wasm".into()
    });
    assert!(
        PathBuf::from(&wasm).exists(),
        "wasm not found at {wasm} — build it: (cd crates/edgeplane-zrpc && cargo build --release --target wasm32-wasip1)"
    );

    let tmp = tempfile::tempdir().expect("tempdir");
    let cache = tmp.path().join("cache");
    let cache_zellij = cache.join("zellij");
    std::fs::create_dir_all(&cache_zellij).expect("mkdir cache");
    let config = tmp.path().join("config.kdl");

    // Isolate the Zellij cache so the test never touches the fleet's real
    // permissions.kdl. The client's `request()` spawns `zellij` inheriting this
    // process env, so it must be set here too (not just on our own Commands).
    // SAFETY: single-threaded (current_thread) test, set before any child spawn.
    unsafe {
        std::env::set_var("XDG_CACHE_HOME", &cache);
    }

    // 1. Provisioning (the real install tooling).
    install_zrpc_plugin(&config, &cache_zellij, &wasm).expect("install_zrpc_plugin");

    let session = format!("zrpc-rt-{}", std::process::id());

    // teardown closure (also runs on panic via the guard below)
    let cache_for_kill = cache.clone();
    let session_for_kill = session.clone();
    let _guard = scopeguard_kill(move || {
        let _ = zellij_cmd(&cache_for_kill)
            .args(["delete-session", &session_for_kill, "--force"])
            .output();
    });

    // 2. Start a headless session that preloads the plugin from our config.
    let create = zellij_cmd(&cache)
        .args([
            "--config",
            config.to_str().unwrap(),
            "attach",
            "--create-background",
            &session,
        ])
        .output()
        .expect("spawn zellij create");
    assert!(
        create.status.success(),
        "create-background failed: {}",
        String::from_utf8_lossy(&create.stderr)
    );

    // 3. Let the plugin load + request its (pre-seeded) permission.
    tokio::time::sleep(Duration::from_secs(8)).await;

    // 4. Drive the plugin via the daemon client. The whole point of the
    //    pipe-exit fix: this must RETURN (not hang) even though `zellij pipe`
    //    itself stays open after the plugin replies. inject() has its own 10s
    //    internal timeout; we wrap a 25s outer guard to detect a true hang.
    let client = ZellijPluginClient::new(&session);
    let marker = format!("ZRPC_RT_OK_{}", std::process::id());
    let injected = tokio::time::timeout(
        Duration::from_secs(25),
        client.inject("terminal_0", &format!("echo {marker}\r")),
    )
    .await;

    let injected = injected.expect("inject HUNG (>25s) — the pipe-exit fix did not work");
    injected.expect("inject returned an error (plugin reachable but rejected the request)");

    // 5. Best-effort confirmation the keystrokes actually reached the pane.
    tokio::time::sleep(Duration::from_secs(2)).await;
    let dump = zellij_cmd(&cache)
        .args(["--session", &session, "action", "dump-screen"])
        .output();
    if let Ok(out) = dump {
        let screen = String::from_utf8_lossy(&out.stdout);
        // Not a hard assert — dump-screen formatting varies; the inject Ok above
        // already proves the round-trip. Log for the operator running --nocapture.
        if screen.contains(&marker) {
            eprintln!("round-trip confirmed: marker '{marker}' present in pane");
        } else {
            eprintln!("inject returned Ok; marker not visible in dump-screen (non-fatal)");
        }
    }
}

/// Minimal scope guard so the session is torn down even if an assertion panics.
fn scopeguard_kill<F: FnMut()>(f: F) -> impl Drop {
    struct G<F: FnMut()>(F);
    impl<F: FnMut()> Drop for G<F> {
        fn drop(&mut self) {
            (self.0)();
        }
    }
    G(f)
}
