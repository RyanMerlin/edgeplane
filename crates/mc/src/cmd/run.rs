/// `mc exec` — execute a capability through the mcd routing layer.
use crate::dispatch::McDispatch;
use anyhow::Result;
use std::io::IsTerminal;

// ---------------------------------------------------------------------------
// Clap types
// ---------------------------------------------------------------------------

#[derive(clap::Args, Debug)]
pub struct ExecArgs {
    /// Capability name in pack.capability format (e.g. kubectl-observe.kubectl-get-pods)
    pub name: String,

    /// Arguments as key=value pairs or a single JSON string
    #[arg(last = true)]
    pub args: Vec<String>,

    /// Output as JSON (default when not a TTY)
    #[arg(long)]
    pub json: bool,

    /// Validate args without executing
    #[arg(long)]
    pub dry_run: bool,

    /// Timeout in seconds
    #[arg(long)]
    pub timeout: Option<u32>,

    /// Domain ID for receipt correlation
    #[arg(long)]
    pub domain_id: Option<String>,

    /// Agent ID for receipt correlation
    #[arg(long)]
    pub agent_id: Option<String>,

    /// Override routing mode: auto|local|remote|backend
    #[arg(long)]
    pub route: Option<String>,
}

// ---------------------------------------------------------------------------
// Argument parsing
// ---------------------------------------------------------------------------

fn parse_args(args: &[String]) -> Result<serde_json::Value> {
    if args.is_empty() {
        return Ok(serde_json::Value::Object(Default::default()));
    }

    // Try as a single JSON string first.
    if args.len() == 1 {
        if let Ok(v) = serde_json::from_str::<serde_json::Value>(&args[0]) {
            return Ok(v);
        }
    }

    // Parse as key=value pairs.
    let mut map = serde_json::Map::new();
    for arg in args {
        let (k, v) = arg
            .split_once('=')
            .ok_or_else(|| anyhow::anyhow!("arg '{}' must be in key=value format", arg))?;
        map.insert(k.to_string(), serde_json::Value::String(v.to_string()));
    }
    Ok(serde_json::Value::Object(map))
}

// ---------------------------------------------------------------------------
// Entry point
// ---------------------------------------------------------------------------

pub async fn run(args: ExecArgs, host: Option<String>) -> Result<()> {
    let parsed_args = parse_args(&args.args)?;

    let dispatch = McDispatch::from_env(host, args.route);

    let result = dispatch
        .dispatch(
            &args.name,
            parsed_args,
            args.dry_run,
            args.timeout,
            args.domain_id,
            args.agent_id,
        )
        .await;

    let is_json = args.json || !std::io::stdout().is_terminal();

    match result {
        Ok(value) => {
            if is_json {
                println!("{}", serde_json::to_string(&value)?);
            } else {
                // Pretty-print for TTY — unwrap a top-level "data" key if present.
                if let Some(data) = value.get("data") {
                    println!("{}", serde_json::to_string_pretty(data)?);
                } else {
                    println!("{}", serde_json::to_string_pretty(&value)?);
                }
            }
        }
        Err(e) => {
            if is_json {
                let err_json = serde_json::json!({
                    "ok": false,
                    "error": e.to_string(),
                    "hint": "Use 'mc capabilities describe <cap>' to see the expected args",
                    "example": format!("mc exec {} --json", args.name),
                });
                eprintln!("{}", serde_json::to_string(&err_json)?);
            } else {
                eprintln!("error: {e}");
                eprintln!(
                    "hint: mc capabilities describe {} for args schema",
                    args.name
                );
            }
            std::process::exit(1);
        }
    }

    Ok(())
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_args_empty() {
        let result = parse_args(&[]).unwrap();
        assert!(result.is_object());
        assert!(result.as_object().unwrap().is_empty());
    }

    #[test]
    fn parse_args_key_value_pairs() {
        let args = vec![
            "namespace=default".to_string(),
            "selector=app=nginx".to_string(),
        ];
        let result = parse_args(&args).unwrap();
        assert_eq!(result["namespace"], "default");
        // The value includes everything after the first '=' so "app=nginx" is preserved.
        assert_eq!(result["selector"], "app=nginx");
    }

    #[test]
    fn parse_args_json_string() {
        let args = vec![r#"{"namespace":"kube-system","verbose":true}"#.to_string()];
        let result = parse_args(&args).unwrap();
        assert_eq!(result["namespace"], "kube-system");
        assert_eq!(result["verbose"], true);
    }

    #[test]
    fn parse_args_invalid_format_errors() {
        let args = vec!["not-a-kv-pair".to_string()];
        let result = parse_args(&args);
        assert!(result.is_err());
    }
}
