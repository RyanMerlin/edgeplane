use crate::{claude, codex, config::McConfig, gemini, goose};
use crate::client::MissionControlClient;
use anyhow::{Result, bail};
use clap::{Args, ValueEnum};

#[derive(ValueEnum, Debug, Clone, Default)]
pub enum RunAction {
    /// Launch the runtime (default).
    #[default]
    Launch,
    /// Inspect and optionally repair runtime readiness.
    Doctor,
    /// Thin native execution — passes args verbatim to the runtime binary.
    Exec,
    /// Read-only runtime status (codex only).
    Status,
    /// Internal Claude lifecycle hook dispatcher.
    #[value(hide = true)]
    Hook,
}

#[derive(ValueEnum, Debug, Clone, Default)]
pub enum RunMode {
    #[default]
    Interactive,
    Headless,
    Solo,
}

#[derive(Args, Debug)]
pub struct RunArgs {
    /// Runtime to launch: claude, codex, gemini, goose.
    #[arg(value_name = "RUNTIME")]
    pub runtime: String,

    /// Action to perform (default: launch).
    #[arg(value_name = "ACTION", default_value = "launch")]
    pub action: RunAction,

    /// Profile name.
    #[arg(short = 'p', long = "profile")]
    pub profile: Option<String>,

    /// Force a new session instead of resuming the last one (launch action).
    #[arg(long, default_value_t = false)]
    pub new: bool,

    /// Non-interactive mode; fail rather than prompt.
    #[arg(long, default_value_t = false)]
    pub headless: bool,

    /// Bind to an existing mission — enables mesh participation via SoloSupervisor (launch action).
    #[arg(long)]
    pub mission: Option<String>,

    /// Bind to an existing kluster (launch action).
    #[arg(long)]
    pub kluster: Option<String>,

    /// Bind to an existing task (launch action).
    #[arg(long)]
    pub task: Option<String>,

    /// Execution mode (launch action).
    #[arg(long, value_enum, default_value = "interactive")]
    pub mode: RunMode,

    /// Apply safe deterministic repairs (doctor action).
    #[arg(long, default_value_t = false)]
    pub fix: bool,

    /// Emit machine-readable JSON output (doctor/status actions).
    #[arg(long, default_value_t = false)]
    pub json: bool,

    /// Hook event name: session-start, post-tool-use, session-end (hook action, internal).
    #[arg(long, hide = true)]
    pub event: Option<String>,

    /// Enable RTK token compression for this agent session (soft: warns if rtk not installed).
    #[arg(long, action = clap::ArgAction::SetTrue)]
    pub with_rtk: bool,

    /// Args forwarded verbatim to the runtime binary (after --).
    #[arg(last = true)]
    pub passthrough: Vec<String>,
}

pub async fn run(args: RunArgs, client: &MissionControlClient, config: &McConfig) -> Result<()> {
    let profile = args.profile.unwrap_or_else(|| "default".to_string());

    match args.action {
        RunAction::Launch => {
            dispatch_launch(
                args.runtime,
                profile,
                args.new,
                args.headless,
                args.mission,
                args.mode,
                args.with_rtk,
                args.passthrough,
                client,
                config,
            )
            .await
        }
        RunAction::Doctor => {
            dispatch_doctor(args.runtime, profile, args.fix, args.json, args.headless, config).await
        }
        RunAction::Exec => dispatch_exec(args.runtime, profile, args.passthrough, config).await,
        RunAction::Status => dispatch_status(args.runtime, profile, args.json, config).await,
        RunAction::Hook => {
            let event = args
                .event
                .ok_or_else(|| anyhow::anyhow!("--event is required for the hook action"))?;
            dispatch_hook(args.runtime, event, config).await
        }
    }
}

