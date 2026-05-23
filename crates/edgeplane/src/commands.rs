use crate::{
    auth,
    booster::AgentBooster,
    channel,
    client::EdgeplaneClient,
    cmd,
    compat,
    config::McConfig,
    discover,
    daemon_ctl, drift, evolve, governance, launch, maintenance, mcp_server, mcp_tools, ops, run,
    output::{self, OutputMode},
    runtime,
    schema_pack::SchemaPack,
    secrets, update,
};
use anyhow::{Context, Result};
use base64::Engine;
use clap::{Args, CommandFactory, Subcommand, ValueEnum};
use clap_complete::Shell;
use serde_json::{Value, json};
use std::fs;
use std::io::Cursor;
use std::path::PathBuf;
use std::process::Command as ProcessCommand;
use std::time::{SystemTime, UNIX_EPOCH};
use tar::{Archive, Builder};
use url::form_urlencoded;

/// Top-level CLI entrypoints for the edgeplane binary.
#[derive(Subcommand, Debug)]
pub enum McCommand {
    /// Show quick local/runtime/auth context for the current shell.
    Status(StatusArgs),
    /// Shortcut for `edgeplane system doctor`.
    Doctor(maintenance::DoctorArgs),
    /// Lightweight backend readiness check.
    Health(HealthArgs),
    /// Show binary + backend version details.
    Version(VersionArgs),
    /// Show effective runtime config (redacted).
    Config(ConfigArgs),
    /// Convenience context/profile switcher.
    Use(UseArgs),
    /// Release the currently attached workspace lease.
    Release(ReleaseArgs),
    /// Tail local Edgeplane logs.
    Logs(LogsArgs),
    /// Legacy shortcut for `edgeplane auth whoami`.
    #[command(hide = true)]
    Whoami(auth::WhoamiArgs),
    /// Generate shell completion scripts.
    Completion(CompletionArgs),
    /// Artifact retrieval and mutation helpers.
    #[command(subcommand)]
    Artifact(ArtifactCommand),
    /// Authentication and identity helpers.
    #[command(subcommand)]
    Auth(AuthCommand),
    /// Governance and admin workflows.
    #[command(subcommand)]
    Admin(AdminCommand),
    /// Data/catalog/read workflows (tools, sync, explorer).
    #[command(subcommand)]
    Data(DataCommand),
    /// Platform diagnostics and release-control workflows.
    #[command(subcommand)]
    System(SystemCommand),
    /// Agent control workflows (remote, evolve, swarm/subagent workflows).
    #[command(subcommand)]
    Agent(AgentCommand),
    /// Runtime fabric workflows (nodes, jobs, leases).
    #[command(subcommand)]
    Runtime(runtime::RuntimeCommand),
    /// Approval workflow commands (requests, decisions).
    #[command(subcommand)]
    Approvals(ApprovalCommand),
    /// Workspace lifecycle helpers (load/heartbeat/artifact/commit/release).
    #[command(subcommand)]
    Workspace(WorkspaceCommand),
    /// Domain operations (lifecycle orchestration and execution workflows).
    #[command(subcommand)]
    Ops(ops::OpsCommand),
    /// Launch an agent with a fully wired Edgeplane harness.
    Launch(launch::LaunchArgs),
    /// Self-update helper for the edgeplane binary.
    Update(update::SelfUpdateArgs),
    /// Initialize MC profile state for first-time usage.
    Init(InitArgs),
    /// Start an MCP server (stdio JSON-RPC 2.0) for LLM runtime connections.
    Serve(mcp_server::ServeMcpArgs),
    /// Claude channel server integrations.
    #[command(subcommand)]
    Channel(channel::ChannelCommand),
    /// Manage Edgeplane user profiles.
    #[command(subcommand)]
    Profile(ProfileCommand),
    /// Secrets provider + reference helpers.
    #[command(subcommand)]
    Secrets(SecretsCommand),
    /// edgeplaned daemon control and work-model commands.
    #[command(name = "daemon", subcommand)]
    Daemon(daemon_ctl::DaemonCommand),
    /// Launch an agent runtime with a unified interface.
    #[command(name = "run")]
    Run(run::RunArgs),
    /// List and describe capability packs available through edgeplaned.
    #[command(subcommand)]
    Capabilities(cmd::capabilities::CapabilitiesCmd),
    /// Execute a capability through the edgeplaned routing layer.
    #[command(name = "exec", about = "Execute a capability")]
    Exec(cmd::run::ExecArgs),
    /// Inspect capability execution receipts stored in the local SQLite audit log.
    #[command(subcommand)]
    Receipts(cmd::receipts::ReceiptsCmd),
    /// Bidirectional git-backed config sync for this node.
    #[command(name = "mesh-sync", subcommand)]
    MeshSync(cmd::sync::SyncCmd),
    /// Discover edgeplane-tower nodes and write ~/.ep/servers.
    Discover(discover::DiscoverArgs),
    /// Launch the terminal UI (ratatui) for fleet monitoring and management.
    Tui(TuiArgs),
    /// Manage named controlplane connection contexts.
    #[command(subcommand)]
    Context(ContextCommand),
    /// Domain attachment and home-domain management for this agent.
    #[command(subcommand)]
    Domain(DomainCommand),
}

#[derive(Subcommand, Debug)]
pub enum ContextCommand {
    /// List all configured contexts, marking the active one with *.
    List,
    /// Show the active context name and URL.
    Current,
    /// Switch the active context.
    Use {
        /// Context name to activate.
        name: String,
    },
    /// Add a new named context.
    Add {
        /// Context name (e.g. "local", "production", "team-alpha").
        name: String,
        /// Controlplane base URL.
        #[arg(long)]
        url: String,
        /// Optional human-readable description.
        #[arg(long)]
        description: Option<String>,
    },
    /// Remove a context (cannot remove the currently active one).
    Remove {
        /// Context name to remove.
        name: String,
    },
}

#[derive(Subcommand, Debug)]
pub enum AuthCommand {
    /// Authenticate and create a session token stored at ~/.edgeplane/session.json.
    Login(auth::LoginArgs),
    /// Revoke the current session token and clear local credentials.
    Logout(auth::LogoutArgs),
    /// Show the current authenticated identity.
    Whoami(auth::WhoamiArgs),
}

#[derive(Subcommand, Debug)]
pub enum DataCommand {
    /// Inspect and invoke Edgeplane MCP tools.
    #[command(subcommand)]
    Tools(ToolsCommand),
    /// Manage local skill sync state for Missions and domains.
    #[command(subcommand)]
    Sync(SyncCommand),
    /// Explore domains, missions, and tasks via the explorer endpoints.
    #[command(subcommand)]
    Explorer(ExplorerCommand),
}

#[derive(Subcommand, Debug)]
pub enum SystemCommand {
    /// Diagnostics + auto-fix helpers.
    Doctor(maintenance::DoctorArgs),
    /// Trigger local backups (postgres, rustfs, or both).
    Backup(maintenance::BackupArgs),
    /// Cleanup local profile/session artifacts with retention limits.
    ProfileGc(maintenance::ProfileGcArgs),
    /// Self-update helper for the edgeplane binary.
    #[command(subcommand)]
    Update(update::UpdateCommand),
    /// Compatibility matrix commands and reports for provider/version drift control.
    #[command(subcommand)]
    Compat(compat::CompatCommand),
    /// Drift ingestion + policy decision helpers for staged release gates.
    #[command(subcommand)]
    Drift(drift::DriftCommand),
}

#[derive(Subcommand, Debug)]
pub enum AgentCommand {
    /// Send a prompt to an agent (auto-resolves local vs controlplane).
    Signal(crate::agent_ops::SignalArgs),
    /// Interrupt an in-flight agent (auto-resolves local vs controlplane).
    Cancel(crate::agent_ops::CancelArgs),
    /// List visible agents — local (edgeplaned-supervised) and/or remote (controlplane).
    List(crate::agent_ops::ListArgs),
    /// Describe a single agent — auto-resolves local vs controlplane.
    Describe(crate::agent_ops::DescribeArgs),
    /// Resident node-agent control verbs.
    #[command(subcommand)]
    Node(runtime::NodeAgentCommand),
    /// Self-improvement loop for Edgeplane itself (agent-driven backlog/code evolution).
    Evolve(evolve::EvolveArgs),
    /// Attach to a persistent ACP session — stream session/update frames
    /// to stdout, forward stdin lines as session/prompt.
    Attach(crate::attach::AttachArgs),
    /// Cron jobs scheduled by edgeplaned (Phase 4 daemon-absorption).
    #[command(subcommand)]
    Cron(CronCommand),
    /// systemd-unit liveness supervision (Phase 5 daemon-absorption).
    #[command(subcommand)]
    Supervise(SuperviseCommand),
}

#[derive(Subcommand, Debug)]
pub enum SuperviseCommand {
    /// List supervised agents and their current unit state.
    List(crate::agent_supervise::ListArgs),
    /// One agent's launch context + recent restart history.
    Status(crate::agent_supervise::StatusArgs),
    /// Manual `systemctl --user restart <agent>` (logged as reason=manual).
    Restart(crate::agent_supervise::RestartArgs),
    /// Pause the auto-restart loop for an agent (or all).
    Pause(crate::agent_supervise::PauseResumeArgs),
    /// Resume the auto-restart loop.
    Resume(crate::agent_supervise::PauseResumeArgs),
    /// Recent restart events across all (or one) agent.
    History(crate::agent_supervise::HistoryArgs),
    /// Stream live SupervisorEvents from edgeplaned (Ctrl-C to exit).
    Events(crate::agent_supervise::EventsArgs),
    /// Live fleet dashboard: agent table + event tail (TUI, q to quit).
    Watch(crate::agent_supervise::WatchArgs),
}

#[derive(Subcommand, Debug)]
pub enum CronCommand {
    /// List all cron jobs from `~/.ep/edgeplaned/cron.toml` + their runtime state.
    List(crate::agent_cron::ListArgs),
    /// Describe one cron job: schedule, last fire, recent history.
    Describe(crate::agent_cron::DescribeArgs),
    /// Re-parse `cron.toml` (edgeplaned reloads on its next tick).
    Reload(crate::agent_cron::ReloadArgs),
    /// Recent fires across all (or one) job.
    History(crate::agent_cron::HistoryArgs),
    /// Force a retention sweep on `agent_cron_fire_log` now.
    GcNow(crate::agent_cron::GcNowArgs),
}

#[derive(Subcommand, Debug)]
pub enum DomainCommand {
    /// Show this agent's home domain.
    Home,
    /// Attach this agent to a domain (sets current_domain_id).
    Attach {
        /// Domain ID to attach to.
        domain_id: String,
    },
    /// Detach from the current domain and return to the home domain.
    Detach,
}

#[derive(Subcommand, Debug)]
pub enum SecretsCommand {
    /// Inspect current secrets provider config for a profile.
    Status {
        #[arg(long, default_value = "default")]
        profile: String,
    },
    /// Configure secrets provider for a profile.
    #[command(subcommand)]
    Provider(SecretsProviderCommand),
    /// Resolve and print one named secret from the active profile mapping.
    Get {
        #[arg(long, default_value = "default")]
        profile: String,
        #[arg(long)]
        name: String,
        /// Show the value (default redacts in human mode).
        #[arg(long, default_value_t = false)]
        reveal: bool,
    },
    /// Seed standard secret refs for the selected provider.
    Bootstrap {
        #[arg(long, default_value = "default")]
        profile: String,
        /// Do not overwrite existing refs.
        #[arg(long, default_value_t = false)]
        keep_existing: bool,
        /// Call backend admin endpoint instead of local file mutation.
        #[arg(long, default_value_t = false)]
        via_api: bool,
    },
    /// Rotate one mapped secret for a profile.
    Rotate {
        #[arg(long, default_value = "default")]
        profile: String,
        #[arg(long)]
        name: String,
        #[arg(long)]
        value: Option<String>,
        #[arg(long, default_value = "token")]
        generator: String,
        /// Call backend admin endpoint instead of local mutation.
        #[arg(long, default_value_t = false)]
        via_api: bool,
    },
    /// Resolve mapped secrets and write a .env-style file.
    ExportEnv {
        #[arg(long, default_value = "default")]
        profile: String,
        #[arg(long)]
        out: PathBuf,
    },
    /// Manage Infisical connection profiles (multi-account, multi-instance).
    #[command(subcommand)]
    Infisical(InfisicalProfileCommand),
}

/// Subcommands for `edgeplane secrets infisical` — manage multi-account Infisical
/// connection profiles stored at `~/.ep/infisical_profiles.json`.
#[derive(Subcommand, Debug)]
pub enum InfisicalProfileCommand {
    /// Add or update a named Infisical connection profile.
    Add {
        /// Profile name (e.g. "work", "personal").
        name: String,
        /// Infisical instance URL (default: https://app.infisical.com).
        #[arg(long, default_value = "https://app.infisical.com")]
        site_url: String,
        /// Service token (mutually exclusive with --client-id / --client-secret).
        #[arg(long)]
        service_token: Option<String>,
        /// Universal Auth client ID.
        #[arg(long)]
        client_id: Option<String>,
        /// Universal Auth client secret.
        #[arg(long)]
        client_secret: Option<String>,
        /// Default project ID.
        #[arg(long)]
        project_id: Option<String>,
        /// Default environment slug.
        #[arg(long, default_value = "prod")]
        environment: String,
        /// Make this the active profile after adding.
        #[arg(long, default_value_t = true)]
        activate: bool,
    },
    /// List all Infisical profiles (marks the active one).
    List,
    /// Switch the active Infisical profile.
    Use {
        /// Profile name to activate.
        name: String,
    },
    /// Test connectivity to the active (or specified) Infisical profile.
    Test {
        /// Profile name to test (default: active profile).
        name: Option<String>,
    },
    /// Remove a named Infisical profile.
    Rm {
        /// Profile name to remove.
        name: String,
    },
    /// Fetch a secret value from Infisical using the active profile.
    Get {
        /// Secret name (key) to fetch.
        secret_name: String,
        /// Override the profile to use (default: active profile).
        #[arg(long)]
        profile: Option<String>,
        /// Override the project ID (default: profile's default_project_id).
        #[arg(long)]
        project_id: Option<String>,
        /// Override the environment slug (default: profile's default_environment).
        #[arg(long)]
        environment: Option<String>,
        /// Secret path (default: /).
        #[arg(long, default_value = "/")]
        path: String,
        /// Print the raw value without redaction (default: redacted).
        #[arg(long)]
        reveal: bool,
    },
}

