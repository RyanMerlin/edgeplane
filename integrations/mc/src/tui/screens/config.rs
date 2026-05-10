use ratatui::{
    buffer::Buffer,
    layout::{Constraint, Direction, Layout, Rect},
    style::{Modifier, Style},
    text::{Line, Span},
    widgets::{Block, BorderType, Borders, List, ListItem, Paragraph, Widget},
};

use mc_mesh_secrets::{InfisicalConfig, InfisicalProfileMap};

use crate::tui::theme;

// ── Add-profile form ──────────────────────────────────────────────────────────

#[derive(Debug, Clone, PartialEq)]
pub enum InfisicalAuthMode {
    UniversalAuth,
    ServiceToken,
}

impl Default for InfisicalAuthMode {
    fn default() -> Self { Self::UniversalAuth }
}

#[derive(Debug, Default, Clone)]
pub struct InfisicalAddForm {
    pub name: String,
    pub mode: InfisicalAuthMode,
    // Universal Auth fields
    pub client_id: String,
    pub client_secret: String,
    // Service Token field
    pub token: String,
    pub focused_field: usize, // 0=name, 1=client_id/token, 2=client_secret (UA only)
    pub error: Option<String>,
}

// ── State ─────────────────────────────────────────────────────────────────────

pub struct ConfigScreenState {
    pub nav_selection: usize,
    pub content_focused: bool, // true = ↑↓ drive content panel, false = ↑↓ drive nav
    pub base_url: String,
    pub connected: bool,
    pub latency_ms: Option<u64>,
    pub version: String,
    pub context_name: String,
    pub token_masked: Option<String>,
    pub server_version: Option<String>,

    // Profile panel
    pub contexts: Vec<(String, crate::context::ContextEntry)>,
    pub context_selection: usize,
    pub(crate) pending_context_switch: Option<String>,

    // Infisical panel
    pub infisical_profiles: InfisicalProfileMap,
    pub infisical_selection: usize,
    pub infisical_form: Option<InfisicalAddForm>,
}

impl Default for ConfigScreenState {
    fn default() -> Self {
        Self {
            nav_selection: 0,
            content_focused: false,
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
            infisical_profiles: InfisicalProfileMap::default(),
            infisical_selection: 0,
            infisical_form: None,
        }
    }
}

impl ConfigScreenState {
    pub fn take_pending_context_switch(&mut self) -> Option<String> {
        self.pending_context_switch.take()
    }

    pub fn reload_contexts(&mut self) {
        let ctxs = crate::context::load_contexts();
        self.contexts = ctxs.contexts.into_iter().collect();
        self.contexts.sort_by(|a, b| a.0.cmp(&b.0));
        self.context_selection = self.contexts
            .iter()
            .position(|(n, _)| n == &self.context_name)
            .unwrap_or(0);
    }

    pub fn reload_infisical_from_disk(&mut self) {
        let path = dirs::home_dir().map(|h| h.join(".mc").join("infisical_profiles.json"));
        if let Some(path) = path {
            if let Ok(s) = std::fs::read_to_string(&path) {
                if let Ok(map) = serde_json::from_str::<InfisicalProfileMap>(&s) {
                    self.infisical_profiles = map;
                    return;
                }
            }
        }
        self.infisical_profiles = InfisicalProfileMap::default();
    }

    /// Panels that support content-panel focus
    fn is_interactive_panel(&self) -> bool {
        matches!(self.nav_selection, 4 | 5)
    }

