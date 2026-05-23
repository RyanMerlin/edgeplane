use ratatui::{
    buffer::Buffer,
    layout::{Constraint, Direction, Layout, Rect},
    style::Style,
    text::{Line, Span},
    widgets::{Block, BorderType, Borders, List, ListItem, ListState, Paragraph, Widget},
};

use crate::tui::data::{MissionSummary, DomainSummary, TaskSummary};
use crate::tui::theme;

/// Which filter is currently being typed.
#[derive(Debug, Clone, PartialEq, Default)]
pub enum FilterActive {
    #[default]
    None,
    Domain,
    Mission,
}

/// Which pane has keyboard focus.
#[derive(Debug, Clone, PartialEq)]
pub enum Focus {
    Domains,
    Missions,
    Tasks,
    Detail,
}

/// A node in the left tree — either a domain or a mission under it.
/// Kept for backwards compatibility with app.rs matrix_enter logic.
#[derive(Debug, Clone)]
pub enum TreeNode {
    Domain { idx: usize },
    Mission { domain_idx: usize, mission_idx: usize },
}

#[derive(Debug, Default)]
pub struct MissionMatrixState {
    pub focus: Focus,
    pub domains: Vec<DomainSummary>,
    pub missions: Vec<MissionSummary>,
    pub tasks: Vec<TaskSummary>,
    // Legacy field kept for compat; not used in rendering after v3
    pub tree_selection: usize,
    pub task_selection: usize,
    pub loading_domains: bool,
    pub loading_missions: bool,
    pub loading_tasks: bool,
    pub selected_domain_id: Option<String>,
    pub selected_mission_id: Option<String>,
    // New v3 fields
    pub domain_selection: usize,
    pub mission_selection: usize,
    pub domain_filter: String,
    pub mission_filter: String,
    pub filter_active: FilterActive,
    pub error: Option<String>,
}

impl Default for Focus {
    fn default() -> Self {
        Focus::Domains
    }
}

impl MissionMatrixState {
    /// Handle a keypress in this screen.  Returns true if the event was consumed.
    pub fn handle_key(&mut self, key: crossterm::event::KeyCode) -> bool {
        use crossterm::event::KeyCode::*;

        // Filter input mode — swallow all keys except the ones we handle
        // so Tab/←/→ can't leak into global nav while typing
        if self.filter_active != FilterActive::None {
            match key {
                Char(c) => {
                    match self.filter_active {
                        FilterActive::Domain => self.domain_filter.push(c),
                        FilterActive::Mission => self.mission_filter.push(c),
                        FilterActive::None => {}
                    }
                }
                Backspace => {
                    match self.filter_active {
                        FilterActive::Domain => { self.domain_filter.pop(); }
                        FilterActive::Mission => { self.mission_filter.pop(); }
                        FilterActive::None => {}
                    }
                }
                Esc => {
                    match self.filter_active {
                        FilterActive::Domain => self.domain_filter.clear(),
                        FilterActive::Mission => self.mission_filter.clear(),
                        FilterActive::None => {}
                    }
                    self.filter_active = FilterActive::None;
                }
                _ => {}
            }
            return true;
        }

        match key {
            // Left/Right move focus between the three columns
            Right => {
                self.focus = match self.focus {
                    Focus::Domains => Focus::Missions,
                    Focus::Missions => Focus::Tasks,
                    Focus::Tasks | Focus::Detail => Focus::Tasks,
                };
                true
            }
            Left => {
                self.focus = match self.focus {
                    Focus::Tasks | Focus::Detail => Focus::Missions,
                    Focus::Missions => Focus::Domains,
                    Focus::Domains => Focus::Domains,
                };
                true
            }
            Char('/') => {
                match self.focus {
                    Focus::Domains => { self.filter_active = FilterActive::Domain; }
                    Focus::Missions => { self.filter_active = FilterActive::Mission; }
                    _ => {}
                }
                true
            }
            Up => {
                match self.focus {
                    Focus::Domains => {
                        if self.domain_selection > 0 {
                            self.domain_selection -= 1;
                        }
                    }
                    Focus::Missions => {
                        if self.mission_selection > 0 {
                            self.mission_selection -= 1;
                        }
                    }
                    Focus::Tasks => {
                        if self.task_selection > 0 {
                            self.task_selection -= 1;
                        }
                    }
                    _ => {}
                }
                true
            }
            Down => {
                let visible_domains = self.visible_domains();
                let visible_missions = self.visible_missions();
                match self.focus {
                    Focus::Domains => {
                        if self.domain_selection + 1 < visible_domains.len() {
                            self.domain_selection += 1;
                        }
                    }
                    Focus::Missions => {
                        if self.mission_selection + 1 < visible_missions.len() {
                            self.mission_selection += 1;
                        }
                    }
                    Focus::Tasks => {
                        if self.task_selection + 1 < self.tasks.len() {
                            self.task_selection += 1;
                        }
                    }
                    _ => {}
                }
                true
            }
            _ => false,
        }
    }