#[derive(Subcommand, Debug)]
pub enum SecretsProviderCommand {
    /// Set provider to env.
    Env {
        #[arg(long, default_value = "default")]
        profile: String,
    },
    /// Set provider to Infisical and persist connection metadata.
    Infisical {
        #[arg(long, default_value = "default")]
        profile: String,
        #[arg(long)]
        project_id: Option<String>,
        #[arg(long)]
        env: Option<String>,
        #[arg(long)]
        path: Option<String>,
    },
}

#[derive(Args, Debug)]
pub struct InitArgs {
    /// Initial profile name to create when none exists.
    #[arg(long, default_value = "default")]
    profile: String,
    /// Bootstrap this node from a sync repo URL (clones repo, stores INFISICAL_TOKEN, writes config).
    #[arg(long)]
    repo: Option<String>,
}

#[derive(Args, Debug, Default)]
pub struct StatusArgs {
    /// Validate active lease by sending a heartbeat call.
    #[arg(long, default_value_t = false)]
    verify_lease: bool,
}

#[derive(Args, Debug, Default)]
pub struct TuiArgs {
    /// Open the TUI pre-focused on a specific domain by ID.
    #[arg(long)]
    pub domain: Option<String>,
}

#[derive(Args, Debug, Default)]
pub struct HealthArgs {}

#[derive(Args, Debug, Default)]
pub struct VersionArgs {}

#[derive(Args, Debug, Default)]
pub struct ConfigArgs {}

#[derive(Args, Debug, Default)]
pub struct UseArgs {
    #[arg(long)]
    profile: Option<String>,
    #[arg(long)]
    mission_id: Option<String>,
    #[arg(long, default_value_t = 900)]
    lease_seconds: u64,
    #[arg(long)]
    workspace_label: Option<String>,
    /// Auto-release existing lease when switching missions.
    #[arg(long, default_value_t = false)]
    auto_release: bool,
    /// Non-interactive confirmation for releasing/switching.
    #[arg(short = 'y', long, default_value_t = false)]
    yes: bool,
    /// Release currently attached lease instead of attaching a mission.
    #[arg(long, default_value_t = false)]
    release: bool,
}

#[derive(Args, Debug, Default)]
pub struct ReleaseArgs {
    /// Optional reason recorded in lease release metadata.
    #[arg(long)]
    reason: Option<String>,
    /// Succeed even when no active lease is tracked.
    #[arg(long, default_value_t = false)]
    ignore_missing: bool,
}

#[derive(Args, Debug, Default)]
pub struct LogsArgs {
    #[arg(long, default_value_t = 120)]
    lines: usize,
}

#[derive(Args, Debug)]
pub struct CompletionArgs {
    #[arg(value_enum)]
    shell: Shell,
}

#[derive(Subcommand, Debug)]
pub enum ToolsCommand {
    /// List all registered MCP tools.
    List,
    /// Call an MCP tool with JSON payload and show the response.
    Call(ToolsCallArgs),
}

#[derive(Subcommand, Debug)]
pub enum ArtifactCommand {
    /// Show artifact metadata.
    Inspect {
        #[arg(long)]
        id: i32,
    },
    /// Retrieve artifact bytes for validation/view.
    View {
        #[arg(long)]
        id: i32,
        /// Optional active lease for workspace-scoped retrieval.
        #[arg(long)]
        lease_id: Option<String>,
        /// Write bytes to local path instead of printing text.
        #[arg(long)]
        out: Option<PathBuf>,
    },
    /// Edit a text artifact in your local editor, then save back.
    Edit {
        #[arg(long)]
        id: i32,
        /// Optional active lease for workspace-scoped authorization check.
        #[arg(long)]
        lease_id: Option<String>,
        #[arg(long)]
        mime: Option<String>,
        /// Confirm cross-mission mutation without explicit --lease-id.
        #[arg(short = 'y', long, default_value_t = false)]
        yes: bool,
    },
    /// Replace artifact bytes from a local file.
    Replace {
        #[arg(long)]
        id: i32,
        #[arg(long)]
        from_file: PathBuf,
        /// Optional active lease for workspace-scoped mutation.
        #[arg(long)]
        lease_id: Option<String>,
        #[arg(long)]
        mime: Option<String>,
        /// Confirm cross-mission mutation without explicit --lease-id.
        #[arg(short = 'y', long, default_value_t = false)]
        yes: bool,
    },
}

#[derive(Subcommand, Debug)]
pub enum WorkspaceCommand {
    /// Load and lease a Mission workspace.
    Load {
        #[arg(long)]
        mission_id: String,
        #[arg(long)]
        workspace_label: Option<String>,
        #[arg(long)]
        agent_id: Option<String>,
        #[arg(long, default_value_t = 900)]
        lease_seconds: u64,
    },
    /// Heartbeat an existing workspace lease.
    Heartbeat {
        #[arg(long)]
        lease_id: String,
    },
    /// Fetch an artifact via the lease (download URL or inline content).
    FetchArtifact {
        #[arg(long)]
        lease_id: String,
        #[arg(long)]
        artifact_id: i32,
        #[arg(long, default_value = "content")]
        mode: String,
        #[arg(long, default_value_t = 60)]
        expires_seconds: u64,
        /// When mode=content, decode and write bytes to this local path.
        #[arg(long)]
        out: Option<PathBuf>,
    },
    /// Commit workspace changes with a JSON change_set.
    Commit {
        #[arg(long)]
        lease_id: String,
        #[arg(long)]
        change_set: String,
        #[arg(long)]
        validation_mode: Option<String>,
    },
    /// Release a lease with an optional reason.
    Release {
        #[arg(long)]
        lease_id: String,
        #[arg(long)]
        reason: Option<String>,
    },
}

#[derive(Args, Debug)]
pub struct ToolsCallArgs {
    /// Name of the MCP tool to call (e.g. edgeplane.mission.load).
    #[arg(long, short)]
    tool: String,
    /// JSON payload to send as MCP tool args. Defaults to empty object.
    #[arg(long, default_value = "{}")]
    payload: String,
}

#[derive(Subcommand, Debug)]
pub enum SyncCommand {
    /// Retrieve the last sync status for a domain/mission/agent.
    Status(SyncStatusArgs),
    /// Promote a skill sync snapshot to Edgeplane’s ledger.
    Promote(SyncPromoteArgs),
}

#[derive(Subcommand, Debug)]
pub enum ProfileCommand {
    /// Create a new profile shell on Edgeplane (empty bundle).
    Create {
        #[arg(long)]
        name: String,
        #[arg(long, default_value = "")]
        description: String,
        #[arg(long)]
        activate: bool,
    },
    /// List current user's profiles.
    List {
        #[arg(long, default_value_t = 50)]
        limit: u32,
    },
    /// Show profile metadata by name.
    Show {
        #[arg(long)]
        name: String,
    },
    /// Activate profile as default.
    Activate {
        #[arg(long)]
        name: String,
    },
    /// Download profile bundle to a local file.
    Download {
        #[arg(long)]
        name: String,
        #[arg(long)]
        out: Option<PathBuf>,
    },
    /// Publish/replace a profile bundle in Edgeplane.
    Publish {
        #[arg(long)]
        name: String,
        #[arg(long)]
        bundle: Option<PathBuf>,
        #[arg(long)]
        from_profile_dir: Option<PathBuf>,
        #[arg(long)]
        description: Option<String>,
        #[arg(long)]
        manifest_file: Option<PathBuf>,
        #[arg(long)]
        activate: bool,
    },
    /// Pull profile bundle from Edgeplane into local profile cache.
    Pull {
        #[arg(long)]
        name: String,
        #[arg(long)]
        apply: bool,
        #[arg(long)]
        allow_pin_mismatch: bool,
    },
    /// Pin a local profile to a specific remote sha256.
    Pin {
        #[arg(long)]
        name: String,
        #[arg(long)]
        sha256: String,
    },
    /// Delete a profile from Edgeplane (requires explicit confirmation flag).
    Delete {
        #[arg(long)]
        name: String,
        #[arg(long, default_value_t = false)]
        confirm_delete: bool,
    },
    /// Show remote/local pin status for a profile.
    Status {
        #[arg(long)]
        name: String,
    },
    /// Activate a profile as default and apply its bundle locally in one step.
    Use {
        #[arg(long)]
        name: String,
    },
}

#[derive(Args, Debug)]
pub struct SyncStatusArgs {
    #[arg(long)]
    domain_id: String,
    #[arg(long)]
    mission_id: Option<String>,
    #[arg(long)]
    agent_id: Option<String>,
}

#[derive(Args, Debug)]
pub struct SyncPromoteArgs {
    #[arg(long)]
    domain_id: String,
    #[arg(long)]
    snapshot_id: String,
    #[arg(long)]
    snapshot_sha256: String,
    #[arg(long)]
    local_overlay_sha256: Option<String>,
    #[arg(long)]
    mission_id: Option<String>,
    #[arg(long)]
    agent_id: Option<String>,
    #[arg(long, default_value_t = false)]
    degraded_offline: bool,
    #[arg(long, default_value_t = false)]
    drift_flag: bool,
    #[arg(long, default_value = "{}")]
    drift_details: String,
}

#[derive(Subcommand, Debug)]
pub enum ExplorerCommand {
    /// Dump the tree-view of domains, missions, and recent tasks.
    Tree(ExplorerTreeArgs),
    /// Inspect a single domain/mission/task node.
    Node(ExplorerNodeArgs),
}

#[derive(Args, Debug)]
pub struct ExplorerTreeArgs {
    #[arg(long)]
    domain_id: Option<String>,
    #[arg(long)]
    status: Option<String>,
    #[arg(long)]
    q: Option<String>,
    #[arg(long)]
    limit_tasks_per_cluster: Option<u32>,
    #[arg(long)]
    limit_missions: Option<u32>,
}

#[derive(Args, Debug)]
pub struct ExplorerNodeArgs {
    #[arg(long, value_enum)]
    node_type: ExplorerNodeType,
    #[arg(long)]
    node_id: String,
}

#[derive(ValueEnum, Clone, Debug)]
pub enum ExplorerNodeType {
    Domain,
    Mission,
    Task,
}

impl ExplorerNodeType {
    fn as_str(&self) -> &'static str {
        match self {
            ExplorerNodeType::Domain => "domain",
            ExplorerNodeType::Mission => "mission",
            ExplorerNodeType::Task => "task",
        }
    }
}

#[derive(Subcommand, Debug)]
pub enum AdminCommand {
    /// Governance policy summaries and event feeds.
    #[command(subcommand)]
    Policy(AdminPolicyCommand),
    /// Governance automation helpers (roles, policies, events).
    #[command(subcommand)]
    Governance(governance::GovernanceCommand),
}

#[derive(Subcommand, Debug)]
pub enum AdminPolicyCommand {
    /// Show the currently active governance policy.
    Active,
    /// List previous policy versions (limit defaults to 50).
    Versions {
        #[arg(long)]
        limit: Option<u32>,
    },
    /// Show the recent policy events emitted from approvals.
    Events {
        #[arg(long)]
        limit: Option<u32>,
    },
}

#[derive(Subcommand, Debug)]
pub enum ApprovalCommand {
    /// Create an approval request for a domain action.
    Create {
        #[arg(long)]
        domain_id: String,
        #[arg(long)]
        action: String,
        #[arg(long)]
        channel: Option<String>,
        #[arg(long)]
        reason: Option<String>,
        #[arg(long)]
        target_entity_type: Option<String>,
        #[arg(long)]
        target_entity_id: Option<String>,
        #[arg(long)]
        request_context: Option<String>,
        #[arg(long)]
        expires_in_seconds: Option<u64>,
    },
    /// List approval requests for a domain.
    List {
        #[arg(long)]
        domain_id: String,
        #[arg(long)]
        status: Option<String>,
        #[arg(long)]
        limit: Option<u32>,
    },
    /// Approve a pending request.
    Approve {
        #[arg(long)]
        approval_id: i32,
        #[arg(long)]
        expires_in_seconds: Option<u64>,
        #[arg(long)]
        note: Option<String>,
    },
    /// Reject a pending request.
    Reject {
        #[arg(long)]
        approval_id: i32,
        #[arg(long)]
        note: Option<String>,
    },
}

pub async fn run(
    command: McCommand,
    client: EdgeplaneClient,
    booster: AgentBooster,
    config: McConfig,
    output_mode: OutputMode,
) -> Result<()> {
    match command {
        McCommand::Status(args) => handle_status(args, client, &config, output_mode).await,
        McCommand::Doctor(args) => maintenance::run_doctor_command(&client, &config, &args).await,
        McCommand::Health(_args) => handle_health(client, output_mode).await,
        McCommand::Version(_args) => handle_version(client, &config, output_mode).await,
        McCommand::Config(_args) => handle_config(&config, output_mode),
        McCommand::Use(args) => handle_use(args, client, output_mode).await,
        McCommand::Release(args) => handle_release(args, client, output_mode).await,
        McCommand::Logs(args) => handle_logs(args, output_mode),
        McCommand::Whoami(_) => auth::whoami(&client).await,
        McCommand::Completion(args) => handle_completion(args),
        McCommand::Artifact(cmd) => {
            handle_artifact(cmd, client, &booster, &config.schema_pack, output_mode).await
        }
        McCommand::Auth(cmd) => handle_auth(cmd, client, &config).await,
        McCommand::Data(cmd) => {
            handle_data(cmd, client, &booster, &config.schema_pack, output_mode).await
        }
        McCommand::Admin(cmd) => handle_admin(cmd, client).await,
        McCommand::System(cmd) => handle_system(cmd, client, &config).await,
        McCommand::Agent(cmd) => handle_agent(cmd, client, &booster, &config.schema_pack).await,
        McCommand::Runtime(cmd) => handle_runtime(cmd, client, output_mode).await,
        McCommand::Workspace(cmd) => {
            handle_workspace(cmd, client, &booster, &config.schema_pack, output_mode).await
        }
        McCommand::Approvals(cmd) => handle_approvals(cmd, client, output_mode).await,
        McCommand::Ops(cmd) => ops::run(cmd, &client, &booster, &config.schema_pack).await,
        McCommand::Daemon(cmd) => daemon_ctl::handle(cmd, &client, &config).await,
        McCommand::Launch(args) => launch::run(args, &client, &config).await,
        McCommand::Update(args) => {
            update::run(update::UpdateCommand::SelfUpdate(args), &config).await
        }
        McCommand::Init(args) => handle_init(args, client, &config, output_mode).await,
        McCommand::Serve(args) => mcp_server::run(&args, &client).await,
        McCommand::Channel(cmd) => channel::run(cmd, &client).await,
        McCommand::Profile(cmd) => handle_profile(cmd, client, output_mode).await,
        McCommand::Secrets(cmd) => handle_secrets(cmd, client, output_mode).await,
        McCommand::Run(args) => run::run(args, &client, &config).await,
        McCommand::Capabilities(sub) => {
            // TODO: wire top-level --host and --route global flags through here
            cmd::capabilities::run(sub, None, None).await
        }
        McCommand::Exec(args) => {
            // TODO: wire top-level --host global flag here when global flags are added
            cmd::run::run(args, None).await
        }
        McCommand::Receipts(sub) => cmd::receipts::run(sub).map_err(Into::into),
        McCommand::MeshSync(sub) => cmd::sync::run(Some(sub)).map_err(Into::into),
        McCommand::Discover(args) => discover::run(args).await,
        McCommand::Tui(args) => handle_tui(args, &config),
        McCommand::Context(cmd) => handle_context(cmd),
        McCommand::Domain(cmd) => handle_domain(cmd, client, &config).await,
    }
}

