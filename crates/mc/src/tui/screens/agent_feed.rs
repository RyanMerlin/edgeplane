use ratatui::{
    buffer::Buffer,
    layout::{Constraint, Direction, Layout, Rect},
    style::{Modifier, Style},
    text::{Line, Span},
    widgets::{Block, BorderType, Borders, List, ListItem, ListState, Paragraph, Widget},
};
use serde::{Deserialize, Serialize};

use crate::tui::theme;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FeedEvent {
    pub ts: String,
    pub agent_id: Option<String>,
    pub mission_id: Option<String>,
    pub event_type: String,
    pub data: String,
}

#[derive(Debug, Default)]
pub struct AgentFeedState {
    pub events: Vec<FeedEvent>,
    pub paused: bool,
    pub selection: usize,
    pub live: bool,
    /// Counts events received while paused (shows user the stream is alive)
    pub buffered_while_paused: usize,
    /// Cap the in-memory ring buffer
    max_events: usize,
    // New v3 fields
    pub filter: String,
    pub filter_active: bool,
    pub alerts_only: bool,
    pub detail_open: bool,
}

impl AgentFeedState {
    pub fn new() -> Self {
        Self { max_events: 500, live: false, ..Default::default() }
    }

    pub fn push_event(&mut self, event: FeedEvent) {
        if self.paused {
            self.buffered_while_paused += 1;
            return;
        }
        self.events.push(event);
        if self.events.len() > self.max_events {
            self.events.remove(0);
        }
        // Keep selection at the tail when not scrolled up
        let visible_len = self.visible_events().len();
        if visible_len > 0 && self.selection + 5 >= visible_len.saturating_sub(5) {
            self.selection = visible_len - 1;
        }
    }

    pub fn visible_events(&self) -> Vec<&FeedEvent> {
        self.events.iter().filter(|ev| {
            if self.alerts_only && !is_alert(&ev.event_type) {
                return false;
            }
            if !self.filter.is_empty() {
                let q = self.filter.to_lowercase();
                let matches = ev.agent_id.as_deref().unwrap_or("").to_lowercase().contains(&q)
                    || ev.mission_id.as_deref().unwrap_or("").to_lowercase().contains(&q)
                    || ev.event_type.to_lowercase().contains(&q)
                    || ev.data.to_lowercase().contains(&q);
                if !matches {
                    return false;
                }
            }
            true
        }).collect()
    }

    pub fn handle_key(&mut self, key: crossterm::event::KeyCode) -> bool {
        use crossterm::event::KeyCode::*;

        // Filter input mode — swallow all keys so Tab can't escape to global nav
        if self.filter_active {
            match key {
                Char(c) => { self.filter.push(c); }
                Backspace => { self.filter.pop(); }
                Esc => { self.filter_active = false; }
                _ => {}
            }
            return true;
        }

        match key {
            Char('p') => {
                self.paused = !self.paused;
                if !self.paused { self.buffered_while_paused = 0; }
                true
            }
            Char('c') => {
                self.events.clear();
                self.selection = 0;
                self.buffered_while_paused = 0;
                true
            }
            Char('/') | Char('f') => {
                self.filter_active = true;
                true
            }
            Char('w') => {
                self.alerts_only = !self.alerts_only;
                // Clamp selection
                let visible_len = self.visible_events().len();
                if visible_len > 0 && self.selection >= visible_len {
                    self.selection = visible_len - 1;
                }
                true
            }
            Enter => {
                self.detail_open = !self.detail_open;
                true
            }
            Esc => {
                if self.detail_open {
                    self.detail_open = false;
                    return true;
                }
                false
            }
            Up => {
                if self.selection > 0 { self.selection -= 1; }
                true
            }
            Down => {
                let visible_len = self.visible_events().len();
                if self.selection + 1 < visible_len { self.selection += 1; }
                true
            }
            _ => false,
        }
    }
}

fn is_alert(event_type: &str) -> bool {
    matches!(event_type, "step_error" | "governance" | "overlap_detected" | "approval_needed")
}

