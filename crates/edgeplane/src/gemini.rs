use crate::{
    client::EdgeplaneClient,
    config::McConfig,
    launch::{self, LaunchArgs},
};
use anyhow::Result;

/// Launch Gemini in a prepared Edgeplane runtime.
pub async fn run_gemini_compat(
    profile: Option<String>,
    _with_rtk: bool,
    extra_args: Vec<String>,
    client: &EdgeplaneClient,
    config: &McConfig,
) -> Result<()> {
    let profile_name = profile.unwrap_or_else(|| "default".to_string());
    let launch_args = LaunchArgs {
        agent: Some("gemini".to_string()),
        no_daemon: false,
        preflight_only: false,
        skip_config_gen: false,
        profile: Some(profile_name),
        legacy_global_config: false,
        allow_pin_mismatch: false,
        no_embed_token: false,
        agent_args: extra_args,
    };
    launch::run(launch_args, client, config).await
}
