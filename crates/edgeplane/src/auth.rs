//! `edgeplane auth login` / `edgeplane auth logout` / `edgeplane auth whoami` — session token management.
//!
//! Session tokens (`mcs_*`) are issued by the Edgeplane server and stored
//! at `~/.ep/session.json` (chmod 600). They are:
//!
//! - Revocable server-side at any time
//! - Never embedded in agent config files (edgeplane launch uses env injection)
//! - Auto-loaded by EdgeplaneConfig from ~/.ep/session.json
//! - Validated for expiry before use, with a clear renewal hint
//!
//! ## Interactive login flow
//!
//! `edgeplane auth login` with no flags prompts the user for everything it needs:
//!   1. EP_BASE_URL (skipped if already in env or ~/.ep/config.json)
//!   2. Auth method: token or OIDC
//!      - token: masked prompt → POST /auth/sessions → save session.json
//!      - oidc:  GET /auth/oidc/cli-initiate → open browser → poll → exchange → save

use crate::{
    client::EdgeplaneClient,
    config::{load_saved_config, save_config},
    ui,
};
use anyhow::{Context, Result, anyhow};
use clap::Args;
use serde::{Deserialize, Serialize};
use std::{
    io::{self, Write},
    path::PathBuf,
    time::Duration,
};

/// Prefix all Edgeplane session tokens use.
pub const SESSION_TOKEN_PREFIX: &str = "mcs_";

// ── CLI arg types ─────────────────────────────────────────────────────────────

#[derive(Args, Debug)]
pub struct LoginArgs {
    /// Session TTL in hours (default: 8, max: 8760)
    #[arg(long, default_value_t = 8)]
    pub ttl_hours: u64,

    /// Print the session token to stdout after login (useful in scripts)
    #[arg(long)]
    pub print_token: bool,

    /// Skip prompts: use EP_AGENT_TOKEN env var directly (non-interactive)
    #[arg(long)]
    pub non_interactive: bool,

    /// Use API token auth instead of OIDC (prompts for token interactively)
    #[arg(long)]
    pub with_token: bool,
}

#[derive(Args, Debug)]
pub struct LogoutArgs {
    /// Only clear the local session file; do not call the revoke endpoint
    #[arg(long)]
    pub local_only: bool,
}

#[derive(Args, Debug)]
pub struct WhoamiArgs {}

// ── Saved session file ────────────────────────────────────────────────────────

#[derive(Serialize, Deserialize, Debug, Clone)]
pub struct SavedSession {
    pub token: String,
    pub subject: String,
    #[serde(default)]
    pub email: Option<String>,
    /// RFC3339 timestamp
    pub expires_at: String,
    /// The base URL this session was created against
    pub base_url: String,
    pub session_id: Option<i64>,
}

/// Returns the session file path for the active context.
/// Falls back to the legacy `~/.ep/session.json` when `contexts.yaml` is absent
/// so existing installs keep working without any migration step.
pub fn session_file_path() -> PathBuf {
    let ctx_path = crate::context::contexts_file_path();

    // Only use the per-context path when contexts.yaml actually exists on disk
    // (i.e. the user has run `edgeplane context add` at least once). Otherwise honour
    // the legacy path so nothing breaks for existing single-server installs.
    if ctx_path.exists() {
        let file = crate::context::load_contexts();
        if let Some((name, _)) = crate::context::active_context(&file) {
            let dir = crate::context::sessions_dir();
            if let Err(e) = std::fs::create_dir_all(&dir) {
                tracing::warn!("could not create sessions dir: {e}");
            }
            return crate::context::session_file_for(&name);
        }
    }

    edgeplaned_paths::session_file_path()
}

