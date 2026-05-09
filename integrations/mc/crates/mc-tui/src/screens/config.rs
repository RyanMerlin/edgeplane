use ratatui::{
    buffer::Buffer,
    layout::{Constraint, Direction, Layout, Rect},
    style::{Modifier, Style},
    text::{Line, Span},
    widgets::{Block, BorderType, Borders, List, ListItem, ListState, Paragraph, Widget},
};

use crate::theme;

pub struct ConfigScreenState {
    pub nav_selection: usize,
    pub base_url: String,
    pub connected: bool,
    pub latency_ms: Option<u64>,
}

impl Default for ConfigScreenState {
    fn default() -> Self {
        Self {
            nav_selection: 0,
            base_url: String::new(),
            connected: false,
            latency_ms: None,
        }
    }
}

impl ConfigScreenState {
    pub fn handle_key(&mut self, key: crossterm::event::KeyCode) -> bool {
        use crossterm::event::KeyCode::*;
        match key {
            Up => {
                if self.nav_selection > 0 {
                    self.nav_selection -= 1;
                }
                true
            }
            Down => {
                if self.nav_selection < 8 {
                    self.nav_selection += 1;
                }
                true
            }
            _ => false,
        }
    }
}

static NAV_ITEMS: &[(&str, &str)] = &[
    ("Connection", "Server"),
    ("Connection", "Auth"),
    ("Fleet", "Nodes"),
    ("Fleet", "Agent Defaults"),
    ("Identity", "Profile"),
    ("Identity", "Infisical"),
    ("Display", "Layout"),
    ("Display", "Refresh"),
    ("About", "Version"),
];

pub struct ConfigScreen<'a> {
    pub state: &'a ConfigScreenState,
    pub base_url: &'a str,
}

impl<'a> Widget for ConfigScreen<'a> {
    fn render(self, area: Rect, buf: &mut Buffer) {
        let bg = Block::default().style(theme::normal());
        bg.render(area, buf);

        // Left nav (22ch) | right content
        let chunks = Layout::default()
            .direction(Direction::Horizontal)
            .constraints([Constraint::Length(22), Constraint::Fill(1)])
            .split(area);

        render_nav(buf, chunks[0], self.state);
        render_content(buf, chunks[1], self.state, self.base_url);
    }
}

fn render_nav(buf: &mut Buffer, area: Rect, state: &ConfigScreenState) {
    let block = Block::default()
        .borders(Borders::ALL)
        .border_type(BorderType::Rounded)
        .border_style(theme::border_normal())
        .title(Span::styled(" Config ", theme::panel_title()))
        .style(theme::normal());
    let inner = block.inner(area);
    block.render(area, buf);

    let mut last_group = "";
    let mut items: Vec<ListItem> = vec![];
    for (i, (group, item)) in NAV_ITEMS.iter().enumerate() {
        if *group != last_group {
            if i > 0 {
                items.push(ListItem::new(Line::from("")));
            }
            items.push(ListItem::new(Line::from(Span::styled(
                format!("  {group}"),
                Style::default().fg(theme::TEXT_MUTED).add_modifier(Modifier::BOLD),
            ))));
            last_group = group;
        }
        let selected = i == state.nav_selection;
        let style = if selected { theme::selected() } else { theme::normal() };
        let prefix = if selected { "▶ " } else { "  " };

        // Status indicators
        let suffix = match *item {
            "Server" => if state.connected { " ✓" } else { " ○" },
            "Nodes" => " !",
            _ => "",
        };

        items.push(ListItem::new(Line::from(vec![
            Span::styled(format!("  {prefix}"), style),
            Span::styled(format!("{item}{suffix}"), style),
        ])));
    }

    let mut ls = ListState::default().with_selected(Some(state.nav_selection));
    ratatui::widgets::StatefulWidget::render(
        List::new(items).style(theme::normal()),
        inner,
        buf,
        &mut ls,
    );
}

fn render_content(buf: &mut Buffer, area: Rect, state: &ConfigScreenState, base_url: &str) {
    let block = Block::default()
        .borders(Borders::ALL)
        .border_type(BorderType::Rounded)
        .border_style(theme::border_normal())
        .title(Span::styled(" Server ", theme::panel_title()))
        .style(theme::normal());
    let inner = block.inner(area);
    block.render(area, buf);

    // Connection status
    let (conn_style, conn_text, latency_text) = if state.connected {
        let lat = state
            .latency_ms
            .map(|ms| format!("  {ms}ms", ms = ms))
            .unwrap_or_default();
        (
            Style::default().fg(theme::OK),
            "● connected",
            lat,
        )
    } else {
        (Style::default().fg(theme::ERR), "○ disconnected", String::new())
    };

    let mut lines: Vec<Line> = vec![
        Line::from(""),
        Line::from(vec![
            Span::styled("  Status  ", theme::muted()),
            Span::styled(conn_text, conn_style),
            Span::styled(latency_text, theme::dim()),
        ]),
        Line::from(vec![
            Span::styled("  URL     ", theme::muted()),
            Span::styled(base_url.to_string(), theme::accent()),
        ]),
        Line::from(""),
        Line::from(Span::styled("  Fleet Nodes", Style::default().fg(theme::TEXT_MUTED).add_modifier(Modifier::BOLD))),
        Line::from(""),
        Line::from(vec![
            Span::styled(format!("  {:<12} {:<20} {:<10} {:<16}", "Status", "Node", "Agents", "Last Seen"), theme::muted()),
        ]),
    ];

    // No node data available directly in config state; show placeholder
    lines.push(Line::from(Span::styled(
        "  (connect to server to load fleet node data)",
        theme::dim(),
    )));

    lines.push(Line::from(""));
    lines.push(Line::from(vec![
        Span::styled("  [Test Connection]", theme::accent()),
        Span::styled("  [Reload Config]", theme::muted()),
    ]));

    Paragraph::new(lines).style(theme::normal()).render(inner, buf);
}
