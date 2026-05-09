use anyhow::Result;
use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
use ratatui::{
    backend::Backend,
    layout::{Constraint, Direction, Layout},
    style::{Color, Modifier, Style},
    text::{Line, Span},
    widgets::Paragraph,
    Frame, Terminal,
};

use super::data::DataClient;
use super::screens::agent_feed::{AgentFeed, AgentFeedState};
use super::screens::agents::{AgentScreen, AgentScreenState};
use super::screens::approval_queue::{ApprovalQueue, ApprovalQueueState};
use super::screens::config::{ConfigScreen, ConfigScreenState};
use super::screens::mission_matrix::{Focus as MatrixFocus, MissionMatrix, MissionMatrixState};
use super::screens::secrets::{SecretsScreen, SecretsState, render_tree_overlay};
use super::theme;
use super::work::{WorkPool, WorkRequest, WorkResult, next_job_id};

#[derive(Debug, Clone, PartialEq)]
pub enum Screen {
    Agents,
    Missions,
    Feed,
    Approvals,
    Secrets,
    Config,
}

pub struct App {
    pub screen: Screen,
    pub base_url: String,
    pub token: Option<String>,
    pub version: String,
    pub context_name: String,
    pub should_quit: bool,

    // Per-screen state
    pub agents: AgentScreenState,
    pub matrix: MissionMatrixState,
    pub agent_feed: AgentFeedState,
    pub approval_queue: ApprovalQueueState,
    pub secrets: SecretsState,
    pub config: ConfigScreenState,

    client: std::sync::Arc<dyn DataClient>,
    pool: WorkPool,
}

impl App {
    pub fn new(
        base_url: String,
        token: Option<String>,
        version: String,
        initial_mission: Option<String>,
        context_name: String,
        client: std::sync::Arc<dyn DataClient>,
    ) -> Self {
        let mut matrix = MissionMatrixState::default();
        if let Some(mid) = initial_mission {
            matrix.selected_mission_id = Some(mid);
        }

        let pool = WorkPool::new();

        // Ping on startup to populate connection status
        pool.dispatch(client.clone(), WorkRequest::Ping { job_id: next_job_id() });
        // Load agents immediately on startup
        pool.dispatch(client.clone(), WorkRequest::ListAgents { job_id: next_job_id() });

        let token_masked = token.as_ref().map(|t| {
            if t.len() > 8 { format!("{}…", &t[..8]) } else { "***".into() }
        });
        let mut config = ConfigScreenState {
            base_url: base_url.clone(),
            connected: false,
            latency_ms: None,
            nav_selection: 0,
            version: version.clone(),
            context_name: context_name.clone(),
            token_masked,
            server_version: None,
            ..Default::default()
        };
        config.reload_contexts();

        Self {
            screen: Screen::Agents,
            base_url,
            token,
            version,
            context_name,
            should_quit: false,
            agents: AgentScreenState::default(),
            matrix,
            agent_feed: AgentFeedState::new(),
            approval_queue: ApprovalQueueState::default(),
            secrets: SecretsState::default(),
            config,
            client,
            pool,
        }
    }

