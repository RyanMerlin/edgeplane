use ratatui::{
    buffer::Buffer,
    layout::{Constraint, Direction, Layout, Rect},
    style::Style,
    text::{Line, Span},
    widgets::{Block, BorderType, Borders, List, ListItem, ListState, Paragraph, Widget},
};

use crate::data::{KlusterSummary, MissionSummary, TaskSummary};
use crate::theme;

/// Which pane has keyboard focus.
#[derive(Debug, Clone, PartialEq)]
pub enum Focus {
    Tree,
    Tasks,
    Detail,
}

/// A node in the left tree — either a mission or a kluster under it.
#[derive(Debug, Clone)]
pub enum TreeNode {
    Mission { idx: usize },
    Kluster { mission_idx: usize, kluster_idx: usize },
}

#[derive(Debug, Default)]
pub struct MissionMatrixState {
    pub focus: Focus,
    pub missions: Vec<MissionSummary>,
    pub klusters: Vec<KlusterSummary>,
    pub tasks: Vec<TaskSummary>,
    pub tree_selection: usize,
    pub task_selection: usize,
    pub loading_missions: bool,
    pub loading_klusters: bool,
    pub loading_tasks: bool,
    pub selected_mission_id: Option<String>,
    pub selected_kluster_id: Option<String>,
}

impl Default for Focus {
    fn default() -> Self { Focus::Tree }
}