/// Read the saved session from disk and validate it is not expired and matches
/// the given base URL. Returns `None` if absent, expired, or URL-mismatched.
pub fn load_saved_session(base_url: &str) -> Option<SavedSession> {
    let path = session_file_path();
    let content = std::fs::read_to_string(&path).ok()?;
    let session: SavedSession = serde_json::from_str(&content).ok()?;

    // URL match: strip trailing slashes before comparing
    let stored = session.base_url.trim_end_matches('/');
    let wanted = base_url.trim_end_matches('/');
    if !stored.eq_ignore_ascii_case(wanted) {
        return None;
    }

    // Expiry: parse and check.
    //
    // Older controlplanes wrote timezone-less timestamps like
    // `2026-05-10T10:02:13.514815137` which fail RFC3339 parsing. The
    // previous fallthrough quietly treated those as non-expired,
    // resurrecting stale sessions long after they were unusable. Treat
    // any unparseable expiry as expired (defensive: fail-closed). We
    // try a couple of common shapes before giving up.
    let parsed = chrono::DateTime::parse_from_rfc3339(&session.expires_at)
        .map(|d| d.with_timezone(&chrono::Utc))
        .or_else(|_| {
            chrono::NaiveDateTime::parse_from_str(
                &session.expires_at,
                "%Y-%m-%dT%H:%M:%S%.f",
            )
            .map(|n| chrono::DateTime::<chrono::Utc>::from_naive_utc_and_offset(n, chrono::Utc))
        });
    match parsed {
        Ok(expires) if expires > chrono::Utc::now() => Some(session),
        _ => None,
    }
}

pub fn save_session(session: &SavedSession) -> Result<()> {
    let path = session_file_path();
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)?;
    }
    let json = serde_json::to_string_pretty(session)?;
    std::fs::write(&path, &json)?;
    // Restrict permissions to owner read/write only — contains a live token
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o600))?;
    }
    Ok(())
}

pub fn clear_session() -> Result<()> {
    let path = session_file_path();
    if path.exists() {
        std::fs::remove_file(&path)?;
    }
    Ok(())
}

/// Returns true if the token looks like an edgeplane session token.
pub fn is_session_token(token: &str) -> bool {
    token.starts_with(SESSION_TOKEN_PREFIX)
}

// ── Interactive helpers ───────────────────────────────────────────────────────

fn prompt(msg: &str) -> Result<String> {
    eprint!("{}", msg);
    io::stderr().flush()?;
    let mut buf = String::new();
    io::stdin().read_line(&mut buf)?;
    Ok(buf.trim().to_string())
}

fn prompt_masked(msg: &str) -> Result<String> {
    eprint!("{}", msg);
    io::stderr().flush()?;
    let value = rpassword::read_password().context("failed to read secret input")?;
    Ok(value.trim().to_string())
}

fn ui_rule(width: usize) -> String {
    format!("{}{}{}", ui::GRAY, "─".repeat(width), ui::RESET)
}

fn ui_section(title: &str) {
    eprintln!();
    eprintln!("{}{}{}{}", ui::BOLD, ui::ORANGE, title, ui::RESET);
    eprintln!("{}", ui_rule(46));
}

fn ui_kv(label: &str, value: &str, value_color: &str) {
    eprintln!(
        "  {}{: <14}{} {}{}{}",
        ui::DIM,
        format!("{}:", label),
        ui::RESET,
        value_color,
        value,
        ui::RESET
    );
}

/// Resolve base_url: env var → saved config → prompt user (and save answer).
fn resolve_base_url(env_base_url: Option<&str>) -> Result<String> {
    // 1. Explicit env / flag
    if let Some(url) = env_base_url {
        let url = url.trim_end_matches('/').to_string();
        if !url.is_empty() {
            return Ok(url);
        }
    }

    // 2. Saved config
    let cfg = load_saved_config();
    if let Some(url) = cfg.base_url.as_deref() {
        if !url.is_empty() {
            eprintln!("edgeplane auth login: using saved server URL: {}", url);
            return Ok(url.trim_end_matches('/').to_string());
        }
    }

    // 3. Interactive prompt
    let input = prompt("  Edgeplane server URL [http://localhost:8008]: ")?;
    let url = if input.is_empty() {
        "http://localhost:8008".to_string()
    } else {
        input.trim_end_matches('/').to_string()
    };

    // Persist for next time
    let mut new_cfg = load_saved_config();
    new_cfg.base_url = Some(url.clone());
    if let Err(e) = save_config(&new_cfg) {
        eprintln!("edgeplane auth login: warning: could not save config: {}", e);
    }

    Ok(url)
}

