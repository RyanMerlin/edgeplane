use anyhow::Result;
use clap::Args;
use reqwest::Client;
use std::time::Duration;

use crate::config::{load_server_list, write_servers_file};

#[derive(Args, Debug)]
pub struct DiscoverArgs {
    /// Candidate mc-controlplane URLs to probe (comma-separated or repeated).
    /// If omitted, probes the current server list + localhost:8008.
    #[arg(long, value_delimiter = ',')]
    pub probe: Vec<String>,

    /// Just print what would be written without saving.
    #[arg(long)]
    pub dry_run: bool,
}

pub async fn run(args: DiscoverArgs) -> Result<()> {
    let candidates = if args.probe.is_empty() {
        let mut c = load_server_list();
        let default = "http://localhost:8008".to_string();
        if !c.contains(&default) {
            c.push(default);
        }
        c
    } else {
        args.probe
    };

    println!("Probing {} candidate(s)...", candidates.len());
    let live = probe_servers(&candidates).await;

    if live.is_empty() {
        anyhow::bail!(
            "No mc-controlplane nodes responded. Try: mc discover --probe https://your-node:8008"
        );
    }

    println!("Found {} live server(s):", live.len());
    for s in &live {
        println!("  {}", s);
    }

    if args.dry_run {
        println!("(dry-run — not writing ~/.mc/servers)");
        return Ok(());
    }

    write_servers_file(&live)?;
    println!(
        "Written to {}",
        crate::config::servers_file_path().display()
    );

    Ok(())
}

/// Probe candidates concurrently, return those that reply to /health.
pub async fn probe_servers(candidates: &[String]) -> Vec<String> {
    let client = Client::builder()
        .timeout(Duration::from_secs(3))
        .danger_accept_invalid_certs(true)
        .build()
        .unwrap_or_default();

    let futs: Vec<_> = candidates
        .iter()
        .map(|url| {
            let client = client.clone();
            let url = url.clone();
            async move {
                let check = format!("{}/health", url.trim_end_matches('/'));
                let alive = client
                    .get(&check)
                    .send()
                    .await
                    .map(|r| r.status().is_success())
                    .unwrap_or(false);
                (url, alive)
            }
        })
        .collect();

    let results = futures_util::future::join_all(futs).await;
    results
        .into_iter()
        .filter_map(|(url, alive)| if alive { Some(url) } else { None })
        .collect()
}
