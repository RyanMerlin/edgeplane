use std::sync::Arc;
use std::time::{Duration, Instant};

use mcd_packs::{
    evaluate_policy, Backend, Decision, ExecutionContext, PackRegistry,
    PolicyBundle,
};
use mcd_receipts::{Receipt, ReceiptStore};
use mcd_secrets::{resolve_credentials, InfisicalConfig, SessionStore};
use serde_json::Value;
use uuid::Uuid;

// ─── Request / Result ────────────────────────────────────────────────────────

/// Caller-supplied execution request.
#[derive(Debug, Clone)]
pub struct DispatchRequest {
    /// Full capability name, e.g. `"kubectl-observe.kubectl.get-pods"`.
    pub full_name: String,
    /// JSON args from the caller (matched to the capability's `inputSchema`).
    pub args: Value,
    /// Agent profile tag, forwarded into `ExecutionContext` for policy evaluation.
    pub profile: String,
    /// Environment slug, forwarded into `ExecutionContext` for policy evaluation.
    pub env: String,
    /// If `true`, validate and return what would run without launching a subprocess.
    pub dry_run: bool,
    /// Agent-specified deadline in seconds. Defaults to 30 s.
    pub timeout_secs: Option<u64>,
    /// Optional domain ID for receipt tracking.
    pub domain_id: Option<String>,
    /// Optional agent ID for receipt tracking.
    pub agent_id: Option<String>,
}

/// Structured result returned by the dispatcher.
#[derive(Debug, Clone)]
pub struct DispatchResult {
    pub ok: bool,
    /// Parsed stdout JSON, or `{"output": "<raw stdout>"}` if not valid JSON.
    pub data: Value,
    /// UUID v4 receipt handle — the receipts crate hooks into this in Phase 2.
    pub receipt_id: String,
    pub execution_time_ms: u64,
    pub exit_code: i32,
    /// Human-readable hint on error (e.g. stderr, policy reason).
    pub hint: Option<String>,
    /// Correct invocation example on error.
    pub example: Option<String>,
}

impl DispatchResult {
    fn error(receipt_id: String, hint: impl Into<String>, example: Option<String>) -> Self {
        DispatchResult {
            ok: false,
            data: Value::Null,
            receipt_id,
            execution_time_ms: 0,
            exit_code: -1,
            hint: Some(hint.into()),
            example,
        }
    }
}

// ─── Dispatcher ──────────────────────────────────────────────────────────────

pub struct CapabilityDispatcher {
    registry: Arc<PackRegistry>,
    infisical_config: Option<InfisicalConfig>,
    policy: PolicyBundle,
    receipt_store: Option<Arc<ReceiptStore>>,
    /// When set, credentials are delivered via the secrets gateway socket
    /// instead of being injected directly as env vars. Agents receive
    /// `MC_SECRETS_SOCKET` and `MC_SECRETS_SESSION` in their environment.
    session_store: Option<Arc<SessionStore>>,
    secrets_socket_path: Option<std::path::PathBuf>,
}

impl CapabilityDispatcher {
    pub fn new(
        registry: Arc<PackRegistry>,
        policy: PolicyBundle,
        infisical_config: Option<InfisicalConfig>,
    ) -> Self {
        CapabilityDispatcher {
            registry,
            infisical_config,
            policy,
            receipt_store: None,
            session_store: None,
            secrets_socket_path: None,
        }
    }

    /// Attach a receipt store; receipts will be written after every dispatch.
    pub fn with_receipt_store(mut self, store: Arc<ReceiptStore>) -> Self {
        self.receipt_store = Some(store);
        self
    }

    /// Enable the broker pattern: credentials are delivered via the secrets
    /// gateway socket instead of being injected directly as env vars.
    pub fn with_session_store(
        mut self,
        store: Arc<SessionStore>,
        socket_path: std::path::PathBuf,
    ) -> Self {
        self.session_store = Some(store);
        self.secrets_socket_path = Some(socket_path);
        self
    }

    /// Execute a capability by full name, applying policy, credentials, and subprocess execution.
    pub async fn dispatch(&self, req: DispatchRequest) -> DispatchResult {
        let result = self.dispatch_inner(&req).await;

        // Write receipt on every execution (success or failure) — never block the caller.
        if let Some(store) = &self.receipt_store {
            let receipt = Receipt {
                id: result.receipt_id.clone(),
                capability: req.full_name.clone(),
                args_json: serde_json::to_string(&req.args).unwrap_or_default(),
                result_json: serde_json::to_string(&result.data).unwrap_or_default(),
                exit_code: result.exit_code,
                execution_time_ms: result.execution_time_ms,
                domain_id: req.domain_id.clone(),
                agent_id: req.agent_id.clone(),
                created_at: chrono::Utc::now(),
            };
            if let Err(e) = store.insert(&receipt) {
                tracing::warn!("failed to write receipt: {e}");
            }
        }

        result
    }