fn handle_context(cmd: ContextCommand) -> Result<()> {
    use crate::context::{active_context, load_contexts, save_contexts, session_file_for};

    match cmd {
        ContextCommand::List => {
            let file = load_contexts();
            let (active_name, _) = active_context(&file);
            if file.contexts.is_empty() {
                println!("No contexts configured. Run: edgeplane context add <name> --url <url>");
                return Ok(());
            }
            for (name, entry) in &file.contexts {
                let marker = if *name == active_name { "*" } else { " " };
                let desc = entry.description.as_deref().unwrap_or("");
                let desc_part = if desc.is_empty() { String::new() } else { format!("  # {}", desc) };
                println!("{} {}  {}{}", marker, name, entry.base_url, desc_part);
            }
        }
        ContextCommand::Current => {
            let file = load_contexts();
            let (name, entry) = active_context(&file);
            println!("{} ({})", name, entry.base_url);
        }
        ContextCommand::Use { name } => {
            let mut file = load_contexts();
            if !file.contexts.contains_key(&name) {
                anyhow::bail!("context '{}' not found — run `edgeplane context list` to see available contexts", name);
            }
            file.active = name.clone();
            save_contexts(&file).map_err(|e| anyhow::anyhow!("failed to save contexts: {e}"))?;
            println!("Switched to context '{}'", name);
        }
        ContextCommand::Add { name, url, description } => {
            let mut file = load_contexts();
            if file.contexts.contains_key(&name) {
                anyhow::bail!("context '{}' already exists — remove it first with `edgeplane context remove {}`", name, name);
            }
            let url = url.trim_end_matches('/').to_string();
            file.contexts.insert(name.clone(), crate::context::ContextEntry {
                base_url: url.clone(),
                description,
            });
            save_contexts(&file).map_err(|e| anyhow::anyhow!("failed to save contexts: {e}"))?;
            println!("Added context '{}' → {}", name, url);
            println!("Run `edgeplane context use {}` to make it active.", name);
        }
        ContextCommand::Remove { name } => {
            let mut file = load_contexts();
            let (active_name, _) = active_context(&file);
            if name == active_name {
                anyhow::bail!("cannot remove the active context '{}' — switch first with `edgeplane context use <other>`", name);
            }
            if file.contexts.remove(&name).is_none() {
                anyhow::bail!("context '{}' not found", name);
            }
            let session = session_file_for(&name);
            if session.exists() {
                let _ = fs::remove_file(&session);
            }
            save_contexts(&file).map_err(|e| anyhow::anyhow!("failed to save contexts: {e}"))?;
            println!("Removed context '{}'", name);
        }
    }
    Ok(())
}

fn handle_tui(args: TuiArgs, config: &McConfig) -> Result<()> {
    let ctxs = crate::context::load_contexts();
    let (context_name, _) = crate::context::active_context(&ctxs);
    let cfg = crate::tui::TuiConfig {
        base_url: config.base_url.to_string(),
        token: config.token.clone(),
        version: env!("CARGO_PKG_VERSION").to_string(),
        initial_domain: args.domain,
        context_name,
    };
    crate::tui::run(cfg)
}

#[derive(serde::Serialize, serde::Deserialize, Default)]
struct ActiveWorkspaceState {
    lease_id: Option<String>,
    domain_id: Option<String>,
    mission_id: Option<String>,
    status: Option<String>,
}

fn active_workspace_path() -> PathBuf {
    crate::config::ep_home_dir().join("active_workspace.json")
}

fn load_active_workspace() -> ActiveWorkspaceState {
    let path = active_workspace_path();
    let raw = match fs::read_to_string(path) {
        Ok(v) => v,
        Err(_) => return ActiveWorkspaceState::default(),
    };
    serde_json::from_str(&raw).unwrap_or_default()
}

fn save_active_workspace(ctx: &ActiveWorkspaceState) -> Result<()> {
    let path = active_workspace_path();
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)?;
    }
    fs::write(path, serde_json::to_string_pretty(ctx)?)?;
    Ok(())
}

fn clear_active_workspace() -> Result<()> {
    let path = active_workspace_path();
    if path.exists() {
        fs::remove_file(path)?;
    }
    Ok(())
}

fn response_lease_view(response: &Value) -> &Value {
    response.get("lease").unwrap_or(response)
}

fn extract_workspace_state(response: &Value) -> ActiveWorkspaceState {
    let lease = response_lease_view(response);
    ActiveWorkspaceState {
        lease_id: lease
            .get("id")
            .and_then(|v| v.as_str())
            .or_else(|| response.get("lease_id").and_then(|v| v.as_str()))
            .map(|s| s.to_string()),
        domain_id: lease
            .get("domain_id")
            .and_then(|v| v.as_str())
            .or_else(|| response.get("domain_id").and_then(|v| v.as_str()))
            .map(|s| s.to_string()),
        mission_id: lease
            .get("mission_id")
            .and_then(|v| v.as_str())
            .or_else(|| response.get("mission_id").and_then(|v| v.as_str()))
            .map(|s| s.to_string()),
        status: lease
            .get("status")
            .and_then(|v| v.as_str())
            .or_else(|| response.get("status").and_then(|v| v.as_str()))
            .map(|s| s.to_string()),
    }
}

fn prompt_confirm(prompt: &str) -> Result<bool> {
    use std::io::{self, Write};
    eprint!("{prompt}");
    io::stderr().flush()?;
    let mut buf = String::new();
    io::stdin().read_line(&mut buf)?;
    let answer = buf.trim().to_ascii_lowercase();
    Ok(matches!(answer.as_str(), "y" | "yes"))
}

async fn handle_health(client: EdgeplaneClient, output_mode: OutputMode) -> Result<()> {
    let response = client.get_json("/mcp/health").await?;
    output::print_value(output_mode, &response);
    Ok(())
}

async fn handle_version(
    client: EdgeplaneClient,
    config: &McConfig,
    output_mode: OutputMode,
) -> Result<()> {
    let backend = client.get_json("/mcp/health").await.ok();
    let payload = json!({
        "ep_version": env!("CARGO_PKG_VERSION"),
        "base_url": config.base_url.as_str(),
        "backend_health": backend,
    });
    output::print_value(output_mode, &payload);
    Ok(())
}

fn handle_config(config: &McConfig, output_mode: OutputMode) -> Result<()> {
    let payload = json!({
        "base_url": config.base_url.as_str(),
        "timeout_secs": config.timeout.as_secs(),
        "allow_insecure": config.allow_insecure,
        "token": if config.token.is_some() { "***redacted***" } else { "" },
        "agent_context": {
            "agent_id": config.agent_context.agent_id,
            "runtime_session_id": config.agent_context.runtime_session_id,
            "profile_name": config.agent_context.profile_name,
        },
        "paths": {
            "ep_home": crate::config::ep_home_dir(),
            "skills_home": crate::config::skills_home_dir(),
            "agent_id_file": crate::config::agent_id_file(),
        }
    });
    output::print_value(output_mode, &payload);
    Ok(())
}

async fn handle_use(
    args: UseArgs,
    client: EdgeplaneClient,
    output_mode: OutputMode,
) -> Result<()> {
    if let Some(profile) = args.profile {
        return handle_profile(ProfileCommand::Use { name: profile }, client, output_mode).await;
    }
    if args.release {
        return handle_release(
            ReleaseArgs {
                reason: Some("released via edgeplane use --release".to_string()),
                ignore_missing: false,
            },
            client,
            output_mode,
        )
        .await;
    }
    let mission_id = args
        .mission_id
        .ok_or_else(|| anyhow::anyhow!("`edgeplane use` requires --mission-id (or --profile)"))?;
    let current = load_active_workspace();
    if let (Some(existing_lease), Some(existing_mission)) =
        (current.lease_id.clone(), current.mission_id.clone())
    {
        if existing_mission != mission_id {
            let should_release = if args.auto_release || args.yes {
                true
            } else {
                prompt_confirm(&format!(
                    "Release existing lease {} for mission {} and switch to {}? [y/N] ",
                    existing_lease, existing_mission, mission_id
                ))?
            };
            if !should_release {
                anyhow::bail!("switch cancelled; existing lease kept active");
            }
            let _ = mcp_tools::call_tool(
                &client,
                None,
                None,
                "release_mission_workspace",
                json!({"lease_id": existing_lease, "reason": "switch mission via edgeplane use"}),
            )
            .await?;
            clear_active_workspace()?;
        }
    }
    let mut tool_args = json!({
        "mission_id": mission_id,
        "lease_seconds": args.lease_seconds,
    });
    if let Some(label) = args.workspace_label {
        tool_args["workspace_label"] = json!(label);
    }
    let response =
        mcp_tools::call_tool(&client, None, None, "load_mission_workspace", tool_args).await?;
    let state = extract_workspace_state(&response);
    save_active_workspace(&state)?;
    output::print_value(
        output_mode,
        &json!({"ok": true, "workspace": state, "lease": response}),
    );
    Ok(())
}

async fn handle_release(
    args: ReleaseArgs,
    client: EdgeplaneClient,
    output_mode: OutputMode,
) -> Result<()> {
    let state = load_active_workspace();
    let Some(lease_id) = state.lease_id.clone() else {
        if args.ignore_missing {
            output::print_value(
                output_mode,
                &json!({"ok": true, "released": false, "reason": "no_active_lease"}),
            );
            return Ok(());
        }
        anyhow::bail!("no active lease is tracked; nothing to release");
    };
    let response = mcp_tools::call_tool(
        &client,
        None,
        None,
        "release_mission_workspace",
        json!({
            "lease_id": lease_id,
            "reason": args.reason.unwrap_or_else(|| "released via edgeplane release".to_string())
        }),
    )
    .await?;
    clear_active_workspace()?;
    output::print_value(
        output_mode,
        &json!({"ok": true, "released": true, "lease": response}),
    );
    Ok(())
}

fn handle_logs(args: LogsArgs, output_mode: OutputMode) -> Result<()> {
    let candidates = [
        crate::config::ep_home_dir().join("daemon.log"),
        crate::config::ep_home_dir().join("logs/daemon.log"),
        crate::config::ep_home_dir().join("logs/edgeplane.log"),
    ];
    let mut entries = Vec::new();
    for path in candidates {
        if !path.exists() {
            continue;
        }
        let raw = fs::read_to_string(&path).unwrap_or_default();
        let lines: Vec<String> = raw
            .lines()
            .rev()
            .take(args.lines)
            .collect::<Vec<_>>()
            .into_iter()
            .rev()
            .map(|s| s.to_string())
            .collect();
        entries.push(json!({"path": path, "lines": lines}));
    }
    let payload = json!({ "logs": entries });
    output::print_value(output_mode, &payload);
    Ok(())
}

fn handle_completion(args: CompletionArgs) -> Result<()> {
    #[derive(clap::Parser)]
    struct CompletionRoot {
        #[command(subcommand)]
        command: McCommand,
    }
    let mut cmd = CompletionRoot::command();
    clap_complete::generate(args.shell, &mut cmd, "edgeplane", &mut std::io::stdout());
    Ok(())
}

fn infer_mime_from_path(path: &std::path::Path) -> &'static str {
    let ext = path
        .extension()
        .and_then(|v| v.to_str())
        .unwrap_or("")
        .to_ascii_lowercase();
    match ext.as_str() {
        "md" => "text/markdown",
        "txt" => "text/plain",
        "json" => "application/json",
        "yaml" | "yml" => "application/yaml",
        "toml" => "application/toml",
        "xml" => "application/xml",
        "csv" => "text/csv",
        "html" => "text/html",
        "js" => "application/javascript",
        "ts" => "application/typescript",
        "py" | "rs" | "go" | "java" | "c" | "h" | "cpp" | "hpp" | "sh" => "text/plain",
        "png" => "image/png",
        "jpg" | "jpeg" => "image/jpeg",
        "gif" => "image/gif",
        "webp" => "image/webp",
        "pdf" => "application/pdf",
        "zip" => "application/zip",
        _ => "application/octet-stream",
    }
}

fn is_text_like_mime(mime: &str) -> bool {
    let m = mime.to_ascii_lowercase();
    m.starts_with("text/")
        || matches!(
            m.as_str(),
            "application/json"
                | "application/yaml"
                | "application/toml"
                | "application/xml"
                | "application/javascript"
                | "application/typescript"
        )
}

