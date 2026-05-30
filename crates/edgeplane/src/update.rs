use crate::config::EdgeplaneConfig;
use anyhow::{Context, Result, anyhow, bail};
use clap::{Args, Subcommand};
use reqwest::Client;
use serde::Deserialize;
use sha2::{Digest, Sha256};
use std::{env, fs};

#[derive(Subcommand, Debug)]
pub enum UpdateCommand {
    /// Update edgeplane by downloading the latest release artifact.
    SelfUpdate(SelfUpdateArgs),
}

#[derive(Args, Debug)]
pub struct SelfUpdateArgs {
    /// Manifest URL describing available releases.
    #[arg(
        long,
        env = "EP_UPDATE_MANIFEST_URL",
        default_value = "https://github.com/RyanMerlin/edgeplane/releases/latest/download/latest.json"
    )]
    pub manifest_url: String,
    /// Skip checksum verification.
    #[arg(long)]
    pub skip_verify: bool,
}

#[derive(Deserialize)]
struct UpdateManifest {
    version: String,
    files: Vec<UpdateFile>,
}

#[derive(Deserialize)]
struct UpdateFile {
    os: String,
    arch: String,
    url: String,
    sha256: Option<String>,
}

pub async fn run(command: UpdateCommand, config: &EdgeplaneConfig) -> Result<()> {
    let UpdateCommand::SelfUpdate(args) = command;
    let client = Client::builder()
        .danger_accept_invalid_certs(config.allow_insecure)
        .build()?;
    let manifest = client
        .get(&args.manifest_url)
        .send()
        .await?
        .error_for_status()?
        .json::<UpdateManifest>()
        .await
        .context("failed to download update manifest")?;
    let target = env::consts::OS;
    let arch = env::consts::ARCH;
    let file = manifest
        .files
        .into_iter()
        .find(|candidate| candidate.os == target && candidate.arch == arch)
        .ok_or_else(|| anyhow!("no release for {target}/{arch}"))?;
    let response = client.get(&file.url).send().await?.error_for_status()?;
    let bytes = response
        .bytes()
        .await
        .context("failed to download update binary")?;
    if !args.skip_verify {
        if let Some(checksum) = file.sha256 {
            let mut hasher = Sha256::new();
            hasher.update(&bytes);
            let digest = hex::encode(hasher.finalize());
            if digest != checksum {
                bail!("checksum mismatch: expected {checksum}, got {digest}");
            }
        }
    }
    let current = env::current_exe().context("unable to locate current executable")?;
    let tmp = current.with_extension("new");
    fs::write(&tmp, &bytes)?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let mut perms = fs::metadata(&tmp)?.permissions();
        perms.set_mode(0o755);
        fs::set_permissions(&tmp, perms)?;
    }
    fs::rename(&tmp, &current).context("failed to replace binary")?;
    println!(
        "Updated edgeplane to version {} at {}",
        manifest.version,
        current.display()
    );
    Ok(())
}
