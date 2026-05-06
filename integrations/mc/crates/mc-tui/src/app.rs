use anyhow::Result;
use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
use ratatui::{
    backend::Backend,
    layout::{Constraint, Direction, Layout},
    text::{Line, Span},
    widgets::{Block, BorderType, Borders, Paragraph},
    Frame, Terminal,
};

use crate::data::DataClient;
use crate::screens::agent_feed::{AgentFeed, AgentFeedState};
use crate::screens::approval_queue::{ApprovalQueue, ApprovalQueueState};
use crate::screens::landing::LandingScreen;
use crate::screens::mission_matrix::{MissionMatrix, MissionMatrixState};
use crate::screens::receipts::{ReceiptsScreen, ReceiptsState};
use crate::screens::secrets::{SecretsScreen, SecretsState, render_tree_overlay};
use crate::theme;
use crate::work::{WorkPool, WorkRequest, WorkResult, next_job_id};
use mc_tui_widgets::status_bar::StatusBar;

#[derive(Debug, Clone, PartialEq)]
pub enum Screen {
    Landing,
    MissionMatrix,
    AgentFeed,
    ApprovalQueue,
    Receipts,
    Secrets,
    Help,
}

pub struct App {
    pub screen: Screen,
    pub base_url: String,
    pub token: Option<String>,
    pub version: String,
    pub should_quit: bool,
    pub status_extra: Option<String>,
    pub raft: Option<crate::data::RaftStatus>,

    // Per-screen state
    pub matrix: MissionMatrixState,
    pub agent_feed: AgentFeedState,
    pub approval_queue: ApprovalQueueState,
    pub receipts: ReceiptsState,
    pub secrets: SecretsState,

    client: std::sync::Arc<dyn DataClient>,
    pool: WorkPool,
}

impl App {
    pub fn new(
        base_url: String,
        token: Option<String>,
        version: String,
        initial_mission: Option<String>,
        client: std::sync::Arc<dyn DataClient>,
    ) -> Self {
        let mut matrix = MissionMatrixState::default();
        let screen = if initial_mission.is_some() {
            matrix.selected_mission_id = initial_mission;
            Screen::MissionMatrix
        } else {
            Screen::Landing
        };

        let pool = WorkPool::new();
        // Ping + raft status on startup to populate the status bar.
        pool.dispatch(client.clone(), WorkRequest::Ping { job_id: next_job_id() });
        pool.dispatch(client.clone(), WorkRequest::FetchRaftStatus { job_id: next_job_id() });

        Self {
            screen,
            base_url,
            token,
            version,
            should_quit: false,
            status_extra: None,
            raft: None,
            matrix,
            agent_feed: AgentFeedState::new(),
            approval_queue: ApprovalQueueState::default(),
            receipts: ReceiptsState::default(),
            secrets: SecretsState::default(),
            client,
            pool,
        }
    }