async fn dispatch_launch(
    runtime: String,
    profile: String,
    new: bool,
    headless: bool,
    mission: Option<String>,
    mode: RunMode,
    with_rtk: bool,
    passthrough: Vec<String>,
    client: &MissionControlClient,
    config: &McConfig,
) -> Result<()> {
    let use_solo = mission.is_some() || matches!(mode, RunMode::Solo);

    if use_solo {
        let mission_id = mission.unwrap_or_default();
        if mission_id.is_empty() {
            bail!("--mission <id> is required when --mode solo is set");
        }
        let config_clone = config.clone();
        let passthrough_clone = passthrough.clone();
        let profile_clone = profile.clone();

        match runtime.as_str() {
            "claude" => {
                let paths = claude::claude_paths(&profile);
                crate::solo_supervisor::run_solo_work_loop(
                    client,
                    &mission_id,
                    "claude_code",
                    &profile,
                    move |agent_id: &str, task_id: &str, task_md_path: &std::path::Path| {
                        claude::launch_claude_blocking(
                            &passthrough_clone,
                            &paths.runtime_home,
                            &config_clone,
                            &profile_clone,
                            agent_id,
                            None,
                            Some(task_id),
                            Some(task_md_path),
                        )
                    },
                )
                .await?;
            }
            "codex" => {
                let paths = codex::codex_paths(&profile);
                crate::solo_supervisor::run_solo_work_loop(
                    client,
                    &mission_id,
                    "codex",
                    &profile,
                    move |agent_id: &str, task_id: &str, task_md_path: &std::path::Path| {
                        codex::launch_codex_blocking(
                            &passthrough_clone,
                            &paths.runtime_home,
                            &config_clone,
                            &profile_clone,
                            agent_id,
                            None,
                            Some(task_id),
                            Some(task_md_path),
                        )
                    },
                )
                .await?;
            }
            "goose" => {
                let paths = goose::goose_paths(&profile);
                crate::solo_supervisor::run_solo_work_loop(
                    client,
                    &mission_id,
                    "goose",
                    &profile,
                    move |agent_id: &str, task_id: &str, task_md_path: &std::path::Path| {
                        goose::launch_goose_blocking(
                            &passthrough_clone,
                            &paths.runtime_home,
                            &config_clone,
                            &profile_clone,
                            agent_id,
                            None,
                            Some(task_id),
                            Some(task_md_path),
                        )
                    },
                )
                .await?;
            }
            other => bail!(
                "--mode solo is not yet supported for runtime '{}'; try claude, codex, or goose",
                other
            ),
        }
        return Ok(());
    }

    // Direct launch — no mesh participation.
    match runtime.as_str() {
        "claude" => claude::run_launch(profile, new, headless, with_rtk, passthrough, config).await,
        "codex" => codex::run_launch(profile, new, headless, with_rtk, passthrough, config).await,
        "gemini" => gemini::run_gemini_compat(Some(profile), with_rtk, passthrough, client, config).await,
        "goose" => goose::run_launch(profile, new, headless, with_rtk, passthrough, config).await,
        other => {
            eprintln!("mc: unknown runtime '{}'. Known: claude, codex, gemini, goose", other);
            std::process::exit(1);
        }
    }
}

async fn dispatch_doctor(
    runtime: String,
    profile: String,
    fix: bool,
    json: bool,
    headless: bool,
    config: &McConfig,
) -> Result<()> {
    match runtime.as_str() {
        "claude" => claude::run_doctor(profile, fix, json, headless, config).await,
        "codex" => codex::run_doctor(profile, fix, json, headless, config).await,
        "gemini" => bail!("gemini does not have a doctor command"),
        other => bail!("unknown runtime '{}'", other),
    }
}

async fn dispatch_exec(
    runtime: String,
    profile: String,
    passthrough: Vec<String>,
    config: &McConfig,
) -> Result<()> {
    match runtime.as_str() {
        "claude" => claude::run_exec(profile, passthrough, config).await,
        "codex" => codex::run_exec(profile, passthrough, config).await,
        "gemini" => bail!("gemini does not have an exec command"),
        "goose" => goose::run_exec(profile, passthrough, config).await,
        other => bail!("unknown runtime '{}'", other),
    }
}

async fn dispatch_status(
    runtime: String,
    profile: String,
    json: bool,
    config: &McConfig,
) -> Result<()> {
    match runtime.as_str() {
        "claude" => bail!(
            "claude does not have a status command; use `mc run claude doctor` for diagnostics"
        ),
        "codex" => codex::run_status(profile, json, config).await,
        "gemini" => bail!("gemini does not have a status command"),
        other => bail!("unknown runtime '{}'", other),
    }
}

async fn dispatch_hook(runtime: String, event: String, config: &McConfig) -> Result<()> {
    match runtime.as_str() {
        "claude" => claude::run_hook(event, config).await,
        other => bail!("hook is only supported for the claude runtime, got '{}'", other),
    }
}