    /// Drain any pending work results and update state.
    pub fn tick(&mut self) {
        while let Ok(result) = self.pool.result_rx.try_recv() {
            match result {
                WorkResult::Pinged { ok, latency_ms, server_version, .. } => {
                    self.config.connected = ok;
                    self.config.latency_ms = Some(latency_ms);
                    if server_version.is_some() {
                        self.config.server_version = server_version;
                    }
                }
                WorkResult::AgentsListed { agents, error, .. } => {
                    self.agents.loading = false;
                    if let Some(e) = error {
                        self.agents.error = Some(e);
                    } else {
                        self.agents.agents = agents;
                        self.agents.agent_selection = 0;
                    }
                }
                WorkResult::MissionsListed { missions, error, .. } => {
                    if let Some(e) = error {
                        self.matrix.error = Some(format!("missions error: {e}"));
                    } else {
                        self.matrix.loading_missions = false;
                        self.matrix.missions = missions;
                        self.matrix.mission_selection = 0;
                        self.matrix.tree_selection = 0;
                    }
                }
                WorkResult::KlustersListed { mission_id, klusters, .. } => {
                    if Some(&mission_id) == self.matrix.selected_mission_id.as_ref() {
                        self.matrix.loading_klusters = false;
                        self.matrix.klusters = klusters;
                        self.matrix.kluster_selection = 0;
                    }
                }
                WorkResult::TasksListed { kluster_id, tasks, .. } => {
                    if Some(&kluster_id) == self.matrix.selected_kluster_id.as_ref() {
                        self.matrix.loading_tasks = false;
                        self.matrix.tasks = tasks;
                        self.matrix.task_selection = 0;
                    }
                }
                WorkResult::FeedConnected => {
                    self.agent_feed.live = true;
                }
                WorkResult::FeedDisconnected { .. } => {
                    self.agent_feed.live = false;
                }
                WorkResult::FeedEvent(ev) => {
                    self.agent_feed.push_event(ev);
                }
                WorkResult::SecretFoldersLoaded { job_id, folders, error } => {
                    if let Some(tree) = &mut self.secrets.tree {
                        tree.deliver_folders(job_id, folders, error);
                    }
                }
                WorkResult::SecretNamesLoaded { job_id, names, error } => {
                    if let Some(tree) = &mut self.secrets.tree {
                        tree.deliver_names(job_id, names, error);
                    }
                }
                WorkResult::ApprovalsListed { approvals, error, .. } => {
                    self.approval_queue.loading = false;
                    if let Some(e) = error {
                        self.approval_queue.last_error = Some(e);
                    } else {
                        self.approval_queue.pending = approvals
                            .into_iter()
                            .map(|a| super::screens::approval_queue::ApprovalRequest {
                                id: a.id,
                                mission_id: a.mission_id,
                                action: a.action,
                                channel: a.channel,
                                reason: a.reason,
                                requested_by: a.requested_by,
                                status: a.status,
                            })
                            .collect();
                        self.approval_queue.selection = 0;
                    }
                }
                WorkResult::ApprovalResponded { approval_id, ok, error, .. } => {
                    if ok {
                        self.approval_queue.pending.retain(|r| r.id.to_string() != approval_id);
                        if self.approval_queue.selection >= self.approval_queue.pending.len()
                            && self.approval_queue.selection > 0
                        {
                            self.approval_queue.selection -= 1;
                        }
                        self.pool.dispatch(self.client.clone(), WorkRequest::FetchApprovals {
                            job_id: next_job_id(), mission_id: None,
                        });
                    } else {
                        self.approval_queue.last_error = error;
                    }
                }
            }
        }
    }

    pub fn handle_key(&mut self, key: KeyEvent) {
        // Global Ctrl-Q / Ctrl-C (handled in event_loop for C) to quit
        if key.code == KeyCode::Char('q') && key.modifiers.contains(KeyModifiers::CONTROL) {
            self.should_quit = true;
            return;
        }

        // Screen-level key routing — each screen gets first crack at nav keys
        let consumed = match &self.screen {
            Screen::Agents => self.agents.handle_key(key.code),
            Screen::Missions => {
                let c = self.matrix.handle_key(key.code);
                if key.code == KeyCode::Enter {
                    match self.matrix.focus {
                        MatrixFocus::Missions => self.missions_enter(),
                        MatrixFocus::Klusters => self.klusters_enter(),
                        _ => {}
                    }
                }
                c
            }
            Screen::Feed => self.agent_feed.handle_key(key.code),
            Screen::Approvals => {
                let c = self.approval_queue.handle_key(key.code);
                if let Some((id, approve)) = self.approval_queue.take_pending_response() {
                    self.pool.dispatch(
                        self.client.clone(),
                        WorkRequest::RespondApproval {
                            job_id: next_job_id(),
                            approval_id: id.to_string(),
                            decision: if approve { "approve".into() } else { "reject".into() },
                            note: None,
                        },
                    );
                }
                c
            }
            Screen::Secrets => {
                let requests = self.secrets.handle_key(key.code);
                for req in requests {
                    self.pool.dispatch(self.client.clone(), req);
                }
                // Consume keys handled by the secrets tree widget;
                // let global nav handle the rest so tab-switching still works.
                matches!(
                    key.code,
                    KeyCode::Up
                        | KeyCode::Down
                        | KeyCode::Left
                        | KeyCode::Right
                        | KeyCode::Enter
                        | KeyCode::Esc
                        | KeyCode::Backspace
                        | KeyCode::PageUp
                        | KeyCode::PageDown
                        | KeyCode::Char(' ')
                        | KeyCode::Char('a')
                )
            }
            Screen::Config => {
                let c = self.config.handle_key(key.code);
                if let Some(name) = self.config.take_pending_context_switch() {
                    self.switch_context(name);
                }
                c
            }
        };
        if !consumed {
            self.handle_global_nav(key);
        }
    }

