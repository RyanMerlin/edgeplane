//! `mc agent supervise watch` — live fleet dashboard.
//!
//! Two-pane ratatui screen:
//!   - top: agent supervise list (polled every `--poll-secs`, default 5s)
//!   - bottom: scrolling SupervisorEvent tail (streamed via mgmt-gateway
//!     `events.subscribe`)
//!
//! Both panes read from the local mcd over the Unix socket (no controlplane
//! involvement). The poller and the streamer each own one Unix-socket
//! connection; the render loop reads a shared state Mutex.
//!
//! `q` or Esc exits.

use anyhow::{Context, Result, bail};
use crossterm::{
    event::{self as cevent, Event, KeyCode, KeyEventKind},
    execute,
    terminal::{EnterAlternateScreen, LeaveAlternateScreen, disable_raw_mode, enable_raw_mode},
};
use ratatui::{
    Terminal,
    backend::CrosstermBackend,
    layout::{Constraint, Direction, Layout},
    style::{Color, Modifier, Style},
    text::{Line, Span},
    widgets::{Block, Borders, Cell, Paragraph, Row, Table},
};
use serde_json::{Value, json};
use std::collections::VecDeque;
use std::io::stdout;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};
use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};

pub use crate::agent_supervise::WatchArgs;

/// Snapshot row for one supervised agent.
#[derive(Debug, Clone, Default)]
struct AgentRow {
    agent_id: String,
    systemd_service: String,
    unit_state: String,
    supervise_paused: bool,
    /// Set + updated by streamed events; persists across snapshot polls
    /// so the operator can see "the last thing that happened" even when
    /// the event itself has scrolled off the tail.
    last_event_label: Option<String>,
    last_event_at: Option<String>,
}

/// One rendered line in the events tail.
#[derive(Debug, Clone)]
struct TailLine {
    text: String,
    color: Color,
}

#[derive(Debug, Default)]
struct State {
    agents: Vec<AgentRow>,
    /// Newest events at the back; bounded by `WatchArgs.tail_size`.
    tail: VecDeque<TailLine>,
    last_poll_ok: Option<Instant>,
    last_poll_err: Option<String>,
    stream_status: StreamStatus,
}

#[derive(Debug, Clone, Default)]
enum StreamStatus {
    #[default]
    Connecting,
    Live,
    Error(String),
    Closed,
}

pub async fn run(args: WatchArgs) -> Result<()> {
    let state = Arc::new(Mutex::new(State::default()));
    let tail_size = args.tail_size;
    let poll_interval = Duration::from_secs(args.poll_secs.max(1));

    // Background: snapshot poller.
    let state_poll = Arc::clone(&state);
    let poller = tokio::spawn(async move {
        let mut ticker = tokio::time::interval(poll_interval);
        loop {
            ticker.tick().await;
            match poll_snapshot().await {
                Ok(rows) => {
                    let mut s = state_poll.lock().unwrap();
                    merge_snapshot(&mut s.agents, rows);
                    s.last_poll_ok = Some(Instant::now());
                    s.last_poll_err = None;
                }
                Err(e) => {
                    let mut s = state_poll.lock().unwrap();
                    s.last_poll_err = Some(format!("{e:#}"));
                }
            }
        }
    });

    // Background: event streamer.
    let state_stream = Arc::clone(&state);
    let streamer = tokio::spawn(async move {
        loop {
            {
                let mut s = state_stream.lock().unwrap();
                s.stream_status = StreamStatus::Connecting;
            }
            match stream_events(Arc::clone(&state_stream), tail_size).await {
                Ok(()) => {
                    let mut s = state_stream.lock().unwrap();
                    s.stream_status = StreamStatus::Closed;
                    // mcd hung up — back off briefly then retry.
                }
                Err(e) => {
                    let mut s = state_stream.lock().unwrap();
                    s.stream_status = StreamStatus::Error(format!("{e:#}"));
                }
            }
            tokio::time::sleep(Duration::from_secs(2)).await;
        }
    });

    // Render loop on a blocking thread — crossterm event reading is sync,
    // and ratatui rendering is sync. We poll the shared state every tick.
    let render_state: Arc<Mutex<State>> = Arc::clone(&state);
    let render_result = tokio::task::spawn_blocking(move || render_loop(render_state)).await;

    // Tear down workers no matter how the render loop exited.
    poller.abort();
    streamer.abort();

    render_result.context("render thread joined")?
}

