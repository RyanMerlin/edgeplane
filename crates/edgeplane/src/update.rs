use crate::config::EdgeplaneConfig;
use anyhow::{Context, Result, anyhow, bail};
use clap::{Args, Subcommand};
use reqwest::Client;
use serde::Deserialize;
use sha2::{Digest, Sha256};
use std::{
    env, fs,
    path::{Path, PathBuf},
};

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
        default_value = "https://github.com/edgeplane/edgeplane/releases/latest/download/latest.json"
    )]
    pub manifest_url: String,
    /// Skip checksum verification.
    #[arg(long)]
    pub skip_verify: bool,
}

#[derive(Debug, Deserialize)]
struct UpdateManifest {
    version: String,
    files: Vec<UpdateFile>,
}

#[derive(Debug, Deserialize)]
struct UpdateFile {
    /// Which binary this artifact installs (e.g. "edgeplane", "edgeplaned").
    ///
    /// Defaults to the CLI for backward compatibility with pre-0.14.1 manifests
    /// that listed only the CLI and carried no `bin` field.
    #[serde(default = "default_bin")]
    bin: String,
    os: String,
    arch: String,
    url: String,
    sha256: Option<String>,
}

fn default_bin() -> String {
    "edgeplane".to_string()
}

/// Select which manifest entries to install on this node.
///
/// `edgeplane update` converges every edgeplane-family binary that is *already
/// installed* alongside the running CLI — the CLI itself plus, e.g., `edgeplaned`
/// on a node that runs the daemon. A manifest entry whose binary is not already
/// present in `install_dir` is skipped, so a dev box that only has the CLI never
/// sprouts an `edgeplaned`. The running CLI (`current`) is always included even
/// if `exists` reports otherwise. `exists` is injected so selection is unit-
/// testable without touching the filesystem.
fn select_targets<'a>(
    files: &'a [UpdateFile],
    os: &str,
    arch: &str,
    install_dir: &Path,
    current: &Path,
    exists: impl Fn(&Path) -> bool,
) -> Vec<(&'a UpdateFile, PathBuf)> {
    files
        .iter()
        .filter(|f| f.os == os && f.arch == arch)
        .filter_map(|f| {
            let target = install_dir.join(&f.bin);
            (target == current || exists(&target)).then_some((f, target))
        })
        .collect()
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

    let os = env::consts::OS;
    let arch = env::consts::ARCH;
    let current = env::current_exe().context("unable to locate current executable")?;
    let install_dir = current
        .parent()
        .ok_or_else(|| anyhow!("current executable has no parent directory"))?
        .to_path_buf();

    let targets = select_targets(&manifest.files, os, arch, &install_dir, &current, |p| {
        p.exists()
    });
    if targets.is_empty() {
        bail!("no release artifact for {os}/{arch} matches any installed edgeplane binary");
    }

    let mut changed = 0usize;
    for (file, target) in targets {
        let bytes = client
            .get(&file.url)
            .send()
            .await?
            .error_for_status()?
            .bytes()
            .await
            .with_context(|| format!("failed to download {}", file.bin))?;

        if !args.skip_verify
            && let Some(expected) = &file.sha256 {
                let mut hasher = Sha256::new();
                hasher.update(&bytes);
                let digest = hex::encode(hasher.finalize());
                if &digest != expected {
                    bail!(
                        "checksum mismatch for {}: expected {expected}, got {digest}",
                        file.bin
                    );
                }
            }

        // Skip the write when the installed binary is already byte-identical, so a
        // no-op run leaves mtimes untouched and a watcher (the update timer) sees no
        // spurious daemon change.
        if target.exists()
            && fs::read(&target)
                .map(|cur| cur.as_slice() == bytes.as_ref())
                .unwrap_or(false)
        {
            println!(
                "{} already at {} ({})",
                file.bin,
                manifest.version,
                target.display()
            );
            continue;
        }

        // Stage next to the target and atomically rename into place. Renaming over a
        // running binary is safe on Unix: the running process keeps its open inode.
        let tmp = target.with_extension("new");
        fs::write(&tmp, &bytes).with_context(|| format!("failed to stage {}", tmp.display()))?;
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let mut perms = fs::metadata(&tmp)?.permissions();
            perms.set_mode(0o755);
            fs::set_permissions(&tmp, perms)?;
        }
        fs::rename(&tmp, &target)
            .with_context(|| format!("failed to replace {}", target.display()))?;
        println!(
            "Updated {} to {} at {}",
            file.bin,
            manifest.version,
            target.display()
        );
        changed += 1;
    }

    if changed == 0 {
        println!(
            "All installed edgeplane binaries already at {}",
            manifest.version
        );
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::HashSet;

    fn file(bin: &str, os: &str, arch: &str) -> UpdateFile {
        UpdateFile {
            bin: bin.to_string(),
            os: os.to_string(),
            arch: arch.to_string(),
            url: format!("https://example/{bin}-{os}-{arch}"),
            sha256: None,
        }
    }

    #[test]
    fn converges_cli_and_installed_daemon() {
        let files = vec![
            file("edgeplane", "linux", "x86_64"),
            file("edgeplaned", "linux", "x86_64"),
            file("edgeplane", "macos", "aarch64"), // wrong platform — must be ignored
        ];
        let dir = Path::new("/opt/ep/bin");
        let current = dir.join("edgeplane");
        let present: HashSet<PathBuf> = [dir.join("edgeplane"), dir.join("edgeplaned")]
            .into_iter()
            .collect();

        let got = select_targets(&files, "linux", "x86_64", dir, &current, |p| {
            present.contains(p)
        });
        let bins: Vec<&str> = got.iter().map(|(f, _)| f.bin.as_str()).collect();
        assert_eq!(bins, vec!["edgeplane", "edgeplaned"]);
    }

    #[test]
    fn skips_uninstalled_siblings() {
        let files = vec![
            file("edgeplane", "linux", "x86_64"),
            file("edgeplaned", "linux", "x86_64"),
        ];
        let dir = Path::new("/home/dev/.cargo/bin");
        let current = dir.join("edgeplane");
        // Dev box: only the CLI is installed, no daemon present.
        let got = select_targets(&files, "linux", "x86_64", dir, &current, |p| *p == current);
        let bins: Vec<&str> = got.iter().map(|(f, _)| f.bin.as_str()).collect();
        assert_eq!(
            bins,
            vec!["edgeplane"],
            "must not introduce an uninstalled edgeplaned"
        );
    }

    #[test]
    fn always_selects_running_cli_even_when_absent() {
        let files = vec![file("edgeplane", "linux", "x86_64")];
        let dir = Path::new("/opt/ep/bin");
        let current = dir.join("edgeplane");
        let got = select_targets(&files, "linux", "x86_64", dir, &current, |_| false);
        assert_eq!(got.len(), 1);
        assert_eq!(got[0].1, current);
    }

    #[test]
    fn legacy_manifest_entry_defaults_to_cli() {
        // pre-0.14.1 manifest entries carried no `bin` field.
        let f: UpdateFile =
            serde_json::from_str(r#"{"os":"linux","arch":"x86_64","url":"u","sha256":null}"#)
                .unwrap();
        assert_eq!(f.bin, "edgeplane");
    }
}