    fn handle_global_nav(&mut self, key: KeyEvent) {
        match key.code {
            // Tab / Shift+Tab cycle through tabs sequentially
            KeyCode::Tab => self.next_tab(),
            KeyCode::BackTab => self.prev_tab(),
            // Single-char shortcuts for direct jumps (work when not consumed by screen)
            KeyCode::Char('a') => self.switch_to_agents(),
            KeyCode::Char('m') => self.switch_to_missions(),
            KeyCode::Char('f') => self.switch_to_feed(),
            KeyCode::Char('p') => self.switch_to_approvals(),
            KeyCode::Char('s') => self.switch_to_secrets(),
            KeyCode::Char('c') => self.screen = Screen::Config,
            _ => {}
        }
    }

    fn next_tab(&mut self) {
        match self.screen {
            Screen::Agents    => self.switch_to_missions(),
            Screen::Missions  => self.switch_to_feed(),
            Screen::Feed      => self.switch_to_approvals(),
            Screen::Approvals => self.switch_to_secrets(),
            Screen::Secrets   => { self.screen = Screen::Config; }
            Screen::Config    => self.switch_to_agents(),
        }
    }

    fn prev_tab(&mut self) {
        match self.screen {
            Screen::Agents    => { self.screen = Screen::Config; }
            Screen::Missions  => self.switch_to_agents(),
            Screen::Feed      => self.switch_to_missions(),
            Screen::Approvals => self.switch_to_feed(),
            Screen::Secrets   => self.switch_to_approvals(),
            Screen::Config    => self.switch_to_secrets(),
        }
    }

    fn switch_to_agents(&mut self) {
        self.screen = Screen::Agents;
        if self.agents.agents.is_empty() && !self.agents.loading {
            self.agents.loading = true;
            self.pool.dispatch(self.client.clone(), WorkRequest::ListAgents { job_id: next_job_id() });
        }
    }

    fn switch_to_missions(&mut self) {
        self.screen = Screen::Missions;
        if self.matrix.missions.is_empty() && !self.matrix.loading_missions {
            self.matrix.loading_missions = true;
            self.pool.dispatch(self.client.clone(), WorkRequest::ListMissions { job_id: next_job_id() });
        }
    }

    fn switch_to_feed(&mut self) {
        self.screen = Screen::Feed;
        if !self.agent_feed.live && !self.agent_feed.paused {
            self.pool.dispatch(
                self.client.clone(),
                WorkRequest::SubscribeFeed {
                    base_url: self.base_url.clone(),
                    token: self.token.clone(),
                },
            );
        }
    }

    fn switch_to_approvals(&mut self) {
        self.screen = Screen::Approvals;
        if self.approval_queue.pending.is_empty() && !self.approval_queue.loading {
            self.approval_queue.loading = true;
            self.pool.dispatch(
                self.client.clone(),
                WorkRequest::FetchApprovals { job_id: next_job_id(), mission_id: None },
            );
        }
    }

    fn switch_to_secrets(&mut self) {
        self.screen = Screen::Secrets;
        if self.secrets.tree.is_some() || self.secrets.no_profile_error.is_some() {
            return; // already initialized
        }

        let profile_path = dirs::home_dir()
            .map(|h| h.join(".mc").join("infisical_profiles.json"));

        let map: Option<mc_mesh_secrets::InfisicalProfileMap> = profile_path
            .and_then(|p| std::fs::read_to_string(p).ok())
            .and_then(|s| serde_json::from_str(&s).ok());

        let cfg = map.as_ref().and_then(|m| m.active_profile().cloned());

        let Some(cfg) = cfg else {
            self.secrets.no_profile_error = Some(
                "No active Infisical profile. Run: mc secrets infisical add <name> --service-token <token> --activate".into(),
            );
            return;
        };

        let Some(pid) = cfg.default_project_id.clone() else {
            self.secrets.no_profile_error = Some(
                "Active profile has no default_project_id configured.".into(),
            );
            return;
        };

        let env = cfg.default_environment.clone();
        self.secrets.cfg = Some(cfg.clone());

        if let Some((fid, nid)) = self.secrets.init_tree(pid.clone(), env.clone()) {
            self.pool.dispatch(
                self.client.clone(),
                WorkRequest::LoadSecretFolders {
                    job_id: fid,
                    project_id: pid.clone(),
                    environment: env.clone(),
                    path: "/".into(),
                    cfg: cfg.clone(),
                },
            );
            self.pool.dispatch(
                self.client.clone(),
                WorkRequest::LoadSecretNames {
                    job_id: nid,
                    project_id: pid,
                    environment: env,
                    path: "/".into(),
                    cfg,
                },
            );
        }
    }

