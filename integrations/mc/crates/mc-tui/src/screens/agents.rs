use ratatui::{
    buffer::Buffer,
    layout::{Constraint, Direction, Layout, Rect},
    style::{Modifier, Style},
    text::{Line, Span},
    widgets::{Block, BorderType, Borders, List, ListItem, ListState, Paragraph, Widget},
};

use crate::data::AgentSummary;
use crate::theme;

#[derive(Debug, Clone, PartialEq)]
pub enum AgentFocus {
    Nodes,
    Agents,
}

impl Default for AgentFocus {
    fn default() -> Self {
        AgentFocus::Agents
    }
}

pub struct AgentScreenState {
    pub agents: Vec<AgentSummary>,
    pub loading: bool,
    pub error: Option<String>,
    pub node_selection: usize, // 0 = "All Nodes", 1+ = index into unique_nodes list
    pub agent_selection: usize,
    pub focus: AgentFocus,
}

impl Default for AgentScreenState {
    fn default() -> Self {
        Self {
            agents: vec![],
            loading: true,
            error: None,
            node_selection: 0,
            agent_selection: 0,
            focus: AgentFocus::Agents,
        }
    }
}

impl AgentScreenState {
    pub fn handle_key(&mut self, key: crossterm::event::KeyCode) -> bool {
        use crossterm::event::KeyCode::*;
        match key {
            Right => {
                self.focus = AgentFocus::Agents;
                true
            }
            Left => {
                self.focus = AgentFocus::Nodes;
                true
            }
            Up => {
                match self.focus {
                    AgentFocus::Nodes => {
                        if self.node_selection > 0 {
                            self.node_selection -= 1;
                        }
                    }
                    AgentFocus::Agents => {
                        if self.agent_selection > 0 {
                            self.agent_selection -= 1;
                        }
                    }
                }
                true
            }
            Down => {
                let nodes = self.unique_nodes();
                let visible = self.visible_agents();
                match self.focus {
                    AgentFocus::Nodes => {
                        if self.node_selection + 1 < nodes.len() + 1 {
                            self.node_selection += 1;
                        }
                    }
                    AgentFocus::Agents => {
                        if self.agent_selection + 1 < visible.len() {
                            self.agent_selection += 1;
                        }
                    }
                }
                true
            }
            _ => false,
        }
    }

    pub fn unique_nodes(&self) -> Vec<String> {
        let mut nodes: Vec<String> = self
            .agents
            .iter()
            .filter_map(|a| a.node_id.clone())
            .collect::<std::collections::HashSet<_>>()
            .into_iter()
            .collect();
        nodes.sort();
        nodes
    }

    pub fn selected_node_id(&self) -> Option<String> {
        if self.node_selection == 0 {
            return None;
        }
        let nodes = self.unique_nodes();
        nodes.get(self.node_selection - 1).cloned()
    }

    pub fn visible_agents(&self) -> Vec<&AgentSummary> {
        let filter = self.selected_node_id();
        self.agents
            .iter()
            .filter(|a| match &filter {
                None => true,
                Some(n) => a.node_id.as_deref() == Some(n.as_str()),
            })
            .collect()
    }
}

pub struct AgentScreen<'a> {
    pub state: &'a AgentScreenState,
}

impl<'a> Widget for AgentScreen<'a> {
    fn render(self, area: Rect, buf: &mut Buffer) {
        let bg = Block::default().style(theme::normal());
        bg.render(area, buf);

        if self.state.loading {
            let p = Paragraph::new(Span::styled("loading agents…", theme::dim())).style(theme::normal());
            p.render(area, buf);
            return;
        }

        if let Some(err) = &self.state.error {
            let p = Paragraph::new(Span::styled(format!("error: {err}"), theme::err())).style(theme::normal());
            p.render(area, buf);
            return;
        }

        // Top content area + bottom detail panel
        let has_selection = !self.state.visible_agents().is_empty();
        let detail_height = if has_selection { 12 } else { 0 };

        let vertical_chunks = Layout::default()
            .direction(Direction::Vertical)
            .constraints([
                Constraint::Fill(1),
                Constraint::Length(detail_height),
            ])
            .split(area);

        // Top: node pane (24ch) | agent table (rest)
        let top_chunks = Layout::default()
            .direction(Direction::Horizontal)
            .constraints([Constraint::Length(24), Constraint::Fill(1)])
            .split(vertical_chunks[0]);

        render_node_pane(buf, top_chunks[0], self.state);
        render_agent_table(buf, top_chunks[1], self.state);

        if has_selection && detail_height > 0 {
            render_detail_panel(buf, vertical_chunks[1], self.state);
        }
    }
}