    /// Drain any pending work results and update state.
    pub fn tick(&mut self) {
        while let Ok(result) = self.pool.result_rx.try_recv() {
            match result {
                WorkResult::Pinged { ok, latency_ms, .. } => {
                    self.update_status_bar(ok, latency_ms);
                }
                WorkResult::RaftStatusFetched { status, .. } => {
                    self.raft = Some(status);
                    // Recomposite: if ping already ran, weave in raft node info.
                    if let Some(extra) = &self.status_extra {
                        let conn = extra.clone();
                        if let Some(r) = &self.raft {
                            self.status_extra = Some(format!("node {} · {} · {}", r.node_id, r.role, conn));
                        }
                    }
                }
                WorkResult::MissionsListed { missions, error, .. } => {
                    if let Some(e) = error {
                        self.status_extra = Some(format!("error: {e}"));
                    } else {
                        self.matrix.loading_missions = false;
                        self.matrix.missions = missions;
                        self.matrix.tree_selection = 0;
                    }
                }
                WorkResult::KlustersListed { mission_id, klusters, .. } => {
                    if Some(&mission_id) == self.matrix.selected_mission_id.as_ref() {
                        self.matrix.loading_klusters = false;
                        self.matrix.klusters = klusters;
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
                            .map(|a| crate::screens::approval_queue::ApprovalRequest {
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
                        self.pool.dispatch(self.client.clone(), crate::work::WorkRequest::FetchApprovals {
                            job_id: crate::work::next_job_id(), mission_id: None,
                        });
                    } else {
                        self.approval_queue.last_error = error;
                    }
                }
                WorkResult::RunsListed { runs, error, .. } => {
                    self.receipts.loading = false;
                    if error.is_none() {
                        use crate::screens::receipts::ReceiptEntry;
                        self.receipts.entries = runs.into_iter().map(|r| ReceiptEntry {
                            id: r.id,
                            created_at: r.created_at,
                            mission_name: None,
                            task_title: r.mesh_agent_id,
                            agent_id: r.owner_subject,
                            capability: r.runtime_kind,
                            outcome: r.status,
                            duration_ms: None,
                            artifact_count: 0,
                            output_summary: None,
                        }).collect();
                    }
                }
            }
        }

        // After draining results, check if the approval screen has a pending response
        if self.screen == Screen::ApprovalQueue {
            if let Some((id, approved)) = self.approval_queue.take_pending_response() {
                self.pool.dispatch(self.client.clone(), crate::work::WorkRequest::RespondApproval {
                    job_id: crate::work::next_job_id(),
                    approval_id: id.to_string(),
                    decision: if approved { "approve".to_string() } else { "reject".to_string() },
                    note: None,
                });
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
            Screen::MissionMatrix => {
                let c = self.matrix.handle_key(key.code);
                if key.code == KeyCode::Enter { self.matrix_enter(); }
                c
            }
            Screen::AgentFeed => self.agent_feed.handle_key(key.code),
            Screen::ApprovalQueue => self.approval_queue.handle_key(key.code),
            Screen::Receipts => self.receipts.handle_key(key.code),
            Screen::Secrets => {
                let reqs = self.secrets.handle_key(key.code);
                for req in reqs {
                    self.pool.dispatch(self.client.clone(), req);
                }
                true
            }
            _ => false,
        };
        if !consumed {
            self.handle_global_nav(key);
        }
    }

    fn handle_global_nav(&mut self, key: KeyEvent) {
        match key.code {
            KeyCode::Char('m') => self.switch_to_matrix(),
            KeyCode::Char('a') => {
                self.screen = Screen::ApprovalQueue;
                self.approval_queue.loading = true;
                self.pool.dispatch(
                    self.client.clone(),
                    crate::work::WorkRequest::FetchApprovals {
                        job_id: crate::work::next_job_id(),
                        mission_id: None,
                    },
                );
            }
            KeyCode::Char('f') => self.switch_to_feed(),
            KeyCode::Char('q') => {
                self.screen = Screen::Receipts;
                if self.receipts.entries.is_empty() && !self.receipts.loading {
                    self.receipts.loading = true;
                    self.pool.dispatch(self.client.clone(), crate::work::WorkRequest::ListRuns {
                        job_id: crate::work::next_job_id(),
                    });
                }
            }
            KeyCode::Char('s') => self.switch_to_secrets(),
            KeyCode::Char('?') => self.screen = Screen::Help,
            KeyCode::Esc => self.screen = Screen::Landing,
            _ => {}
        }
    }

    fn switch_to_feed(&mut self) {
        self.screen = Screen::AgentFeed;
        if !self.agent_feed.live && !self.agent_feed.paused {
            // Spawn the SSE subscription thread
            self.pool.dispatch(
                self.client.clone(),
                crate::work::WorkRequest::SubscribeFeed {
                    base_url: self.base_url.clone(),
                    token: self.token.clone(),
                },
            );
        }
    }

    fn switch_to_matrix(&mut self) {
        self.screen = Screen::MissionMatrix;
        if self.matrix.missions.is_empty() && !self.matrix.loading_missions {
            self.matrix.loading_missions = true;
            self.pool.dispatch(self.client.clone(), WorkRequest::ListMissions { job_id: next_job_id() });
        }
    }

    fn update_status_bar(&mut self, ok: bool, latency_ms: u64) {
        let conn = if ok {
            format!("connected {latency_ms}ms")
        } else {
            "backend unreachable".to_string()
        };
        self.status_extra = Some(match &self.raft {
            Some(r) => format!("node {} · {} · {}", r.node_id, r.role, conn),
            None => conn,
        });
    }

    fn switch_to_secrets(&mut self) {
        self.screen = Screen::Secrets;
        if self.secrets.tree.is_some() { return; }

        // Load the active Infisical profile from disk
        let path = {
            let home = dirs::home_dir().unwrap_or_default();
            home.join(".mc").join("infisical_profiles.json")
        };
        let map = if path.exists() {
            std::fs::read_to_string(&path)
                .ok()
                .and_then(|raw| serde_json::from_str::<mc_mesh_secrets::InfisicalProfileMap>(&raw).ok())
                .unwrap_or_default()
        } else {
            mc_mesh_secrets::InfisicalProfileMap::default()
        };

        match map.active_profile() {
            None => {
                self.secrets.no_profile_error = Some(
                    "No active Infisical profile. Run: mc secrets infisical add <name> --activate".to_string()
                );
            }
            Some(cfg) => {
                self.secrets.no_profile_error = None;
                let cfg = cfg.clone();
                let project_id = cfg.default_project_id.clone().unwrap_or_default();
                let environment = cfg.default_environment.clone();
                self.secrets.cfg = Some(cfg.clone());

                if let Some((fid, nid)) = self.secrets.init_tree(project_id.clone(), environment.clone()) {
                    self.pool.dispatch(
                        self.client.clone(),
                        WorkRequest::LoadSecretFolders {
                            job_id: fid,
                            project_id: project_id.clone(),
                            environment: environment.clone(),
                            path: "/".to_string(),
                            cfg: cfg.clone(),
                        },
                    );
                    self.pool.dispatch(
                        self.client.clone(),
                        WorkRequest::LoadSecretNames {
                            job_id: nid,
                            project_id,
                            environment,
                            path: "/".to_string(),
                            cfg,
                        },
                    );
                }
            }
        }
    }

    fn matrix_enter(&mut self) {
        use crate::screens::mission_matrix::TreeNode;
        let nodes = self.matrix.tree_nodes();
        let Some(node) = nodes.get(self.matrix.tree_selection) else { return };
        match node.clone() {
            TreeNode::Mission { idx } => {
                let mission = &self.matrix.missions[idx];
                let mid = mission.id.clone();
                self.matrix.selected_mission_id = Some(mid.clone());
                self.matrix.selected_kluster_id = None;
                self.matrix.klusters.clear();
                self.matrix.tasks.clear();
                self.matrix.loading_klusters = true;
                self.pool.dispatch(
                    self.client.clone(),
                    WorkRequest::ListKlusters { mission_id: mid, job_id: next_job_id() },
                );
            }
            TreeNode::Kluster { kluster_idx, .. } => {
                let kluster = &self.matrix.klusters[kluster_idx];
                let kid = kluster.id.clone();
                self.matrix.selected_kluster_id = Some(kid.clone());
                self.matrix.tasks.clear();
                self.matrix.loading_tasks = true;
                self.pool.dispatch(
                    self.client.clone(),
                    WorkRequest::ListTasks { kluster_id: kid, job_id: next_job_id() },
                );
            }
        }
    }

    pub fn draw<B: Backend>(&mut self, terminal: &mut Terminal<B>) -> Result<()> {
        self.tick();
        terminal.draw(|f| self.render(f))?;
        Ok(())
    }

    fn render(&self, f: &mut Frame<'_>) {
        let area = f.area();

        // Layout: (thin top spacer) | content | status bar
        let chunks = Layout::default()
            .direction(Direction::Vertical)
            .constraints([
                Constraint::Length(0),
                Constraint::Fill(1),
                Constraint::Length(1),
            ])
            .split(area);

        // Content area
        match &self.screen {
            Screen::Landing => f.render_widget(LandingScreen, chunks[1]),
            Screen::MissionMatrix => {
                f.render_widget(MissionMatrix { state: &self.matrix }, chunks[1]);
            }
            Screen::AgentFeed => {
                f.render_widget(AgentFeed { state: &self.agent_feed }, chunks[1]);
            }
            Screen::ApprovalQueue => {
                f.render_widget(ApprovalQueue { state: &self.approval_queue }, chunks[1]);
            }
            Screen::Receipts => {
                f.render_widget(ReceiptsScreen { state: &self.receipts }, chunks[1]);
            }
            Screen::Secrets => {
                f.render_widget(SecretsScreen { state: &self.secrets }, chunks[1]);
                render_tree_overlay(&self.secrets, f, chunks[1]);
            }
            Screen::Help => self.render_help(f, chunks[1]),
        }

        // Status bar
        let mut status = StatusBar::new(format!("v{}", self.version), self.base_url.clone());
        if let Some(extra) = &self.status_extra {
            status = status.with_extra(extra.clone());
        }
        f.render_widget(status, chunks[2]);
    }

    fn render_help(&self, f: &mut Frame<'_>, area: ratatui::layout::Rect) {
        let block = Block::default()
            .borders(Borders::ALL)
            .border_type(BorderType::Rounded)
            .border_style(theme::border_focused())
            .title(Span::styled(" Help ", theme::panel_title()))
            .style(theme::normal());

        let inner = block.inner(area);
        f.render_widget(block, area);

        let lines: Vec<Line> = vec![
            Line::from(""),
            Line::from(Span::styled("  Navigation", theme::muted())),
            Line::from(""),
            key_line("m", "Mission matrix"),
            key_line("f", "Agent feed"),
            key_line("a", "Approval queue"),
            key_line("q", "Receipts"),
            key_line("s", "Secrets browser"),
            key_line("?", "This help screen"),
            key_line("Esc", "Return to landing"),
            Line::from(""),
            Line::from(Span::styled("  Global", theme::muted())),
            Line::from(""),
            key_line("Ctrl+Q", "Quit"),
            key_line("Ctrl+C", "Quit"),
        ];
        f.render_widget(Paragraph::new(lines).style(theme::normal()), inner);
    }
}

fn key_line<'a>(key: &'a str, desc: &'a str) -> Line<'a> {
    Line::from(vec![
        Span::styled(format!("  {key:<10}", key = key), theme::accent()),
        Span::styled(desc, theme::dim()),
    ])
}