    fn switch_context(&mut self, name: String) {
        let mut ctxs = crate::context::load_contexts();
        let Some(entry) = ctxs.contexts.get(&name).cloned() else { return };
        ctxs.active = name.clone();
        let _ = crate::context::save_contexts(&ctxs);

        let Ok(new_client) = super::data::RemoteDataClient::new(entry.base_url.clone(), self.token.clone())
        else { return };
        let new_client: std::sync::Arc<dyn DataClient> = std::sync::Arc::new(new_client);

        self.client = new_client.clone();
        self.base_url = entry.base_url.clone();
        self.context_name = name.clone();

        self.config.base_url = entry.base_url.clone();
        self.config.context_name = name.clone();
        self.config.connected = false;
        self.config.latency_ms = None;
        self.config.server_version = None;
        self.config.reload_contexts();

        // Reset all data that belonged to the old server.
        self.agents.agents.clear();
        self.agents.loading = false;
        self.agents.error = None;
        self.matrix.missions.clear();
        self.matrix.klusters.clear();
        self.matrix.tasks.clear();
        self.matrix.loading_missions = false;
        self.matrix.loading_klusters = false;
        self.matrix.loading_tasks = false;
        self.matrix.error = None;
        self.agent_feed.live = false;
        self.agent_feed.paused = false;
        self.approval_queue.pending.clear();
        self.approval_queue.loading = false;
        self.approval_queue.last_error = None;

        self.pool.dispatch(new_client.clone(), WorkRequest::Ping { job_id: next_job_id() });
        self.pool.dispatch(new_client, WorkRequest::ListAgents { job_id: next_job_id() });
    }

    fn missions_enter(&mut self) {
        let visible = self.matrix.visible_missions();
        let Some(mission) = visible.get(self.matrix.mission_selection) else { return };
        let mid = mission.id.clone();
        self.matrix.selected_mission_id = Some(mid.clone());
        self.matrix.selected_kluster_id = None;
        self.matrix.klusters.clear();
        self.matrix.tasks.clear();
        self.matrix.loading_klusters = true;
        self.matrix.kluster_selection = 0;
        self.pool.dispatch(
            self.client.clone(),
            WorkRequest::ListKlusters { mission_id: mid, job_id: next_job_id() },
        );
    }

    fn klusters_enter(&mut self) {
        let visible = self.matrix.visible_klusters();
        let Some(kluster) = visible.get(self.matrix.kluster_selection) else { return };
        let kid = kluster.id.clone();
        let mid = self.matrix.selected_mission_id.clone().unwrap_or_default();
        self.matrix.selected_kluster_id = Some(kid.clone());
        self.matrix.tasks.clear();
        self.matrix.loading_tasks = true;
        self.matrix.task_selection = 0;
        self.pool.dispatch(
            self.client.clone(),
            WorkRequest::ListTasks { mission_id: mid, kluster_id: kid, job_id: next_job_id() },
        );
    }

    pub fn draw<B: Backend>(&mut self, terminal: &mut Terminal<B>) -> Result<()> {
        self.tick();
        terminal.draw(|f| self.render(f))?;
        Ok(())
    }

    fn render(&self, f: &mut Frame<'_>) {
        let area = f.area();

        // v3 layout: tab bar (1) | content | hints bar (1)
        let chunks = Layout::default()
            .direction(Direction::Vertical)
            .constraints([
                Constraint::Length(1), // tab bar
                Constraint::Fill(1),   // content
                Constraint::Length(1), // hints bar
            ])
            .split(area);

        self.render_tab_bar(f, chunks[0]);

        match &self.screen {
            Screen::Agents => f.render_widget(AgentScreen { state: &self.agents }, chunks[1]),
            Screen::Missions => f.render_widget(MissionMatrix { state: &self.matrix }, chunks[1]),
            Screen::Feed => f.render_widget(AgentFeed { state: &self.agent_feed }, chunks[1]),
            Screen::Approvals => f.render_widget(ApprovalQueue { state: &self.approval_queue }, chunks[1]),
            Screen::Secrets => {
                f.render_widget(SecretsScreen { state: &self.secrets }, chunks[1]);
                render_tree_overlay(&self.secrets, f, chunks[1]);
            }
            Screen::Config => f.render_widget(ConfigScreen { state: &self.config }, chunks[1]),
        }

        self.render_hints(f, chunks[2]);
    }