    pub fn handle_key(&mut self, key: crossterm::event::KeyCode) -> bool {
        use crossterm::event::KeyCode::*;

        // Form captures all input when open
        if self.infisical_form.is_some() {
            return self.handle_infisical_form_key(key);
        }

        match key {
            // → or Enter enters the content panel (on interactive panels only)
            Right | Enter if !self.content_focused && self.is_interactive_panel() => {
                self.content_focused = true;
                true
            }
            // ← / Esc returns to the nav panel
            Left | Esc if self.content_focused => {
                self.content_focused = false;
                true
            }
            // ↑↓ — nav panel when unfocused, content list when focused
            Up => {
                if self.content_focused {
                    match self.nav_selection {
                        4 => { if self.context_selection > 0 { self.context_selection -= 1; } }
                        5 => { if self.infisical_selection > 0 { self.infisical_selection -= 1; } }
                        _ => {}
                    }
                } else {
                    if self.nav_selection > 0 {
                        self.nav_selection -= 1;
                    }
                    self.content_focused = false;
                }
                true
            }
            Down => {
                if self.content_focused {
                    match self.nav_selection {
                        4 => {
                            if self.context_selection + 1 < self.contexts.len() {
                                self.context_selection += 1;
                            }
                        }
                        5 => {
                            let n = self.infisical_profiles.profiles.len();
                            if n > 0 && self.infisical_selection + 1 < n {
                                self.infisical_selection += 1;
                            }
                        }
                        _ => {}
                    }
                } else {
                    if self.nav_selection < NAV_ITEMS.len() - 1 {
                        self.nav_selection += 1;
                    }
                    self.content_focused = false;
                }
                true
            }
            // n — open add-profile form (Infisical panel, any focus state)
            Char('n') if self.nav_selection == 5 => {
                self.infisical_form = Some(InfisicalAddForm::default());
                true
            }
            // d — delete selected Infisical profile (content focused)
            Char('d') if self.nav_selection == 5 && self.content_focused => {
                self.delete_selected_infisical_profile();
                true
            }
            // Enter — action depends on panel + focus
            Enter => {
                if self.content_focused {
                    match self.nav_selection {
                        4 => {
                            if let Some((name, _)) = self.contexts.get(self.context_selection) {
                                if name != &self.context_name {
                                    self.pending_context_switch = Some(name.clone());
                                }
                            }
                            true
                        }
                        5 => {
                            self.activate_selected_infisical_profile();
                            true
                        }
                        _ => false,
                    }
                } else {
                    false
                }
            }
            _ => false,
        }
    }

    fn handle_infisical_form_key(&mut self, key: crossterm::event::KeyCode) -> bool {
        use crossterm::event::KeyCode::*;
        let form = self.infisical_form.as_mut().unwrap();
        let max_field = if form.mode == InfisicalAuthMode::UniversalAuth { 2 } else { 1 };
        match key {
            // F2 / Alt-t toggles auth mode
            F(2) => {
                form.mode = if form.mode == InfisicalAuthMode::UniversalAuth {
                    InfisicalAuthMode::ServiceToken
                } else {
                    InfisicalAuthMode::UniversalAuth
                };
                form.focused_field = form.focused_field.min(max_field);
                form.error = None;
            }
            Tab => {
                form.focused_field = (form.focused_field + 1) % (max_field + 1);
            }
            BackTab => {
                if form.focused_field == 0 {
                    form.focused_field = max_field;
                } else {
                    form.focused_field -= 1;
                }
            }
            Backspace => {
                form.error = None;
                match form.focused_field {
                    0 => { form.name.pop(); }
                    1 if form.mode == InfisicalAuthMode::UniversalAuth => { form.client_id.pop(); }
                    1 => { form.token.pop(); }
                    _ => { form.client_secret.pop(); }
                }
            }
            Char(c) => {
                form.error = None;
                match form.focused_field {
                    0 => form.name.push(c),
                    1 if form.mode == InfisicalAuthMode::UniversalAuth => form.client_id.push(c),
                    1 => form.token.push(c),
                    _ => form.client_secret.push(c),
                }
            }
            Enter => {
                let name = form.name.trim().to_string();
                if name.is_empty() {
                    form.error = Some("Name is required".into());
                    form.focused_field = 0;
                    return true;
                }
                match form.mode {
                    InfisicalAuthMode::UniversalAuth => {
                        let id = form.client_id.trim().to_string();
                        let secret = form.client_secret.trim().to_string();
                        if id.is_empty() {
                            form.error = Some("Client ID is required".into());
                            form.focused_field = 1;
                        } else if secret.is_empty() {
                            form.error = Some("Client Secret is required".into());
                            form.focused_field = 2;
                        } else {
                            self.infisical_form = None;
                            self.save_infisical_ua_profile(name, id, secret);
                        }
                    }
                    InfisicalAuthMode::ServiceToken => {
                        let token = form.token.trim().to_string();
                        if token.is_empty() {
                            form.error = Some("Service token is required".into());
                            form.focused_field = 1;
                        } else {
                            self.infisical_form = None;
                            self.save_infisical_profile(name, token);
                        }
                    }
                }
            }
            Esc => {
                self.infisical_form = None;
            }
            _ => {}
        }
        true // always consume when form is active
    }

