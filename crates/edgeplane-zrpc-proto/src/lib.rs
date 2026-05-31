//! Wire protocol for the `edgeplane-zrpc` Zellij control plugin.
//!
//! edgeplaned and the in-session WASM plugin speak newline-delimited JSON
//! (NDJSON) over Zellij pipes:
//!
//! * **Requests** flow edgeplaned → plugin, one [`Request`] per line. Each
//!   carries a correlation `id`, a `method`, and method-specific `params`.
//!   [`Request::call`] validates the method+params into a typed [`Call`].
//! * **Responses** flow plugin → edgeplaned via `cli_pipe_output`, one
//!   [`Response`] per line, correlated by `id`.
//! * **Events** ([`PluginEvent`]) are unsolicited plugin → edgeplaned pushes
//!   (pane exited / closed / updated) over a long-lived blocking pipe.
//!
//! This crate is target-independent (no `zellij-tile`) so it compiles for the
//! host and is unit-tested with `cargo nextest`; the wasm plugin and the
//! edgeplaned adapter both depend on it for a single source of protocol truth.

use serde::{Deserialize, Serialize};

/// Error parsing or validating a protocol message.
#[derive(Debug, thiserror::Error, PartialEq, Eq)]
pub enum ProtoError {
    /// The line was not valid JSON.
    #[error("malformed JSON: {0}")]
    Json(String),
    /// The `method` field named a method the plugin does not implement.
    #[error("unknown method: {0}")]
    UnknownMethod(String),
    /// `params` did not match the shape required by `method`.
    #[error("invalid params for {method}: {detail}")]
    InvalidParams { method: String, detail: String },
}

/// A single JSON-RPC request line (edgeplaned → plugin).
#[derive(Debug, Clone, PartialEq, Eq, Deserialize, Serialize)]
pub struct Request {
    /// Correlation id echoed back on the matching [`Response`].
    pub id: String,
    /// Method name (snake_case): `inject`, `cancel`, `read_scrollback`,
    /// `classify`, `list_agent_panes`.
    pub method: String,
    /// Method-specific parameters; `null`/absent for parameterless methods.
    #[serde(default)]
    pub params: serde_json::Value,
}

/// A validated, typed request ready for the plugin to dispatch against
/// `zellij-tile`. Exhaustive so the plugin's `match` covers every method.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Call {
    /// Focus-free text injection into a pane (`write_chars_to_pane_id`).
    Inject { pane_id: String, text: String },
    /// Interrupt a pane (`send_sigint_to_pane_id`).
    Cancel { pane_id: String },
    /// Read pane scrollback (`get_pane_scrollback`); `lines` caps the tail.
    ReadScrollback {
        pane_id: String,
        lines: Option<usize>,
    },
    /// Read scrollback and classify the agent's state.
    Classify { pane_id: String },
    /// Enumerate panes that look like hosted-agent panes.
    ListAgentPanes,
}

impl Request {
    /// Build an `inject` request (focus-free text write into a pane).
    pub fn inject(id: impl Into<String>, pane_id: &str, text: &str) -> Self {
        Self {
            id: id.into(),
            method: "inject".into(),
            params: serde_json::json!({ "pane_id": pane_id, "text": text }),
        }
    }

    /// Build a `cancel` request (interrupt a pane).
    pub fn cancel(id: impl Into<String>, pane_id: &str) -> Self {
        Self {
            id: id.into(),
            method: "cancel".into(),
            params: serde_json::json!({ "pane_id": pane_id }),
        }
    }

    /// Build a `read_scrollback` request.
    pub fn read_scrollback(id: impl Into<String>, pane_id: &str, lines: Option<usize>) -> Self {
        Self {
            id: id.into(),
            method: "read_scrollback".into(),
            params: serde_json::json!({ "pane_id": pane_id, "lines": lines }),
        }
    }

    /// Build a `classify` request.
    pub fn classify(id: impl Into<String>, pane_id: &str) -> Self {
        Self {
            id: id.into(),
            method: "classify".into(),
            params: serde_json::json!({ "pane_id": pane_id }),
        }
    }

    /// Build a `list_agent_panes` request.
    pub fn list_agent_panes(id: impl Into<String>) -> Self {
        Self {
            id: id.into(),
            method: "list_agent_panes".into(),
            params: serde_json::Value::Null,
        }
    }