// ─── Background workers ──────────────────────────────────────────────────────

async fn poll_snapshot() -> Result<Vec<AgentRow>> {
    let resp = call_mgmt_once("agent.supervise.list", json!({})).await?;
    let agents = resp.get("agents").and_then(|v| v.as_array()).cloned().unwrap_or_default();
    let mut rows = Vec::with_capacity(agents.len());
    for a in agents {
        rows.push(AgentRow {
            agent_id: a.get("agent_id").and_then(|v| v.as_str()).unwrap_or("?").to_string(),
            systemd_service: a.get("systemd_service").and_then(|v| v.as_str()).unwrap_or("-").to_string(),
            unit_state: a.get("unit_state").and_then(|v| v.as_str()).unwrap_or("?").to_string(),
            supervise_paused: a.get("supervise_paused").and_then(|v| v.as_bool()).unwrap_or(false),
            last_event_label: None,
            last_event_at: None,
        });
    }
    Ok(rows)
}

/// Merge a fresh snapshot into the table while preserving each row's
/// `last_event_*` fields (those come from the stream, not the snapshot).
fn merge_snapshot(existing: &mut Vec<AgentRow>, fresh: Vec<AgentRow>) {
    let mut new_rows = Vec::with_capacity(fresh.len());
    for mut row in fresh {
        if let Some(prev) = existing.iter().find(|p| p.agent_id == row.agent_id) {
            row.last_event_label = prev.last_event_label.clone();
            row.last_event_at = prev.last_event_at.clone();
        }
        new_rows.push(row);
    }
    *existing = new_rows;
}

async fn stream_events(state: Arc<Mutex<State>>, tail_size: usize) -> Result<()> {
    let path = mgmt_socket_path();
    let stream = tokio::net::UnixStream::connect(&path).await
        .with_context(|| format!("connect to {} (is mcd running?)", path.display()))?;
    let (read_half, mut write_half) = stream.into_split();
    let mut reader = BufReader::new(read_half);

    let req = json!({
        "jsonrpc": "2.0",
        "id": 1,
        "method": "events.subscribe",
        "params": {},
    });
    let mut bytes = serde_json::to_vec(&req)?;
    bytes.push(b'\n');
    write_half.write_all(&bytes).await?;

    // Ack frame.
    let mut ack_line = String::new();
    reader.read_line(&mut ack_line).await.context("read ack")?;
    let ack: Value = serde_json::from_str(ack_line.trim())
        .with_context(|| format!("parse ack: {}", ack_line.trim()))?;
    if let Some(err) = ack.get("error") {
        bail!(
            "mgmt-gateway: {}",
            err.get("message").and_then(|m| m.as_str()).unwrap_or("unknown error")
        );
    }
    {
        let mut s = state.lock().unwrap();
        s.stream_status = StreamStatus::Live;
    }

    // Event frames.
    let mut line = String::new();
    loop {
        line.clear();
        let n = reader.read_line(&mut line).await.context("read event")?;
        if n == 0 {
            return Ok(());
        }
        let trimmed = line.trim();
        if trimmed.is_empty() {
            continue;
        }
        let frame: Value = match serde_json::from_str(trimmed) {
            Ok(v) => v,
            Err(_) => continue,
        };
        if frame.get("error").and_then(|v| v.as_str()) == Some("lag") {
            let skipped = frame.get("skipped").and_then(|v| v.as_u64()).unwrap_or(0);
            bail!("broadcast lag — {skipped} events skipped");
        }

        // Apply the event: update the agent row's `last_event_*` AND push
        // to the tail.
        let mut s = state.lock().unwrap();
        apply_event(&mut s, &frame, tail_size);
    }
}

