use ratatui::{
    buffer::Buffer,
    layout::{Constraint, Direction, Layout, Rect},
    style::{Modifier, Style},
    text::{Line, Span},
    widgets::{Block, BorderType, Borders, List, ListItem, Paragraph, Widget},
};

use mc_mesh_secrets::{InfisicalConfig, InfisicalProfileMap};

use crate::tui::theme;

// ── Controlplane edit form ────────────────────────────────────────────────────

#[derive(Debug, Clone)]
pub struct ControlplaneEditForm {
    pub url: String,
    pub focused_field: usize, // 0 = URL text, 1 = Test button, 2 = Apply button
    pub test_result: Option<ControlplaneTestResult>,
    pub context_name: String,
}

#[derive(Debug, Clone)]
pub enum ControlplaneTestResult {
    Testing,
    Ok { latency_ms: u64, version: Option<String> },
    Failed { error: String },
}

// ── OIDC panel state ──────────────────────────────────────────────────────────

#[derive(Debug, Clone)]
pub enum OidcPanelState {
    Initiating,
    AwaitingBrowser { authorize_url: String, started: std::time::Instant },
    TimedOut,
    Failed { error: String },
}

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
    pub client_id: String,
    pub client_secret: String,
    pub token: String,
    pub project_id: String,
    pub environment: String,
    pub focused_field: usize, // 0=name,1=cred1,2=cred2(UA),3=project_id,4=environment
    pub is_edit: bool,        // true = editing existing profile (name locked)
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

    /// Doctor panel snapshot — populated by `App::tick` from live state so
    /// the panel renderer doesn't need to reach into the rest of the app.
    pub doctor: Vec<DoctorCheckRow>,

    // Controlplane panel
    pub controlplane_edit: Option<ControlplaneEditForm>,
    pub pending_url_test: Option<(String, String)>,   // (context_name, url)
    pub pending_url_apply: Option<(String, String)>,  // (context_name, url)

    // Auth panel
    pub auth_oidc_state: Option<OidcPanelState>,
    pub pending_oidc_start: bool,
}

/// Single check displayed in the Doctor panel.
#[derive(Debug, Clone)]
pub struct DoctorCheckRow {
    pub name: &'static str,
    pub status: DoctorStatus,
    pub detail: String,
    /// Short remediation hint shown under failing rows. Empty when not
    /// applicable (status is `Ok`) or when the failure is self-explanatory.
    pub hint: Option<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DoctorStatus {
    Ok,
    Warn,
    Err,
    Unknown,
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
            doctor: Vec::new(),
            controlplane_edit: None,
            pending_url_test: None,
            pending_url_apply: None,
            auth_oidc_state: None,
            pending_oidc_start: false,
        }
    }
}

impl ConfigScreenState {
    pub fn take_pending_context_switch(&mut self) -> Option<String> {
        self.pending_context_switch.take()
    }

    pub fn take_pending_url_test(&mut self) -> Option<(String, String)> {
        self.pending_url_test.take()
    }

    pub fn take_pending_url_apply(&mut self) -> Option<(String, String)> {
        self.pending_url_apply.take()
    }

    pub fn take_pending_oidc_start(&mut self) -> bool {
        let v = self.pending_oidc_start;
        self.pending_oidc_start = false;
        v
    }

    pub fn set_controlplane_test_result(
        &mut self,
        ok: bool,
        latency_ms: u64,
        version: Option<String>,
        error: Option<String>,
    ) {
        if let Some(form) = &mut self.controlplane_edit {
            form.test_result = Some(if ok {
                ControlplaneTestResult::Ok { latency_ms, version }
            } else {
                ControlplaneTestResult::Failed {
                    error: error.unwrap_or_else(|| "unknown error".into()),
                }
            });
        }
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
        matches!(self.nav_selection, 0 | 1 | 4 | 5)
    }

