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
    pub domain_id: Option<String>,
    pub action: String,
    #[serde(default)]
    pub channel: Option<String>,
    #[serde(default)]
    pub reason: Option<String>,
    #[serde(default)]
    pub requested_by: Option<String>,
    pub status: String,
    #[serde(default)]
    pub request_context: Option<serde_json::Value>,
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
    /// Approve responses dispatch immediately. Cleared after dispatch.
    pub pending_response: Option<(i64, bool)>,
    /// Deny is destructive — caller should wrap this in a confirm modal.
    /// Stores (approval_id, action_text) so the modal can show what's being denied.
    pub pending_deny_confirm: Option<(i64, String)>,
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
                    self.pending_deny_confirm = Some((req.id, req.action.clone()));
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

    /// Take and clear a deny that's awaiting confirmation. The caller wraps it
    /// in a modal and, on confirm, calls `confirm_deny` to record the response.
    pub fn take_pending_deny_confirm(&mut self) -> Option<(i64, String)> {
        self.pending_deny_confirm.take()
    }

    /// Record a deny after the user has confirmed via modal.
    pub fn confirm_deny(&mut self, approval_id: i64) {
        self.pending_response = Some((approval_id, false));
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
                    req.domain_id.as_deref().unwrap_or("—"),
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
            Span::styled("Domain ", theme::muted()),
            Span::styled(req.domain_id.as_deref().unwrap_or("—"), theme::dim()),
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

    if let Some(ctx) = &req.request_context {
        lines.push(Line::from(Span::styled("Context", theme::muted())));
        let tool = ctx.get("tool").or_else(|| ctx.get("name"))
            .and_then(|v| v.as_str())
            .unwrap_or("");
        match tool {
            "str_replace_editor" | "Edit" => {
                if let Some(path) = ctx.get("path").and_then(|v| v.as_str()) {
                    lines.push(Line::from(Span::styled(path.to_string(), theme::accent())));
                }
                if let Some(old) = ctx.get("old_string").and_then(|v| v.as_str()) {
                    for l in old.lines().take(8) {
                        lines.push(Line::from(Span::styled(format!("- {l}"), theme::err())));
                    }
                }
                if let Some(new) = ctx.get("new_string").and_then(|v| v.as_str()) {
                    for l in new.lines().take(8) {
                        lines.push(Line::from(Span::styled(format!("+ {l}"), theme::ok())));
                    }
                }
            }
            "write_file" | "Write" => {
                if let Some(path) = ctx.get("path").and_then(|v| v.as_str()) {
                    lines.push(Line::from(Span::styled(path.to_string(), theme::accent())));
                }
                if let Some(content) = ctx.get("content").and_then(|v| v.as_str()) {
                    for l in content.lines().take(10) {
                        lines.push(Line::from(Span::styled(format!("  {l}"), theme::dim())));
                    }
                }
            }
            "Bash" | "bash" | "computer" => {
                if let Some(cmd) = ctx.get("command").or_else(|| ctx.get("cmd"))
                    .and_then(|v| v.as_str())
                {
                    lines.push(Line::from(Span::styled(truncate(cmd, 120), theme::dim())));
                }
            }
            _ => {
                let raw = truncate(&ctx.to_string(), 200);
                lines.push(Line::from(Span::styled(raw, theme::dim())));
            }
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

#[cfg(test)]
mod tests {
    use super::*;
    use crossterm::event::KeyCode;

    fn req(id: i64, action: &str) -> ApprovalRequest {
        ApprovalRequest {
            id, domain_id: None, action: action.into(),
            channel: None, reason: None, requested_by: None,
            status: "pending".into(), request_context: None,
        }
    }

    fn state_with(reqs: Vec<ApprovalRequest>) -> ApprovalQueueState {
        ApprovalQueueState { pending: reqs, ..Default::default() }
    }

    #[test]
    fn y_arms_pending_response() {
        let mut s = state_with(vec![req(1, "deploy")]);
        s.handle_key(KeyCode::Char('y'));
        assert_eq!(s.take_pending_response(), Some((1, true)));
        assert!(s.take_pending_response().is_none(), "take must clear");
    }

    #[test]
    fn n_arms_deny_confirm_not_response() {
        let mut s = state_with(vec![req(7, "drop_table")]);
        s.handle_key(KeyCode::Char('n'));
        assert!(s.take_pending_response().is_none(), "deny must not dispatch directly");
        assert_eq!(s.take_pending_deny_confirm(), Some((7, "drop_table".into())));
    }

    #[test]
    fn confirm_deny_sets_pending_response() {
        let mut s = state_with(vec![req(7, "drop_table")]);
        s.confirm_deny(7);
        assert_eq!(s.take_pending_response(), Some((7, false)));
    }

    #[test]
    fn down_clamps_at_last_index() {
        let mut s = state_with(vec![req(1, "a"), req(2, "b")]);
        s.handle_key(KeyCode::Down); // 0 -> 1
        s.handle_key(KeyCode::Down); // would go to 2 but list only has 2
        assert_eq!(s.selection, 1);
    }

    #[test]
    fn y_with_empty_list_is_noop() {
        let mut s = state_with(vec![]);
        s.handle_key(KeyCode::Char('y'));
        assert!(s.take_pending_response().is_none());
    }
}