fn apply_event(s: &mut State, frame: &Value, tail_size: usize) {
    let kind = frame.get("kind").and_then(|v| v.as_str()).unwrap_or("?");
    let at = frame.get("at").and_then(|v| v.as_str()).unwrap_or("?");
    let agent = frame.get("agent_id").and_then(|v| v.as_str()).unwrap_or("?");
    let label = match kind {
        "unit_dead_detected" => "DEAD".to_string(),
        "unit_restarted" => {
            let result = frame.get("result").and_then(|v| v.as_str()).unwrap_or("?");
            format!("RESTART/{result}")
        }
        "supervise_paused" => "PAUSED".to_string(),
        "supervise_resumed" => "RESUMED".to_string(),
        "nightly_restart_fired" => "NIGHTLY".to_string(),
        other => other.to_uppercase(),
    };

    // Update the per-agent "last event" pointer.
    if let Some(row) = s.agents.iter_mut().find(|r| r.agent_id == agent) {
        row.last_event_label = Some(label.clone());
        row.last_event_at = Some(short_time(at).to_string());
    }

    // Append to the tail.
    let line_text = match kind {
        "unit_restarted" => {
            let reason = frame.get("reason").and_then(|v| v.as_str()).unwrap_or("?");
            let result = frame.get("result").and_then(|v| v.as_str()).unwrap_or("?");
            format!("{}  RESTART    {agent} reason={reason} result={result}", short_time(at))
        }
        "unit_dead_detected" => format!("{}  DEAD       {agent}", short_time(at)),
        "supervise_paused" => format!("{}  PAUSED     {agent}", short_time(at)),
        "supervise_resumed" => format!("{}  RESUMED    {agent}", short_time(at)),
        "nightly_restart_fired" => format!("{}  NIGHTLY    {agent}", short_time(at)),
        other => format!("{}  {other:10} {agent}", short_time(at)),
    };
    let color = match kind {
        "unit_dead_detected" => Color::Red,
        "unit_restarted" => Color::Yellow,
        "supervise_paused" | "supervise_resumed" => Color::Cyan,
        "nightly_restart_fired" => Color::Magenta,
        _ => Color::Gray,
    };
    s.tail.push_back(TailLine { text: line_text, color });
    while s.tail.len() > tail_size {
        s.tail.pop_front();
    }
}

/// Take an RFC3339 string like `2026-05-20T20:30:00Z` and return `20:30:00`.
/// Falls back to the input if parsing fails.
fn short_time(at: &str) -> &str {
    at.split('T').nth(1).and_then(|t| t.split('.').next()).map(|t| t.trim_end_matches('Z')).unwrap_or(at)
}

// ─── Render loop ─────────────────────────────────────────────────────────────

fn render_loop(state: Arc<Mutex<State>>) -> Result<()> {
    enable_raw_mode().context("enable raw mode")?;
    let mut out = stdout();
    execute!(out, EnterAlternateScreen).context("enter alt screen")?;
    let backend = CrosstermBackend::new(out);
    let mut terminal = Terminal::new(backend).context("init terminal")?;

    let result = (|| -> Result<()> {
        loop {
            // Read shared state under a blocking lock — held briefly
            // for a single clone of the rendering inputs.
            let snapshot = state.lock().unwrap().clone_snapshot();
            terminal.draw(|f| render(f, &snapshot))?;

            // Poll keyboard with 100ms timeout so the screen refreshes
            // even when the user isn't typing.
            if cevent::poll(Duration::from_millis(100))? {
                if let Event::Key(key) = cevent::read()? {
                    if key.kind != KeyEventKind::Press {
                        continue;
                    }
                    match key.code {
                        KeyCode::Char('q') | KeyCode::Esc => break,
                        KeyCode::Char('c')
                            if key
                                .modifiers
                                .contains(crossterm::event::KeyModifiers::CONTROL) =>
                        {
                            break;
                        }
                        _ => {}
                    }
                }
            }
        }
        Ok(())
    })();

    disable_raw_mode().ok();
    execute!(terminal.backend_mut(), LeaveAlternateScreen).ok();
    terminal.show_cursor().ok();
    result
}

/// A cheap snapshot of the shared state for one render frame. Avoids
/// holding the lock across `terminal.draw()`.
#[derive(Default, Clone)]
struct RenderSnapshot {
    agents: Vec<AgentRow>,
    tail: Vec<TailLine>,
    last_poll_ok: Option<Instant>,
    last_poll_err: Option<String>,
    stream_status: StreamStatus,
}

impl State {
    fn clone_snapshot(&self) -> RenderSnapshot {
        RenderSnapshot {
            agents: self.agents.clone(),
            tail: self.tail.iter().cloned().collect(),
            last_poll_ok: self.last_poll_ok,
            last_poll_err: self.last_poll_err.clone(),
            stream_status: self.stream_status.clone(),
        }
    }
}