    fn save_infisical_profile(&mut self, name: String, token: String) {
        let cfg = InfisicalConfig::with_service_token("https://app.infisical.com", &token);
        self.infisical_profiles.upsert(name, cfg);
        self.save_infisical_map();
    }

    fn save_infisical_ua_profile(&mut self, name: String, client_id: String, client_secret: String) {
        let cfg = InfisicalConfig::with_ua("https://app.infisical.com", client_id, client_secret);
        self.infisical_profiles.upsert(name, cfg);
        self.save_infisical_map();
    }

    fn activate_selected_infisical_profile(&mut self) {
        let names: Vec<String> = self.infisical_profiles.profiles.keys().cloned().collect();
        if let Some(name) = names.get(self.infisical_selection) {
            self.infisical_profiles.active = Some(name.clone());
            self.save_infisical_map();
        }
    }

    fn delete_selected_infisical_profile(&mut self) {
        let names: Vec<String> = self.infisical_profiles.profiles.keys().cloned().collect();
        if let Some(name) = names.get(self.infisical_selection) {
            self.infisical_profiles.remove(name);
            if self.infisical_selection > 0 {
                self.infisical_selection -= 1;
            }
            self.save_infisical_map();
        }
    }

    fn save_infisical_map(&self) {
        let path = dirs::home_dir().map(|h| h.join(".mc").join("infisical_profiles.json"));
        if let Some(path) = path {
            if let Ok(json) = serde_json::to_string_pretty(&self.infisical_profiles) {
                if let Some(parent) = path.parent() {
                    std::fs::create_dir_all(parent).ok();
                }
                std::fs::write(path, json).ok();
            }
        }
    }
}

// ── Nav items ─────────────────────────────────────────────────────────────────

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

// ── Widget ────────────────────────────────────────────────────────────────────

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
            "Infisical" => {
                if state.infisical_profiles.active_profile().is_some() { " ✓" } else { " ○" }
            }
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

    let focused = state.content_focused || state.infisical_form.is_some();
    let block = Block::default()
        .borders(Borders::ALL)
        .border_type(BorderType::Rounded)
        .border_style(theme::border_for(focused))
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
        5 => panel_infisical(state),
        6 => panel_placeholder("Layout preferences are not yet implemented."),
        7 => panel_placeholder("Refresh interval preferences are not yet implemented."),
        8 => panel_version(state),
        _ => vec![],
    };

    Paragraph::new(lines).style(theme::normal()).render(inner, buf);
}

// ── Panel renderers ───────────────────────────────────────────────────────────

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
    if state.content_focused {
        lines.push(Line::from(Span::styled(
            "  ↑↓ navigate   Enter switch   ← back to nav",
            theme::dim(),
        )));
    } else {
        lines.push(Line::from(Span::styled(
            "  → focus list   mc context add <name> --url <url> to add",
            theme::dim(),
        )));
    }

    lines
}