// ── Command handlers ──────────────────────────────────────────────────────────

pub async fn login(
    args: LoginArgs,
    _client: &EdgeplaneClient,
    current_base_url: &str,
) -> Result<()> {
    if args.non_interactive {
        // Non-interactive: use EP_AGENT_TOKEN env directly with the resolved URL
        let token =
            std::env::var("EP_AGENT_TOKEN").context("--non-interactive requires EP_AGENT_TOKEN to be set")?;
        let client = EdgeplaneClient::new_with_token(current_base_url, &token)
            .context("could not build client")?;
        let ttl = args.ttl_hours.clamp(1, 8760);
        let resp = client
            .post_json("/auth/sessions", &serde_json::json!({ "ttl_hours": ttl }))
            .await
            .context("token rejected — verify EP_AGENT_TOKEN and EP_BASE_URL")?;
        return finish_session_login(resp, current_base_url, args.print_token);
    }

    ui_section("Edgeplane Login");

    // Context selection: when no explicit server was provided via env/flag and
    // saved contexts exist, let the user pick one (or confirm the active one).
    let base_url = if current_base_url.is_empty() {
        let ctxs = crate::context::load_contexts();
        if !ctxs.contexts.is_empty() {
            resolve_base_url_with_context_prompt(&ctxs, &ctxs.active)?
        } else {
            resolve_base_url(None)?
        }
    } else {
        resolve_base_url(Some(current_base_url))?
    };

    // Always show which server we are about to authenticate against.
    display_login_target(&base_url);

    if args.with_token {
        login_with_token(&base_url, args.ttl_hours, args.print_token).await
    } else {
        login_oidc(&base_url, args.ttl_hours, args.print_token).await
    }
}

/// Print the Server / Context block that always precedes authentication.
fn display_login_target(base_url: &str) {
    let ctxs = crate::context::load_contexts();
    let context_label = ctxs
        .contexts
        .iter()
        .find(|(_, e)| e.base_url.trim_end_matches('/') == base_url.trim_end_matches('/'))
        .map(|(name, _)| name.as_str())
        .unwrap_or("(ad-hoc — not a saved context)")
        .to_string();
    ui_kv("Server", base_url, ui::CYAN);
    ui_kv("Context", &context_label, ui::DIM);
    eprintln!();
}

/// When multiple contexts are configured and no server was explicitly specified,
/// list them and let the user pick. Returns the resolved base_url.
fn resolve_base_url_with_context_prompt(
    ctxs: &crate::context::ContextsFile,
    active_name: &str,
) -> Result<String> {
    let entries: Vec<(&String, &crate::context::ContextEntry)> = ctxs.contexts.iter().collect();

    if entries.len() == 1 {
        // Single context — use it without prompting.
        return Ok(entries[0].1.base_url.trim_end_matches('/').to_string());
    }

    // Multiple contexts — list and prompt.
    eprintln!();
    eprintln!("  {}Available contexts:{}  (active: {}{}{})", ui::BOLD, ui::RESET, ui::CYAN, active_name, ui::RESET);
    eprintln!();
    for (i, (name, entry)) in entries.iter().enumerate() {
        let desc = entry.description.as_deref().unwrap_or("");
        let desc_part = if desc.is_empty() { String::new() } else { format!("  # {}", desc) };
        let active_marker = if *name == active_name { format!("{}*{} ", ui::GREEN, ui::RESET) } else { "  ".to_string() };
        eprintln!(
            "  {}{}{}{} {}  {}{}",
            active_marker, ui::BOLD, i + 1, ui::RESET, name, entry.base_url, desc_part
        );
    }
    eprintln!();

    let default_idx = entries.iter().position(|(n, _)| *n == active_name).unwrap_or(0) + 1;
    let raw = prompt(&format!(
        "  Select context [1-{}] (or Enter for active '{}'): ",
        entries.len(),
        active_name
    ))?;

    let chosen = if raw.is_empty() {
        entries
            .iter()
            .find(|(n, _)| *n == active_name)
            .or_else(|| entries.first())
            .map(|(_, e)| e.base_url.trim_end_matches('/').to_string())
            .unwrap_or_default()
    } else {
        let idx: usize = raw
            .parse::<usize>()
            .ok()
            .filter(|&n| n >= 1 && n <= entries.len())
            .unwrap_or(default_idx);
        entries[idx - 1].1.base_url.trim_end_matches('/').to_string()
    };

    Ok(chosen)
}

