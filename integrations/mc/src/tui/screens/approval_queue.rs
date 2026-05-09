use ratatui::{
    buffer::Buffer,
    layout::{Constraint, Direction, Layout, Rect},
    text::{Line, Span},
    widgets::{Block, BorderType, Borders, List, ListItem, ListState, Paragraph, Widget},
};
use serde::{Deserialize, Serialize};

use crate::tui::theme;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ApprovalRequest {
    pub id: i64,
    #[serde(default)]
    pub mission_id: Option<String>,
    pub action: String,
    #[serde(default)]
    pub channel: Option<String>,
    #[serde(default)]
    pub reason: Option<String>,
    #[serde(default)]
    pub requested_by: Option<String>,
    pub status: String,
}

#[derive(Debug, Clone, PartialEq)]
pub enum Focus {
    Queue,
    Detail,
}

#[derive(Debug, Default)]
pub struct ApprovalQueueState {
    pub focus: Focus,
    pub pending: Vec<ApprovalRequest>,
    pub history: Vec<(String, String)>, // (action, decision)
    pub selection: usize,
    pub loading: bool,
    pub last_error: Option<String>,
    /// Set when y/n was pressed; cleared after dispatch.
    pub pending_response: Option<(i64, bool)>,
}

impl Default for Focus {
    fn default() -> Self { Focus::Queue }
}

impl ApprovalQueueState {
    pub fn handle_key(&mut self, key: crossterm::event::KeyCode) -> bool {
        use crossterm::event::KeyCode::*;
        match key {
            Right => {
                self.focus = Focus::Detail;
                true
            }
            Left => {
                self.focus = Focus::Queue;
                true
            }
            Up if self.focus == Focus::Queue => {
                if self.selection > 0 { self.selection -= 1; }
                true
            }
            Down if self.focus == Focus::Queue => {
                if self.selection + 1 < self.pending.len() { self.selection += 1; }
                true
            }
            Char('y') => {
                if let Some(req) = self.pending.get(self.selection) {
                    self.pending_response = Some((req.id, true));
                }
                true
            }
            Char('n') => {
                if let Some(req) = self.pending.get(self.selection) {
                    self.pending_response = Some((req.id, false));
                }
                true
            }
            Char('s') => {
                if self.selection + 1 < self.pending.len() { self.selection += 1; }
                true
            }
            _ => false,
        }
    }

    /// Take and clear the pending response (called by app.rs after dispatching).
    pub fn take_pending_response(&mut self) -> Option<(i64, bool)> {
        self.pending_response.take()
    }

    pub fn selected(&self) -> Option<&ApprovalRequest> {
        self.pending.get(self.selection)
    }
}

pub struct ApprovalQueue<'a> {
    pub state: &'a ApprovalQueueState,
}

impl<'a> Widget for ApprovalQueue<'a> {
    fn render(self, area: Rect, buf: &mut Buffer) {
        let bg = Block::default().style(theme::normal());
        bg.render(area, buf);

        let chunks = Layout::default()
            .direction(Direction::Horizontal)
            .constraints([Constraint::Percentage(45), Constraint::Percentage(55)])
            .split(area);

        render_queue(buf, chunks[0], self.state);
        render_detail(buf, chunks[1], self.state);
    }
}