fn panel_infisical(state: &ConfigScreenState) -> Vec<Line<'static>> {
    if let Some(form) = &state.infisical_form {
        return panel_infisical_form(form);
    }

    let profiles: Vec<(&String, &InfisicalConfig)> =
        state.infisical_profiles.profiles.iter().collect();

    if profiles.is_empty() {
        return vec![
            Line::from(""),
            Line::from(Span::styled("  No Infisical profiles configured.", theme::muted())),
            Line::from(""),
            Line::from(Span::styled(
                "  Press n to add a profile with a service token.",
                theme::dim(),
            )),
        ];
    }

    let active = state.infisical_profiles.active.as_deref();
    let mut lines = vec![Line::from("")];

    for (i, (name, cfg)) in profiles.iter().enumerate() {
        let is_active = active == Some(name.as_str());
        let is_cursor = i == state.infisical_selection;

        let bullet = if is_active { "● " } else { "○ " };
        let row_prefix = if is_cursor { "▶ " } else { "  " };

        let auth_kind = if cfg.service_token.is_some() {
            "service-token"
        } else if cfg.client_id.is_some() {
            "universal-auth"
        } else {
            "unconfigured"
        };

        let name_style = if is_cursor {
            theme::selected()
        } else if is_active {
            Style::default().fg(theme::OK)
        } else {
            theme::normal()
        };

        lines.push(Line::from(vec![
            Span::styled(format!("  {row_prefix}"), name_style),
            Span::styled(format!("{bullet}{name}  ", name = name.as_str().to_string()), name_style),
            Span::styled(format!("({auth_kind})  "), theme::dim()),
            Span::styled(cfg.site_url.clone(), theme::dim()),
        ]));
    }

    lines.push(Line::from(""));
    if state.content_focused {
        lines.push(Line::from(Span::styled(
            "  ↑↓ navigate   Enter activate   d delete   ← back to nav   n add",
            theme::dim(),
        )));
    } else {
        lines.push(Line::from(Span::styled(
            "  → focus list   n add profile",
            theme::dim(),
        )));
    }

    lines
}

fn panel_infisical_form(form: &InfisicalAddForm) -> Vec<Line<'static>> {
    let is_ua = form.mode == InfisicalAuthMode::UniversalAuth;

    let f = |i: usize| if form.focused_field == i { theme::selected() } else { theme::normal() };
    let c = |i: usize| if form.focused_field == i { "▌" } else { "" };

    let mode_label = if is_ua { "Universal Auth (machine identity)" } else { "Service Token (legacy)" };
    let mode_inactive = if is_ua { "  F2 switch to Service Token" } else { "  F2 switch to Universal Auth" };

    let mut lines = vec![
        Line::from(""),
        Line::from(Span::styled("  Add Infisical Profile", theme::accent())),
        Line::from(""),
        Line::from(vec![
            Span::styled("  Mode     ", theme::muted()),
            Span::styled(mode_label, theme::accent()),
            Span::styled(mode_inactive, theme::dim()),
        ]),
        Line::from(""),
        Line::from(vec![
            Span::styled("  Name     ", theme::muted()),
            Span::styled(format!("[{}{}]", form.name, c(0)), f(0)),
        ]),
    ];

    if is_ua {
        lines.push(Line::from(vec![
            Span::styled("  Client ID", theme::muted()),
            Span::styled(format!("[{}{}]", form.client_id, c(1)), f(1)),
        ]));
        let secret_masked: String = "*".repeat(form.client_secret.len());
        lines.push(Line::from(vec![
            Span::styled("  Secret   ", theme::muted()),
            Span::styled(format!("[{}{}]", secret_masked, c(2)), f(2)),
        ]));
    } else {
        let token_masked: String = "*".repeat(form.token.len());
        lines.push(Line::from(vec![
            Span::styled("  Token    ", theme::muted()),
            Span::styled(format!("[{}{}]", token_masked, c(1)), f(1)),
        ]));
    }

    lines.push(Line::from(""));
    lines.push(Line::from(Span::styled(
        "  Tab next field   Enter save   Esc cancel",
        theme::dim(),
    )));

    if let Some(err) = &form.error {
        lines.push(Line::from(""));
        lines.push(Line::from(Span::styled(
            format!("  ✗ {err}"),
            Style::default().fg(theme::ERR),
        )));
    }

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