async fn handle_artifact(
    command: ArtifactCommand,
    client: EdgeplaneClient,
    _booster: &AgentBooster,
    _schema_pack: &SchemaPack,
    output_mode: OutputMode,
) -> Result<()> {
    let json_output = output_mode.is_machine();
    match command {
        ArtifactCommand::Inspect { id } => {
            let path = format!("/artifacts/{id}");
            let artifact = client.get_json(&path).await?;
            output::print_value(output_mode, &artifact);
        }
        ArtifactCommand::View { id, lease_id, out } => {
            let artifact = client.get_json(&format!("/artifacts/{id}")).await?;
            let mime = artifact
                .get("mime_type")
                .and_then(|v| v.as_str())
                .unwrap_or("application/octet-stream");
            let content_b64 = if let Some(lease_id) = lease_id {
                let response = mcp_tools::call_tool(
                    &client,
                    None,
                    None,
                    "fetch_workspace_artifact",
                    json!({"lease_id": lease_id, "artifact_id": id, "mode": "content"}),
                )
                .await?;
                response
                    .get("content_b64")
                    .and_then(|v| v.as_str())
                    .map(|s| s.to_string())
                    .context("workspace artifact fetch did not return content_b64")?
            } else {
                let mut req = client
                    .request_builder(reqwest::Method::GET, &format!("/artifacts/{id}/content"))?;
                req = req.header("accept", "*/*");
                let resp = req
                    .send()
                    .await
                    .context("artifact content request failed")?;
                let resp = resp
                    .error_for_status()
                    .context("artifact content request returned error status")?;
                let bytes = resp
                    .bytes()
                    .await
                    .context("failed reading artifact content body")?;
                base64::engine::general_purpose::STANDARD.encode(bytes)
            };
            let bytes = base64::engine::general_purpose::STANDARD
                .decode(content_b64)
                .context("artifact content_b64 decode failed")?;
            if let Some(out_path) = out {
                if let Some(parent) = out_path.parent() {
                    fs::create_dir_all(parent)?;
                }
                fs::write(&out_path, &bytes)
                    .with_context(|| format!("failed writing {}", out_path.display()))?;
                output::print_value(
                    output_mode,
                    &json!({"ok": true, "artifact_id": id, "written_path": out_path.display().to_string(), "size_bytes": bytes.len(), "mime_type": mime}),
                );
            } else if is_text_like_mime(mime) {
                let text = String::from_utf8(bytes)
                    .context("artifact appears text-like but is not valid UTF-8; use --out")?;
                if json_output {
                    output::print_value(
                        output_mode,
                        &json!({"artifact_id": id, "mime_type": mime, "text": text}),
                    );
                } else {
                    println!("{text}");
                }
            } else {
                anyhow::bail!("binary/non-text artifact requires --out for safe viewing");
            }
        }
        ArtifactCommand::Edit {
            id,
            lease_id,
            mime,
            yes,
        } => {
            let artifact = client.get_json(&format!("/artifacts/{id}")).await?;
            let artifact_mission = artifact
                .get("mission_id")
                .and_then(|v| v.as_str())
                .unwrap_or("")
                .to_string();
            let active = load_active_workspace();
            if lease_id.is_none() {
                if let Some(active_mission) = active.mission_id.as_deref() {
                    if !artifact_mission.is_empty() && active_mission != artifact_mission && !yes {
                        anyhow::bail!(
                            "cross-mission mutation without --lease-id requires -y (active={}, target={})",
                            active_mission,
                            artifact_mission
                        );
                    }
                }
            }
            let detected_mime = artifact
                .get("mime_type")
                .and_then(|v| v.as_str())
                .unwrap_or("application/octet-stream");
            let resolved_mime = mime.unwrap_or_else(|| detected_mime.to_string());
            if !is_text_like_mime(&resolved_mime) {
                anyhow::bail!(
                    "edgeplane artifact edit only supports text-like MIME; got {} (use `edgeplane artifact replace --id {} --from-file ...`)",
                    resolved_mime,
                    id
                );
            }

            let content_b64 = if let Some(lease_id) = lease_id.as_ref() {
                let response = mcp_tools::call_tool(
                    &client,
                    None,
                    None,
                    "fetch_workspace_artifact",
                    json!({"lease_id": lease_id, "artifact_id": id, "mode": "content"}),
                )
                .await?;
                response
                    .get("content_b64")
                    .and_then(|v| v.as_str())
                    .map(|s| s.to_string())
                    .context("workspace artifact fetch did not return content_b64")?
            } else {
                let resp = client
                    .request_builder(reqwest::Method::GET, &format!("/artifacts/{id}/content"))?
                    .header("accept", "*/*")
                    .send()
                    .await
                    .context("artifact content request failed")?
                    .error_for_status()
                    .context("artifact content request returned error status")?;
                let bytes = resp
                    .bytes()
                    .await
                    .context("failed reading artifact content body")?;
                base64::engine::general_purpose::STANDARD.encode(bytes)
            };
            let original_bytes = base64::engine::general_purpose::STANDARD
                .decode(content_b64)
                .context("artifact content decode failed")?;
            let original_text = String::from_utf8(original_bytes.clone()).context(
                "artifact content is not UTF-8 text; use `edgeplane artifact replace --from-file`",
            )?;

            let editor = std::env::var("VISUAL")
                .ok()
                .filter(|v| !v.trim().is_empty())
                .or_else(|| {
                    std::env::var("EDITOR")
                        .ok()
                        .filter(|v| !v.trim().is_empty())
                })
                .unwrap_or_else(|| "vi".to_string());
            let stamp = SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .unwrap_or_default()
                .as_millis();
            let mut tmp = std::env::temp_dir();
            tmp.push(format!("edgeplane-artifact-{}-{}.tmp", id, stamp));
            fs::write(&tmp, original_text.as_bytes())
                .with_context(|| format!("failed writing temp file {}", tmp.display()))?;

            let status = ProcessCommand::new(&editor)
                .arg(&tmp)
                .status()
                .with_context(|| format!("failed launching editor `{editor}`"))?;
            if !status.success() {
                anyhow::bail!("editor exited with non-zero status: {}", status);
            }
            let new_bytes = fs::read(&tmp)
                .with_context(|| format!("failed reading edited file {}", tmp.display()))?;
            let _ = fs::remove_file(&tmp);

            if new_bytes == original_bytes {
                output::print_value(
                    output_mode,
                    &json!({"ok": true, "artifact_id": id, "changed": false}),
                );
                return Ok(());
            }
            let content_b64 = base64::engine::general_purpose::STANDARD.encode(&new_bytes);
            if let Some(lease_id) = lease_id.as_ref() {
                let _ = mcp_tools::call_tool(
                    &client,
                    None,
                    None,
                    "fetch_workspace_artifact",
                    json!({"lease_id": lease_id, "artifact_id": id, "mode": "content"}),
                )
                .await
                .context("lease scope check failed for artifact edit")?;
            }
            let response = client
                .put_json(
                    &format!("/artifacts/{id}"),
                    &json!({"content_b64": content_b64, "mime_type": resolved_mime}),
                )
                .await?;
            output::print_value(
                output_mode,
                &json!({"ok": true, "artifact_id": id, "changed": true, "mode": "editor_update", "artifact": response}),
            );
        }
        ArtifactCommand::Replace {
            id,
            from_file,
            lease_id,
            mime,
            yes,
        } => {
            let artifact = client.get_json(&format!("/artifacts/{id}")).await?;
            let artifact_mission = artifact
                .get("mission_id")
                .and_then(|v| v.as_str())
                .unwrap_or("")
                .to_string();
            let active = load_active_workspace();
            if lease_id.is_none() {
                if let Some(active_mission) = active.mission_id.as_deref() {
                    if !artifact_mission.is_empty() && active_mission != artifact_mission && !yes {
                        anyhow::bail!(
                            "cross-mission mutation without --lease-id requires -y (active={}, target={})",
                            active_mission,
                            artifact_mission
                        );
                    }
                }
            }
            let bytes = fs::read(&from_file)
                .with_context(|| format!("failed reading {}", from_file.display()))?;
            let resolved_mime =
                mime.unwrap_or_else(|| infer_mime_from_path(&from_file).to_string());
            let content_b64 = base64::engine::general_purpose::STANDARD.encode(&bytes);
            if let Some(lease_id) = lease_id {
                // Security guard: ensure provided lease can access this artifact
                // before permitting mutation.
                let _ = mcp_tools::call_tool(
                    &client,
                    None,
                    None,
                    "fetch_workspace_artifact",
                    json!({"lease_id": lease_id, "artifact_id": id, "mode": "content"}),
                )
                .await
                .context("lease scope check failed for artifact replacement")?;
            }
            let response = client
                .put_json(
                    &format!("/artifacts/{id}"),
                    &json!({"content_b64": content_b64, "mime_type": resolved_mime}),
                )
                .await?;
            output::print_value(
                output_mode,
                &json!({"ok": true, "artifact_id": id, "mode": "direct_update", "artifact": response}),
            );
        }
    }
    Ok(())
}

async fn handle_status(
    args: StatusArgs,
    client: EdgeplaneClient,
    config: &McConfig,
    output_mode: OutputMode,
) -> Result<()> {
    let local_session = auth::load_saved_session(config.base_url.as_str());
    let effective_agent_id = config
        .agent_context
        .agent_id
        .clone()
        .or_else(|| crate::config::default_agent_id_from_session(config.base_url.as_str()));
    let remote = match client.get_json("/auth/whoami").await {
        Ok(value) => Some(value),
        Err(err) => Some(json!({ "reachable": false, "error": err.to_string() })),
    };
    let workspace = load_active_workspace();
    let lease_verification = if args.verify_lease {
        if let Some(lease_id) = workspace.lease_id.clone() {
            match mcp_tools::call_tool(
                &client,
                None,
                None,
                "heartbeat_workspace_lease",
                json!({"lease_id": lease_id}),
            )
            .await
            {
                Ok(v) => Some(json!({"ok": true, "result": v})),
                Err(err) => Some(json!({"ok": false, "error": err.to_string()})),
            }
        } else {
            Some(json!({"ok": false, "error": "no active lease"}))
        }
    } else {
        None
    };
    let payload = json!({
        "base_url": config.base_url.as_str(),
        "output": output_mode.as_str(),
        "auth": {
            "has_session": local_session.is_some(),
            "session_subject": local_session.as_ref().map(|s| s.subject.clone()),
            "session_email": local_session.as_ref().and_then(|s| s.email.clone()),
            "session_expires_at": local_session.as_ref().map(|s| s.expires_at.clone()),
            "has_token": config.token.is_some(),
            "remote_whoami": remote,
        },
        "runtime": {
            "profile": config.agent_context.profile_name.clone(),
            "runtime_session_id": config.agent_context.runtime_session_id.clone(),
            "agent_id": effective_agent_id,
        },
        "context": {
            "lease_id": workspace.lease_id,
            "domain_id": workspace.domain_id,
            "mission_id": workspace.mission_id,
            "status": workspace.status,
        },
        "lease_verification": lease_verification,
    });
    output::print_value(output_mode, &payload);
    Ok(())
}

async fn handle_auth(
    command: AuthCommand,
    client: EdgeplaneClient,
    config: &McConfig,
) -> Result<()> {
    match command {
        AuthCommand::Login(args) => auth::login(args, &client, config.base_url.as_str()).await,
        AuthCommand::Logout(args) => auth::logout(args, &client).await,
        AuthCommand::Whoami(_) => auth::whoami(&client).await,
    }
}