fn render_node_pane(buf: &mut Buffer, area: Rect, state: &AgentScreenState) {
    let focused = state.focus == AgentFocus::Nodes;
    let block = Block::default()
        .borders(Borders::ALL)
        .border_type(BorderType::Rounded)
        .border_style(theme::border_for(focused))
        .title(Span::styled(" Nodes ", theme::panel_title()))
        .style(theme::normal());
    let inner = block.inner(area);
    block.render(area, buf);

    let nodes = state.unique_nodes();

    // Count agents per node
    let total_count = state.agents.len();
    let all_row_selected = state.node_selection == 0;
    let all_style = if all_row_selected && focused {
        theme::selected()
    } else if all_row_selected {
        Style::default().fg(theme::ACCENT)
    } else {
        theme::normal()
    };

    let mut items: Vec<ListItem> = vec![ListItem::new(Line::from(vec![
        Span::styled(if all_row_selected { "▶ " } else { "  " }, all_style),
        Span::styled(format!("All Nodes ({total_count})"), all_style),
    ]))];

    for (i, node) in nodes.iter().enumerate() {
        let node_agents: Vec<&AgentSummary> = state.agents.iter().filter(|a| a.node_id.as_deref() == Some(node)).collect();
        let count = node_agents.len();
        let online = node_agents.iter().any(|a| a.status != "offline");
        let indicator = if online {
            Span::styled("● ", Style::default().fg(theme::OK))
        } else {
            Span::styled("○ ", theme::dim())
        };

        let row_selected = state.node_selection == i + 1;
        let row_style = if row_selected && focused {
            theme::selected()
        } else if row_selected {
            Style::default().fg(theme::ACCENT)
        } else {
            theme::normal()
        };

        let truncated_node = truncate(node, 10);
        items.push(ListItem::new(Line::from(vec![
            Span::styled(if row_selected { "▶ " } else { "  " }, row_style),
            indicator,
            Span::styled(format!("{truncated_node} ({count})"), row_style),
        ])));
    }

    let mut ls = ListState::default().with_selected(Some(state.node_selection));
    ratatui::widgets::StatefulWidget::render(
        List::new(items).style(theme::normal()),
        inner,
        buf,
        &mut ls,
    );
}

fn render_agent_table(buf: &mut Buffer, area: Rect, state: &AgentScreenState) {
    let focused = state.focus == AgentFocus::Agents;
    let block = Block::default()
        .borders(Borders::ALL)
        .border_type(BorderType::Rounded)
        .border_style(theme::border_for(focused))
        .title(Span::styled(" Agents ", theme::panel_title()))
        .style(theme::normal());
    let inner = block.inner(area);
    block.render(area, buf);

    if inner.height == 0 {
        return;
    }

    let visible = state.visible_agents();

    if visible.is_empty() {
        let msg = if state.agents.is_empty() { "no agents registered" } else { "no agents on this node" };
        Paragraph::new(Span::styled(msg, theme::muted()))
            .style(theme::normal())
            .render(inner, buf);
        return;
    }

    // Header
    let header_area = Rect { height: 1, ..inner };
    let content_area = Rect {
        y: inner.y + 1,
        height: inner.height.saturating_sub(1),
        ..inner
    };

    let header = Line::from(vec![
        Span::styled(format!("{:<2} ", "●"), theme::muted()),
        Span::styled(format!("{:<20} ", "Agent"), theme::muted()),
        Span::styled(format!("{:<16} ", "Node"), theme::muted()),
        Span::styled(format!("{:<10} ", "Runtime"), theme::muted()),
        Span::styled(format!("{:<22} ", "Mission"), theme::muted()),
        Span::styled("Active Task", theme::muted()),
    ]);
    Paragraph::new(header).style(theme::normal()).render(header_area, buf);

    let items: Vec<ListItem> = visible.iter().enumerate().map(|(i, agent)| {
        let selected = i == state.agent_selection && focused;
        let base_style = if selected { theme::selected() } else { theme::normal() };

        let (dot, dot_style) = agent_status_dot(&agent.status);
        let runtime_style = runtime_style(agent.runtime.as_deref().unwrap_or(""));

        ListItem::new(Line::from(vec![
            Span::styled(format!("{dot} "), dot_style),
            Span::styled(format!("{:<20} ", truncate(&agent.name, 18)), base_style),
            Span::styled(
                format!("{:<16} ", truncate(agent.node_id.as_deref().unwrap_or("—"), 14)),
                if selected { theme::selected() } else { theme::dim() },
            ),
            Span::styled(
                format!("{:<10} ", truncate(agent.runtime.as_deref().unwrap_or("—"), 8)),
                if selected { theme::selected() } else { runtime_style },
            ),
            Span::styled(
                format!("{:<22} ", truncate(agent.mission_name.as_deref().unwrap_or("—"), 20)),
                if selected { theme::selected() } else { theme::muted() },
            ),
            Span::styled(
                truncate(agent.current_task_title.as_deref().unwrap_or("—"), 24),
                if selected { theme::selected() } else { theme::dim() },
            ),
        ]))
    }).collect();

    let mut ls = ListState::default().with_selected(if focused { Some(state.agent_selection) } else { None });
    ratatui::widgets::StatefulWidget::render(
        List::new(items).style(theme::normal()),
        content_area,
        buf,
        &mut ls,
    );
}