async fn login_with_token(base_url: &str, ttl_hours: u64, print_token: bool) -> Result<()> {
    eprintln!();
    let raw_token = prompt_masked("  API token: ")?;
    if raw_token.is_empty() {
        return Err(anyhow!("no token provided"));
    }
    eprintln!();

    let ttl = ttl_hours.clamp(1, 8760);
    let client = EdgeplaneClient::new_with_token(base_url, &raw_token)
        .context("could not build client with provided token")?;

    let resp = client
        .post_json("/auth/sessions", &serde_json::json!({ "ttl_hours": ttl }))
        .await
        .context("token rejected — verify the token and server URL")?;

    finish_session_login(resp, base_url, print_token)
}

async fn login_oidc(base_url: &str, ttl_hours: u64, print_token: bool) -> Result<()> {
    // Unauthenticated client — cli-initiate and cli-poll don't require a token
    let anon_client =
        EdgeplaneClient::new_with_token(base_url, "").context("could not build client")?;
    eprintln!();
    eprintln!("  {}Starting OIDC login…{}", ui::CYAN, ui::RESET);

    // Call the CLI-specific initiate endpoint (no EP_AGENT_TOKEN required)
    let init: serde_json::Value = anon_client
        .get_json("/auth/oidc/cli-initiate")
        .await
        .map_err(|e| {
            // Distinguish connection failures from OIDC-not-configured errors so
            // the user gets an actionable message instead of a raw reqwest chain.
            let msg = e.to_string();
            if msg.contains("connection refused")
                || msg.contains("error sending request")
                || msg.contains("failed to connect")
                || msg.contains("dns error")
                || msg.contains("No such host")
                || msg.contains("tcp connect")
            {
                anyhow!(
                    "could not reach the EdgePlane server at {} — check the URL \
                     (edgeplane context list) and that the tower is reachable",
                    base_url
                )
            } else {
                anyhow!(
                    "OIDC is not configured on this server (GET /auth/oidc/cli-initiate failed): {}",
                    e
                )
            }
        })?;

    let authorize_url = init["authorize_url"]
        .as_str()
        .ok_or_else(|| anyhow!("server returned no authorize_url"))?
        .to_string();
    let cli_nonce = init["cli_nonce"]
        .as_str()
        .ok_or_else(|| anyhow!("server returned no cli_nonce"))?
        .to_string();

    eprintln!();
    eprintln!(
        "  {}Opening your browser to complete authentication…{}",
        ui::BOLD,
        ui::RESET
    );
    eprintln!(
        "  {}If the browser doesn't open, visit this URL manually:{}",
        ui::DIM,
        ui::RESET
    );
    eprintln!();
    eprintln!("    {}{}{}", ui::CYAN, authorize_url, ui::RESET);
    eprintln!();

    // Best-effort browser launch
    if let Err(e) = open::that(&authorize_url) {
        eprintln!("  (could not open browser automatically: {})", e);
    }

    // Poll until the browser flow completes (up to 60 seconds before fallback).
    eprintln!(
        "  {}Waiting for browser authentication…{}",
        ui::DIM,
        ui::RESET
    );
    let poll_url = format!("/auth/oidc/cli-poll/{}", cli_nonce);
    let poll_deadline = std::time::Instant::now() + Duration::from_secs(60);

    let grant_id = 'poll: {
        while std::time::Instant::now() < poll_deadline {
            tokio::time::sleep(Duration::from_secs(2)).await;
            match anon_client.get_json(&poll_url).await {
                Ok(resp) if resp["status"].as_str() == Some("ready") => {
                    let gid = resp["grant_id"]
                        .as_str()
                        .ok_or_else(|| anyhow!("ready but no grant_id in poll response"))?
                        .to_string();
                    break 'poll gid;
                }
                _ => {} // pending or transient error — keep trying
            }
        }

        // Poll timed out — fall back to paste-from-browser.
        // The browser was redirected to /auth/oidc/cli-success?grant_id=... which shows the code.
        eprintln!();
        eprintln!("  Auto-detection timed out.");
        eprintln!("  Your browser should show a page titled \"Authentication Complete\"");
        eprintln!("  with a code. Copy it and paste it here.");
        eprintln!();
        let code = prompt("  Paste code: ")?;
        code.trim().to_string()
    };

    if grant_id.is_empty() {
        return Err(anyhow!("no code provided"));
    }
    eprintln!(
        "  {}Browser authentication complete.{}",
        ui::GREEN,
        ui::RESET
    );

    // Exchange grant for a session token
    let ttl = ttl_hours.clamp(1, 8760);
    let resp = anon_client
        .post_json(
            "/auth/oidc/exchange",
            &serde_json::json!({ "grant_id": grant_id, "ttl_hours": ttl }),
        )
        .await
        .context("failed to exchange OIDC grant for session token")?;

    finish_session_login(resp, base_url, print_token)
}