    /// Visible domains after applying domain_filter.
    pub fn visible_domains(&self) -> Vec<&DomainSummary> {
        if self.domain_filter.is_empty() {
            return self.domains.iter().collect();
        }
        let q = self.domain_filter.to_lowercase();
        self.domains.iter().filter(|m| m.name.to_lowercase().contains(&q)).collect()
    }

    /// Visible missions after applying mission_filter.
    pub fn visible_missions(&self) -> Vec<&MissionSummary> {
        if self.mission_filter.is_empty() {
            return self.missions.iter().collect();
        }
        let q = self.mission_filter.to_lowercase();
        self.missions.iter().filter(|k| k.name.to_lowercase().contains(&q)).collect()
    }

    /// Flattened list of tree nodes in display order (kept for app.rs compat).
    pub fn tree_nodes(&self) -> Vec<TreeNode> {
        let mut nodes = vec![];
        for (mi, _) in self.domains.iter().enumerate() {
            nodes.push(TreeNode::Domain { idx: mi });
            if Some(mi) == self.selected_domain_idx() {
                for (ki, _) in self.missions.iter().enumerate() {
                    nodes.push(TreeNode::Mission { domain_idx: mi, mission_idx: ki });
                }
            }
        }
        nodes
    }

    fn selected_domain_idx(&self) -> Option<usize> {
        if let Some(mid) = &self.selected_domain_id {
            self.domains.iter().position(|m| &m.id == mid)
        } else {
            None
        }
    }

    pub fn selected_task(&self) -> Option<&TaskSummary> {
        self.tasks.get(self.task_selection)
    }
}

pub struct MissionMatrix<'a> {
    pub state: &'a MissionMatrixState,
}

impl<'a> Widget for MissionMatrix<'a> {
    fn render(self, area: Rect, buf: &mut Buffer) {
        // Fill background
        let bg_block = Block::default().style(theme::normal());
        bg_block.render(area, buf);

        // 3 flat panes: Domains | Missions | Tasks (33% / 33% / 34%)
        let chunks = Layout::default()
            .direction(Direction::Horizontal)
            .constraints([
                Constraint::Percentage(33),
                Constraint::Percentage(33),
                Constraint::Percentage(34),
            ])
            .split(area);

        render_domains_pane(buf, chunks[0], self.state);
        render_missions_pane(buf, chunks[1], self.state);
        render_tasks_pane(buf, chunks[2], self.state);
    }
}