    fn render_tab_bar(&self, f: &mut Frame<'_>, area: ratatui::layout::Rect) {
        let panel_bg = Color::Rgb(22, 27, 34);
        let panel_style = Style::default().bg(panel_bg);

        let tabs: &[(Screen, &str)] = &[
            (Screen::Agents, "Agents"),
            (Screen::Missions, "Missions"),
            (Screen::Feed, "Feed"),
            (Screen::Approvals, "Approvals"),
            (Screen::Secrets, "Secrets"),
            (Screen::Config, "Config"),
        ];

        let mut spans: Vec<Span> = vec![
            Span::styled(
                " mc ",
                Style::default().fg(theme::ACCENT).bg(panel_bg).add_modifier(Modifier::BOLD),
            ),
            Span::styled(" │ ", Style::default().fg(theme::PANEL_BORDER).bg(panel_bg)),
        ];

        for (screen, label) in tabs {
            let active = std::mem::discriminant(&self.screen) == std::mem::discriminant(screen);
            let style = if active {
                Style::default().fg(theme::TEXT).bg(theme::BG).add_modifier(Modifier::BOLD)
            } else {
                Style::default().fg(theme::TEXT_MUTED).bg(panel_bg)
            };
            spans.push(Span::styled(format!(" {label} "), style));
            spans.push(Span::styled(" │ ", Style::default().fg(theme::PANEL_BORDER).bg(panel_bg)));
        }

        // Right side: "<context> ● connected" or "<context> ● offline"
        let (dot, dot_style, status_str) = if self.config.connected {
            ("●", Style::default().fg(theme::OK).bg(panel_bg), "connected")
        } else {
            ("●", Style::default().fg(theme::ERR).bg(panel_bg), "offline")
        };

        let right_part = format!("  {}  {} {}  ", self.context_name, dot, status_str);
        let left_width: usize = spans.iter().map(|s| s.content.len()).sum();
        let pad = (area.width as usize).saturating_sub(left_width + right_part.len());

        spans.push(Span::styled(" ".repeat(pad), panel_style));
        spans.push(Span::styled(
            format!("  {}  ", self.context_name),
            Style::default().fg(theme::TEXT_MUTED).bg(panel_bg),
        ));
        spans.push(Span::styled(format!("{} ", dot), dot_style));
        spans.push(Span::styled(
            format!("{}  ", status_str),
            Style::default().fg(theme::TEXT_MUTED).bg(panel_bg),
        ));

        f.render_widget(
            Paragraph::new(Line::from(spans)).style(panel_style),
            area,
        );
    }

    fn render_hints(&self, f: &mut Frame<'_>, area: ratatui::layout::Rect) {
        let hints: &[(&str, &str)] = match &self.screen {
            Screen::Agents => &[
                ("Tab/S+Tab", "next/prev tab"),
                ("←→", "panels"),
                ("↑↓", "navigate"),
                ("Ctrl+Q", "quit"),
            ],
            Screen::Missions => &[
                ("Tab/S+Tab", "next/prev tab"),
                ("←→", "panes"),
                ("↑↓", "navigate"),
                ("/", "search"),
                ("Enter", "select"),
                ("Ctrl+Q", "quit"),
            ],
            Screen::Feed => &[
                ("Tab/S+Tab", "next/prev tab"),
                ("p", "pause"),
                ("c", "clear"),
                ("Ctrl+Q", "quit"),
            ],
            Screen::Approvals => &[
                ("Tab/S+Tab", "next/prev tab"),
                ("←→", "queue/detail"),
                ("↑↓", "navigate"),
                ("y", "approve"),
                ("n", "deny"),
                ("s", "skip"),
                ("Ctrl+Q", "quit"),
            ],
            Screen::Secrets => &[
                ("Tab/S+Tab", "next/prev tab"),
                ("↑↓", "navigate"),
                ("→/Enter", "expand"),
                ("←", "collapse"),
                ("Ctrl+Q", "quit"),
            ],
            Screen::Config if self.config.nav_selection == 4 => &[
                ("Tab/S+Tab", "next/prev tab"),
                ("↑↓", "contexts"),
                ("Enter", "switch"),
                ("Ctrl+Q", "quit"),
            ],
            Screen::Config => &[
                ("Tab/S+Tab", "next/prev tab"),
                ("↑↓", "navigate"),
                ("Ctrl+Q", "quit"),
            ],
        };

        let mut spans: Vec<Span> = vec![];
        for (key, desc) in hints {
            spans.push(Span::styled(format!("  {key}"), theme::muted()));
            spans.push(Span::styled(format!(" {desc}", desc = desc), theme::dim()));
        }

        f.render_widget(
            Paragraph::new(Line::from(spans)).style(theme::dim()),
            area,
        );
    }
}