    /// Inner dispatch logic — returns a `DispatchResult` without touching the receipt store.
    async fn dispatch_inner(&self, req: &DispatchRequest) -> DispatchResult {
        let receipt_id = Uuid::new_v4().to_string();

        // ── 1. Look up capability ───────────────────────────────────────────
        let manifest = match self.registry.get_by_full_name(&req.full_name) {
            Some(m) => m,
            None => {
                tracing::warn!(capability = %req.full_name, "capability not found in registry");
                return DispatchResult::error(
                    receipt_id,
                    format!("capability '{}' not found", req.full_name),
                    None,
                );
            }
        };

        tracing::info!(
            capability = %req.full_name,
            risk = %manifest.risk,
            dry_run = req.dry_run,
            "dispatching capability"
        );

        // ── 2. Build ExecutionContext and evaluate policy ────────────────────
        let ctx = ExecutionContext {
            profile: req.profile.clone(),
            env: req.env.clone(),
        };

        match evaluate_policy(&self.policy, &ctx, manifest) {
            Decision::Allow => {}
            Decision::Deny { reason } => {
                tracing::warn!(capability = %req.full_name, %reason, "policy denied capability");
                return DispatchResult::error(receipt_id, format!("policy denied: {reason}"), None);
            }
            Decision::RequireApproval { reason } => {
                tracing::warn!(
                    capability = %req.full_name,
                    %reason,
                    "capability requires approval; treating as deny until approval flow is wired"
                );
                return DispatchResult::error(
                    receipt_id,
                    format!("approval required: {reason}"),
                    None,
                );
            }
        }

        // ── 3. Dry-run early exit ────────────────────────────────────────────
        if req.dry_run {
            tracing::info!(capability = %req.full_name, "dry-run: skipping execution");
            return DispatchResult {
                ok: true,
                data: serde_json::json!({
                    "dry_run": true,
                    "capability": req.full_name,
                    "backend": format!("{:?}", manifest.backend),
                }),
                receipt_id,
                execution_time_ms: 0,
                exit_code: 0,
                hint: None,
                example: None,
            };
        }

        // ── 4. Resolve credentials ───────────────────────────────────────────
        let default_cfg = InfisicalConfig::default();
        let infisical_cfg = self.infisical_config.as_ref().unwrap_or(&default_cfg);
        let credentials = match resolve_credentials(&manifest.credentials, infisical_cfg).await {
            Ok(c) => c,
            Err(e) => {
                tracing::error!(capability = %req.full_name, error = %e, "credential resolution failed");
                return DispatchResult::error(
                    receipt_id,
                    format!("credential resolution failed: {e}"),
                    None,
                );
            }
        };

        // ── 5. Build and run command ─────────────────────────────────────────
        let timeout = Duration::from_secs(req.timeout_secs.unwrap_or(30));
        let start = Instant::now();

        // Broker pattern: if a session store is wired in, register the resolved
        // credentials as an ephemeral session and give the agent the socket path
        // + session ID. Otherwise inject raw values (legacy / dev mode).
        let (credential_env, session_cleanup) =
            if let (Some(store), Some(socket)) = (&self.session_store, &self.secrets_socket_path) {
                let session_id = store.create(credentials.env_vars);
                let env = vec![
                    (
                        "MC_SECRETS_SOCKET".to_string(),
                        socket.to_string_lossy().to_string(),
                    ),
                    ("MC_SECRETS_SESSION".to_string(), session_id.clone()),
                ];
                let cleanup = Some((Arc::clone(store), session_id));
                (env, cleanup)
            } else {
                (credentials.into_env_pairs(), None)
            };

        let run_result = match &manifest.backend {
            Backend::Subprocess { command, args } => {
                run_subprocess(command, args, &req.args, &credential_env, timeout)
                    .await
            }
            Backend::Builtin { name } => {
                run_builtin(name, &req.args, timeout).await
            }
            Backend::Remote { url } => {
                tracing::warn!(capability = %req.full_name, %url, "remote backend not yet implemented");
                Err(format!("remote backend not implemented (url={})", url))
            }
        };

        let elapsed_ms = start.elapsed().as_millis() as u64;

        // Clean up the credential session now that the subprocess has exited.
        if let Some((store, sid)) = session_cleanup {
            store.remove(&sid);
        }

        match run_result {
            Ok((stdout, exit_code)) => {
                if exit_code != 0 {
                    tracing::warn!(
                        capability = %req.full_name,
                        exit_code,
                        "subprocess exited with non-zero code"
                    );
                    DispatchResult {
                        ok: false,
                        data: Value::Null,
                        receipt_id,
                        execution_time_ms: elapsed_ms,
                        exit_code,
                        hint: Some(stdout.clone()),
                        example: None,
                    }
                } else {
                    let data = parse_output(&stdout);
                    tracing::info!(
                        capability = %req.full_name,
                        execution_time_ms = elapsed_ms,
                        "capability dispatched successfully"
                    );
                    DispatchResult {
                        ok: true,
                        data,
                        receipt_id,
                        execution_time_ms: elapsed_ms,
                        exit_code: 0,
                        hint: None,
                        example: None,
                    }
                }
            }
            Err(e) => {
                tracing::error!(capability = %req.full_name, error = %e, "subprocess execution failed");
                DispatchResult::error(receipt_id, e, None)
            }
        }
    }
}