fn render_domains_pane(buf: &mut Buffer, area: Rect, state: &MissionMatrixState) {
    let focused = state.focus == Focus::Domains;
    let filter_active = state.filter_active == FilterActive::Domain;

    let title = format!(
        " Domains [/ search]{}",
        if !state.domain_filter.is_empty() {
            format!(" · \"{}\"", state.domain_filter)
        } else {
            String::new()
        }
    );
    let block = Block::default()
        .borders(Borders::ALL)
        .border_type(BorderType::Rounded)
        .border_style(theme::border_for(focused))
        .title(Span::styled(title, theme::panel_title()))
        .style(theme::normal());
    let inner = block.inner(area);
    block.render(area, buf);

    if state.loading_domains {
        Paragraph::new(Span::styled("loading…", theme::dim()))
            .style(theme::normal())
            .render(inner, buf);
        return;
    }

    let (list_area, filter_area) = if filter_active {
        let va = Layout::default()
            .direction(Direction::Vertical)
            .constraints([Constraint::Length(1), Constraint::Fill(1)])
            .split(inner);
        (va[1], Some(va[0]))
    } else {
        (inner, None)
    };

    // Filter input line
    if let Some(fa) = filter_area {
        let filter_line = Line::from(vec![
            Span::styled("/ ", theme::accent()),
            Span::styled(state.domain_filter.clone(), theme::normal()),
            Span::styled("_", theme::accent()),
        ]);
        Paragraph::new(filter_line).style(theme::normal()).render(fa, buf);
    }

    let visible = state.visible_domains();
    if visible.is_empty() {
        Paragraph::new(Span::styled("no domains", theme::muted()))
            .style(theme::normal())
            .render(list_area, buf);
        return;
    }

    let items: Vec<ListItem> = visible.iter().enumerate().map(|(i, m)| {
        let selected = i == state.domain_selection;
        let style = if selected && focused { theme::selected() } else if selected { Style::default().fg(theme::ACCENT) } else { theme::normal() };
        let dot = status_dot(&m.status);
        let prefix = if selected { "▶ " } else { "  " };
        ListItem::new(Line::from(vec![
            Span::styled(prefix, style),
            Span::styled(dot, status_style(&m.status)),
            Span::styled(format!(" {}", m.name), style),
        ]))
    }).collect();

    let sel = if focused { Some(state.domain_selection) } else { None };
    let mut ls = ListState::default().with_selected(sel);
    ratatui::widgets::StatefulWidget::render(
        List::new(items).style(theme::normal()),
        list_area,
        buf,
        &mut ls,
    );
}

fn render_missions_pane(buf: &mut Buffer, area: Rect, state: &MissionMatrixState) {
    let focused = state.focus == Focus::Missions;
    let filter_active = state.filter_active == FilterActive::Mission;

    let title = format!(
        " Missions [/ search]{}",
        if !state.mission_filter.is_empty() {
            format!(" · \"{}\"", state.mission_filter)
        } else {
            String::new()
        }
    );
    let block = Block::default()
        .borders(Borders::ALL)
        .border_type(BorderType::Rounded)
        .border_style(theme::border_for(focused))
        .title(Span::styled(title, theme::panel_title()))
        .style(theme::normal());
    let inner = block.inner(area);
    block.render(area, buf);

    if state.loading_missions {
        Paragraph::new(Span::styled("loading…", theme::dim()))
            .style(theme::normal())
            .render(inner, buf);
        return;
    }

    if state.selected_domain_id.is_none() {
        Paragraph::new(Span::styled("select a domain", theme::muted()))
            .style(theme::normal())
            .render(inner, buf);
        return;
    }

    let (list_area, filter_area) = if filter_active {
        let va = Layout::default()
            .direction(Direction::Vertical)
            .constraints([Constraint::Length(1), Constraint::Fill(1)])
            .split(inner);
        (va[1], Some(va[0]))
    } else {
        (inner, None)
    };

    if let Some(fa) = filter_area {
        let filter_line = Line::from(vec![
            Span::styled("/ ", theme::accent()),
            Span::styled(state.mission_filter.clone(), theme::normal()),
            Span::styled("_", theme::accent()),
        ]);
        Paragraph::new(filter_line).style(theme::normal()).render(fa, buf);
    }

    let visible = state.visible_missions();
    if visible.is_empty() {
        Paragraph::new(Span::styled("no missions", theme::muted()))
            .style(theme::normal())
            .render(list_area, buf);
        return;
    }

    let items: Vec<ListItem> = visible.iter().enumerate().map(|(i, k)| {
        let selected = i == state.mission_selection;
        let style = if selected && focused { theme::selected() } else if selected { Style::default().fg(theme::ACCENT) } else { theme::normal() };
        let dot = status_dot(&k.status);
        let prefix = if selected { "▶ " } else { "  " };
        ListItem::new(Line::from(vec![
            Span::styled(prefix, style),
            Span::styled(dot, status_style(&k.status)),
            Span::styled(format!(" {}", k.name), style),
        ]))
    }).collect();

    let sel = if focused { Some(state.mission_selection) } else { None };
    let mut ls = ListState::default().with_selected(sel);
    ratatui::widgets::StatefulWidget::render(
        List::new(items).style(theme::normal()),
        list_area,
        buf,
        &mut ls,
    );
}