fn render(f: &mut ratatui::Frame, s: &RenderSnapshot) {
    let chunks = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Length(3),    // header
            Constraint::Min(8),       // agents table
            Constraint::Length(12),   // events tail
            Constraint::Length(1),    // footer
        ])
        .split(f.area());

    // Header.
    let status_line = match &s.stream_status {
        StreamStatus::Connecting => Span::styled("● connecting", Style::default().fg(Color::Yellow)),
        StreamStatus::Live => Span::styled("● live", Style::default().fg(Color::Green)),
        StreamStatus::Closed => Span::styled("● closed (reconnecting)", Style::default().fg(Color::Yellow)),
        StreamStatus::Error(e) => Span::styled(format!("● {e}"), Style::default().fg(Color::Red)),
    };
    let poll_line = if let Some(err) = &s.last_poll_err {
        Span::styled(format!("snapshot: ERR {err}"), Style::default().fg(Color::Red))
    } else if let Some(t) = s.last_poll_ok {
        Span::styled(
            format!("snapshot: {}s ago", t.elapsed().as_secs()),
            Style::default().fg(Color::DarkGray),
        )
    } else {
        Span::styled("snapshot: …", Style::default().fg(Color::DarkGray))
    };
    let header = Paragraph::new(vec![
        Line::from(vec![
            Span::styled("mc agent supervise watch", Style::default().add_modifier(Modifier::BOLD)),
            Span::raw("   "),
            status_line,
            Span::raw("   "),
            poll_line,
        ]),
    ])
    .block(Block::default().borders(Borders::BOTTOM));
    f.render_widget(header, chunks[0]);

    // Agents table.
    let header_row = Row::new(vec![
        Cell::from("AGENT"),
        Cell::from("SYSTEMD"),
        Cell::from("STATE"),
        Cell::from("PAUSED"),
        Cell::from("LAST"),
    ])
    .style(Style::default().add_modifier(Modifier::BOLD));

    let rows: Vec<Row> = s.agents.iter().map(|a| {
        let state_color = match a.unit_state.as_str() {
            "active" => Color::Green,
            "failed" | "inactive" => Color::Red,
            "activating" | "deactivating" => Color::Yellow,
            _ => Color::Gray,
        };
        let paused_cell = if a.supervise_paused {
            Cell::from("YES").style(Style::default().fg(Color::Cyan))
        } else {
            Cell::from("no").style(Style::default().fg(Color::DarkGray))
        };
        let last = match (&a.last_event_at, &a.last_event_label) {
            (Some(at), Some(label)) => format!("{at} {label}"),
            _ => "-".to_string(),
        };
        Row::new(vec![
            Cell::from(a.agent_id.clone()),
            Cell::from(a.systemd_service.clone()),
            Cell::from(a.unit_state.clone()).style(Style::default().fg(state_color)),
            paused_cell,
            Cell::from(last),
        ])
    }).collect();

    let table = Table::new(
        rows,
        [
            Constraint::Length(24),
            Constraint::Length(28),
            Constraint::Length(12),
            Constraint::Length(8),
            Constraint::Min(20),
        ],
    )
    .header(header_row)
    .block(Block::default().borders(Borders::ALL).title(" Fleet "));
    f.render_widget(table, chunks[1]);

    // Events tail.
    let tail_lines: Vec<Line> = s.tail.iter().rev().take(chunks[2].height.saturating_sub(2) as usize).rev().map(|l| {
        Line::from(Span::styled(l.text.clone(), Style::default().fg(l.color)))
    }).collect();
    let tail_widget = Paragraph::new(tail_lines)
        .block(Block::default().borders(Borders::ALL).title(" Events (live) "));
    f.render_widget(tail_widget, chunks[2]);

    // Footer.
    let footer = Paragraph::new(Line::from(vec![
        Span::styled("q/Esc/Ctrl-C", Style::default().fg(Color::Cyan)),
        Span::raw(" quit   "),
        Span::styled(format!("{} agents · {} events buffered", s.agents.len(), s.tail.len()), Style::default().fg(Color::DarkGray)),
    ]))
    .style(Style::default().bg(Color::Reset));
    f.render_widget(footer, chunks[3]);
}

// ─── mgmt-gateway helpers (parallel to agent_supervise.rs, kept local
// so the watch module is self-contained) ─────────────────────────────────────