async fn handle_secrets(
    command: SecretsCommand,
    client: EdgeplaneClient,
    output_mode: OutputMode,
) -> Result<()> {
    match command {
        SecretsCommand::Status { profile } => {
            let cfg = secrets::load_profile_secrets(profile.trim());
            let payload = json!({
                "profile": profile,
                "path": secrets::profile_secrets_path(profile.trim()),
                "provider": cfg.provider.unwrap_or_else(|| "env".to_string()),
                "infisical_project_id": cfg.infisical_project_id,
                "infisical_env": cfg.infisical_env,
                "infisical_path": cfg.infisical_path,
                "refs_count": cfg.refs.len(),
                "refs": cfg.refs,
            });
            output::print_value(output_mode, &payload);
            Ok(())
        }
        SecretsCommand::Provider(provider_cmd) => {
            handle_secrets_provider(provider_cmd, output_mode).await
        }
        SecretsCommand::Get {
            profile,
            name,
            reveal,
        } => {
            let cfg = secrets::load_profile_secrets(profile.trim());
            let reference = cfg
                .refs
                .get(name.trim())
                .ok_or_else(|| anyhow::anyhow!("no secret reference mapped for '{}'", name.trim()))?
                .to_string();
            let resolved = secrets::resolve_maybe_secret_ref(&reference).await?;
            let display = if reveal || output_mode.is_machine() {
                resolved.clone()
            } else {
                "***redacted***".to_string()
            };
            let payload = json!({
                "profile": profile,
                "name": name,
                "reference": reference,
                "value": display,
            });
            output::print_value(output_mode, &payload);
            Ok(())
        }
        SecretsCommand::Bootstrap {
            profile,
            keep_existing,
            via_api,
        } => {
            let profile_name = profile.trim();
            if via_api {
                let payload = json!({
                    "profile": profile_name,
                    "keep_existing": keep_existing,
                });
                let response = client.post_json("/ops/secrets/bootstrap", &payload).await?;
                output::print_value(output_mode, &response);
                return Ok(());
            }
            let mut cfg = secrets::load_profile_secrets(profile_name);
            let provider = cfg
                .provider
                .clone()
                .unwrap_or_else(|| "env".to_string())
                .to_ascii_lowercase();
            let standard_names = [
                "EP_TOKEN",
                "MQTT_PASSWORD",
                "POSTGRES_PASSWORD",
                "EP_OBJECT_STORAGE_ACCESS_KEY",
                "EP_OBJECT_STORAGE_ACCESS_SECRET",
            ];
            for name in standard_names {
                if keep_existing && cfg.refs.contains_key(name) {
                    continue;
                }
                let reference = if provider == "infisical" {
                    secrets::build_infisical_ref(
                        name,
                        cfg.infisical_project_id.as_deref(),
                        cfg.infisical_env.as_deref(),
                        cfg.infisical_path.as_deref(),
                    )
                } else {
                    format!("secret://env/{name}")
                };
                cfg.refs.insert(name.to_string(), reference);
            }
            secrets::save_profile_secrets(profile_name, &cfg)?;
            let payload = json!({
                "ok": true,
                "profile": profile_name,
                "provider": provider,
                "path": secrets::profile_secrets_path(profile_name),
                "refs_count": cfg.refs.len(),
                "refs": cfg.refs,
            });
            output::print_value(output_mode, &payload);
            Ok(())
        }
        SecretsCommand::Rotate {
            profile,
            name,
            value,
            generator,
            via_api,
        } => {
            let profile_name = profile.trim();
            let secret_name = name.trim();
            if secret_name.is_empty() {
                anyhow::bail!("--name cannot be empty");
            }
            if via_api {
                let payload = json!({
                    "profile": profile_name,
                    "name": secret_name,
                    "value": value,
                    "generator": generator,
                });
                let response = client.post_json("/ops/secrets/rotate", &payload).await?;
                output::print_value(output_mode, &response);
                return Ok(());
            }
            let cfg = secrets::load_profile_secrets(profile_name);
            let reference = cfg
                .refs
                .get(secret_name)
                .ok_or_else(|| anyhow::anyhow!("no secret reference mapped for '{}'", secret_name))?
                .to_string();
            let rotated = if let Some(v) = value {
                if v.trim().is_empty() {
                    anyhow::bail!("--value cannot be empty when provided");
                }
                v
            } else if generator.trim().eq_ignore_ascii_case("hex") {
                uuid::Uuid::new_v4().simple().to_string()
                    + &uuid::Uuid::new_v4().simple().to_string()
            } else {
                uuid::Uuid::new_v4().to_string() + "-" + &uuid::Uuid::new_v4().to_string()
            };
            let parsed = url::Url::parse(&reference).context("invalid secret reference URL")?;
            let provider = parsed.host_str().unwrap_or_default();
            let target_name = parsed.path().trim_start_matches('/').to_string();
            match provider {
                "env" => {
                    // SAFETY: no other threads access env at this point.
                    unsafe { std::env::set_var(&target_name, &rotated) };
                }
                "infisical" => {
                    let mut cmd = std::process::Command::new("infisical");
                    cmd.args(["secrets", "set", &target_name, &rotated]);
                    for (k, v) in parsed.query_pairs() {
                        match k.as_ref() {
                            "projectId" => {
                                cmd.arg("--projectId");
                                cmd.arg(v.as_ref());
                            }
                            "env" => {
                                cmd.arg("--env");
                                cmd.arg(v.as_ref());
                            }
                            "path" => {
                                cmd.arg("--path");
                                cmd.arg(v.as_ref());
                            }
                            _ => {}
                        }
                    }
                    let out = cmd.output().context("failed to execute infisical CLI")?;
                    if !out.status.success() {
                        anyhow::bail!("infisical secrets set failed for '{}'", target_name);
                    }
                }
                other => anyhow::bail!("provider '{}' is not mutable via local rotate", other),
            }
            let payload = json!({
                "ok": true,
                "profile": profile_name,
                "name": secret_name,
                "reference": reference,
                "provider": provider,
                "updated": true,
                "value": if output_mode.is_machine() { rotated } else { "***redacted***".to_string() },
            });
            output::print_value(output_mode, &payload);
            Ok(())
        }
        SecretsCommand::ExportEnv { profile, out } => {
            let profile_name = profile.trim();
            let cfg = secrets::load_profile_secrets(profile_name);
            if cfg.refs.is_empty() {
                anyhow::bail!("no refs configured for profile '{}'", profile_name);
            }
            let mut lines: Vec<String> = Vec::new();
            let mut names: Vec<String> = cfg.refs.keys().cloned().collect();
            names.sort();
            for name in names {
                let reference = cfg
                    .refs
                    .get(&name)
                    .ok_or_else(|| anyhow::anyhow!("missing ref for '{}'", name))?;
                let value = secrets::resolve_maybe_secret_ref(reference).await?;
                let escaped = value.replace('\\', "\\\\").replace('"', "\\\"");
                lines.push(format!("{name}=\"{escaped}\""));
            }
            if let Some(parent) = out.parent() {
                fs::create_dir_all(parent)?;
            }
            fs::write(&out, lines.join("\n") + "\n")?;
            output::print_value(
                output_mode,
                &json!({
                    "ok": true,
                    "profile": profile_name,
                    "out": out,
                    "count": cfg.refs.len(),
                }),
            );
            Ok(())
        }
        SecretsCommand::Infisical(cmd) => handle_infisical_profiles(cmd, output_mode),
    }
}

async fn handle_secrets_provider(
    command: SecretsProviderCommand,
    output_mode: OutputMode,
) -> Result<()> {
    match command {
        SecretsProviderCommand::Env { profile } => {
            let profile_name = profile.trim();
            let mut cfg = secrets::load_profile_secrets(profile_name);
            cfg.provider = Some("env".to_string());
            cfg.infisical_project_id = None;
            cfg.infisical_env = None;
            cfg.infisical_path = None;
            secrets::save_profile_secrets(profile_name, &cfg)?;
            output::print_value(
                output_mode,
                &json!({
                    "ok": true,
                    "profile": profile_name,
                    "provider": "env",
                    "path": secrets::profile_secrets_path(profile_name),
                }),
            );
            Ok(())
        }
        SecretsProviderCommand::Infisical {
            profile,
            project_id,
            env,
            path,
        } => {
            let profile_name = profile.trim();
            let mut cfg = secrets::load_profile_secrets(profile_name);
            cfg.provider = Some("infisical".to_string());
            cfg.infisical_project_id = project_id.filter(|v| !v.trim().is_empty());
            cfg.infisical_env = env.filter(|v| !v.trim().is_empty());
            cfg.infisical_path = path.filter(|v| !v.trim().is_empty());
            secrets::save_profile_secrets(profile_name, &cfg)?;
            output::print_value(
                output_mode,
                &json!({
                    "ok": true,
                    "profile": profile_name,
                    "provider": "infisical",
                    "path": secrets::profile_secrets_path(profile_name),
                    "infisical_project_id": cfg.infisical_project_id,
                    "infisical_env": cfg.infisical_env,
                    "infisical_path": cfg.infisical_path,
                }),
            );
            Ok(())
        }
    }
}

fn handle_infisical_profiles(command: InfisicalProfileCommand, output_mode: OutputMode) -> Result<()> {
    let path = crate::config::ep_home_dir().join("infisical_profiles.json");
    let mut map = if path.exists() {
        let raw = fs::read_to_string(&path)?;
        serde_json::from_str::<edgeplaned_secrets::InfisicalProfileMap>(&raw)
            .unwrap_or_default()
    } else {
        edgeplaned_secrets::InfisicalProfileMap::default()
    };

    let save = |map: &edgeplaned_secrets::InfisicalProfileMap| -> Result<()> {
        if let Some(parent) = path.parent() { fs::create_dir_all(parent)?; }
        fs::write(&path, serde_json::to_string_pretty(map)?)?;
        Ok(())
    };

    match command {
        InfisicalProfileCommand::Add {
            name,
            site_url,
            service_token,
            client_id,
            client_secret,
            project_id,
            environment,
            activate,
        } => {
            use edgeplaned_secrets::InfisicalConfig;
            let cfg = InfisicalConfig {
                site_url,
                service_token: service_token.filter(|s| !s.is_empty()),
                client_id: client_id.filter(|s| !s.is_empty()),
                client_secret: client_secret.filter(|s| !s.is_empty()),
                default_project_id: project_id.filter(|s| !s.is_empty()),
                default_environment: environment,
            };
            map.upsert(name.clone(), cfg);
            if activate {
                let _ = map.set_active(&name);
            }
            save(&map)?;
            output::print_value(output_mode, &json!({
                "ok": true,
                "name": name,
                "active": map.active,
                "path": path,
            }));
        }
        InfisicalProfileCommand::List => {
            let profiles: Vec<serde_json::Value> = map.profiles.iter().map(|(name, cfg)| {
                json!({
                    "name": name,
                    "active": map.active.as_deref() == Some(name),
                    "site_url": cfg.site_url,
                    "auth_type": if cfg.service_token.is_some() { "service_token" } else { "universal_auth" },
                    "configured": cfg.is_configured(),
                    "default_project_id": cfg.default_project_id,
                    "default_environment": cfg.default_environment,
                })
            }).collect();
            output::print_value(output_mode, &json!({ "profiles": profiles, "active": map.active }));
        }
        InfisicalProfileCommand::Use { name } => {
            map.set_active(&name).map_err(|e| anyhow::anyhow!("{e}"))?;
            save(&map)?;
            output::print_value(output_mode, &json!({ "ok": true, "active": name }));
        }
        InfisicalProfileCommand::Test { name } => {
            let profile_name = name.as_deref().or(map.active.as_deref());
            let Some(profile_name) = profile_name else {
                anyhow::bail!("no active profile — run `edgeplane secrets infisical add <name>` first");
            };
            let cfg = map.profiles.get(profile_name)
                .ok_or_else(|| anyhow::anyhow!("profile '{profile_name}' not found"))?;
            // Quick connectivity test: fetch /api/v3/auth endpoint (no auth)
            let test_url = format!("{}/api/v1/auth/token/renew", cfg.site_url.trim_end_matches('/'));
            let rt = tokio::runtime::Handle::current();
            let resp = rt.block_on(async {
                reqwest::Client::builder()
                    .timeout(std::time::Duration::from_secs(10))
                    .build()
                    .unwrap()
                    .post(&test_url)
                    .send()
                    .await
            });
            let reachable = matches!(resp, Ok(r) if r.status().as_u16() < 500);
            output::print_value(output_mode, &json!({
                "ok": reachable,
                "profile": profile_name,
                "site_url": cfg.site_url,
                "reachable": reachable,
            }));
        }
        InfisicalProfileCommand::Rm { name } => {
            let removed = map.remove(&name);
            if removed { save(&map)?; }
            output::print_value(output_mode, &json!({
                "ok": removed,
                "name": name,
                "active": map.active,
            }));
        }
        InfisicalProfileCommand::Get {
            secret_name,
            profile,
            project_id,
            environment,
            path,
            reveal,
        } => {
            // Resolve which profile config to use.
            let cfg = match &profile {
                Some(n) => map.profiles.get(n).cloned()
                    .ok_or_else(|| anyhow::anyhow!("profile '{}' not found", n))?,
                None => map.active_profile().cloned()
                    .ok_or_else(|| anyhow::anyhow!("no active Infisical profile — run: edgeplane secrets infisical add <name> --activate"))?,
            };

            let proj = project_id
                .as_deref()
                .or(cfg.default_project_id.as_deref())
                .unwrap_or("")
                .to_string();
            let env_slug = environment.unwrap_or_else(|| cfg.default_environment.clone());

            let client = edgeplaned_secrets::InfisicalClient::new(&cfg)?;
            let rt = tokio::runtime::Handle::current();
            let value = rt.block_on(client.fetch_secret(&secret_name, &proj, &env_slug, &path))?;

            let display = if reveal || output_mode.is_machine() {
                value.clone()
            } else {
                "***redacted***".to_string()
            };
            output::print_value(output_mode, &json!({
                "ok": true,
                "secret_name": secret_name,
                "project_id": proj,
                "environment": env_slug,
                "path": path,
                "value": display,
            }));
        }
    }
    Ok(())
}

async fn handle_data(
    command: DataCommand,
    client: EdgeplaneClient,
    booster: &AgentBooster,
    schema_pack: &SchemaPack,
    output_mode: OutputMode,
) -> Result<()> {
    match command {
        DataCommand::Tools(cmd) => {
            handle_tools(cmd, client, booster, schema_pack, output_mode).await
        }
        DataCommand::Sync(cmd) => handle_sync(cmd, client).await,
        DataCommand::Explorer(cmd) => handle_explorer(cmd, client).await,
    }
}

async fn handle_system(
    command: SystemCommand,
    client: EdgeplaneClient,
    config: &McConfig,
) -> Result<()> {
    match command {
        SystemCommand::Doctor(args) => {
            maintenance::run_doctor_command(&client, config, &args).await
        }
        SystemCommand::Backup(args) => maintenance::run_backup_command(&client, args).await,
        SystemCommand::ProfileGc(args) => maintenance::run_profile_gc_command(config, args),
        SystemCommand::Update(cmd) => update::run(cmd, config).await,
        SystemCommand::Compat(cmd) => compat::run(cmd).await,
        SystemCommand::Drift(cmd) => drift::run(cmd).await,
    }
}

async fn handle_agent(
    command: AgentCommand,
    client: EdgeplaneClient,
    booster: &AgentBooster,
    schema_pack: &SchemaPack,
) -> Result<()> {
    let _ = (booster, schema_pack);
    match command {
        AgentCommand::Signal(args) => crate::agent_ops::run_signal(args, &client).await,
        AgentCommand::Cancel(args) => crate::agent_ops::run_cancel(args, &client).await,
        AgentCommand::List(args) => crate::agent_ops::run_list(args, &client).await,
        AgentCommand::Describe(args) => crate::agent_ops::run_describe(args, &client).await,
        AgentCommand::Node(cmd) => runtime::run_node_agent(cmd, &client).await,
        AgentCommand::Evolve(args) => evolve::run(args, &client).await,
        AgentCommand::Attach(args) => crate::attach::run(args, &client).await,
        AgentCommand::Cron(cmd) => match cmd {
            CronCommand::List(a) => crate::agent_cron::run_list(a).await,
            CronCommand::Describe(a) => crate::agent_cron::run_describe(a).await,
            CronCommand::Reload(a) => crate::agent_cron::run_reload(a).await,
            CronCommand::History(a) => crate::agent_cron::run_history(a).await,
            CronCommand::GcNow(a) => crate::agent_cron::run_gc_now(a).await,
        },
        AgentCommand::Supervise(cmd) => match cmd {
            SuperviseCommand::List(a) => crate::agent_supervise::run_list(a).await,
            SuperviseCommand::Status(a) => crate::agent_supervise::run_status(a).await,
            SuperviseCommand::Restart(a) => crate::agent_supervise::run_restart(a).await,
            SuperviseCommand::Pause(a) => crate::agent_supervise::run_pause(a).await,
            SuperviseCommand::Resume(a) => crate::agent_supervise::run_resume(a).await,
            SuperviseCommand::History(a) => crate::agent_supervise::run_history(a).await,
            SuperviseCommand::Events(a) => crate::agent_supervise::run_events(a).await,
            SuperviseCommand::Watch(a) => crate::agent_supervise_watch::run(a).await,
        },
    }
}

async fn handle_runtime(
    command: runtime::RuntimeCommand,
    client: EdgeplaneClient,
    output_mode: OutputMode,
) -> Result<()> {
    runtime::run(command, &client, output_mode).await
}