fn render_queue(buf: &mut Buffer, area: Rect, state: &ApprovalQueueState) {
    let focused = state.focus == Focus::Queue;
    let outer = Layout::default()
        .direction(Direction::Vertical)
        .constraints([Constraint::Fill(1), Constraint::Length(8)])
        .split(area);

    // Pending list
    let block = Block::default()
        .borders(Borders::ALL)
        .border_type(BorderType::Rounded)
        .border_style(theme::border_for(focused))
        .title(Span::styled(" Pending Approvals ", theme::panel_title()))
        .style(theme::normal());
    let inner = block.inner(outer[0]);
    block.render(outer[0], buf);

    if state.loading {
        Paragraph::new(Span::styled("loading…", theme::dim()))
            .style(theme::normal())
            .render(inner, buf);
        return;
    }

    if state.pending.is_empty() {
        Paragraph::new(Line::from(vec![
            Span::styled("✓ ", theme::ok()),
            Span::styled("no pending approvals", theme::dim()),
        ]))
        .style(theme::normal())
        .render(inner, buf);
    } else {
        let items: Vec<ListItem> = state.pending.iter().enumerate().map(|(i, req)| {
            let selected = i == state.selection && focused;
            let style = if selected { theme::selected() } else { theme::normal() };
            ListItem::new(Line::from(vec![
                Span::styled(format!("{:<20} ", truncate(&req.action, 18)), style),
                Span::styled(
                    req.mission_id.as_deref().unwrap_or("—"),
                    theme::dim(),
                ),
            ]))
        }).collect();
        let mut ls = ListState::default().with_selected(
            if focused { Some(state.selection) } else { None }
        );
        ratatui::widgets::StatefulWidget::render(
            List::new(items).style(theme::normal()),
            inner, buf, &mut ls,
        );
    }

    // History
    let hblock = Block::default()
        .borders(Borders::ALL)
        .border_type(BorderType::Rounded)
        .border_style(theme::border_normal())
        .title(Span::styled(" Recent Decisions ", theme::panel_title()))
        .style(theme::normal());
    let hinner = hblock.inner(outer[1]);
    hblock.render(outer[1], buf);

    let hist_items: Vec<Line> = state.history.iter().rev().take(5).map(|(action, decision)| {
        let (dot, sty) = match decision.as_str() {
            "approved" => ("✓", theme::ok()),
            "denied" => ("✗", theme::err()),
            _ => ("?", theme::dim()),
        };
        Line::from(vec![
            Span::styled(dot, sty),
            Span::styled(format!(" {}", truncate(action, 16)), theme::dim()),
            Span::styled(format!("  {}", decision), sty),
        ])
    }).collect();
    Paragraph::new(hist_items).style(theme::normal()).render(hinner, buf);
}

fn render_detail(buf: &mut Buffer, area: Rect, state: &ApprovalQueueState) {
    let focused = state.focus == Focus::Detail;
    let block = Block::default()
        .borders(Borders::ALL)
        .border_type(BorderType::Rounded)
        .border_style(theme::border_for(focused))
        .title(Span::styled(" Request Detail ", theme::panel_title()))
        .style(theme::normal());
    let inner = block.inner(area);
    block.render(area, buf);

    let Some(req) = state.selected() else {
        Paragraph::new(Span::styled("select a request", theme::muted()))
            .style(theme::normal())
            .render(inner, buf);
        return;
    };

    let mut lines: Vec<Line> = vec![
        Line::from(vec![
            Span::styled("Action  ", theme::muted()),
            Span::styled(req.action.clone(), theme::accent()),
        ]),
        Line::from(vec![
            Span::styled("Channel ", theme::muted()),
            Span::styled(req.channel.as_deref().unwrap_or("—"), theme::dim()),
        ]),
        Line::from(vec![
            Span::styled("By      ", theme::muted()),
            Span::styled(req.requested_by.as_deref().unwrap_or("—"), theme::dim()),
        ]),
        Line::from(vec![
            Span::styled("Mission ", theme::muted()),
            Span::styled(req.mission_id.as_deref().unwrap_or("—"), theme::dim()),
        ]),
        Line::from(""),
    ];

    if let Some(reason) = &req.reason {
        lines.push(Line::from(Span::styled("Reason", theme::muted())));
        for part in reason.lines().take(5) {
            lines.push(Line::from(Span::styled(part.to_string(), theme::dim())));
        }
        lines.push(Line::from(""));
    }

    // Action hint
    lines.push(Line::from(vec![
        Span::styled("  y ", theme::ok()),
        Span::styled("approve  ", theme::dim()),
        Span::styled("  n ", theme::err()),
        Span::styled("deny  ", theme::dim()),
        Span::styled("  s ", theme::muted()),
        Span::styled("skip", theme::muted()),
    ]));

    Paragraph::new(lines)
        .wrap(ratatui::widgets::Wrap { trim: true })
        .style(theme::normal())
        .render(inner, buf);
}

fn truncate(s: &str, max: usize) -> String {
    if s.len() <= max { s.to_string() }
    else { format!("{}…", &s[..max.saturating_sub(1)]) }
}
