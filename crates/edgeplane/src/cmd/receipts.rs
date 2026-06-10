/// `edgeplane receipts` — inspect capability execution receipts stored locally.
///
/// Reads directly from `~/.ep/receipts.db` — no daemon required.
use anyhow::Result;
use clap::Subcommand;

// ---------------------------------------------------------------------------
// Clap types
// ---------------------------------------------------------------------------

#[derive(Subcommand, Debug)]
pub enum ReceiptsCmd {
    /// Show most recent capability executions.
    #[command(about = "Show most recent capability executions")]
    Last {
        #[arg(long, default_value = "10")]
        limit: usize,
        #[arg(long)]
        json: bool,
    },
    /// Get a specific receipt by ID.
    #[command(about = "Get a specific receipt by ID")]
    Get {
        id: String,
        #[arg(long)]
        json: bool,
    },
    /// List receipts with optional filters.
    #[command(name = "ls", about = "List receipts with optional filters")]
    Ls {
        #[arg(long, default_value = "20")]
        limit: usize,
        #[arg(long)]
        domain: Option<String>,
        #[arg(long)]
        agent: Option<String>,
        #[arg(long)]
        json: bool,
    },
}

// ---------------------------------------------------------------------------
// Entry point
// ---------------------------------------------------------------------------

pub fn run(cmd: ReceiptsCmd) -> Result<()> {
    use edgeplaned_receipts::{ReceiptFilter, ReceiptStore};

    let db_path = edgeplaned_paths::receipts_db_path();
    let store = ReceiptStore::open(&db_path)?;

    match cmd {
        ReceiptsCmd::Last { limit, json } => {
            let receipts = store.last(limit)?;
            print_receipts(&receipts, json)?;
        }
        ReceiptsCmd::Get { id, json } => match store.get(&id)? {
            Some(r) => print_receipt(&r, json)?,
            None => anyhow::bail!("receipt '{}' not found", id),
        },
        ReceiptsCmd::Ls { limit, domain, agent, json } => {
            let filter = ReceiptFilter {
                domain_id: domain,
                agent_id: agent,
                limit,
                ..Default::default()
            };
            let receipts = store.list(filter)?;
            print_receipts(&receipts, json)?;
        }
    }
    Ok(())
}

// ---------------------------------------------------------------------------
// Formatting helpers
// ---------------------------------------------------------------------------

fn print_receipts(receipts: &[edgeplaned_receipts::Receipt], as_json: bool) -> Result<()> {
    if as_json {
        println!("{}", serde_json::to_string_pretty(receipts)?);
    } else {
        for r in receipts {
            let status = if r.exit_code == 0 { "✓" } else { "✗" };
            println!(
                "{}  {}  {}  [{}]",
                r.created_at.format("%Y-%m-%dT%H:%M:%SZ"),
                r.capability,
                status,
                r.id,
            );
        }
    }
    Ok(())
}

fn print_receipt(r: &edgeplaned_receipts::Receipt, as_json: bool) -> Result<()> {
    if as_json {
        println!("{}", serde_json::to_string_pretty(r)?);
    } else {
        let status = if r.exit_code == 0 { "✓" } else { "✗" };
        println!(
            "{}  {}  {}  [{}]",
            r.created_at.format("%Y-%m-%dT%H:%M:%SZ"),
            r.capability,
            status,
            r.id,
        );
        println!("  args:   {}", r.args_json);
        println!("  result: {}", r.result_json);
        println!("  time:   {}ms", r.execution_time_ms);
        if let Some(m) = &r.domain_id {
            println!("  domain: {m}");
        }
        if let Some(a) = &r.agent_id {
            println!("  agent: {a}");
        }
    }
    Ok(())
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use edgeplaned_receipts::{Receipt, ReceiptFilter, ReceiptStore};

    fn make_receipt(id: &str, domain_id: Option<&str>) -> Receipt {
        Receipt {
            id: id.to_string(),
            capability: "kubectl-observe.kubectl-get-pods".to_string(),
            args_json: r#"{"namespace":"default"}"#.to_string(),
            result_json: r#"{"ok":true}"#.to_string(),
            exit_code: 0,
            execution_time_ms: 10,
            domain_id: domain_id.map(|s| s.to_string()),
            agent_id: None,
            created_at: chrono::Utc::now(),
        }
    }

    #[test]
    fn receipts_last_from_empty_store() {
        let dir = tempfile::tempdir().unwrap();
        let store = ReceiptStore::open(&dir.path().join("r.db")).unwrap();
        let receipts = store.last(10).unwrap();
        assert!(receipts.is_empty());
    }

    #[test]
    fn receipts_list_with_domain_filter() {
        let dir = tempfile::tempdir().unwrap();
        let store = ReceiptStore::open(&dir.path().join("r.db")).unwrap();

        let r1 = make_receipt("receipt-1", Some("domain-alpha"));
        let r2 = make_receipt("receipt-2", Some("domain-beta"));
        store.insert(&r1).unwrap();
        store.insert(&r2).unwrap();

        let results = store
            .list(ReceiptFilter {
                domain_id: Some("domain-alpha".to_string()),
                limit: 10,
                ..Default::default()
            })
            .unwrap();

        assert_eq!(results.len(), 1);
        assert_eq!(results[0].id, "receipt-1");
    }
}