fn render_detail_panel(buf: &mut Buffer, area: Rect, state: &AgentScreenState) {
    let visible = state.visible_agents();
    let Some(agent) = visible.get(state.agent_selection) else { return };

    let block = Block::default()
        .borders(Borders::ALL)
        .border_type(BorderType::Rounded)
        .border_style(theme::border_normal())
        .title(Span::styled(format!(" {} ", agent.name), theme::panel_title()))
        .style(theme::normal());
    let inner = block.inner(area);
    block.render(area, buf);

    if inner.width < 3 || inner.height < 2 {
        return;
    }

    // 3-column layout: Identity | Current Task | Operations
    let cols = Layout::default()
        .direction(Direction::Horizontal)
        .constraints([
            Constraint::Percentage(33),
            Constraint::Percentage(34),
            Constraint::Percentage(33),
        ])
        .split(inner);

    // Identity column
    let identity_lines: Vec<Line> = vec![
        Line::from(Span::styled("Identity", Style::default().fg(theme::TEXT_MUTED).add_modifier(Modifier::BOLD))),
        Line::from(""),
        Line::from(vec![
            Span::styled("ID      ", theme::muted()),
            Span::styled(truncate(&agent.id, 20), theme::accent()),
        ]),
        Line::from(vec![
            Span::styled("Status  ", theme::muted()),
            Span::styled(agent.status.clone(), status_fg_style(&agent.status)),
        ]),
        Line::from(vec![
            Span::styled("Runtime ", theme::muted()),
            Span::styled(
                agent.runtime.as_deref().unwrap_or("—").to_string(),
                runtime_style(agent.runtime.as_deref().unwrap_or("")),
            ),
        ]),
        Line::from(vec![
            Span::styled("Node    ", theme::muted()),
            Span::styled(agent.node_id.as_deref().unwrap_or("—").to_string(), theme::normal()),
        ]),
        Line::from(vec![
            Span::styled("Seen    ", theme::muted()),
            Span::styled(agent.last_seen.as_deref().unwrap_or("—").to_string(), theme::dim()),
        ]),
    ];
    Paragraph::new(identity_lines).style(theme::normal()).render(cols[0], buf);

    // Current Task column
    let task_lines: Vec<Line> = vec![
        Line::from(Span::styled("Current Task", Style::default().fg(theme::TEXT_MUTED).add_modifier(Modifier::BOLD))),
        Line::from(""),
        Line::from(vec![
            Span::styled("Mission ", theme::muted()),
            Span::styled(agent.mission_name.as_deref().unwrap_or("—").to_string(), theme::normal()),
        ]),
        Line::from(vec![
            Span::styled("Task    ", theme::muted()),
            Span::styled(agent.current_task_title.as_deref().unwrap_or("—").to_string(), theme::dim()),
        ]),
    ];
    Paragraph::new(task_lines).style(theme::normal()).render(cols[1], buf);

    // Operations column
    let ops_lines: Vec<Line> = vec![
        Line::from(Span::styled("Operations", Style::default().fg(theme::TEXT_MUTED).add_modifier(Modifier::BOLD))),
        Line::from(""),
        Line::from(Span::styled("[g Give Task]", theme::accent())),
        Line::from(Span::styled("[r Restart]", theme::muted())),
        Line::from(Span::styled("[x Clear Ctx]", theme::muted())),
        Line::from(Span::styled("[d Remove]", theme::err())),
    ];
    Paragraph::new(ops_lines).style(theme::normal()).render(cols[2], buf);
}

fn agent_status_dot(status: &str) -> (&'static str, Style) {
    match status {
        "busy" => ("⟳", Style::default().fg(theme::ACCENT)),
        "online" | "idle" => ("●", Style::default().fg(theme::OK)),
        "offline" => ("○", theme::dim()),
        _ => ("◌", theme::dim()),
    }
}

fn status_fg_style(status: &str) -> Style {
    match status {
        "busy" => Style::default().fg(theme::ACCENT),
        "online" | "idle" => Style::default().fg(theme::OK),
        "offline" => theme::dim(),
        _ => theme::muted(),
    }
}

fn runtime_style(runtime: &str) -> Style {
    match runtime.to_lowercase().as_str() {
        "goose" => Style::default().fg(theme::PURPLE),
        "claude" => Style::default().fg(theme::OK),
        "codex" => Style::default().fg(theme::ACCENT),
        "gemini" => Style::default().fg(theme::WARN),
        _ => theme::muted(),
    }
}

fn truncate(s: &str, max: usize) -> String {
    if s.len() <= max {
        s.to_string()
    } else {
        format!("{}…", &s[..max.saturating_sub(1)])
    }
}