async fn handle_init(
    args: InitArgs,
    client: EdgeplaneClient,
    config: &McConfig,
    output_mode: OutputMode,
) -> Result<()> {
    // --repo bootstrap flow — runs independently of the MC backend.
    if let Some(repo_url) = args.repo.as_deref() {
        return cmd::init::run_from_repo(repo_url, Some(args.profile.as_str())).await;
    }

    let json_output = output_mode.is_machine();
    let profile_name = args.profile.trim();
    if profile_name.is_empty() {
        anyhow::bail!("--profile cannot be empty");
    }
    validate_init_base_url(config)?;

    let login_client_holder: Option<EdgeplaneClient> = if config.token.is_none() {
        if auth::load_saved_session(config.base_url.as_str()).is_none() {
            eprintln!(
                "edgeplane: no valid session found for {}",
                config.base_url.as_str()
            );
            eprintln!("edgeplane: running `edgeplane auth login` to authenticate...");
            if let Err(err) = auth::login(
                auth::LoginArgs {
                    ttl_hours: 8,
                    print_token: false,
                    non_interactive: false,
                    with_token: false,
                },
                &client,
                config.base_url.as_str(),
            )
            .await
            {
                return bootstrap_local_profile(
                    profile_name,
                    Some(format!(
                        "login failed; continuing in local-only mode: {err}"
                    )),
                    output_mode,
                );
            }
        }
        let session_token = auth::load_saved_session(config.base_url.as_str())
            .map(|s| s.token)
            .ok_or_else(|| anyhow::anyhow!("session not found after login"))?;
        Some(
            EdgeplaneClient::new_with_token(config.base_url.as_str(), &session_token)
                .context("failed to build client with session token")?,
        )
    } else {
        None
    };
    let effective_client: &EdgeplaneClient = login_client_holder.as_ref().unwrap_or(&client);

    let listed = match mcp_profile_call(effective_client, "list_profiles", json!({ "limit": 1 }))
        .await
    {
        Ok(v) => v,
        Err(err) => {
            return bootstrap_local_profile(
                profile_name,
                Some(format!(
                    "unable to reach Edgeplane profile service; continuing in local-only mode: {err}"
                )),
                output_mode,
            );
        }
    };
    let has_profiles = listed
        .get("profiles")
        .and_then(|v| v.as_array())
        .map(|arr| !arr.is_empty())
        .unwrap_or(false);

    if has_profiles {
        let activated = mcp_profile_call(
            effective_client,
            "activate_profile",
            json!({ "name": profile_name }),
        )
        .await;
        if activated.is_err() {
            let payload = json!({
                "ok": true,
                "created": false,
                "note": "profiles already exist; no bootstrap profile created",
            });
            if json_output {
                print_json(&payload);
            } else {
                println!(
                    "{}{}{} profiles already exist; no bootstrap profile created",
                    crate::ui::YELLOW,
                    "⚑ ",
                    crate::ui::RESET
                );
            }
            return Ok(());
        }
        write_synced_profile_state(profile_name)?;
        let payload = json!({
            "ok": true,
            "created": false,
            "activated_profile": profile_name,
        });
        if json_output {
            print_json(&payload);
        } else {
            println!(
                "{}{}{} profile active: {}{}{}",
                crate::ui::GREEN,
                "✓ ",
                crate::ui::RESET,
                crate::ui::CYAN,
                profile_name,
                crate::ui::RESET
            );
        }
        return Ok(());
    }

    let tarball_b64 = empty_profile_tarball_b64()?;
    let published = match mcp_profile_call(
        effective_client,
        "publish_profile",
        json!({
            "name": profile_name,
            "description": "Bootstrap profile created by edgeplane init",
            "is_default": true,
            "manifest": [],
            "tarball_b64": tarball_b64
        }),
    )
    .await
    {
        Ok(v) => v,
        Err(err) => {
            return bootstrap_local_profile(
                profile_name,
                Some(format!(
                    "failed to publish profile to Edgeplane; continuing in local-only mode: {err}"
                )),
                output_mode,
            );
        }
    };
    write_synced_profile_state(profile_name)?;
    let payload = json!({
        "ok": true,
        "created": true,
        "synced": true,
        "profile": published.get("profile").cloned().unwrap_or(Value::Null),
    });
    if json_output {
        print_json(&payload);
    } else {
        println!(
            "{}{}{} profile created + synced: {}{}{}",
            crate::ui::GREEN,
            "✓ ",
            crate::ui::RESET,
            crate::ui::CYAN,
            profile_name,
            crate::ui::RESET
        );
    }
    Ok(())
}

async fn handle_tools(
    command: ToolsCommand,
    client: EdgeplaneClient,
    booster: &AgentBooster,
    schema_pack: &SchemaPack,
    output_mode: OutputMode,
) -> Result<()> {
    match command {
        ToolsCommand::List => {
            let response = client.get_json("/mcp/tools").await?;
            if output_mode.is_machine() {
                output::print_value(output_mode, &response);
            } else {
                print_tools_human(&response);
            }
        }
        ToolsCommand::Call(args) => {
            let payload = serde_json::from_str::<Value>(&args.payload)
                .context("failed to parse payload JSON")?;
            let response = mcp_tools::call_tool(
                &client,
                Some(booster),
                Some(schema_pack),
                &args.tool,
                payload,
            )
            .await?;
            print_json(&response);
        }
    }
    Ok(())
}

fn print_json(value: &Value) {
    output::print_value(OutputMode::Json, value);
}

fn ui_section(title: &str) {
    println!();
    println!(
        "{}{}{}{}",
        crate::ui::BOLD,
        crate::ui::ORANGE,
        title,
        crate::ui::RESET
    );
    println!("{}{}{}", crate::ui::GRAY, "─".repeat(56), crate::ui::RESET);
}

fn ui_row(label: &str, value: &str, value_color: &str) {
    println!(
        "  {}{: <16}{} {}{}{}",
        crate::ui::DIM,
        format!("{}:", label),
        crate::ui::RESET,
        value_color,
        value,
        crate::ui::RESET
    );
}

fn print_tools_human(value: &Value) {
    let items: Vec<Value> = match value {
        Value::Array(arr) => arr.clone(),
        Value::Object(obj) => obj
            .get("tools")
            .and_then(|v| v.as_array())
            .cloned()
            .unwrap_or_default(),
        _ => Vec::new(),
    };
    ui_section("MCP Tools");
    ui_row("Count", &items.len().to_string(), crate::ui::CYAN);
    if items.is_empty() {
        println!(
            "  {}no tools returned{}",
            crate::ui::YELLOW,
            crate::ui::RESET
        );
        return;
    }
    for tool in items {
        let name = tool
            .get("name")
            .and_then(|v| v.as_str())
            .unwrap_or("unknown");
        let description = tool
            .get("description")
            .and_then(|v| v.as_str())
            .unwrap_or("");
        println!("  {}•{} {}", crate::ui::CYAN, crate::ui::RESET, name);
        if !description.is_empty() {
            println!("    {}{}{}", crate::ui::DIM, description, crate::ui::RESET);
        }
    }
}

fn print_profiles_human(value: &Value) {
    let Some(items) = value.as_array() else {
        print_json(value);
        return;
    };
    if items.is_empty() {
        println!("no profiles");
        return;
    }
    for p in items {
        let name = p.get("name").and_then(|v| v.as_str()).unwrap_or("unknown");
        let sha = p.get("sha256").and_then(|v| v.as_str()).unwrap_or("-");
        let is_default = p
            .get("is_default")
            .and_then(|v| v.as_bool())
            .unwrap_or(false);
        if is_default {
            println!(
                "{}*{} {}  {}",
                crate::ui::GREEN,
                crate::ui::RESET,
                name,
                sha
            );
        } else {
            println!("  {}  {}", name, sha);
        }
    }
}

fn print_profile_human(profile: &Value) {
    let name = profile
        .get("name")
        .and_then(|v| v.as_str())
        .unwrap_or("unknown");
    let sha = profile
        .get("sha256")
        .and_then(|v| v.as_str())
        .unwrap_or("-");
    let is_default = profile
        .get("is_default")
        .and_then(|v| v.as_bool())
        .unwrap_or(false);
    let default_marker = if is_default { " (default)" } else { "" };
    println!(
        "{}{}{}{}  {}",
        crate::ui::CYAN,
        name,
        crate::ui::RESET,
        default_marker,
        sha
    );
}

fn print_profile_status_human(value: &Value) {
    let name = value
        .get("profile")
        .and_then(|v| v.as_str())
        .unwrap_or("unknown");
    let remote_sha = value
        .get("remote_sha256")
        .and_then(|v| v.as_str())
        .unwrap_or("-");
    let local_pin = value
        .get("local_pin")
        .and_then(|v| v.get("pinned_sha256"))
        .and_then(|v| v.as_str())
        .unwrap_or("(none)");
    let synced = value
        .get("local_state")
        .and_then(|v| v.get("synced"))
        .and_then(|v| v.as_bool())
        .unwrap_or(false);
    println!("profile: {}{}{}", crate::ui::CYAN, name, crate::ui::RESET);
    println!("remote_sha256: {}", remote_sha);
    println!("local_pin: {}", local_pin);
    println!("synced: {}", synced);
}

fn build_path_with_query(base: &str, query: String) -> String {
    if query.is_empty() {
        base.to_string()
    } else {
        format!("{}?{}", base, query)
    }
}

async fn handle_sync(command: SyncCommand, client: EdgeplaneClient) -> Result<()> {
    match command {
        SyncCommand::Status(args) => {
            let mut serializer = form_urlencoded::Serializer::new(String::new());
            serializer.append_pair("domain_id", &args.domain_id);
            if let Some(mission) = &args.mission_id {
                serializer.append_pair("mission_id", mission);
            }
            if let Some(agent) = &args.agent_id {
                serializer.append_pair("agent_id", agent);
            }
            let path = build_path_with_query("/skills/sync/status", serializer.finish());
            let response = client.get_json(&path).await?;
            print_json(&response);
        }
        SyncCommand::Promote(args) => {
            let mut body = json!({
                "domain_id": args.domain_id,
                "snapshot_id": args.snapshot_id,
                "snapshot_sha256": args.snapshot_sha256,
                "degraded_offline": args.degraded_offline,
                "drift_flag": args.drift_flag,
            });
            if let Some(mission_id) = args.mission_id {
                body["mission_id"] = json!(mission_id);
            }
            if let Some(agent_id) = args.agent_id {
                body["agent_id"] = json!(agent_id);
            }
            if let Some(local_overlay) = args.local_overlay_sha256 {
                body["local_overlay_sha256"] = json!(local_overlay);
            }
            let drift_details: Value = serde_json::from_str(&args.drift_details)
                .context("failed to decode drift_details JSON")?;
            body["drift_details"] = drift_details;
            let response = client.post_json("/skills/sync/ack", &body).await?;
            print_json(&response);
        }
    }
    Ok(())
}

