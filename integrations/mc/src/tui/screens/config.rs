use ratatui::{
    buffer::Buffer,
    layout::{Constraint, Direction, Layout, Rect},
    style::{Modifier, Style},
    text::{Line, Span},
    widgets::{Block, BorderType, Borders, List, ListItem, Paragraph, Widget},
};

use crate::tui::theme;

pub struct ConfigScreenState {
    pub nav_selection: usize,
    pub base_url: String,
    pub connected: bool,
    pub latency_ms: Option<u64>,
    pub version: String,
    pub context_name: String,
    pub token_masked: Option<String>,
    pub server_version: Option<String>,
    pub contexts: Vec<(String, crate::context::ContextEntry)>,
    pub context_selection: usize,
    pub(crate) pending_context_switch: Option<String>,
}

impl Default for ConfigScreenState {
    fn default() -> Self {
        Self {
            nav_selection: 0,
            base_url: String::new(),
            connected: false,
            latency_ms: None,
            version: String::new(),
            context_name: String::new(),
            token_masked: None,
            server_version: None,
            contexts: Vec::new(),
            context_selection: 0,
            pending_context_switch: None,
        }
    }
}

impl ConfigScreenState {
    /// Returns the name of the context the user wants to switch to, if any.
    pub fn take_pending_context_switch(&mut self) -> Option<String> {
        self.pending_context_switch.take()
    }

    /// Reload context list from disk and sync selection to the active context.
    pub fn reload_contexts(&mut self) {
        let ctxs = crate::context::load_contexts();
        self.contexts = ctxs.contexts.into_iter().collect();
        self.contexts.sort_by(|a, b| a.0.cmp(&b.0));
        self.context_selection = self.contexts
            .iter()
            .position(|(n, _)| n == &self.context_name)
            .unwrap_or(0);
    }

    pub fn handle_key(&mut self, key: crossterm::event::KeyCode) -> bool {
        use crossterm::event::KeyCode::*;
        const PROFILE_NAV: usize = 4;

        match key {
            Up => {
                if self.nav_selection == PROFILE_NAV && !self.contexts.is_empty() {
                    if self.context_selection > 0 {
                        self.context_selection -= 1;
                    }
                } else if self.nav_selection > 0 {
                    self.nav_selection -= 1;
                }
                true
            }
            Down => {
                if self.nav_selection == PROFILE_NAV && !self.contexts.is_empty() {
                    if self.context_selection < self.contexts.len() - 1 {
                        self.context_selection += 1;
                    }
                } else if self.nav_selection < NAV_ITEMS.len() - 1 {
                    self.nav_selection += 1;
                }
                true
            }
            Enter => {
                if self.nav_selection == PROFILE_NAV {
                    if let Some((name, _)) = self.contexts.get(self.context_selection) {
                        if name != &self.context_name {
                            self.pending_context_switch = Some(name.clone());
                        }
                    }
                    true
                } else {
                    false
                }
            }
            _ => false,
        }
    }
}

static NAV_ITEMS: &[(&str, &str)] = &[
    ("Connection", "Controlplane"),
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
}

impl<'a> Widget for ConfigScreen<'a> {
    fn render(self, area: Rect, buf: &mut Buffer) {
        let bg = Block::default().style(theme::normal());
        bg.render(area, buf);

        let chunks = Layout::default()
            .direction(Direction::Horizontal)
            .constraints([Constraint::Length(22), Constraint::Fill(1)])
            .split(area);

        render_nav(buf, chunks[0], self.state);
        render_content(buf, chunks[1], self.state);
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

        let suffix = match *item {
            "Controlplane" => if state.connected { " ✓" } else { " ○" },
            _ => "",
        };

        items.push(ListItem::new(Line::from(vec![
            Span::styled(format!("  {prefix}"), style),
            Span::styled(format!("{item}{suffix}"), style),
        ])));
    }

    List::new(items).style(theme::normal()).render(inner, buf);
}

fn render_content(buf: &mut Buffer, area: Rect, state: &ConfigScreenState) {
    let (_, item_name) = NAV_ITEMS[state.nav_selection];
    let title = format!(" {item_name} ");

    let block = Block::default()
        .borders(Borders::ALL)
        .border_type(BorderType::Rounded)
        .border_style(theme::border_normal())
        .title(Span::styled(title, theme::panel_title()))
        .style(theme::normal());
    let inner = block.inner(area);
    block.render(area, buf);

    let lines = match state.nav_selection {
        0 => panel_server(state),
        1 => panel_auth(state),
        2 => panel_placeholder("Fleet node data is not yet loaded in the config panel.\n\nVisit the Missions tab to browse your fleet."),
        3 => panel_placeholder("Agent runtime defaults are not yet configurable from the TUI.\n\nEdit ~/.mc/config.json to adjust defaults."),
        4 => panel_profile(state),
        5 => panel_placeholder("Infisical integration is configured via:\n\n  mc secrets infisical add\n\nVisit the Secrets tab to browse project secrets."),
        6 => panel_placeholder("Layout preferences are not yet implemented."),
        7 => panel_placeholder("Refresh interval preferences are not yet implemented."),
        8 => panel_version(state),
        _ => vec![],
    };

    Paragraph::new(lines).style(theme::normal()).render(inner, buf);
}

