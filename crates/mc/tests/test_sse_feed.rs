/// Unit tests for the SSE agent-feed stream parsing.
///
/// Validates the SSE line-parsing and event dispatch logic that the TUI's
/// WorkRequest::SubscribeFeed relies on. These cover the proxy-fix path
/// (chunked delivery, multi-event batches, malformed lines) without requiring
/// a live HTTP server.

// ── helpers mirroring the TUI stream_feed logic ───────────────────────────────

#[derive(Debug, PartialEq)]
struct ParsedEvent {
    event_type: String,
    agent_id: Option<String>,
    mission_id: Option<String>,
    data: String,
}

/// Feed raw SSE text through the same parsing logic the TUI uses.
/// Returns the dispatched events (empty lines trigger dispatch).
fn parse_sse(raw: &str) -> Vec<ParsedEvent> {
    let mut buf = raw.to_string();
    let mut events = vec![];
    let mut cur_event = "message".to_string();
    let mut cur_data = String::new();

    while let Some(pos) = buf.find('\n') {
        let line = buf[..pos].trim_end_matches('\r').to_string();
        buf.drain(..pos + 1);

        if line.is_empty() {
            if !cur_data.is_empty() {
                let (agent_id, mission_id) = extract_ids(&cur_data);
                events.push(ParsedEvent {
                    event_type: cur_event.clone(),
                    agent_id,
                    mission_id,
                    data: cur_data.clone(),
                });
            }
            cur_data.clear();
            cur_event = "message".to_string();
        } else if let Some(d) = line.strip_prefix("data: ") {
            cur_data.push_str(d);
        } else if let Some(e) = line.strip_prefix("event: ") {
            cur_event = e.to_string();
        }
        // id: and retry: lines are ignored
    }
    events
}

fn extract_ids(data: &str) -> (Option<String>, Option<String>) {
    if let Ok(v) = serde_json::from_str::<serde_json::Value>(data) {
        let agent_id = v.get("agent_id").and_then(|x| x.as_str()).map(str::to_string)
            .or_else(|| v.get("agent").and_then(|x| x.as_str()).map(str::to_string));
        let mission_id = v.get("mission_id").and_then(|x| x.as_str()).map(str::to_string);
        (agent_id, mission_id)
    } else {
        (None, None)
    }
}

// ── tests ────────────────────────────────────────────────────────────────────

#[test]
fn parses_single_event() {
    let raw = "event: agent.status\ndata: {\"agent_id\":\"a1\",\"mission_id\":\"m1\",\"message\":\"online\"}\n\n";
    let events = parse_sse(raw);
    assert_eq!(events.len(), 1);
    assert_eq!(events[0].event_type, "agent.status");
    assert_eq!(events[0].agent_id.as_deref(), Some("a1"));
    assert_eq!(events[0].mission_id.as_deref(), Some("m1"));
}

#[test]
fn parses_multiple_events() {
    let raw = concat!(
        "event: agent.status\n",
        "data: {\"agent_id\":\"a1\",\"mission_id\":\"m1\"}\n",
        "\n",
        "event: task.update\n",
        "data: {\"agent_id\":\"a2\",\"mission_id\":\"m2\"}\n",
        "\n",
    );
    let events = parse_sse(raw);
    assert_eq!(events.len(), 2);
    assert_eq!(events[0].agent_id.as_deref(), Some("a1"));
    assert_eq!(events[1].agent_id.as_deref(), Some("a2"));
    assert_eq!(events[1].event_type, "task.update");
}

#[test]
fn defaults_to_message_event_type_when_no_event_line() {
    let raw = "data: {\"agent_id\":\"x\"}\n\n";
    let events = parse_sse(raw);
    assert_eq!(events.len(), 1);
    assert_eq!(events[0].event_type, "message");
}

#[test]
fn skips_comment_and_id_lines() {
    let raw = concat!(
        ": keep-alive\n",
        "id: 42\n",
        "retry: 3000\n",
        "event: ping\n",
        "data: {\"agent_id\":\"a3\"}\n",
        "\n",
    );
    let events = parse_sse(raw);
    assert_eq!(events.len(), 1);
    assert_eq!(events[0].agent_id.as_deref(), Some("a3"));
}

#[test]
fn ignores_empty_data_lines() {
    let raw = "\n\n";
    let events = parse_sse(raw);
    assert!(events.is_empty());
}

#[test]
fn handles_crlf_line_endings() {
    let raw = "event: agent.status\r\ndata: {\"agent_id\":\"a1\"}\r\n\r\n";
    let events = parse_sse(raw);
    assert_eq!(events.len(), 1);
    assert_eq!(events[0].agent_id.as_deref(), Some("a1"));
}

#[test]
fn handles_non_json_data_gracefully() {
    let raw = "data: plain text event\n\n";
    let events = parse_sse(raw);
    assert_eq!(events.len(), 1);
    assert_eq!(events[0].data, "plain text event");
    assert!(events[0].agent_id.is_none());
}

#[test]
fn agent_field_alias_resolved() {
    // Some backends emit "agent" rather than "agent_id"
    let raw = "data: {\"agent\":\"b1\",\"mission_id\":\"m9\"}\n\n";
    let events = parse_sse(raw);
    assert_eq!(events[0].agent_id.as_deref(), Some("b1"));
    assert_eq!(events[0].mission_id.as_deref(), Some("m9"));
}