    pub fn handle_key(&mut self, key: crossterm::event::KeyCode) -> bool {
        use crossterm::event::KeyCode::*;

        // Controlplane edit form captures all input when open
        if self.controlplane_edit.is_some() {
            return self.handle_controlplane_edit_key(key);
        }

        // Infisical form captures all input when open
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
                self.auth_oidc_state = None; // cancel any in-progress OIDC flow
                true
            }
            // ↑↓ — nav panel when unfocused, content list when focused
            Up => {
                if self.content_focused {
                    match self.nav_selection {
                        0 | 4 => { if self.context_selection > 0 { self.context_selection -= 1; } }
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
                        0 | 4 => {
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
            // n — open add-profile form
            Char('n') if self.nav_selection == 5 => {
                self.infisical_form = Some(InfisicalAddForm {
                    environment: "prod".into(),
                    ..InfisicalAddForm::default()
                });
                true
            }
            // e — edit URL of selected context (panel 0 = Controlplane)
            Char('e') if self.nav_selection == 0 && self.content_focused => {
                if let Some((name, entry)) = self.contexts.get(self.context_selection) {
                    self.controlplane_edit = Some(ControlplaneEditForm {
                        url: entry.base_url.clone(),
                        focused_field: 0,
                        test_result: None,
                        context_name: name.clone(),
                    });
                }
                true
            }
            // e — edit selected Infisical profile
            Char('e') if self.nav_selection == 5 && self.content_focused => {
                self.edit_selected_infisical_profile();
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
                        0 => {
                            // Switch to selected context
                            if let Some((name, _)) = self.contexts.get(self.context_selection) {
                                if name != &self.context_name {
                                    self.pending_context_switch = Some(name.clone());
                                }
                            }
                            true
                        }
                        1 => {
                            // Trigger OIDC sign-in (only if no flow in progress)
                            if self.auth_oidc_state.is_none() {
                                self.pending_oidc_start = true;
                                self.auth_oidc_state = Some(OidcPanelState::Initiating);
                            }
                            true
                        }
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

    fn handle_controlplane_edit_key(&mut self, key: crossterm::event::KeyCode) -> bool {
        use crossterm::event::KeyCode::*;
        let form = self.controlplane_edit.as_mut().unwrap();
        match key {
            Tab => { form.focused_field = (form.focused_field + 1) % 3; }
            BackTab => {
                form.focused_field = if form.focused_field == 0 { 2 } else { form.focused_field - 1 };
            }
            Backspace if form.focused_field == 0 => {
                form.url.pop();
                form.test_result = None;
            }
            Char(c) if form.focused_field == 0 => {
                form.url.push(c);
                form.test_result = None;
            }
            Enter => {
                match form.focused_field {
                    0 => { form.focused_field = 1; }
                    1 => {
                        // Test — signal app.rs
                        form.test_result = Some(ControlplaneTestResult::Testing);
                        let name = form.context_name.clone();
                        let url = form.url.clone();
                        self.pending_url_test = Some((name, url));
                    }
                    2 => {
                        // Apply — signal app.rs
                        let name = form.context_name.clone();
                        let url = form.url.clone();
                        self.pending_url_apply = Some((name, url));
                        self.controlplane_edit = None;
                    }
                    _ => {}
                }
            }
            Esc => { self.controlplane_edit = None; }
            _ => {}
        }
        true
    }

    fn handle_infisical_form_key(&mut self, key: crossterm::event::KeyCode) -> bool {
        use crossterm::event::KeyCode::*;
        let form = self.infisical_form.as_mut().unwrap();
        // field indices: 0=name, 1=cred1, 2=cred2(UA only), 3=project_id, 4=environment
        let cred_fields = if form.mode == InfisicalAuthMode::UniversalAuth { 2 } else { 1 };
        let max_field = cred_fields + 2; // + project_id + environment
        let name_locked = form.is_edit;
        match key {
            F(2) if !form.is_edit => {
                form.mode = if form.mode == InfisicalAuthMode::UniversalAuth {
                    InfisicalAuthMode::ServiceToken
                } else {
                    InfisicalAuthMode::UniversalAuth
                };
                form.focused_field = form.focused_field.min(cred_fields + 2);
                form.error = None;
            }
            Tab => { form.focused_field = (form.focused_field + 1) % (max_field + 1); }
            BackTab => {
                if form.focused_field == 0 { form.focused_field = max_field; }
                else { form.focused_field -= 1; }
            }
            Backspace => {
                form.error = None;
                let is_ua = form.mode == InfisicalAuthMode::UniversalAuth;
                match form.focused_field {
                    0 if !name_locked => { form.name.pop(); }
                    1 if is_ua  => { form.client_id.pop(); }
                    1           => { form.token.pop(); }
                    2 if is_ua  => { form.client_secret.pop(); }
                    f if f == cred_fields + 1 => { form.project_id.pop(); }
                    _           => { form.environment.pop(); }
                }
            }
            Char(c) => {
                form.error = None;
                let is_ua = form.mode == InfisicalAuthMode::UniversalAuth;
                match form.focused_field {
                    0 if !name_locked => form.name.push(c),
                    1 if is_ua  => form.client_id.push(c),
                    1           => form.token.push(c),
                    2 if is_ua  => form.client_secret.push(c),
                    f if f == cred_fields + 1 => form.project_id.push(c),
                    _           => form.environment.push(c),
                }
            }
            Enter => {
                let name = form.name.trim().to_string();
                let project_id = form.project_id.trim().to_string();
                let environment = {
                    let e = form.environment.trim().to_string();
                    if e.is_empty() { "prod".to_string() } else { e }
                };
                if name.is_empty() {
                    form.error = Some("Name is required".into());
                    form.focused_field = 0;
                    return true;
                }
                if project_id.is_empty() {
                    form.error = Some("Project ID is required — find it in Infisical project settings".into());
                    form.focused_field = cred_fields + 1;
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
                            self.save_infisical_ua_profile(name, id, secret, project_id, environment);
                        }
                    }
                    InfisicalAuthMode::ServiceToken => {
                        let token = form.token.trim().to_string();
                        if token.is_empty() {
                            form.error = Some("Service token is required".into());
                            form.focused_field = 1;
                        } else {
                            self.infisical_form = None;
                            self.save_infisical_profile(name, token, project_id, environment);
                        }
                    }
                }
            }
            Esc => { self.infisical_form = None; }
            _ => {}
        }
        true // always consume when form is active
    }

    fn save_infisical_profile(&mut self, name: String, token: String, project_id: String, environment: String) {
        let mut cfg = InfisicalConfig::with_service_token("https://app.infisical.com", &token);
        cfg.default_project_id = Some(project_id);
        cfg.default_environment = environment;
        self.infisical_profiles.upsert(name, cfg);
        self.save_infisical_map();
    }

    fn save_infisical_ua_profile(&mut self, name: String, client_id: String, client_secret: String, project_id: String, environment: String) {
        let mut cfg = InfisicalConfig::with_ua("https://app.infisical.com", client_id, client_secret);
        cfg.default_project_id = Some(project_id);
        cfg.default_environment = environment;
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

    fn edit_selected_infisical_profile(&mut self) {
        let names: Vec<String> = self.infisical_profiles.profiles.keys().cloned().collect();
        let Some(name) = names.get(self.infisical_selection).cloned() else { return };
        let Some(cfg) = self.infisical_profiles.profiles.get(&name).cloned() else { return };
        let mode = if cfg.service_token.is_some() {
            InfisicalAuthMode::ServiceToken
        } else {
            InfisicalAuthMode::UniversalAuth
        };
        self.infisical_form = Some(InfisicalAddForm {
            name: name.clone(),
            mode,
            client_id: cfg.client_id.unwrap_or_default(),
            client_secret: cfg.client_secret.unwrap_or_default(),
            token: cfg.service_token.unwrap_or_default(),
            project_id: cfg.default_project_id.unwrap_or_default(),
            environment: cfg.default_environment,
            is_edit: true,
            focused_field: 1, // start on first credential field
            error: None,
        });
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
    ("Diagnostics", "Doctor"),
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

    let focused = state.content_focused
        || state.infisical_form.is_some()
        || state.controlplane_edit.is_some();
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
        9 => panel_doctor(state),
        _ => vec![],
    };

    Paragraph::new(lines).style(theme::normal()).render(inner, buf);
}

// ── Panel renderers ───────────────────────────────────────────────────────────

fn panel_server(state: &ConfigScreenState) -> Vec<Line<'static>> {
    // Edit form takes over the whole panel
    if let Some(form) = &state.controlplane_edit {
        return panel_server_edit(form);
    }

    let (conn_style, conn_text) = if state.connected {
        (Style::default().fg(theme::OK), "● connected")
    } else {
        (Style::default().fg(theme::ERR), "○ disconnected")
    };
    let latency = state.latency_ms.map(|ms| format!("  {ms}ms")).unwrap_or_default();

    let mut lines = vec![Line::from("")];

    // Context list
    if state.contexts.is_empty() {
        lines.push(Line::from(Span::styled("  No contexts configured.", theme::muted())));
        lines.push(Line::from(""));
        lines.push(Line::from(Span::styled("  mc context add <name> --url <url>", theme::dim())));
    } else {
        for (i, (name, entry)) in state.contexts.iter().enumerate() {
            let is_active = name == &state.context_name;
            let is_cursor = i == state.context_selection && state.content_focused;

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
    }

    lines.push(Line::from(""));

    // Active connection details
    lines.push(Line::from(vec![
        Span::styled("  Status   ", theme::muted()),
        Span::styled(conn_text, conn_style),
        Span::styled(latency, theme::dim()),
    ]));
    lines.push(Line::from(vec![
        Span::styled("  MC URL   ", theme::muted()),
        Span::styled(state.base_url.clone(), theme::accent()),
    ]));
    lines.push(Line::from(vec![
        Span::styled("  Context  ", theme::muted()),
        Span::styled(state.context_name.clone(), theme::normal()),
    ]));

    lines.push(Line::from(""));
    if state.content_focused {
        lines.push(Line::from(Span::styled(
            "  ↑↓ navigate   Enter switch   e edit URL   ← back",
            theme::dim(),
        )));
    } else {
        lines.push(Line::from(Span::styled(
            "  Enter to select profile  ·  e to edit MC URL",
            theme::dim(),
        )));
    }

    lines
}

fn panel_server_edit(form: &ControlplaneEditForm) -> Vec<Line<'static>> {
    let f = |i: usize| if form.focused_field == i { theme::selected() } else { theme::normal() };
    let cursor = |i: usize| if form.focused_field == i { "▌" } else { "" };
    let btn = |i: usize, label: &'static str| -> Line<'static> {
        if form.focused_field == i {
            Line::from(vec![
                Span::styled("  ", theme::normal()),
                Span::styled(format!("[ {label} ]"), theme::selected()),
            ])
        } else {
            Line::from(vec![
                Span::styled("  ", theme::normal()),
                Span::styled(format!("  {label}  "), theme::dim()),
            ])
        }
    };

    let mut lines = vec![
        Line::from(""),
        Line::from(Span::styled(
            format!("  Editing: {}", form.context_name),
            theme::accent(),
        )),
        Line::from(""),
        Line::from(vec![
            Span::styled("  MC URL   ", theme::muted()),
            Span::styled(format!("[{}{}]", form.url, cursor(0)), f(0)),
        ]),
        Line::from(""),
        btn(1, "Test Connection"),
        btn(2, "Save & Apply"),
        Line::from(""),
    ];

    // Test result
    match &form.test_result {
        None => {}
        Some(ControlplaneTestResult::Testing) => {
            lines.push(Line::from(Span::styled("  ○ Testing…", theme::muted())));
        }
        Some(ControlplaneTestResult::Ok { latency_ms, version }) => {
            let ver = version.as_deref().unwrap_or("?");
            lines.push(Line::from(Span::styled(
                format!("  ● Connected — {latency_ms}ms   server v{ver}"),
                Style::default().fg(theme::OK),
            )));
        }
        Some(ControlplaneTestResult::Failed { error }) => {
            lines.push(Line::from(Span::styled(
                format!("  ✗ {error}"),
                Style::default().fg(theme::ERR),
            )));
        }
    }

    lines.push(Line::from(""));
    lines.push(Line::from(Span::styled(
        "  Tab next   Enter activate   Esc cancel",
        theme::dim(),
    )));

    lines
}

fn panel_auth(state: &ConfigScreenState) -> Vec<Line<'static>> {
    let mut lines = vec![Line::from("")];

    match &state.auth_oidc_state {
        None => {
            // Show current auth status + instructions
            let (status_style, status_text) = match &state.token_masked {
                Some(t) => (
                    Style::default().fg(theme::OK),
                    format!("● signed in  (token: {t})"),
                ),
                None => (
                    Style::default().fg(theme::WARN),
                    "○ not signed in".to_string(),
                ),
            };
            lines.push(Line::from(vec![
                Span::styled("  Status   ", theme::muted()),
                Span::styled(status_text, status_style),
            ]));
            lines.push(Line::from(""));
            if state.token_masked.is_none() {
                if state.content_focused {
                    lines.push(Line::from(Span::styled(
                        "  Press Enter to sign in via browser (OIDC)",
                        theme::accent(),
                    )));
                    lines.push(Line::from(""));
                    lines.push(Line::from(Span::styled(
                        "  Or set MC_TOKEN env var for API key auth.",
                        theme::dim(),
                    )));
                } else {
                    lines.push(Line::from(Span::styled(
                        "  Press Enter to open auth panel",
                        theme::dim(),
                    )));
                }
            } else {
                lines.push(Line::from(Span::styled(
                    "  mc auth logout   to clear the session",
                    theme::dim(),
                )));
            }
        }
        Some(OidcPanelState::Initiating) => {
            lines.push(Line::from(Span::styled(
                "  ○ Connecting to server…",
                theme::muted(),
            )));
        }
        Some(OidcPanelState::AwaitingBrowser { authorize_url, started }) => {
            let elapsed = started.elapsed().as_secs();
            lines.push(Line::from(Span::styled(
                "  ○ Waiting for browser authentication…",
                theme::muted(),
            )));
            lines.push(Line::from(""));
            lines.push(Line::from(Span::styled(
                "  Open this URL in your browser:",
                theme::muted(),
            )));
            lines.push(Line::from(""));
            // Wrap the URL across multiple lines if needed
            let url = authorize_url.clone();
            let max_w = 80usize;
            if url.len() <= max_w {
                lines.push(Line::from(Span::styled(
                    format!("  {url}"),
                    Style::default().fg(theme::ACCENT),
                )));
            } else {
                for chunk in url.as_bytes().chunks(max_w) {
                    let s = String::from_utf8_lossy(chunk).to_string();
                    lines.push(Line::from(Span::styled(
                        format!("  {s}"),
                        Style::default().fg(theme::ACCENT),
                    )));
                }
            }
            lines.push(Line::from(""));
            lines.push(Line::from(Span::styled(
                format!("  Waiting… {elapsed}s   authentication completes automatically"),
                theme::dim(),
            )));
            lines.push(Line::from(Span::styled("  ← Esc to cancel", theme::muted())));
        }
        Some(OidcPanelState::TimedOut) => {
            lines.push(Line::from(Span::styled(
                "  ✗ Browser auth timed out.",
                Style::default().fg(theme::WARN),
            )));
            lines.push(Line::from(""));
            lines.push(Line::from(Span::styled(
                "  Complete sign-in in the browser, then press R to retry.",
                theme::dim(),
            )));
            lines.push(Line::from(Span::styled("  ← Esc to reset", theme::muted())));
        }
        Some(OidcPanelState::Failed { error }) => {
            lines.push(Line::from(Span::styled(
                "  ✗ Authentication failed:",
                Style::default().fg(theme::ERR),
            )));
            lines.push(Line::from(""));
            lines.push(Line::from(Span::styled(
                format!("  {error}"),
                theme::muted(),
            )));
            lines.push(Line::from(""));
            lines.push(Line::from(Span::styled(
                "  ← Esc to reset   Enter to retry",
                theme::dim(),
            )));
        }
    }

    lines
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

    // project_id and environment always shown
    let pid_field = if is_ua { 3 } else { 2 };
    let env_field = pid_field + 1;
    lines.push(Line::from(""));
    lines.push(Line::from(vec![
        Span::styled("  Project ID", theme::muted()),
        Span::styled(format!("[{}{}]", form.project_id, c(pid_field)), f(pid_field)),
    ]));
    lines.push(Line::from(vec![
        Span::styled("  Environment", theme::muted()),
        Span::styled(format!("[{}{}]", form.environment, c(env_field)), f(env_field)),
        Span::styled("  (prod/dev/staging)", theme::dim()),
    ]));

    lines.push(Line::from(""));
    let mode_hint = if form.is_edit { "" } else { "   F2 toggle mode" };
    lines.push(Line::from(Span::styled(
        format!("  Tab next field   Enter save   Esc cancel{mode_hint}"),
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
            Span::styled("  controlplane  ", theme::muted()),
            Span::styled(server_ver, match_style),
        ]),
        Line::from(""),
        Line::from(Span::styled("  To update:", theme::muted())),
        Line::from(Span::styled("  mc update", theme::dim())),
    ]
}

/// Render the Doctor panel from the snapshot in `state.doctor`. The snapshot
/// is refreshed by `App::tick` on every event-loop pass — there's no extra
/// I/O here; the panel reads what the rest of the app already knows about
/// reachability, auth state, and per-screen fetch health.
fn panel_doctor(state: &ConfigScreenState) -> Vec<Line<'static>> {
    let mut lines: Vec<Line<'static>> = vec![Line::from("")];
    if state.doctor.is_empty() {
        lines.push(Line::from(Span::styled(
            "  Gathering check results…",
            theme::muted(),
        )));
        return lines;
    }

    // Summary: count statuses for the header line.
    let mut ok = 0usize;
    let mut warn = 0usize;
    let mut err = 0usize;
    let mut unknown = 0usize;
    for c in &state.doctor {
        match c.status {
            DoctorStatus::Ok => ok += 1,
            DoctorStatus::Warn => warn += 1,
            DoctorStatus::Err => err += 1,
            DoctorStatus::Unknown => unknown += 1,
        }
    }
    let summary = format!(
        "  {} OK   {} warn   {} err   {} unknown",
        ok, warn, err, unknown
    );
    lines.push(Line::from(Span::styled(
        summary,
        Style::default()
            .fg(if err > 0 {
                theme::ERR
            } else if warn > 0 {
                theme::WARN
            } else {
                theme::OK
            })
            .add_modifier(Modifier::BOLD),
    )));
    lines.push(Line::from(""));

    for c in &state.doctor {
        let (glyph, glyph_style) = match c.status {
            DoctorStatus::Ok => ("●", Style::default().fg(theme::OK)),
            DoctorStatus::Warn => ("●", Style::default().fg(theme::WARN)),
            DoctorStatus::Err => ("●", Style::default().fg(theme::ERR)),
            DoctorStatus::Unknown => ("○", Style::default().fg(theme::TEXT_DIM)),
        };
        lines.push(Line::from(vec![
            Span::styled(format!("  {glyph} "), glyph_style),
            Span::styled(format!("{:<22}", c.name), theme::normal()),
            Span::styled(c.detail.clone(), theme::dim()),
        ]));
        if let Some(hint) = c.hint.as_deref() {
            if !hint.is_empty() {
                lines.push(Line::from(Span::styled(
                    format!("      → {hint}"),
                    theme::muted(),
                )));
            }
        }
    }

    lines.push(Line::from(""));
    lines.push(Line::from(Span::styled(
        "  Press R to refresh (re-checks session on disk + refetches panels).",
        theme::dim(),
    )));

    lines
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