// ─── Helpers ─────────────────────────────────────────────────────────────────

/// Parse stdout as JSON; fall back to `{"output": "<raw>"}`.
fn parse_output(stdout: &str) -> Value {
    serde_json::from_str(stdout).unwrap_or_else(|_| serde_json::json!({"output": stdout}))
}

/// Run a subprocess backend with args injected as `CAP_ARG_<KEY>` env vars.
///
/// `args` in the manifest are passed as-is to the process (no template substitution yet).
/// Input args from the caller are injected as env vars: `CAP_ARG_<UPPERCASE_KEY>`.
/// Credentials are also injected as env vars.
///
/// TODO(sandbox): apply sandbox profile via pre-exec hook (Task 8)
async fn run_subprocess(
    command: &str,
    static_args: &[String],
    input_args: &Value,
    credential_env: &[(String, String)],
    timeout: Duration,
) -> Result<(String, i32), String> {
    use tokio::process::Command;

    let mut cmd = Command::new(command);
    cmd.args(static_args);

    // Inject caller-supplied input args as CAP_ARG_* env vars.
    if let Some(obj) = input_args.as_object() {
        for (k, v) in obj {
            let env_key = format!("CAP_ARG_{}", k.to_uppercase().replace('-', "_"));
            let env_val = match v {
                Value::String(s) => s.clone(),
                other => other.to_string(),
            };
            cmd.env(env_key, env_val);
        }
    }

    // Inject resolved credentials.
    for (k, v) in credential_env {
        cmd.env(k, v);
    }

    cmd.stdout(std::process::Stdio::piped());
    cmd.stderr(std::process::Stdio::piped());

    let run = async move {
        let output = cmd
            .output()
            .await
            .map_err(|e| format!("failed to spawn '{}': {}", command, e))?;

        let exit_code = output.status.code().unwrap_or(-1);
        // On non-zero exit, return stderr as the output so the caller can surface it.
        let text = if exit_code != 0 {
            String::from_utf8_lossy(&output.stderr).into_owned()
        } else {
            String::from_utf8_lossy(&output.stdout).into_owned()
        };
        Ok::<_, String>((text, exit_code))
    };

    tokio::time::timeout(timeout, run)
        .await
        .map_err(|_| format!("command '{}' timed out after {}s", command, timeout.as_secs()))?
}

/// Run a builtin capability (simple in-process implementations).
async fn run_builtin(name: &str, input_args: &Value, _timeout: Duration) -> Result<(String, i32), String> {
    match name {
        "echo" => {
            let msg = input_args
                .get("message")
                .and_then(|v| v.as_str())
                .unwrap_or("");
            let output = serde_json::json!({"message": msg}).to_string();
            Ok((output, 0))
        }
        other => Err(format!("unknown builtin capability: '{}'", other)),
    }
}