    /// Validate `method` + `params` into a typed [`Call`].
    pub fn call(&self) -> Result<Call, ProtoError> {
        // Each method deserializes `params` into its own struct; a `null`
        // or missing field surfaces as `InvalidParams`.
        let invalid = |detail: String| ProtoError::InvalidParams {
            method: self.method.clone(),
            detail,
        };

        match self.method.as_str() {
            "inject" => {
                #[derive(Deserialize)]
                struct P {
                    pane_id: String,
                    text: String,
                }
                let p: P =
                    serde_json::from_value(self.params.clone()).map_err(|e| invalid(e.to_string()))?;
                Ok(Call::Inject {
                    pane_id: p.pane_id,
                    text: p.text,
                })
            }
            "cancel" => {
                #[derive(Deserialize)]
                struct P {
                    pane_id: String,
                }
                let p: P =
                    serde_json::from_value(self.params.clone()).map_err(|e| invalid(e.to_string()))?;
                Ok(Call::Cancel { pane_id: p.pane_id })
            }
            "read_scrollback" => {
                #[derive(Deserialize)]
                struct P {
                    pane_id: String,
                    #[serde(default)]
                    lines: Option<usize>,
                }
                let p: P =
                    serde_json::from_value(self.params.clone()).map_err(|e| invalid(e.to_string()))?;
                Ok(Call::ReadScrollback {
                    pane_id: p.pane_id,
                    lines: p.lines,
                })
            }
            "classify" => {
                #[derive(Deserialize)]
                struct P {
                    pane_id: String,
                }
                let p: P =
                    serde_json::from_value(self.params.clone()).map_err(|e| invalid(e.to_string()))?;
                Ok(Call::Classify { pane_id: p.pane_id })
            }
            "list_agent_panes" => Ok(Call::ListAgentPanes),
            other => Err(ProtoError::UnknownMethod(other.to_string())),
        }
    }
}

/// Parse an NDJSON payload into per-line results. Blank lines are skipped;
/// a malformed line yields an `Err` for that line without aborting the rest.
pub fn parse_requests(payload: &str) -> Vec<Result<Request, ProtoError>> {
    payload
        .lines()
        .filter(|line| !line.trim().is_empty())
        .map(|line| {
            serde_json::from_str::<Request>(line).map_err(|e| ProtoError::Json(e.to_string()))
        })
        .collect()
}

/// A JSON-RPC response line (plugin → edgeplaned), correlated by `id`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Response {
    pub id: String,
    pub ok: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub result: Option<serde_json::Value>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub error: Option<String>,
}

impl Response {
    /// Build a success response.
    pub fn ok(id: impl Into<String>, result: serde_json::Value) -> Self {
        Self {
            id: id.into(),
            ok: true,
            result: Some(result),
            error: None,
        }
    }

    /// Build an error response.
    pub fn error(id: impl Into<String>, msg: impl Into<String>) -> Self {
        Self {
            id: id.into(),
            ok: false,
            result: None,
            error: Some(msg.into()),
        }
    }

    /// Serialize to a single NDJSON line (no trailing newline; the caller
    /// frames). Effectively infallible — a `Response` always serializes; the
    /// fallback avoids ever panicking inside the wasm plugin.
    pub fn to_ndjson_line(&self) -> String {
        serde_json::to_string(self).unwrap_or_else(|_| {
            let id = serde_json::to_string(&self.id).unwrap_or_else(|_| "\"?\"".to_string());
            format!(r#"{{"id":{id},"ok":false,"error":"response serialization failed"}}"#)
        })
    }
}

/// Unsolicited plugin → edgeplaned push events over the long-lived pipe.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "event", rename_all = "snake_case")]
pub enum PluginEvent {
    /// Pane layout changed; carries the affected terminal pane ids.
    PaneUpdate { panes: Vec<u32> },
    /// A command pane exited with an optional status code.
    CommandPaneExited { pane_id: u32, exit_code: Option<i32> },
    /// A pane was closed.
    PaneClosed { pane_id: u32 },
}

impl PluginEvent {
    /// Serialize to a single NDJSON line (no trailing newline; the caller
    /// frames). Effectively infallible — falls back rather than panicking.
    pub fn to_ndjson_line(&self) -> String {
        serde_json::to_string(self)
            .unwrap_or_else(|_| r#"{"event":"serialization_failed"}"#.to_string())
    }
}

/// Classified agent state derived from a pane's visible scrollback.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AgentState {
    Idle,
    Working,
    Error,
    Unknown,
}

impl AgentState {
    /// Stable lowercase label, matching edgeplaned's existing state strings.
    pub fn as_str(&self) -> &'static str {
        match self {
            AgentState::Idle => "idle",
            AgentState::Working => "working",
            AgentState::Error => "error",
            AgentState::Unknown => "unknown",
        }
    }
}