fn render_tasks_pane(buf: &mut Buffer, area: Rect, state: &MissionMatrixState) {
    let focused = state.focus == Focus::Tasks;
    let block = Block::default()
        .borders(Borders::ALL)
        .border_type(BorderType::Rounded)
        .border_style(theme::border_for(focused))
        .title(Span::styled(" Tasks ", theme::panel_title()))
        .style(theme::normal());
    let inner = block.inner(area);
    block.render(area, buf);

    if state.loading_tasks {
        Paragraph::new(Span::styled("loading…", theme::dim()))
            .style(theme::normal())
            .render(inner, buf);
        return;
    }

    if state.tasks.is_empty() {
        Paragraph::new(Span::styled(
            if state.selected_mission_id.is_some() { "no tasks" } else { "select a mission" },
            theme::muted(),
        ))
        .style(theme::normal())
        .render(inner, buf);
        return;
    }

    // Header row
    let header_area = Rect { height: 1, ..inner };
    let content_area = Rect {
        y: inner.y + 1,
        height: inner.height.saturating_sub(1),
        ..inner
    };

    let header = Line::from(vec![
        Span::styled(format!("{:<4} ", "#"), theme::muted()),
        Span::styled(format!("{:<30} ", "Task"), theme::muted()),
        Span::styled(format!("{:<14} ", "Status"), theme::muted()),
        Span::styled("Owner", theme::muted()),
    ]);
    Paragraph::new(header).style(theme::normal()).render(header_area, buf);

    let items: Vec<ListItem> = state.tasks.iter().enumerate().map(|(i, t)| {
        let selected = i == state.task_selection && focused;
        let style = if selected { theme::selected() } else { theme::normal() };
        let dot = status_dot(&t.status);
        ListItem::new(Line::from(vec![
            Span::styled(format!("{:<4} ", i + 1), style),
            Span::styled(format!("{:<30} ", truncate(&t.title, 28)), style),
            Span::styled(dot, status_style(&t.status)),
            Span::styled(format!(" {:<12} ", truncate(&t.status, 10)), style),
            Span::styled(truncate(&t.owner, 12), theme::dim()),
        ]))
    }).collect();

    let mut list_state = ListState::default().with_selected(if focused { Some(state.task_selection) } else { None });
    ratatui::widgets::StatefulWidget::render(
        List::new(items).style(theme::normal()),
        content_area,
        buf,
        &mut list_state,
    );
}

fn status_dot(status: &str) -> &'static str {
    match status.to_lowercase().as_str() {
        "active" | "running" | "in_progress" => "●",
        "done" | "completed" | "success" => "●",
        "failed" | "error" => "●",
        "proposed" | "pending" | "waiting" => "○",
        _ => "◌",
    }
}

fn status_style(status: &str) -> Style {
    match status.to_lowercase().as_str() {
        "active" | "running" | "in_progress" => theme::ok(),
        "done" | "completed" | "success" => Style::default().fg(theme::ACCENT),
        "failed" | "error" => theme::err(),
        "proposed" | "pending" | "waiting" => theme::warn(),
        _ => theme::dim(),
    }
}

fn truncate(s: &str, max: usize) -> String {
    if s.len() <= max {
        s.to_string()
    } else {
        format!("{}…", &s[..max.saturating_sub(1)])
    }
}