impl MissionMatrixState {
    /// Handle a keypress in this screen.  Returns true if the event was consumed.
    pub fn handle_key(&mut self, key: crossterm::event::KeyCode) -> bool {
        use crossterm::event::KeyCode::*;
        match key {
            Tab => {
                self.focus = match self.focus {
                    Focus::Tree => Focus::Tasks,
                    Focus::Tasks => Focus::Detail,
                    Focus::Detail => Focus::Tree,
                };
                true
            }
            Up => {
                match self.focus {
                    Focus::Tree => {
                        if self.tree_selection > 0 { self.tree_selection -= 1; }
                    }
                    Focus::Tasks => {
                        if self.task_selection > 0 { self.task_selection -= 1; }
                    }
                    _ => {}
                }
                true
            }
            Down => {
                let tree_nodes = self.tree_nodes();
                match self.focus {
                    Focus::Tree => {
                        if self.tree_selection + 1 < tree_nodes.len() {
                            self.tree_selection += 1;
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

    /// Flattened list of tree nodes in display order.
    pub fn tree_nodes(&self) -> Vec<TreeNode> {
        let mut nodes = vec![];
        for (mi, _) in self.missions.iter().enumerate() {
            nodes.push(TreeNode::Mission { idx: mi });
            // Only expand klusters if this mission is selected
            if Some(mi) == self.selected_mission_idx() {
                for (ki, _) in self.klusters.iter().enumerate() {
                    nodes.push(TreeNode::Kluster { mission_idx: mi, kluster_idx: ki });
                }
            }
        }
        nodes
    }

    fn selected_mission_idx(&self) -> Option<usize> {
        if let Some(mid) = &self.selected_mission_id {
            self.missions.iter().position(|m| &m.id == mid)
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

        // 3-pane split: 20% tree | 50% tasks | 30% detail
        let chunks = Layout::default()
            .direction(Direction::Horizontal)
            .constraints([
                Constraint::Percentage(22),
                Constraint::Percentage(48),
                Constraint::Percentage(30),
            ])
            .split(area);

        render_tree(buf, chunks[0], self.state);
        render_tasks(buf, chunks[1], self.state);
        render_detail(buf, chunks[2], self.state);
    }
}

fn render_tree(buf: &mut Buffer, area: Rect, state: &MissionMatrixState) {
    let focused = state.focus == Focus::Tree;
    let block = Block::default()
        .borders(Borders::ALL)
        .border_type(BorderType::Rounded)
        .border_style(theme::border_for(focused))
        .title(Span::styled(" Missions ", theme::panel_title()))
        .style(theme::normal());
    let inner = block.inner(area);
    block.render(area, buf);

    if state.loading_missions {
        let p = Paragraph::new(Span::styled("loading…", theme::dim())).style(theme::normal());
        p.render(inner, buf);
        return;
    }

    let nodes = state.tree_nodes();
    let items: Vec<ListItem> = nodes.iter().enumerate().map(|(i, node)| {
        let selected = i == state.tree_selection;
        match node {
            TreeNode::Mission { idx } => {
                let m = &state.missions[*idx];
                let dot = status_dot(&m.status);
                let prefix = if selected { "▶ " } else { "  " };
                let style = if selected { theme::selected() } else { theme::normal() };
                ListItem::new(Line::from(vec![
                    Span::styled(prefix, style),
                    Span::styled(dot, status_style(&m.status)),
                    Span::styled(format!(" {}", m.name), style),
                ]))
            }
            TreeNode::Kluster { kluster_idx, .. } => {
                let k = &state.klusters[*kluster_idx];
                let dot = status_dot(&k.status);
                let prefix = if selected { "  ▶ " } else { "    " };
                let style = if selected { theme::selected() } else { theme::dim() };
                ListItem::new(Line::from(vec![
                    Span::styled(prefix, style),
                    Span::styled(dot, status_style(&k.status)),
                    Span::styled(format!(" {}", k.name), style),
                ]))
            }
        }
    }).collect();

    let mut list_state = ListState::default().with_selected(Some(state.tree_selection));
    ratatui::widgets::StatefulWidget::render(
        List::new(items).style(theme::normal()),
        inner,
        buf,
        &mut list_state,
    );
}

fn render_tasks(buf: &mut Buffer, area: Rect, state: &MissionMatrixState) {
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
        let p = Paragraph::new(Span::styled("loading…", theme::dim())).style(theme::normal());
        p.render(inner, buf);
        return;
    }

    if state.tasks.is_empty() {
        let p = Paragraph::new(Span::styled(
            if state.selected_kluster_id.is_some() { "no tasks" } else { "select a kluster" },
            theme::muted(),
        ))
        .style(theme::normal());
        p.render(inner, buf);
        return;
    }

    // Header row
    let header_area = Rect { height: 1, ..inner };
    let content_area = Rect { y: inner.y + 1, height: inner.height.saturating_sub(1), ..inner };

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
            Span::styled(
                format!("{:<30} ", truncate(&t.title, 28)),
                style,
            ),
            Span::styled(dot, status_style(&t.status)),
            Span::styled(format!(" {:<12} ", truncate(&t.status, 10)), style),
            Span::styled(truncate(&t.owner, 12), theme::dim()),
        ]))
    }).collect();

    let mut list_state = ListState::default().with_selected(
        if focused { Some(state.task_selection) } else { None }
    );
    ratatui::widgets::StatefulWidget::render(
        List::new(items).style(theme::normal()),
        content_area,
        buf,
        &mut list_state,
    );
}

fn render_detail(buf: &mut Buffer, area: Rect, state: &MissionMatrixState) {
    let focused = state.focus == Focus::Detail;
    let block = Block::default()
        .borders(Borders::ALL)
        .border_type(BorderType::Rounded)
        .border_style(theme::border_for(focused))
        .title(Span::styled(" Detail ", theme::panel_title()))
        .style(theme::normal());
    let inner = block.inner(area);
    block.render(area, buf);

    let Some(task) = state.selected_task() else {
        let p = Paragraph::new(Span::styled("select a task", theme::muted())).style(theme::normal());
        p.render(inner, buf);
        return;
    };

    let lines: Vec<Line> = vec![
        Line::from(vec![
            Span::styled("ID     ", theme::muted()),
            Span::styled(task.public_id.clone(), theme::accent()),
        ]),
        Line::from(vec![
            Span::styled("Status ", theme::muted()),
            Span::styled(status_dot(&task.status), status_style(&task.status)),
            Span::styled(format!(" {}", task.status), theme::normal()),
        ]),
        Line::from(vec![
            Span::styled("Owner  ", theme::muted()),
            Span::styled(task.owner.clone(), theme::normal()),
        ]),
        Line::from(""),
        Line::from(Span::styled("Description", theme::muted())),
        Line::from(""),
    ];
    let mut all_lines = lines;
    for part in task.description.lines().take(10) {
        all_lines.push(Line::from(Span::styled(part.to_string(), theme::dim())));
    }

    Paragraph::new(all_lines)
        .wrap(ratatui::widgets::Wrap { trim: true })
        .style(theme::normal())
        .render(inner, buf);
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
    if s.len() <= max { s.to_string() }
    else { format!("{}…", &s[..max.saturating_sub(1)]) }
}