/// True when the screen looks idle — Claude's prompt marker `❯` is visible
/// at the start of a (trimmed) line.
pub fn is_idle_screen(lines: &[&str]) -> bool {
    lines.iter().any(|l| {
        let t = l.trim();
        t == "❯" || t.starts_with("❯ ")
    })
}

/// Classify agent state from a tail of the visible viewport. Idle wins over
/// any stale working/error signal still on screen.
pub fn classify_state(tail: &[&str]) -> AgentState {
    if is_idle_screen(tail) {
        return AgentState::Idle;
    }
    const SPINNERS: &[char] = &['⠋', '⠙', '⠹', '⠸', '⠼', '⠴', '⠦', '⠧', '⠇', '⠏'];
    for line in tail {
        let t = line.trim();
        if t.contains("Running tool") || SPINNERS.iter().any(|&c| t.contains(c)) {
            return AgentState::Working;
        }
        if t.contains("Error:") || t.contains('✗') {
            return AgentState::Error;
        }
    }
    AgentState::Unknown
}

/// Kind of a Zellij pane, mirroring `zellij_tile::PaneId` without depending on
/// `zellij-tile` (so this crate stays host-testable).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PaneKind {
    Terminal,
    Plugin,
}

/// Parse a wire pane id (`"terminal_3"` / `"plugin_7"`) into `(kind, id)`.
/// The wasm plugin maps the result onto `zellij_tile::PaneId`.
pub fn parse_pane_ref(s: &str) -> Result<(PaneKind, u32), String> {
    let (kind, num) = s
        .split_once('_')
        .ok_or_else(|| format!("invalid pane id (expected `<kind>_<n>`): {s}"))?;
    let id: u32 = num
        .parse()
        .map_err(|_| format!("invalid pane id number in `{s}`"))?;
    match kind {
        "terminal" => Ok((PaneKind::Terminal, id)),
        "plugin" => Ok((PaneKind::Plugin, id)),
        other => Err(format!("unknown pane kind `{other}` in `{s}`")),
    }
}

/// The pane operations the plugin performs against `zellij-tile`, abstracted
/// so request dispatch is testable without a live Zellij runtime. The wasm
/// plugin supplies the real impl (calling the `zellij-tile` shim); tests use
/// a mock. `&mut self` because the real impl caches the latest pane manifest.
pub trait PaneOps {
    /// Focus-free inject (`write_chars_to_pane_id`).
    fn inject(&mut self, pane_id: &str, text: &str) -> Result<(), String>;
    /// Interrupt (`send_sigint_to_pane_id`).
    fn cancel(&mut self, pane_id: &str) -> Result<(), String>;
    /// Read scrollback lines (`get_pane_scrollback`); `lines` caps the tail.
    fn read_scrollback(&mut self, pane_id: &str, lines: Option<usize>) -> Result<Vec<String>, String>;
    /// Pane ids that look like hosted-agent panes (from the cached manifest).
    fn list_agent_panes(&mut self) -> Result<Vec<String>, String>;
}

/// Top-level entry the plugin calls per request line: validate, dispatch,
/// and always produce a correlated [`Response`] (errors become error
/// responses rather than propagating).
pub fn handle<O: PaneOps>(req: &Request, ops: &mut O) -> Response {
    match req.call() {
        Ok(call) => dispatch(&req.id, call, ops),
        Err(e) => Response::error(&req.id, e.to_string()),
    }
}