fn panel_server(state: &ConfigScreenState) -> Vec<Line<'static>> {
    let (conn_style, conn_text) = if state.connected {
        (Style::default().fg(theme::OK), "● connected")
    } else {
        (Style::default().fg(theme::ERR), "○ disconnected")
    };
    let latency = state.latency_ms.map(|ms| format!("  {ms}ms")).unwrap_or_default();

    vec![
        Line::from(""),
        Line::from(vec![
            Span::styled("  Status   ", theme::muted()),
            Span::styled(conn_text, conn_style),
            Span::styled(latency, theme::dim()),
        ]),
        Line::from(vec![
            Span::styled("  URL      ", theme::muted()),
            Span::styled(state.base_url.clone(), theme::accent()),
        ]),
        Line::from(vec![
            Span::styled("  Context  ", theme::muted()),
            Span::styled(state.context_name.clone(), theme::normal()),
        ]),
        Line::from(""),
        Line::from(Span::styled("  ↑↓ to navigate, Tab/S+Tab to switch tabs", theme::dim())),
    ]
}

fn panel_auth(state: &ConfigScreenState) -> Vec<Line<'static>> {
    let token_line = match &state.token_masked {
        Some(t) => Line::from(vec![
            Span::styled("  Token    ", theme::muted()),
            Span::styled(t.clone(), theme::normal()),
        ]),
        None => Line::from(vec![
            Span::styled("  Token    ", theme::muted()),
            Span::styled("none (anonymous)", Style::default().fg(theme::WARN)),
        ]),
    };

    vec![
        Line::from(""),
        token_line,
        Line::from(""),
        Line::from(Span::styled("  To authenticate:", theme::muted())),
        Line::from(Span::styled("  mc auth login --server <url>", theme::dim())),
        Line::from(""),
        Line::from(Span::styled("  To set a static token:", theme::muted())),
        Line::from(Span::styled("  export MC_TOKEN=<token>", theme::dim())),
    ]
}

fn panel_profile(state: &ConfigScreenState) -> Vec<Line<'static>> {
    if state.contexts.is_empty() {
        return vec![
            Line::from(""),
            Line::from(Span::styled("  No contexts found.", theme::muted())),
            Line::from(""),
            Line::from(Span::styled("  mc context add <name> --url <url>", theme::dim())),
        ];
    }

    let mut lines = vec![Line::from("")];

    for (i, (name, entry)) in state.contexts.iter().enumerate() {
        let is_active = name == &state.context_name;
        let is_cursor = i == state.context_selection;

        let bullet = if is_active { "● " } else { "○ " };
        let row_prefix = if is_cursor { "▶ " } else { "  " };

        let name_style = if is_cursor {
            theme::selected()
        } else if is_active {
            Style::default().fg(theme::OK)
        } else {
            theme::normal()
        };

        lines.push(Line::from(vec![
            Span::styled(format!("  {row_prefix}"), name_style),
            Span::styled(format!("{bullet}{name}  ", name = name.clone()), name_style),
            Span::styled(entry.base_url.clone(), theme::dim()),
        ]));
    }

    lines.push(Line::from(""));
    lines.push(Line::from(Span::styled(
        "  ↑↓ navigate   Enter switch   mc context add <name> --url <url> to add",
        theme::dim(),
    )));

    lines
}

fn panel_version(state: &ConfigScreenState) -> Vec<Line<'static>> {
    let server_ver = state.server_version.clone().unwrap_or_else(|| "—".into());
    let match_style = if state.server_version.as_deref() == Some(state.version.as_str()) {
        Style::default().fg(theme::OK)
    } else {
        Style::default().fg(theme::WARN)
    };

    vec![
        Line::from(""),
        Line::from(vec![
            Span::styled("  mc client  ", theme::muted()),
            Span::styled(state.version.clone(), theme::accent()),
        ]),
        Line::from(vec![
            Span::styled("  mc-server  ", theme::muted()),
            Span::styled(server_ver, match_style),
        ]),
        Line::from(""),
        Line::from(Span::styled("  To update:", theme::muted())),
        Line::from(Span::styled("  mc update", theme::dim())),
    ]
}

fn panel_placeholder(msg: &'static str) -> Vec<Line<'static>> {
    let mut lines = vec![Line::from("")];
    for line in msg.lines() {
        if line.is_empty() {
            lines.push(Line::from(""));
        } else {
            lines.push(Line::from(Span::styled(
                format!("  {line}"),
                theme::muted(),
            )));
        }
    }
    lines
}