fn finish_session_login(resp: serde_json::Value, base_url: &str, print_token: bool) -> Result<()> {
    let token = resp["token"]
        .as_str()
        .or_else(|| resp["access_token"].as_str())
        .ok_or_else(|| anyhow!("server response missing 'token' field"))?
        .to_string();
    let subject = resp["subject"].as_str().unwrap_or("unknown").to_string();
    let email = resp["email"].as_str().map(|s| s.to_string());
    let expires_at = resp["expires_at"].as_str().unwrap_or("").to_string();
    let session_id = resp["session_id"].as_i64();

    let session = SavedSession {
        token: token.clone(),
        subject: subject.clone(),
        email: email.clone(),
        expires_at: expires_at.clone(),
        base_url: base_url.trim_end_matches('/').to_string(),
        session_id,
    };
    save_session(&session).context("failed to write session file")?;

    if print_token {
        println!("{}", token);
    } else {
        ui_section("Login Complete");
        let display_identity = email.as_deref().unwrap_or(&subject);
        ui_kv("Logged in as", display_identity, ui::GREEN);
        ui_kv("Token expires", &expires_at, ui::CYAN);
        ui_kv(
            "Session saved",
            &session_file_path().display().to_string(),
            ui::DIM,
        );
        eprintln!();
        eprintln!(
            "  {}Next:{}  {}edgeplane run claude{}  ·  {}edgeplane auth whoami{}",
            ui::BOLD,
            ui::RESET,
            ui::CYAN,
            ui::RESET,
            ui::CYAN,
            ui::RESET
        );
        eprintln!();
    }

    Ok(())
}

/// Run the OIDC browser flow and return the raw session token string.
/// Used by `edgeplane daemon profile add` to obtain a daemon-owned credential
/// without overwriting the user's ~/.ep/session.json.
pub async fn acquire_oidc_token(base_url: &str, ttl_hours: u64) -> Result<String> {
    let anon_client =
        EdgeplaneClient::new_with_token(base_url, "").context("could not build client")?;

    eprintln!();
    eprintln!("  {}Starting OIDC login for daemon profile…{}", crate::ui::CYAN, crate::ui::RESET);

    let init: serde_json::Value = anon_client
        .get_json("/auth/oidc/cli-initiate")
        .await
        .context("OIDC is not configured on this server")?;

    let authorize_url = init["authorize_url"]
        .as_str()
        .ok_or_else(|| anyhow!("server returned no authorize_url"))?
        .to_string();
    let cli_nonce = init["cli_nonce"]
        .as_str()
        .ok_or_else(|| anyhow!("server returned no cli_nonce"))?
        .to_string();

    eprintln!("  Opening browser for authentication…");
    eprintln!("  URL: {}{}{}", crate::ui::CYAN, authorize_url, crate::ui::RESET);
    if let Err(e) = open::that(&authorize_url) {
        eprintln!("  (could not open browser: {})", e);
    }

    eprintln!("  {}Waiting for browser authentication…{}", crate::ui::DIM, crate::ui::RESET);
    let poll_url = format!("/auth/oidc/cli-poll/{}", cli_nonce);
    let poll_deadline = std::time::Instant::now() + std::time::Duration::from_secs(60);

    let grant_id = 'poll: {
        while std::time::Instant::now() < poll_deadline {
            tokio::time::sleep(std::time::Duration::from_secs(2)).await;
            match anon_client.get_json(&poll_url).await {
                Ok(resp) if resp["status"].as_str() == Some("ready") => {
                    let gid = resp["grant_id"]
                        .as_str()
                        .ok_or_else(|| anyhow!("ready but no grant_id"))?
                        .to_string();
                    break 'poll gid;
                }
                _ => {}
            }
        }
        eprintln!("  Auto-detection timed out. Paste the code from your browser:");
        prompt("  Code: ")?.trim().to_string()
    };

    if grant_id.is_empty() {
        return Err(anyhow!("no code provided"));
    }

    let ttl = ttl_hours.clamp(1, 8760);
    let resp = anon_client
        .post_json(
            "/auth/oidc/exchange",
            &serde_json::json!({ "grant_id": grant_id, "ttl_hours": ttl }),
        )
        .await
        .context("failed to exchange OIDC grant for session token")?;

    let token = resp["token"]
        .as_str()
        .or_else(|| resp["access_token"].as_str())
        .ok_or_else(|| anyhow!("server response missing 'token' field"))?
        .to_string();

    eprintln!("  {}OIDC login complete.{}", crate::ui::GREEN, crate::ui::RESET);
    Ok(token)
}