async fn handle_profile(
    command: ProfileCommand,
    client: EdgeplaneClient,
    output_mode: OutputMode,
) -> Result<()> {
    let json_output = output_mode.is_machine();
    match command {
        ProfileCommand::Create {
            name,
            description,
            activate,
        } => {
            let tarball_b64 = empty_profile_tarball_b64()?;
            let response = mcp_profile_call(
                &client,
                "publish_profile",
                json!({
                    "name": name,
                    "description": description,
                    "is_default": activate,
                    "manifest": [],
                    "tarball_b64": tarball_b64
                }),
            )
            .await?;
            let profile = response.get("profile").cloned().unwrap_or(Value::Null);
            if json_output {
                print_json(&profile);
            } else {
                let shown = profile
                    .get("name")
                    .and_then(|v| v.as_str())
                    .unwrap_or("unknown");
                println!(
                    "{}{}{} created profile: {}{}{}",
                    crate::ui::GREEN,
                    "✓ ",
                    crate::ui::RESET,
                    crate::ui::CYAN,
                    shown,
                    crate::ui::RESET
                );
            }
        }
        ProfileCommand::List { limit } => {
            let response =
                mcp_profile_call(&client, "list_profiles", json!({ "limit": limit })).await?;
            let profiles = response
                .get("profiles")
                .cloned()
                .unwrap_or_else(|| json!([]));
            if json_output {
                print_json(&profiles);
            } else {
                print_profiles_human(&profiles);
            }
        }
        ProfileCommand::Show { name } => {
            let response =
                mcp_profile_call(&client, "get_profile", json!({ "name": name })).await?;
            let profile = response.get("profile").cloned().unwrap_or(Value::Null);
            if json_output {
                print_json(&profile);
            } else {
                print_profile_human(&profile);
            }
        }
        ProfileCommand::Activate { name } => {
            let response =
                mcp_profile_call(&client, "activate_profile", json!({ "name": name })).await?;
            let profile = response.get("profile").cloned().unwrap_or(Value::Null);
            if json_output {
                print_json(&profile);
            } else {
                let shown = profile
                    .get("name")
                    .and_then(|v| v.as_str())
                    .unwrap_or(&name);
                println!(
                    "{}{}{} default profile: {}{}{}",
                    crate::ui::GREEN,
                    "✓ ",
                    crate::ui::RESET,
                    crate::ui::CYAN,
                    shown,
                    crate::ui::RESET
                );
            }
        }
        ProfileCommand::Download { name, out } => {
            let response =
                mcp_profile_call(&client, "download_profile", json!({ "name": name })).await?;
            let tarball = response
                .get("tarball_b64")
                .and_then(|v| v.as_str())
                .context("profile download response missing tarball_b64")?;
            let bytes = base64::engine::general_purpose::STANDARD
                .decode(tarball)
                .context("tarball_b64 is not valid base64")?;
            let out_path = out.unwrap_or_else(|| PathBuf::from(format!("{}.profile.tar", name)));
            std::fs::write(&out_path, bytes)
                .with_context(|| format!("failed to write {}", out_path.display()))?;
            let payload =
                json!({"ok": true, "profile": name, "out": out_path.display().to_string()});
            if json_output {
                print_json(&payload);
            } else {
                println!(
                    "{}{}{} downloaded profile {}{}{} -> {}",
                    crate::ui::GREEN,
                    "✓ ",
                    crate::ui::RESET,
                    crate::ui::CYAN,
                    name,
                    crate::ui::RESET,
                    out_path.display()
                );
            }
        }
        ProfileCommand::Publish {
            name,
            bundle,
            from_profile_dir,
            description,
            manifest_file,
            activate,
        } => {
            let bundle_bytes = if let Some(bundle_path) = bundle {
                fs::read(&bundle_path)
                    .with_context(|| format!("failed to read bundle {}", bundle_path.display()))?
            } else if let Some(profile_dir) = from_profile_dir {
                build_tar_from_dir(&profile_dir)?
            } else {
                anyhow::bail!("provide --bundle or --from-profile-dir");
            };
            let tarball_b64 = base64::engine::general_purpose::STANDARD.encode(bundle_bytes);
            let manifest = if let Some(path) = manifest_file {
                let raw = fs::read_to_string(&path)
                    .with_context(|| format!("failed to read manifest {}", path.display()))?;
                serde_json::from_str::<Value>(&raw).context("manifest file must be valid JSON")?
            } else {
                json!([])
            };
            let manifest = if manifest.is_array() {
                manifest
            } else {
                json!([manifest])
            };
            let response = mcp_profile_call(
                &client,
                "publish_profile",
                json!({
                "name": name,
                "description": description.unwrap_or_default(),
                "is_default": activate,
                "manifest": manifest,
                "tarball_b64": tarball_b64
                }),
            )
            .await?;
            let profile = response.get("profile").cloned().unwrap_or(Value::Null);
            if json_output {
                print_json(&profile);
            } else {
                let shown = profile
                    .get("name")
                    .and_then(|v| v.as_str())
                    .unwrap_or(&name);
                println!(
                    "{}{}{} published profile: {}{}{}",
                    crate::ui::GREEN,
                    "✓ ",
                    crate::ui::RESET,
                    crate::ui::CYAN,
                    shown,
                    crate::ui::RESET
                );
            }
        }
        ProfileCommand::Pull {
            name,
            apply,
            allow_pin_mismatch,
        } => {
            let profile_root = crate::config::ep_home_dir().join("profiles").join(&name);
            let mut pull_args = json!({ "name": name });
            if let Some(pinned_sha) = read_local_pinned_sha(&profile_root)? {
                if !allow_pin_mismatch {
                    pull_args["if_sha256"] = json!(pinned_sha);
                }
            }
            let response = mcp_profile_call(&client, "download_profile", pull_args).await?;
            let tarball = response
                .get("tarball_b64")
                .and_then(|v| v.as_str())
                .context("profile download response missing tarball_b64")?;
            let bytes = base64::engine::general_purpose::STANDARD
                .decode(tarball)
                .context("tarball_b64 is not valid base64")?;
            let sha = response
                .get("profile")
                .and_then(|v| v.get("sha256"))
                .and_then(|v| v.as_str())
                .unwrap_or("unknown");
            if let Some(pinned_sha) = read_local_pinned_sha(&profile_root)? {
                if pinned_sha != sha && !allow_pin_mismatch {
                    anyhow::bail!(
                        "profile '{}' is pinned to sha256 '{}' but remote is '{}'; rerun with --allow-pin-mismatch to override",
                        name,
                        pinned_sha,
                        sha
                    );
                }
            }
            let bundles = profile_root.join("bundles");
            fs::create_dir_all(&bundles)?;
            let tar_path = bundles.join(format!("{}.tar", sha));
            fs::write(&tar_path, bytes)?;
            if apply {
                extract_tar_to_dir(&tar_path, &profile_root)?;
            }
            let state = json!({
                "profile": name,
                "last_pulled_sha256": sha,
                "bundle_path": tar_path.display().to_string(),
                "applied": apply,
                "pulled_at": chrono::Utc::now().to_rfc3339(),
            });
            fs::write(
                profile_root.join("state.json"),
                serde_json::to_string_pretty(&state)?,
            )?;
            let payload = json!({
                "ok": true,
                "profile": name,
                "last_pulled_sha256": sha,
                "bundle_path": tar_path.display().to_string(),
                "applied": apply,
            });
            if json_output {
                print_json(&payload);
            } else {
                println!(
                    "{}{}{} pulled profile {}{}{} @ {}",
                    crate::ui::GREEN,
                    "✓ ",
                    crate::ui::RESET,
                    crate::ui::CYAN,
                    name,
                    crate::ui::RESET,
                    sha
                );
            }
        }
        ProfileCommand::Pin { name, sha256 } => {
            let profile_root = crate::config::ep_home_dir().join("profiles").join(&name);
            fs::create_dir_all(&profile_root)?;
            let pin = json!({
                "profile": name,
                "pinned_sha256": sha256,
                "pinned_at": chrono::Utc::now().to_rfc3339(),
            });
            fs::write(
                profile_root.join("pin.json"),
                serde_json::to_string_pretty(&pin)?,
            )?;
            let payload = json!({"ok": true, "profile": name, "pinned_sha256": sha256});
            if json_output {
                print_json(&payload);
            } else {
                println!(
                    "{}{}{} pinned {}{}{} -> {}",
                    crate::ui::GREEN,
                    "✓ ",
                    crate::ui::RESET,
                    crate::ui::CYAN,
                    name,
                    crate::ui::RESET,
                    sha256
                );
            }
        }
        ProfileCommand::Delete {
            name,
            confirm_delete,
        } => {
            if !confirm_delete {
                anyhow::bail!(
                    "refusing to delete profile '{}'; rerun with --confirm-delete",
                    name
                );
            }
            let response =
                mcp_profile_call(&client, "delete_profile", json!({ "name": name })).await?;
            if json_output {
                print_json(&response);
            } else {
                println!(
                    "{}{}{} deleted profile: {}{}{}",
                    crate::ui::GREEN,
                    "✓ ",
                    crate::ui::RESET,
                    crate::ui::CYAN,
                    name,
                    crate::ui::RESET
                );
            }
        }
        ProfileCommand::Status { name } => {
            let remote = mcp_profile_call(&client, "get_profile", json!({ "name": name })).await?;
            let remote_sha = remote
                .get("profile")
                .and_then(|v| v.get("sha256"))
                .cloned()
                .unwrap_or(Value::Null);
            let profile_root = crate::config::ep_home_dir().join("profiles").join(&name);
            let local_pin_path = profile_root.join("pin.json");
            let local_state_path = profile_root.join("state.json");
            let local_pin = if local_pin_path.exists() {
                serde_json::from_str::<Value>(&fs::read_to_string(&local_pin_path)?)
                    .unwrap_or(Value::Null)
            } else {
                Value::Null
            };
            let local_state = if local_state_path.exists() {
                serde_json::from_str::<Value>(&fs::read_to_string(&local_state_path)?)
                    .unwrap_or(Value::Null)
            } else {
                Value::Null
            };
            let pin_check = if let Some(pinned) = local_pin
                .get("pinned_sha256")
                .and_then(|v| v.as_str())
                .map(str::to_string)
            {
                mcp_profile_call(
                    &client,
                    "pin_profile_version",
                    json!({ "name": name, "sha256": pinned }),
                )
                .await
                .unwrap_or_else(|_| json!({"matches": false}))
            } else {
                Value::Null
            };
            let payload = json!({
                "profile": name,
                "remote_sha256": remote_sha,
                "local_pin": local_pin,
                "local_state": local_state,
                "pin_check": pin_check
            });
            if json_output {
                print_json(&payload);
            } else {
                print_profile_status_human(&payload);
            }
        }
        ProfileCommand::Use { name } => {
            // 1. Mark as default on the backend.
            mcp_profile_call(&client, "activate_profile", json!({ "name": name })).await?;

            // 2. Download bundle.
            let response =
                mcp_profile_call(&client, "download_profile", json!({ "name": name })).await?;
            let tarball = response
                .get("tarball_b64")
                .and_then(|v| v.as_str())
                .context("profile download response missing tarball_b64")?;
            let bytes = base64::engine::general_purpose::STANDARD
                .decode(tarball)
                .context("tarball_b64 is not valid base64")?;
            let sha = response
                .get("profile")
                .and_then(|v| v.get("sha256"))
                .and_then(|v| v.as_str())
                .unwrap_or("unknown");

            // 3. Store bundle tarball and extract into the live profile directory.
            let profile_root = crate::config::ep_home_dir().join("profiles").join(&name);
            let bundles = profile_root.join("bundles");
            fs::create_dir_all(&bundles)?;
            let tar_path = bundles.join(format!("{}.tar", sha));
            fs::write(&tar_path, &bytes)?;
            extract_tar_to_dir(&tar_path, &profile_root)?;

            // 4. Write state.
            write_synced_profile_state(&name)?;

            // 5. Notify any live sessions using this profile.
            let marker_json = serde_json::to_string(&json!({
                "additionalContext": format!(
                    "⚠ Profile '{}' was updated (sha256: {}). \
                     File-based config (CLAUDE.md, agents) is live now via symlinks. \
                     MCP server changes require a restart — type /exit and relaunch with `edgeplane launch`.",
                    name, sha
                )
            }))?;
            let active_sessions = crate::launch::sessions_for_profile(&name);
            let mut notified = 0usize;
            for session in &active_sessions {
                let ep_dir = PathBuf::from(&session.instance_home).join("edgeplane");
                if ep_dir.exists() {
                    if fs::write(ep_dir.join("profile-updated"), &marker_json).is_ok() {
                        notified += 1;
                    }
                }
            }

            let payload = json!({
                "ok": true,
                "profile": name,
                "sha256": sha,
                "sessions_notified": notified,
            });
            if json_output {
                print_json(&payload);
            } else {
                println!(
                    "{}{}{} switched to profile: {}{}{}",
                    crate::ui::GREEN,
                    "✓ ",
                    crate::ui::RESET,
                    crate::ui::CYAN,
                    name,
                    crate::ui::RESET
                );
                if notified > 0 {
                    println!(
                        "{}  {} session(s) will be prompted to restart on next message{}",
                        crate::ui::YELLOW,
                        notified,
                        crate::ui::RESET
                    );
                }
            }
        }
    }
    Ok(())
}

async fn mcp_profile_call(client: &EdgeplaneClient, tool: &str, args: Value) -> Result<Value> {
    let response = client
        .post_json("/mcp/call", &json!({ "tool": tool, "args": args }))
        .await?;
    let ok = response
        .get("ok")
        .and_then(|v| v.as_bool())
        .unwrap_or(false);
    if !ok {
        let err = response
            .get("error")
            .and_then(|v| v.as_str())
            .unwrap_or("mcp profile tool failed");
        anyhow::bail!("{}", err);
    }
    Ok(response.get("result").cloned().unwrap_or_else(|| json!({})))
}

fn build_tar_from_dir(dir: &PathBuf) -> Result<Vec<u8>> {
    if !dir.exists() || !dir.is_dir() {
        anyhow::bail!("profile dir not found: {}", dir.display());
    }
    let mut out = Vec::<u8>::new();
    {
        let mut builder = Builder::new(&mut out);
        builder.append_dir_all(".", dir)?;
        builder.finish()?;
    }
    Ok(out)
}

fn extract_tar_to_dir(tar_path: &PathBuf, out_dir: &PathBuf) -> Result<()> {
    let bytes = fs::read(tar_path)?;
    let cursor = Cursor::new(bytes);
    let mut archive = Archive::new(cursor);
    archive.unpack(out_dir)?;
    Ok(())
}

fn read_local_pinned_sha(profile_root: &std::path::Path) -> Result<Option<String>> {
    let pin_path = profile_root.join("pin.json");
    if !pin_path.exists() {
        return Ok(None);
    }
    let value: Value = serde_json::from_str(&fs::read_to_string(&pin_path)?)
        .context("pin.json is not valid JSON")?;
    Ok(value
        .get("pinned_sha256")
        .and_then(|v| v.as_str())
        .map(|v| v.to_string()))
}

fn empty_profile_tarball_b64() -> Result<String> {
    let mut bytes = Vec::<u8>::new();
    {
        let mut builder = Builder::new(&mut bytes);
        builder.finish()?;
    }
    Ok(base64::engine::general_purpose::STANDARD.encode(bytes))
}

fn validate_init_base_url(config: &McConfig) -> Result<()> {
    let base = config.base_url.as_str();
    let parsed = url::Url::parse(base).with_context(|| format!("invalid MC base URL: {base}"))?;
    let scheme = parsed.scheme();
    if scheme != "http" && scheme != "https" {
        anyhow::bail!("MC base URL must use http/https, got '{}'", scheme);
    }
    if parsed.host_str().unwrap_or("").trim().is_empty() {
        anyhow::bail!("MC base URL missing host: {}", base);
    }
    if std::env::var("EP_BASE_URL").is_err() && base == "http://localhost:8008" {
        eprintln!(
            "edgeplane: warning: EP_BASE_URL is not set; using default {}",
            base
        );
    }
    Ok(())
}

fn write_synced_profile_state(profile_name: &str) -> Result<()> {
    let profile_root = crate::config::ep_home_dir()
        .join("profiles")
        .join(profile_name);
    fs::create_dir_all(&profile_root)?;
    let payload = json!({
        "profile": profile_name,
        "mode": "remote",
        "synced": true,
        "updated_at": chrono::Utc::now().to_rfc3339(),
    });
    fs::write(
        profile_root.join("state.json"),
        serde_json::to_string_pretty(&payload)?,
    )?;
    Ok(())
}

fn bootstrap_local_profile(
    profile_name: &str,
    warning: Option<String>,
    output_mode: OutputMode,
) -> Result<()> {
    let json_output = output_mode.is_machine();
    let profile_root = crate::config::ep_home_dir()
        .join("profiles")
        .join(profile_name);
    fs::create_dir_all(&profile_root)?;
    let payload = json!({
        "profile": profile_name,
        "mode": "local_only",
        "synced": false,
        "warning": warning.clone().unwrap_or_else(|| "local-only bootstrap".to_string()),
        "created_at": chrono::Utc::now().to_rfc3339(),
    });
    fs::write(
        profile_root.join("state.json"),
        serde_json::to_string_pretty(&payload)?,
    )?;
    if let Some(w) = warning {
        eprintln!("edgeplane: warning: {}", w);
        eprintln!(
            "edgeplane: warning: this profile is local-only and will not sync until Edgeplane connectivity/auth is fixed"
        );
    }
    let payload = json!({
        "ok": true,
        "created": true,
        "synced": false,
        "profile": profile_name,
        "mode": "local_only"
    });
    if json_output {
        print_json(&payload);
    } else {
        println!(
            "{}{}{} local-only profile initialized: {}{}{}",
            crate::ui::YELLOW,
            "⚑ ",
            crate::ui::RESET,
            crate::ui::CYAN,
            profile_name,
            crate::ui::RESET
        );
    }
    Ok(())
}