async fn call_mgmt_once(method: &str, params: Value) -> Result<Value> {
    let path = mgmt_socket_path();
    let stream = tokio::net::UnixStream::connect(&path).await.with_context(|| {
        format!("connect to {} (is mcd running?)", path.display())
    })?;
    let (read_half, mut write_half) = stream.into_split();
    let mut reader = BufReader::new(read_half);

    let request = json!({
        "jsonrpc": "2.0",
        "id": 1,
        "method": method,
        "params": params,
    });
    let mut bytes = serde_json::to_vec(&request)?;
    bytes.push(b'\n');
    write_half.write_all(&bytes).await?;

    let mut line = String::new();
    reader.read_line(&mut line).await.context("read response")?;

    let parsed: Value = serde_json::from_str(line.trim())
        .with_context(|| format!("parse mgmt response: {}", line.trim()))?;

    if let Some(err) = parsed.get("error") {
        let code = err.get("code").and_then(|c| c.as_i64()).unwrap_or(0);
        let msg = err.get("message").and_then(|m| m.as_str()).unwrap_or("unknown error");
        bail!("mgmt-gateway error {code}: {msg}");
    }
    Ok(parsed.get("result").cloned().unwrap_or(Value::Null))
}

fn mgmt_socket_path() -> std::path::PathBuf {
    crate::config::mc_home_dir().join("mcd").join("mgmt.sock")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn short_time_extracts_hms() {
        assert_eq!(short_time("2026-05-20T20:30:00Z"), "20:30:00");
        assert_eq!(short_time("2026-05-20T20:30:00.123Z"), "20:30:00");
        assert_eq!(short_time("malformed"), "malformed");
    }

    #[test]
    fn merge_snapshot_preserves_last_event() {
        let mut existing = vec![AgentRow {
            agent_id: "work".to_string(),
            systemd_service: "old.service".to_string(),
            unit_state: "active".to_string(),
            supervise_paused: false,
            last_event_label: Some("RESTART/started".to_string()),
            last_event_at: Some("14:23:01".to_string()),
        }];
        let fresh = vec![AgentRow {
            agent_id: "work".to_string(),
            systemd_service: "new.service".to_string(),
            unit_state: "failed".to_string(),
            supervise_paused: true,
            last_event_label: None, // fresh snapshot doesn't know
            last_event_at: None,
        }];
        merge_snapshot(&mut existing, fresh);
        let row = &existing[0];
        // Snapshot fields take the fresh values.
        assert_eq!(row.systemd_service, "new.service");
        assert_eq!(row.unit_state, "failed");
        assert!(row.supervise_paused);
        // Event fields are preserved from before the poll.
        assert_eq!(row.last_event_label.as_deref(), Some("RESTART/started"));
        assert_eq!(row.last_event_at.as_deref(), Some("14:23:01"));
    }

    #[test]
    fn apply_event_updates_agent_row_and_tail() {
        let mut s = State::default();
        s.agents.push(AgentRow {
            agent_id: "work".to_string(),
            systemd_service: "aria-work.service".to_string(),
            unit_state: "active".to_string(),
            supervise_paused: false,
            last_event_label: None,
            last_event_at: None,
        });
        let frame = json!({
            "kind": "unit_restarted",
            "agent_id": "work",
            "result": "started",
            "reason": "manual",
            "at": "2026-05-20T14:23:01Z",
        });
        apply_event(&mut s, &frame, 100);
        assert_eq!(s.agents[0].last_event_label.as_deref(), Some("RESTART/started"));
        assert_eq!(s.agents[0].last_event_at.as_deref(), Some("14:23:01"));
        assert_eq!(s.tail.len(), 1);
        assert!(s.tail.back().unwrap().text.contains("RESTART"));
        assert_eq!(s.tail.back().unwrap().color, Color::Yellow);
    }

    #[test]
    fn apply_event_trims_tail_to_capacity() {
        let mut s = State::default();
        for i in 0..10 {
            let frame = json!({
                "kind": "supervise_paused",
                "agent_id": format!("agent{i}"),
                "at": "2026-05-20T14:23:01Z",
            });
            apply_event(&mut s, &frame, 3);
        }
        assert_eq!(s.tail.len(), 3);
        // Newest events retained (FIFO drop from the front).
        assert!(s.tail.back().unwrap().text.contains("agent9"));
        assert!(s.tail.front().unwrap().text.contains("agent7"));
    }
}