pub struct AgentFeed<'a> {
    pub state: &'a AgentFeedState,
}

impl<'a> Widget for AgentFeed<'a> {
    fn render(self, area: Rect, buf: &mut Buffer) {
        let bg = Block::default().style(theme::normal());
        bg.render(area, buf);

        let chunks = Layout::default()
            .direction(Direction::Vertical)
            .constraints([Constraint::Length(1), Constraint::Fill(1)])
            .split(area);

        render_filter_bar(buf, chunks[0], self.state);

        // Content area: feed list + optional detail pane
        if self.state.detail_open {
            let content_chunks = Layout::default()
                .direction(Direction::Horizontal)
                .constraints([Constraint::Fill(1), Constraint::Length(40)])
                .split(chunks[1]);
            render_feed(buf, content_chunks[0], self.state);
            render_detail_panel(buf, content_chunks[1], self.state);
        } else {
            render_feed(buf, chunks[1], self.state);
        }
    }
}

fn render_filter_bar(buf: &mut Buffer, area: Rect, state: &AgentFeedState) {
    let live_span = if state.live && !state.paused {
        Span::styled("● LIVE", Style::default().fg(theme::OK).add_modifier(Modifier::BOLD))
    } else if state.paused {
        let buf_count = if state.buffered_while_paused > 0 {
            format!("PAUSED (+{})", state.buffered_while_paused)
        } else {
            "PAUSED".to_string()
        };
        Span::styled(buf_count, Style::default().fg(theme::WARN).add_modifier(Modifier::BOLD))
    } else {
        Span::styled("CONNECTING", Style::default().fg(theme::TEXT_DIM))
    };

    let filter_display = if state.filter_active {
        format!("/ [{}_ ]", state.filter)
    } else if !state.filter.is_empty() {
        format!("/ [{}]", state.filter)
    } else {
        "/ filter".to_string()
    };

    let alerts_span = if state.alerts_only {
        Span::styled("  ⚠ Alerts only", Style::default().fg(theme::WARN).add_modifier(Modifier::BOLD))
    } else {
        Span::styled("  [w alerts]", theme::dim())
    };

    let count = format!("  {} events", state.visible_events().len());

    let line = Line::from(vec![
        Span::styled(format!("  {filter_display}"), if state.filter_active { theme::accent() } else { theme::muted() }),
        Span::styled("  Errors  Governance  Artifacts  Heartbeat", theme::dim()),
        alerts_span,
        Span::styled("  ", theme::dim()),
        live_span,
        Span::styled(count, theme::dim()),
    ]);
    Paragraph::new(line).style(theme::normal()).render(area, buf);
}

fn render_feed(buf: &mut Buffer, area: Rect, state: &AgentFeedState) {
    let block = Block::default()
        .borders(Borders::ALL)
        .border_type(BorderType::Rounded)
        .border_style(theme::border_focused())
        .title(Span::styled(" Agent Feed ", theme::panel_title()))
        .style(theme::normal());
    let inner = block.inner(area);
    block.render(area, buf);

    let visible = state.visible_events();

    if visible.is_empty() {
        let msg = if state.live { "waiting for events…" } else { "connecting to backend…" };
        Paragraph::new(Span::styled(msg, theme::dim()))
            .style(theme::normal())
            .render(inner, buf);
        return;
    }

    let items: Vec<ListItem> = visible.iter().enumerate().map(|(i, ev)| {
        let selected = i == state.selection;
        let style = if selected { theme::selected() } else { theme::normal() };
        let agent = ev.agent_id.as_deref().unwrap_or("?");
        let context = ev.mission_id.as_deref().unwrap_or("—");
        let (type_style, type_str) = event_style(&ev.event_type);

        // Alert margin prefix
        let (margin, margin_style) = if matches!(ev.event_type.as_str(), "step_error") {
            ("▎ ", Style::default().fg(theme::ERR))
        } else if matches!(ev.event_type.as_str(), "governance" | "overlap_detected") {
            ("▎ ", Style::default().fg(theme::WARN))
        } else {
            ("  ", theme::dim())
        };

        ListItem::new(Line::from(vec![
            Span::styled(margin, margin_style),
            Span::styled(format!("{:<10} ", truncate(&ev.ts, 8)), theme::dim()),
            Span::styled(format!("{:<12} ", truncate(agent, 10)), theme::muted()),
            Span::styled(format!("{:<20} ", truncate(context, 18)), theme::dim()),
            Span::styled(format!("{:<16} ", truncate(type_str, 14)), type_style),
            Span::styled(truncate(&ev.data, 40), style),
        ]))
    }).collect();

    let mut ls = ListState::default().with_selected(Some(state.selection));
    ratatui::widgets::StatefulWidget::render(
        List::new(items).style(theme::normal()),
        inner,
        buf,
        &mut ls,
    );
}