async fn handle_workspace(
    command: WorkspaceCommand,
    client: EdgeplaneClient,
    booster: &AgentBooster,
    schema_pack: &SchemaPack,
    output_mode: OutputMode,
) -> Result<()> {
    let json_output = output_mode.is_machine();
    match command {
        WorkspaceCommand::Load {
            mission_id,
            workspace_label,
            agent_id,
            lease_seconds,
        } => {
            let mut args = json!({
                "mission_id": mission_id,
                "lease_seconds": lease_seconds,
            });
            if let Some(label) = workspace_label {
                args["workspace_label"] = json!(label);
            }
            if let Some(agent) = agent_id {
                args["agent_id"] = json!(agent);
            }
            let response = call_mcp_tool(
                &client,
                booster,
                schema_pack,
                "load_mission_workspace",
                args,
            )
            .await?;
            if json_output {
                print_json(&response);
            } else {
                print_workspace_human("Workspace Lease", &response);
            }
        }
        WorkspaceCommand::Heartbeat { lease_id } => {
            let args = json!({"lease_id": lease_id});
            let response = call_mcp_tool(
                &client,
                booster,
                schema_pack,
                "heartbeat_workspace_lease",
                args,
            )
            .await?;
            if json_output {
                print_json(&response);
            } else {
                print_workspace_human("Lease Heartbeat", &response);
            }
        }
        WorkspaceCommand::FetchArtifact {
            lease_id,
            artifact_id,
            mode,
            expires_seconds,
            out,
        } => {
            let args = json!({
                "lease_id": lease_id,
                "artifact_id": artifact_id,
                "mode": mode,
                "expires_seconds": expires_seconds,
            });
            let mut response = call_mcp_tool(
                &client,
                booster,
                schema_pack,
                "fetch_workspace_artifact",
                args,
            )
            .await?;
            if let Some(out_path) = out {
                if mode.trim().eq_ignore_ascii_case("content") {
                    let content_b64 = response
                        .get("content_b64")
                        .and_then(|v| v.as_str())
                        .context("artifact content response missing content_b64")?;
                    let bytes = base64::engine::general_purpose::STANDARD
                        .decode(content_b64)
                        .context("artifact content_b64 is not valid base64")?;
                    if let Some(parent) = out_path.parent() {
                        fs::create_dir_all(parent).with_context(|| {
                            format!("failed to create parent directory {}", parent.display())
                        })?;
                    }
                    fs::write(&out_path, &bytes)
                        .with_context(|| format!("failed to write {}", out_path.display()))?;
                    response["written_path"] = json!(out_path.display().to_string());
                    response["written_size_bytes"] = json!(bytes.len());
                } else {
                    anyhow::bail!("--out requires --mode content");
                }
            }
            if json_output {
                print_json(&response);
            } else {
                print_workspace_human("Artifact Fetch", &response);
            }
        }
        WorkspaceCommand::Commit {
            lease_id,
            change_set,
            validation_mode,
        } => {
            let change_set_value: Value =
                serde_json::from_str(&change_set).context("change_set must be valid JSON array")?;
            let mut args = json!({
                "lease_id": lease_id,
                "change_set": change_set_value,
            });
            if let Some(mode) = validation_mode {
                args["validation_mode"] = json!(mode);
            }
            let response = call_mcp_tool(
                &client,
                booster,
                schema_pack,
                "commit_mission_workspace",
                args,
            )
            .await?;
            if json_output {
                print_json(&response);
            } else {
                print_workspace_human("Workspace Commit", &response);
            }
        }
        WorkspaceCommand::Release { lease_id, reason } => {
            let mut args = json!({"lease_id": lease_id});
            if let Some(reason_value) = reason {
                args["reason"] = json!(reason_value);
            }
            let response = call_mcp_tool(
                &client,
                booster,
                schema_pack,
                "release_mission_workspace",
                args,
            )
            .await?;
            if json_output {
                print_json(&response);
            } else {
                print_workspace_human("Lease Release", &response);
            }
        }
    }
    Ok(())
}

fn print_workspace_human(title: &str, value: &Value) {
    ui_section(title);
    if let Some(lease) = value.get("lease_id").and_then(|v| v.as_str()) {
        ui_row("Lease", lease, crate::ui::CYAN);
    }
    if let Some(status) = value.get("status").and_then(|v| v.as_str()) {
        let color = match status {
            "active" | "ok" | "success" => crate::ui::GREEN,
            "released" => crate::ui::YELLOW,
            _ => crate::ui::CYAN,
        };
        ui_row("Status", status, color);
    }
    if let Some(url) = value.get("download_url").and_then(|v| v.as_str()) {
        ui_row("Download URL", url, crate::ui::CYAN);
    }
    println!(
        "  {}Result:{} {}",
        crate::ui::DIM,
        crate::ui::RESET,
        serde_json::to_string_pretty(value).unwrap_or_else(|_| value.to_string())
    );
}

async fn handle_explorer(command: ExplorerCommand, client: EdgeplaneClient) -> Result<()> {
    match command {
        ExplorerCommand::Tree(args) => {
            let mut serializer = form_urlencoded::Serializer::new(String::new());
            if let Some(domain_id) = &args.domain_id {
                serializer.append_pair("domain_id", domain_id);
            }
            if let Some(status) = &args.status {
                serializer.append_pair("status", status);
            }
            if let Some(query) = &args.q {
                serializer.append_pair("q", query);
            }
            if let Some(limit) = args.limit_tasks_per_cluster {
                serializer.append_pair("limit_tasks_per_cluster", &limit.to_string());
            }
            if let Some(limit) = args.limit_missions {
                serializer.append_pair("limit_missions", &limit.to_string());
            }
            let path = build_path_with_query("/explorer/tree", serializer.finish());
            let response = client.get_json(&path).await?;
            print_json(&response);
        }
        ExplorerCommand::Node(args) => {
            let path = format!(
                "/explorer/node/{}/{}",
                args.node_type.as_str(),
                args.node_id
            );
            let response = client.get_json(&path).await?;
            print_json(&response);
        }
    }
    Ok(())
}

async fn handle_admin(command: AdminCommand, client: EdgeplaneClient) -> Result<()> {
    match command {
        AdminCommand::Policy(AdminPolicyCommand::Active) => {
            let response = client.get_json("/governance/policy/active").await?;
            print_json(&response);
        }
        AdminCommand::Policy(AdminPolicyCommand::Versions { limit }) => {
            let mut serializer = form_urlencoded::Serializer::new(String::new());
            if let Some(limit) = limit {
                serializer.append_pair("limit", &limit.to_string());
            }
            let path = build_path_with_query("/governance/policy/versions", serializer.finish());
            let response = client.get_json(&path).await?;
            print_json(&response);
        }
        AdminCommand::Policy(AdminPolicyCommand::Events { limit }) => {
            let mut serializer = form_urlencoded::Serializer::new(String::new());
            if let Some(limit) = limit {
                serializer.append_pair("limit", &limit.to_string());
            }
            let path = build_path_with_query("/governance/policy/events", serializer.finish());
            let response = client.get_json(&path).await?;
            print_json(&response);
        }
        AdminCommand::Governance(cmd) => governance::run(cmd, &client).await?,
    }
    Ok(())
}

async fn handle_approvals(
    command: ApprovalCommand,
    client: EdgeplaneClient,
    output_mode: OutputMode,
) -> Result<()> {
    let json_output = output_mode.is_machine();
    match command {
        ApprovalCommand::Create {
            domain_id,
            action,
            channel,
            reason,
            target_entity_type,
            target_entity_id,
            request_context,
            expires_in_seconds,
        } => {
            let mut body = json!({
                "domain_id": domain_id,
                "action": action,
            });
            if let Some(channel) = channel {
                body["channel"] = json!(channel);
            }
            if let Some(reason) = reason {
                body["reason"] = json!(reason);
            }
            if let Some(target_type) = target_entity_type {
                body["target_entity_type"] = json!(target_type);
            }
            if let Some(target_id) = target_entity_id {
                body["target_entity_id"] = json!(target_id);
            }
            if let Some(req_ctx) = request_context {
                body["request_context"] =
                    serde_json::from_str(&req_ctx).unwrap_or_else(|_| json!({ "raw": req_ctx }));
            }
            if let Some(expires) = expires_in_seconds {
                body["expires_in_seconds"] = json!(expires);
            }
            let response = client.post_json("/approvals/requests", &body).await?;
            if json_output {
                print_json(&response);
            } else {
                print_approval_single("Approval Request Created", &response);
            }
        }
        ApprovalCommand::List {
            domain_id,
            status,
            limit,
        } => {
            let mut serializer = form_urlencoded::Serializer::new(String::new());
            serializer.append_pair("domain_id", &domain_id);
            if let Some(status) = status {
                serializer.append_pair("status", &status);
            }
            if let Some(limit) = limit {
                serializer.append_pair("limit", &limit.to_string());
            }
            let path = build_path_with_query("/approvals", serializer.finish());
            let response = client.get_json(&path).await?;
            if json_output {
                print_json(&response);
            } else {
                print_approvals_list_human(&response);
            }
        }
        ApprovalCommand::Approve {
            approval_id,
            expires_in_seconds,
            note,
        } => {
            let mut body = json!({});
            if let Some(expires) = expires_in_seconds {
                body["expires_in_seconds"] = json!(expires);
            }
            if let Some(note) = note {
                body["note"] = json!(note);
            }
            let path = format!("/approvals/{}/approve", approval_id);
            let response = client.post_json(&path, &body).await?;
            if json_output {
                print_json(&response);
            } else {
                print_approval_single("Approval Granted", &response);
            }
        }
        ApprovalCommand::Reject { approval_id, note } => {
            let mut body = json!({});
            if let Some(note) = note {
                body["note"] = json!(note);
            }
            let path = format!("/approvals/{}/reject", approval_id);
            let response = client.post_json(&path, &body).await?;
            if json_output {
                print_json(&response);
            } else {
                print_approval_single("Approval Rejected", &response);
            }
        }
    }
    Ok(())
}

fn print_approvals_list_human(value: &Value) {
    let items: Vec<Value> = match value {
        Value::Array(arr) => arr.clone(),
        Value::Object(obj) => obj
            .get("items")
            .and_then(|v| v.as_array())
            .cloned()
            .or_else(|| obj.get("approvals").and_then(|v| v.as_array()).cloned())
            .unwrap_or_default(),
        _ => Vec::new(),
    };
    ui_section("Approvals");
    ui_row("Count", &items.len().to_string(), crate::ui::CYAN);
    for item in items {
        let id = item.get("id").and_then(|v| v.as_i64()).unwrap_or_default();
        let domain_id = item
            .get("domain_id")
            .and_then(|v| v.as_str())
            .unwrap_or("-");
        let action = item.get("action").and_then(|v| v.as_str()).unwrap_or("-");
        let status = item.get("status").and_then(|v| v.as_str()).unwrap_or("-");
        let status_color = match status {
            "approved" => crate::ui::GREEN,
            "rejected" => crate::ui::RED,
            "pending" => crate::ui::YELLOW,
            _ => crate::ui::CYAN,
        };
        println!(
            "  {}#{}{} {}{}{} {}{}{} {}{}{}",
            crate::ui::DIM,
            id,
            crate::ui::RESET,
            crate::ui::CYAN,
            domain_id,
            crate::ui::RESET,
            crate::ui::BOLD,
            action,
            crate::ui::RESET,
            status_color,
            status,
            crate::ui::RESET
        );
    }
}

fn print_approval_single(title: &str, value: &Value) {
    ui_section(title);
    if let Some(id) = value.get("id").and_then(|v| v.as_i64()) {
        ui_row("ID", &id.to_string(), crate::ui::CYAN);
    }
    if let Some(status) = value.get("status").and_then(|v| v.as_str()) {
        let color = match status {
            "approved" => crate::ui::GREEN,
            "rejected" => crate::ui::RED,
            "pending" => crate::ui::YELLOW,
            _ => crate::ui::CYAN,
        };
        ui_row("Status", status, color);
    }
    if let Some(action) = value.get("action").and_then(|v| v.as_str()) {
        ui_row("Action", action, crate::ui::BOLD);
    }
    if let Some(domain) = value.get("domain_id").and_then(|v| v.as_str()) {
        ui_row("Domain", domain, crate::ui::CYAN);
    }
}

async fn call_mcp_tool(
    client: &EdgeplaneClient,
    booster: &AgentBooster,
    schema_pack: &SchemaPack,
    tool: &str,
    args: Value,
) -> Result<Value> {
    mcp_tools::call_tool(client, Some(booster), Some(schema_pack), tool, args).await
}

// ── edgeplane domain ────────────────────────────────────────────────────────────────

async fn handle_domain(cmd: DomainCommand, client: EdgeplaneClient, config: &McConfig) -> Result<()> {
    // Resolve the agent id for this runtime. Prefer the configured agent_id;
    // fall back to the default derived from the session state file.
    let agent_id = config
        .agent_context
        .agent_id
        .clone()
        .or_else(|| crate::config::default_agent_id_from_session(config.base_url.as_str()))
        .context("No agent_id configured. Run `edgeplane init` or set EP_AGENT_ID.")?;

    match cmd {
        DomainCommand::Home => {
            let agent = client.get_json(&format!("/agents/{agent_id}")).await?;
            let home_id   = agent.get("home_domain_id").and_then(|v| v.as_str()).unwrap_or("—");
            let curr_id   = agent.get("current_domain_id").and_then(|v| v.as_str()).unwrap_or("—");
            let miss_name = agent.get("domain_name").and_then(|v| v.as_str()).unwrap_or("—");
            println!("Home domain : {home_id}");
            println!("Current      : {curr_id}  ({miss_name})");
        }
        DomainCommand::Attach { domain_id } => {
            let body = json!({ "domain_id": domain_id });
            let agent = client.patch_json(&format!("/agents/{agent_id}/domain"), &body).await?;
            let curr  = agent.get("current_domain_id").and_then(|v| v.as_str()).unwrap_or("—");
            let name  = agent.get("domain_name").and_then(|v| v.as_str()).unwrap_or("—");
            println!("Attached to domain {curr} ({name})");
        }
        DomainCommand::Detach => {
            let body = json!({ "domain_id": null });
            let agent = client.patch_json(&format!("/agents/{agent_id}/domain"), &body).await?;
            let curr  = agent.get("current_domain_id").and_then(|v| v.as_str()).unwrap_or("—");
            let name  = agent.get("domain_name").and_then(|v| v.as_str()).unwrap_or("—");
            println!("Detached — returned to home domain {curr} ({name})");
        }
    }
    Ok(())
}