/// Route a validated [`Call`] to the matching [`PaneOps`] method and shape the
/// [`Response`]. `classify` reads the visible scrollback then runs
/// [`classify_state`].
pub fn dispatch<O: PaneOps>(id: &str, call: Call, ops: &mut O) -> Response {
    match call {
        Call::Inject { pane_id, text } => match ops.inject(&pane_id, &text) {
            Ok(()) => Response::ok(id, serde_json::json!({"injected": true})),
            Err(e) => Response::error(id, e),
        },
        Call::Cancel { pane_id } => match ops.cancel(&pane_id) {
            Ok(()) => Response::ok(id, serde_json::json!({"cancelled": true})),
            Err(e) => Response::error(id, e),
        },
        Call::ReadScrollback { pane_id, lines } => match ops.read_scrollback(&pane_id, lines) {
            Ok(l) => Response::ok(id, serde_json::json!({ "lines": l })),
            Err(e) => Response::error(id, e),
        },
        Call::Classify { pane_id } => match ops.read_scrollback(&pane_id, None) {
            Ok(l) => {
                let refs: Vec<&str> = l.iter().map(String::as_str).collect();
                Response::ok(id, serde_json::json!({"state": classify_state(&refs).as_str()}))
            }
            Err(e) => Response::error(id, e),
        },
        Call::ListAgentPanes => match ops.list_agent_panes() {
            Ok(p) => Response::ok(id, serde_json::json!({ "panes": p })),
            Err(e) => Response::error(id, e),
        },
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    /// Records calls and replays canned results so dispatch routing is
    /// exercised without Zellij.
    #[derive(Default)]
    struct MockOps {
        injected: Vec<(String, String)>,
        cancelled: Vec<String>,
        scrollback: Vec<String>,
        scrollback_err: Option<String>,
        panes: Vec<String>,
    }

    impl PaneOps for MockOps {
        fn inject(&mut self, pane_id: &str, text: &str) -> Result<(), String> {
            self.injected.push((pane_id.into(), text.into()));
            Ok(())
        }
        fn cancel(&mut self, pane_id: &str) -> Result<(), String> {
            self.cancelled.push(pane_id.into());
            Ok(())
        }
        fn read_scrollback(&mut self, _pane_id: &str, _lines: Option<usize>) -> Result<Vec<String>, String> {
            match &self.scrollback_err {
                Some(e) => Err(e.clone()),
                None => Ok(self.scrollback.clone()),
            }
        }
        fn list_agent_panes(&mut self) -> Result<Vec<String>, String> {
            Ok(self.panes.clone())
        }
    }

    fn req(id: &str, method: &str, params: serde_json::Value) -> Request {
        Request {
            id: id.into(),
            method: method.into(),
            params,
        }
    }

    // ── Request constructors round-trip through the validator ───────────

    #[test]
    fn inject_constructor_round_trips() {
        let req = Request::inject("1", "terminal_3", "hi");
        assert_eq!(req.method, "inject");
        assert_eq!(
            req.call().unwrap(),
            Call::Inject {
                pane_id: "terminal_3".into(),
                text: "hi".into()
            }
        );
    }

    #[test]
    fn cancel_constructor_round_trips() {
        let req = Request::cancel("2", "terminal_3");
        assert_eq!(req.call().unwrap(), Call::Cancel { pane_id: "terminal_3".into() });
    }

    #[test]
    fn read_scrollback_constructor_round_trips() {
        let req = Request::read_scrollback("3", "terminal_3", Some(40));
        assert_eq!(
            req.call().unwrap(),
            Call::ReadScrollback {
                pane_id: "terminal_3".into(),
                lines: Some(40)
            }
        );
    }

    #[test]
    fn classify_constructor_round_trips() {
        let req = Request::classify("4", "terminal_3");
        assert_eq!(req.call().unwrap(), Call::Classify { pane_id: "terminal_3".into() });
    }

    #[test]
    fn list_agent_panes_constructor_round_trips() {
        let req = Request::list_agent_panes("5");
        assert_eq!(req.id, "5");
        assert_eq!(req.call().unwrap(), Call::ListAgentPanes);
    }

    // ── parse_pane_ref ──────────────────────────────────────────────────

    #[test]
    fn parse_pane_ref_terminal() {
        assert_eq!(parse_pane_ref("terminal_3").unwrap(), (PaneKind::Terminal, 3));
    }

    #[test]
    fn parse_pane_ref_plugin() {
        assert_eq!(parse_pane_ref("plugin_7").unwrap(), (PaneKind::Plugin, 7));
    }

    #[test]
    fn parse_pane_ref_rejects_missing_underscore() {
        assert!(parse_pane_ref("terminal3").is_err());
    }

    #[test]
    fn parse_pane_ref_rejects_non_numeric_id() {
        assert!(parse_pane_ref("terminal_x").is_err());
    }

    #[test]
    fn parse_pane_ref_rejects_unknown_kind() {
        assert!(parse_pane_ref("weird_3").is_err());
    }

    // ── handle / dispatch routing ───────────────────────────────────────

    #[test]
    fn handle_inject_invokes_ops_and_returns_ok() {
        let mut ops = MockOps::default();
        let r = handle(
            &req("i1", "inject", json!({"pane_id": "terminal_2", "text": "go"})),
            &mut ops,
        );
        assert_eq!(ops.injected, vec![("terminal_2".to_string(), "go".to_string())]);
        assert_eq!(r.id, "i1");
        assert!(r.ok);
    }

    #[test]
    fn handle_cancel_invokes_ops() {
        let mut ops = MockOps::default();
        let r = handle(&req("c1", "cancel", json!({"pane_id": "terminal_2"})), &mut ops);
        assert_eq!(ops.cancelled, vec!["terminal_2".to_string()]);
        assert!(r.ok);
    }

    #[test]
    fn handle_read_scrollback_returns_lines() {
        let mut ops = MockOps {
            scrollback: vec!["line one".into(), "line two".into()],
            ..Default::default()
        };
        let r = handle(
            &req("s1", "read_scrollback", json!({"pane_id": "terminal_2", "lines": 2})),
            &mut ops,
        );
        assert!(r.ok);
        assert_eq!(r.result.unwrap()["lines"], json!(["line one", "line two"]));
    }

    #[test]
    fn handle_classify_reads_scrollback_then_classifies() {
        let mut ops = MockOps {
            scrollback: vec!["Running tool: foo".into(), "❯".into()],
            ..Default::default()
        };
        let r = handle(&req("k1", "classify", json!({"pane_id": "terminal_2"})), &mut ops);
        assert!(r.ok);
        assert_eq!(r.result.unwrap()["state"], "idle");
    }

    #[test]
    fn handle_list_agent_panes_returns_panes() {
        let mut ops = MockOps {
            panes: vec!["terminal_0".into(), "terminal_3".into()],
            ..Default::default()
        };
        let r = handle(&req("l1", "list_agent_panes", serde_json::Value::Null), &mut ops);
        assert!(r.ok);
        assert_eq!(r.result.unwrap()["panes"], json!(["terminal_0", "terminal_3"]));
    }

    #[test]
    fn handle_propagates_ops_error_as_error_response() {
        let mut ops = MockOps {
            scrollback_err: Some("pane gone".into()),
            ..Default::default()
        };
        let r = handle(&req("s2", "read_scrollback", json!({"pane_id": "terminal_9"})), &mut ops);
        assert!(!r.ok);
        assert_eq!(r.id, "s2");
        assert_eq!(r.error.as_deref(), Some("pane gone"));
    }

    #[test]
    fn handle_unknown_method_returns_error_response_with_id() {
        let mut ops = MockOps::default();
        let r = handle(&req("u1", "nope", serde_json::Value::Null), &mut ops);
        assert!(!r.ok);
        assert_eq!(r.id, "u1");
        assert!(r.error.unwrap().contains("unknown method"));
    }

    #[test]
    fn handle_invalid_params_returns_error_response() {
        let mut ops = MockOps::default();
        // inject missing `text`
        let r = handle(&req("i2", "inject", json!({"pane_id": "terminal_2"})), &mut ops);
        assert!(!r.ok);
        assert!(ops.injected.is_empty(), "must not inject on invalid params");
    }

    // ── Request::call — method + params validation ──────────────────────

    #[test]
    fn inject_request_parses_to_call() {
        let req = Request {
            id: "1".into(),
            method: "inject".into(),
            params: json!({"pane_id": "terminal_3", "text": "hello"}),
        };
        assert_eq!(
            req.call().unwrap(),
            Call::Inject {
                pane_id: "terminal_3".into(),
                text: "hello".into()
            }
        );
    }

    #[test]
    fn cancel_request_parses_to_call() {
        let req = Request {
            id: "2".into(),
            method: "cancel".into(),
            params: json!({"pane_id": "terminal_3"}),
        };
        assert_eq!(
            req.call().unwrap(),
            Call::Cancel {
                pane_id: "terminal_3".into()
            }
        );
    }

    #[test]
    fn read_scrollback_defaults_lines_to_none() {
        let req = Request {
            id: "3".into(),
            method: "read_scrollback".into(),
            params: json!({"pane_id": "terminal_3"}),
        };
        assert_eq!(
            req.call().unwrap(),
            Call::ReadScrollback {
                pane_id: "terminal_3".into(),
                lines: None
            }
        );
    }

    #[test]
    fn read_scrollback_with_lines() {
        let req = Request {
            id: "4".into(),
            method: "read_scrollback".into(),
            params: json!({"pane_id": "terminal_3", "lines": 50}),
        };
        assert_eq!(
            req.call().unwrap(),
            Call::ReadScrollback {
                pane_id: "terminal_3".into(),
                lines: Some(50)
            }
        );
    }

    #[test]
    fn classify_request_parses_to_call() {
        let req = Request {
            id: "5".into(),
            method: "classify".into(),
            params: json!({"pane_id": "terminal_7"}),
        };
        assert_eq!(
            req.call().unwrap(),
            Call::Classify {
                pane_id: "terminal_7".into()
            }
        );
    }

    #[test]
    fn list_agent_panes_parses_with_absent_params() {
        let req = Request {
            id: "6".into(),
            method: "list_agent_panes".into(),
            params: serde_json::Value::Null,
        };
        assert_eq!(req.call().unwrap(), Call::ListAgentPanes);
    }

    #[test]
    fn unknown_method_errors() {
        let req = Request {
            id: "7".into(),
            method: "frobnicate".into(),
            params: serde_json::Value::Null,
        };
        assert_eq!(
            req.call(),
            Err(ProtoError::UnknownMethod("frobnicate".into()))
        );
    }

    #[test]
    fn inject_missing_text_is_invalid_params() {
        let req = Request {
            id: "8".into(),
            method: "inject".into(),
            params: json!({"pane_id": "terminal_3"}),
        };
        match req.call() {
            Err(ProtoError::InvalidParams { method, .. }) => assert_eq!(method, "inject"),
            other => panic!("expected InvalidParams, got {other:?}"),
        }
    }

    // ── parse_requests — NDJSON framing ─────────────────────────────────

    #[test]
    fn parse_requests_splits_lines() {
        let payload = concat!(
            r#"{"id":"a","method":"cancel","params":{"pane_id":"terminal_1"}}"#,
            "\n",
            r#"{"id":"b","method":"list_agent_panes"}"#,
        );
        let out = parse_requests(payload);
        assert_eq!(out.len(), 2);
        assert_eq!(out[0].as_ref().unwrap().id, "a");
        assert_eq!(out[1].as_ref().unwrap().id, "b");
    }

    #[test]
    fn parse_requests_skips_blank_lines() {
        let payload = "\n  \n{\"id\":\"a\",\"method\":\"list_agent_panes\"}\n\n";
        let out = parse_requests(payload);
        assert_eq!(out.len(), 1);
        assert_eq!(out[0].as_ref().unwrap().id, "a");
    }

    #[test]
    fn parse_requests_reports_malformed_line() {
        let payload = concat!(
            r#"{"id":"a","method":"list_agent_panes"}"#,
            "\n",
            "not json at all",
        );
        let out = parse_requests(payload);
        assert_eq!(out.len(), 2);
        assert!(out[0].is_ok());
        assert!(matches!(out[1], Err(ProtoError::Json(_))));
    }

    // ── Response serialization ──────────────────────────────────────────

    #[test]
    fn response_ok_shape() {
        let resp = Response::ok("id1", json!({"state": "idle"}));
        let v: serde_json::Value = serde_json::from_str(&resp.to_ndjson_line()).unwrap();
        assert_eq!(v["id"], "id1");
        assert_eq!(v["ok"], true);
        assert_eq!(v["result"]["state"], "idle");
        assert!(v.get("error").is_none(), "error must be omitted on ok");
    }

    #[test]
    fn response_error_shape() {
        let resp = Response::error("id2", "boom");
        let v: serde_json::Value = serde_json::from_str(&resp.to_ndjson_line()).unwrap();
        assert_eq!(v["id"], "id2");
        assert_eq!(v["ok"], false);
        assert_eq!(v["error"], "boom");
        assert!(v.get("result").is_none(), "result must be omitted on error");
    }

    #[test]
    fn ndjson_line_has_no_embedded_newline() {
        let resp = Response::ok("id3", json!({"a": 1}));
        assert!(!resp.to_ndjson_line().contains('\n'));
    }

    // ── PluginEvent serialization ───────────────────────────────────────

    #[test]
    fn command_pane_exited_serializes_with_tag() {
        let ev = PluginEvent::CommandPaneExited {
            pane_id: 3,
            exit_code: Some(0),
        };
        let v = serde_json::to_value(&ev).unwrap();
        assert_eq!(v["event"], "command_pane_exited");
        assert_eq!(v["pane_id"], 3);
        assert_eq!(v["exit_code"], 0);
    }

    #[test]
    fn plugin_event_round_trips() {
        let ev = PluginEvent::PaneClosed { pane_id: 9 };
        let s = serde_json::to_string(&ev).unwrap();
        assert_eq!(serde_json::from_str::<PluginEvent>(&s).unwrap(), ev);
    }

    #[test]
    fn plugin_event_ndjson_line_round_trips() {
        let ev = PluginEvent::CommandPaneExited {
            pane_id: 4,
            exit_code: None,
        };
        let line = ev.to_ndjson_line();
        assert!(!line.contains('\n'));
        assert_eq!(serde_json::from_str::<PluginEvent>(&line).unwrap(), ev);
    }

    // ── classify_state / is_idle_screen (ported, behavior-identical to
    //    edgeplaned-runtimes::zellij_session) ────────────────────────────

    #[test]
    fn idle_detects_bare_prompt() {
        assert!(is_idle_screen(&["❯"]));
    }

    #[test]
    fn idle_detects_prompt_with_space() {
        assert!(is_idle_screen(&["❯ some context"]));
    }

    #[test]
    fn idle_handles_leading_whitespace() {
        assert!(is_idle_screen(&["  ❯ "]));
    }

    #[test]
    fn idle_returns_false_for_empty() {
        let v: Vec<&str> = vec![];
        assert!(!is_idle_screen(&v));
    }

    #[test]
    fn idle_returns_false_for_normal_output() {
        assert!(!is_idle_screen(&["thinking...", "Running tool: web_search"]));
    }

    #[test]
    fn classify_idle() {
        assert_eq!(classify_state(&["❯"]), AgentState::Idle);
    }

    #[test]
    fn classify_working_via_running_tool() {
        assert_eq!(
            classify_state(&["Running tool: read_file", "path: foo.rs"]),
            AgentState::Working
        );
    }

    #[test]
    fn classify_working_via_spinner() {
        assert_eq!(classify_state(&["⠋ thinking..."]), AgentState::Working);
    }

    #[test]
    fn classify_error() {
        assert_eq!(
            classify_state(&["Error: rate limit exceeded"]),
            AgentState::Error
        );
    }

    #[test]
    fn classify_error_via_xmark() {
        assert_eq!(classify_state(&["✗ build failed"]), AgentState::Error);
    }

    #[test]
    fn classify_unknown() {
        assert_eq!(classify_state(&["some random line"]), AgentState::Unknown);
    }

    #[test]
    fn classify_prefers_idle_over_other_signals() {
        assert_eq!(
            classify_state(&["Running tool: foo", "❯"]),
            AgentState::Idle
        );
    }

    #[test]
    fn agent_state_labels() {
        assert_eq!(AgentState::Idle.as_str(), "idle");
        assert_eq!(AgentState::Working.as_str(), "working");
        assert_eq!(AgentState::Error.as_str(), "error");
        assert_eq!(AgentState::Unknown.as_str(), "unknown");
    }
}