// ─── Tests ────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use mcd_packs::{PackRegistry, PolicyBundle, PolicyAction, PolicyRule};

    fn base_request(full_name: &str) -> DispatchRequest {
        DispatchRequest {
            full_name: full_name.to_string(),
            args: serde_json::json!({}),
            profile: "test".to_string(),
            env: "default".to_string(),
            dry_run: false,
            timeout_secs: Some(5),
            domain_id: None,
            agent_id: None,
        }
    }

    fn allow_all_dispatcher() -> CapabilityDispatcher {
        let registry = Arc::new(PackRegistry::load_builtin().expect("builtin registry"));
        CapabilityDispatcher::new(registry, PolicyBundle::allow_all(), None)
    }

    /// Dispatching an unknown capability name returns ok=false.
    #[tokio::test]
    async fn test_dispatch_unknown_capability() {
        let dispatcher = allow_all_dispatcher();
        let req = base_request("nonexistent-pack.no-such-cap");
        let result = dispatcher.dispatch(req).await;

        assert!(!result.ok, "unknown capability should return ok=false");
        assert!(result.hint.is_some(), "hint should be set on error");
        assert!(
            result.hint.as_deref().unwrap_or("").contains("not found"),
            "hint should mention 'not found'"
        );
        // Receipt ID is always generated, even on error.
        assert!(!result.receipt_id.is_empty(), "receipt_id must always be set");
    }

    /// Dry-run with a known capability returns ok=true and no subprocess is launched.
    #[tokio::test]
    async fn test_dispatch_dry_run() {
        let registry = Arc::new(PackRegistry::load_builtin().expect("builtin registry"));
        let dispatcher = CapabilityDispatcher::new(registry, PolicyBundle::allow_all(), None);

        // base.system.echo is a builtin capability — always present, no credentials.
        let mut req = base_request("base.system.echo");
        req.dry_run = true;
        req.args = serde_json::json!({"message": "hello"});

        let result = dispatcher.dispatch(req).await;

        assert!(result.ok, "dry_run should return ok=true: hint={:?}", result.hint);
        assert_eq!(result.exit_code, 0);
        assert!(
            result.data.get("dry_run").and_then(|v| v.as_bool()).unwrap_or(false),
            "data should contain dry_run=true"
        );
        // Verify no actual subprocess was launched (no blocking, no side effects).
        assert_eq!(result.execution_time_ms, 0, "dry_run must not run the subprocess");
    }

    /// A deny-all policy causes every capability to return ok=false.
    #[tokio::test]
    async fn test_dispatch_policy_deny() {
        let registry = Arc::new(PackRegistry::load_builtin().expect("builtin registry"));
        // PolicyBundle::default() has default_action=Deny.
        let deny_all = PolicyBundle::default();
        let dispatcher = CapabilityDispatcher::new(registry, deny_all, None);

        let req = base_request("base.system.echo");
        let result = dispatcher.dispatch(req).await;

        assert!(!result.ok, "deny policy should return ok=false");
        assert!(
            result.hint.as_deref().unwrap_or("").contains("denied"),
            "hint should contain 'denied', got: {:?}",
            result.hint
        );
        assert!(!result.receipt_id.is_empty(), "receipt_id must always be set");
    }

    /// An explicit deny rule targeting a specific capability returns ok=false.
    #[tokio::test]
    async fn test_dispatch_explicit_deny_rule() {
        let registry = Arc::new(PackRegistry::load_builtin().expect("builtin registry"));
        let mut policy = PolicyBundle::allow_all();
        policy.rules.push(PolicyRule {
            capability: Some("system.echo".to_string()),
            action: PolicyAction::Deny,
            reason: Some("echo is blocked in test".to_string()),
            ..Default::default()
        });

        let dispatcher = CapabilityDispatcher::new(registry, policy, None);
        let req = base_request("base.system.echo");
        let result = dispatcher.dispatch(req).await;

        assert!(!result.ok);
        assert!(result.hint.as_deref().unwrap_or("").contains("denied"));
    }

    /// with_receipt_store() wires up the store and dispatching a builtin writes a receipt.
    #[tokio::test]
    async fn with_receipt_store_builds_and_records() {
        use mcd_receipts::ReceiptStore;
        use tempfile::tempdir;

        let dir = tempdir().unwrap();
        let db_path = dir.path().join("receipts.db");
        let store = Arc::new(ReceiptStore::open(&db_path).unwrap());

        let registry = Arc::new(PackRegistry::load_builtin().expect("builtin registry"));
        let dispatcher = CapabilityDispatcher::new(registry, PolicyBundle::allow_all(), None)
            .with_receipt_store(Arc::clone(&store));

        // Verify store is wired.
        assert!(dispatcher.receipt_store.is_some());

        // Dispatch a real builtin to verify a receipt is written.
        let mut req = base_request("base.system.echo");
        req.args = serde_json::json!({"message": "receipt-test"});
        req.domain_id = Some("m-test".to_string());
        req.agent_id = Some("a-test".to_string());

        let result = dispatcher.dispatch(req).await;
        assert!(result.ok, "echo builtin should succeed: {:?}", result.hint);

        // Check the receipt was persisted.
        let receipts = store.last(5).unwrap();
        assert_eq!(receipts.len(), 1, "exactly one receipt expected");
        assert_eq!(receipts[0].id, result.receipt_id);
        assert_eq!(receipts[0].capability, "base.system.echo");
        assert_eq!(receipts[0].domain_id.as_deref(), Some("m-test"));
        assert_eq!(receipts[0].agent_id.as_deref(), Some("a-test"));
    }
}