pub async fn logout(args: LogoutArgs, client: &EdgeplaneClient) -> Result<()> {
    if !args.local_only {
        // Best-effort server-side revoke; don't fail if the session is already expired
        match client.delete("/auth/sessions/current").await {
            Ok(_) => eprintln!("edgeplane auth logout: session revoked on server"),
            Err(e) => eprintln!(
                "edgeplane auth logout: server revoke failed ({}); clearing local file anyway",
                e
            ),
        }
    }
    clear_session()?;
    eprintln!("edgeplane auth logout: cleared {}", session_file_path().display());
    Ok(())
}

pub async fn whoami(client: &EdgeplaneClient) -> Result<()> {
    // Show local session file info first
    let session_path = session_file_path();
    if session_path.exists() {
        if let Ok(content) = std::fs::read_to_string(&session_path) {
            if let Ok(session) = serde_json::from_str::<SavedSession>(&content) {
                ui_section("Local Session");
                ui_kv("Subject", &session.subject, ui::CYAN);
                if let Some(email) = session.email.as_deref().filter(|e| !e.is_empty()) {
                    ui_kv("Email", email, ui::GREEN);
                }
                ui_kv("Expires", &session.expires_at, ui::DIM);
            }
        }
    }

    // Fetch live identity from server
    let resp = client
        .get_json("/auth/me")
        .await
        .context("failed to fetch identity — check auth credentials")?;

    println!("{}", serde_json::to_string_pretty(&resp)?);
    Ok(())
}

// ── Public helper used by main.rs ─────────────────────────────────────────────

/// Resolve EP_BASE_URL for the main CLI startup, incorporating saved config as fallback.
/// Unlike the login flow, this does NOT prompt — it just returns the best available value.
pub fn resolve_startup_base_url(flag_or_env: Option<String>, default: &str) -> String {
    // 1. Explicit CLI flag or env var — always wins
    if let Some(ref url) = flag_or_env {
        let url = url.trim_end_matches('/');
        if !url.is_empty() {
            return url.to_string();
        }
    }

    // 2. Active context from contexts.yaml (preferred over legacy config.json)
    let ctx_file = crate::context::contexts_file_path();
    if ctx_file.exists() {
        let ctxs = crate::context::load_contexts();
        if let Some((_, entry)) = crate::context::active_context(&ctxs) {
            let url = entry.base_url.trim_end_matches('/');
            if !url.is_empty() {
                return url.to_string();
            }
        }
    }

    // 3. Legacy config.json
    let cfg = load_saved_config();
    if let Some(url) = cfg.base_url.as_deref() {
        if !url.is_empty() {
            return url.trim_end_matches('/').to_string();
        }
    }

    default.trim_end_matches('/').to_string()
}