fn render_detail_panel(buf: &mut Buffer, area: Rect, state: &AgentFeedState) {
    let visible = state.visible_events();
    let block = Block::default()
        .borders(Borders::ALL)
        .border_type(BorderType::Rounded)
        .border_style(theme::border_normal())
        .title(Span::styled(" Event Detail ", theme::panel_title()))
        .style(theme::normal());
    let inner = block.inner(area);
    block.render(area, buf);

    let Some(ev) = visible.get(state.selection) else {
        Paragraph::new(Span::styled("no event selected", theme::muted()))
            .style(theme::normal())
            .render(inner, buf);
        return;
    };

    let mut lines: Vec<Line> = vec![
        Line::from(vec![Span::styled("Time    ", theme::muted()), Span::styled(ev.ts.clone(), theme::normal())]),
        Line::from(vec![
            Span::styled("Agent   ", theme::muted()),
            Span::styled(ev.agent_id.as_deref().unwrap_or("—").to_string(), theme::accent()),
        ]),
        Line::from(vec![
            Span::styled("Mission ", theme::muted()),
            Span::styled(ev.mission_id.as_deref().unwrap_or("—").to_string(), theme::normal()),
        ]),
        Line::from(vec![Span::styled("Type    ", theme::muted()), Span::styled(ev.event_type.clone(), theme::muted())]),
        Line::from(""),
        Line::from(Span::styled("Data", theme::muted())),
        Line::from(""),
    ];

    // Try to pretty-print JSON data
    let data_display = if let Ok(v) = serde_json::from_str::<serde_json::Value>(&ev.data) {
        serde_json::to_string_pretty(&v).unwrap_or_else(|_| ev.data.clone())
    } else {
        ev.data.clone()
    };

    for line in data_display.lines().take(20) {
        lines.push(Line::from(Span::styled(line.to_string(), theme::dim())));
    }

    Paragraph::new(lines)
        .wrap(ratatui::widgets::Wrap { trim: true })
        .style(theme::normal())
        .render(inner, buf);
}

fn event_style(event_type: &str) -> (ratatui::style::Style, &str) {
    match event_type {
        "step_started" => (theme::accent(), "step_started"),
        "step_finished" => (theme::ok(), "step_finished"),
        "step_error" => (theme::err(), "step_error"),
        "task_finished" => (Style::default().fg(theme::ACCENT).add_modifier(Modifier::BOLD), "task_finished"),
        "approval_needed" => (theme::warn(), "approval_needed"),
        "artifact_produced" => (theme::purple(), "artifact_produced"),
        "task_claimed" => (theme::ok(), "task_claimed"),
        "heartbeat" => (theme::dim(), "heartbeat"),
        "kluster_started" => (theme::accent(), "kluster_started"),
        "mission_started" => (theme::accent(), "mission_started"),
        "governance" => (theme::warn(), "governance"),
        "overlap_detected" => (theme::warn(), "overlap_detected"),
        _ => (theme::muted(), event_type),
    }
}

fn truncate(s: &str, max: usize) -> String {
    if s.len() <= max {
        s.to_string()
    } else {
        format!("{}…", &s[..max.saturating_sub(1)])
    }
}
