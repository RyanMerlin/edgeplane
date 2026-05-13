use std::time::{Duration, Instant};

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

use super::data::{is_auth_error, DataClient, RemoteDataClient};
use super::screens::agent_feed::{AgentFeed, AgentFeedState};
use super::screens::agents::{AgentOp, AgentScreen, AgentScreenState};
use super::screens::approval_queue::{ApprovalQueue, ApprovalQueueState};
use super::screens::config::{ConfigScreen, ConfigScreenState, DoctorCheckRow, DoctorStatus};
use super::screens::mission_matrix::{Focus as MatrixFocus, MissionMatrix, MissionMatrixState};
use super::screens::secrets::{SecretsScreen, SecretsState, render_tree_overlay};
use super::theme;
use super::widgets::help::{HelpEntry, HelpOverlay, GLOBAL_HELP};
use super::widgets::modal::{ConfirmModal, InfoModal, ModalAction, OidcLoginModal, OidcLoginState};
use super::work::{OidcFlowEvent, WorkPool, WorkRequest, WorkResult, next_job_id};

/// Identity / session state used to drive the badge in the tab bar and to
/// decide whether to surface a "you're not signed in" prompt in panels that
/// require auth. This is the Phase 1 surface from `mc-tui-auth-spec.md` —
/// it detects and reports state but does not yet drive an in-TUI login flow.
#[derive(Debug, Clone)]
pub enum AuthState {
    /// No session file on disk, no explicit token. First-launch state.
    Anonymous,
    /// A valid saved session was loaded from `~/.missioncontrol/`.
    SessionValid {
        subject: String,
        email: Option<String>,
        expires_at: String,
    },
    /// A session file exists but is expired or URL-mismatched. The badge
    /// flags this so the operator knows the silent-empty panels aren't a
    /// real outage.
    SessionExpired,
    /// Token came from `--token` / `MC_TOKEN`. We don't know its expiry, so
    /// the badge labels it explicitly rather than guessing.
    SessionFromFlag,
}

impl AuthState {
    /// True when API calls should be expected to return private data. False
    /// when panels that require auth should render the unauthenticated prompt
    /// instead of pretending the empty result is the truth.
    pub fn is_authenticated(&self) -> bool {
        matches!(
            self,
            AuthState::SessionValid { .. } | AuthState::SessionFromFlag
        )
    }
}

/// What action a confirm modal should trigger when the user accepts.
#[derive(Debug, Clone)]
pub enum PendingAction {
    DeleteAgent(String),
    RestartAgent(String),
    ClearAgentContext(String),
    DenyApproval(i64),
}

/// All currently-supported modal kinds. Extending this enum is the way to
/// add new dialogs (task picker, etc.) without sprinkling overlays across screens.
pub enum AppModal {
    Confirm { modal: ConfirmModal, action: PendingAction },
    Info { modal: InfoModal },
    OidcLogin { modal: OidcLoginModal },
}

impl AppModal {
    fn render(&self, area: ratatui::layout::Rect, buf: &mut ratatui::buffer::Buffer) {
        match self {
            AppModal::Confirm { modal, .. } => modal.render(area, buf),
            AppModal::Info { modal } => modal.render(area, buf),
            AppModal::OidcLogin { modal } => modal.render(area, buf),
        }
    }
}

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

    /// Identity / session state. Mutated by `tick()` when 401 responses are
    /// observed; rendered in the tab-bar identity badge and consulted by the
    /// "you're not signed in" prompt in panels that require auth.
    pub auth_state: AuthState,

    // Per-screen state
    pub agents: AgentScreenState,
    pub matrix: MissionMatrixState,
    pub agent_feed: AgentFeedState,
    pub approval_queue: ApprovalQueueState,
    pub secrets: SecretsState,
    pub config: ConfigScreenState,

    // Auto-refresh bookkeeping. The Instant is the time of the last successful
    // fetch for each list-style screen; tick() compares it against an interval
    // and redispatches if the screen is currently visible.
    pub agents_last_refresh: Option<Instant>,
    pub approvals_last_refresh: Option<Instant>,
    pub missions_last_refresh: Option<Instant>,

    /// Last time we polled `~/.missioncontrol/session.json` looking for a
    /// fresh session (the user running `mc auth login` in another shell).
    /// Throttled to a couple of seconds — see `try_reauth_from_disk`.
    pub auth_last_check: Option<Instant>,

    /// Active modal overlay, if any. Modals consume input first.
    pub modal: Option<AppModal>,

    /// Whether the global help overlay (?) is currently shown.
    pub help_open: bool,

    client: std::sync::Arc<dyn DataClient>,
    pool: WorkPool,
}

const AGENTS_REFRESH_INTERVAL: Duration = Duration::from_secs(3);
const APPROVALS_REFRESH_INTERVAL: Duration = Duration::from_secs(3);
const MISSIONS_REFRESH_INTERVAL: Duration = Duration::from_secs(5);

const AGENTS_HELP: &[HelpEntry] = &[
    HelpEntry { keys: "↑↓",      desc: "navigate agents/nodes" },
    HelpEntry { keys: "←→",      desc: "switch between Nodes and Agents pane" },
    HelpEntry { keys: "r",       desc: "restart selected agent (confirm)" },
    HelpEntry { keys: "x",       desc: "clear selected agent's context (confirm)" },
    HelpEntry { keys: "d",       desc: "remove selected agent (confirm)" },
];

const MISSIONS_HELP: &[HelpEntry] = &[
    HelpEntry { keys: "↑↓",      desc: "navigate within the focused pane" },
    HelpEntry { keys: "←→",      desc: "move focus between Missions / Klusters / Tasks" },
    HelpEntry { keys: "/",       desc: "filter missions or klusters (Esc to clear)" },
    HelpEntry { keys: "Enter",   desc: "drill into selected mission/kluster" },
];

const FEED_HELP: &[HelpEntry] = &[
    HelpEntry { keys: "↑↓",      desc: "scroll feed" },
    HelpEntry { keys: "p",       desc: "pause / resume" },
    HelpEntry { keys: "c",       desc: "clear buffer" },
    HelpEntry { keys: "/",       desc: "filter events" },
];

const APPROVALS_HELP: &[HelpEntry] = &[
    HelpEntry { keys: "↑↓",      desc: "navigate queue" },
    HelpEntry { keys: "←→",      desc: "switch Queue / Detail focus" },
    HelpEntry { keys: "y",       desc: "approve" },
    HelpEntry { keys: "n",       desc: "deny (confirm)" },
    HelpEntry { keys: "s",       desc: "skip to next" },
];

const SECRETS_HELP: &[HelpEntry] = &[
    HelpEntry { keys: "↑↓",      desc: "navigate tree" },
    HelpEntry { keys: "→/Enter", desc: "expand folder" },
    HelpEntry { keys: "←",       desc: "collapse / go to parent" },
    HelpEntry { keys: "r",       desc: "retry root load when an error is shown" },
    HelpEntry { keys: "Esc",     desc: "leave the secrets browser" },
];

const CONFIG_HELP: &[HelpEntry] = &[
    HelpEntry { keys: "↑↓",      desc: "navigate sections / panel content" },
    HelpEntry { keys: "→/Enter", desc: "focus the panel for the selected section" },
    HelpEntry { keys: "←/Esc",   desc: "return focus to the section list" },
    HelpEntry { keys: "n e d",   desc: "(Infisical) add / edit / delete a profile" },
];

impl App {
    pub fn new(
        base_url: String,
        token: Option<String>,
        version: String,
        initial_mission: Option<String>,
        context_name: String,
        auth_state: AuthState,
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
        config.reload_infisical_from_disk();

        Self {
            screen: Screen::Agents,
            base_url,
            token,
            version,
            context_name,
            should_quit: false,
            auth_state,
            agents: AgentScreenState::default(),
            matrix,
            agent_feed: AgentFeedState::new(),
            approval_queue: ApprovalQueueState::default(),
            secrets: SecretsState::default(),
            config,
            agents_last_refresh: None,
            approvals_last_refresh: None,
            missions_last_refresh: None,
            auth_last_check: None,
            modal: None,
            help_open: false,
            client,
            pool,
        }
    }

    /// Poll the on-disk session file and, if a fresh valid session has
    /// appeared since we last checked, swap the data client to use the new
    /// token and trigger a refresh of the current panel. This is how the
    /// TUI auto-recovers when the operator runs `mc auth login` in another
    /// shell — they come back to the TUI and within a couple of seconds it
    /// reloads.
    ///
    /// `force` makes the check run regardless of the throttle window — used
    /// by the explicit `R` keybind so the user gets an immediate response.
    /// Returns `true` if the auth state actually changed.
    pub fn try_reauth_from_disk(&mut self, force: bool) -> bool {
        const POLL_INTERVAL: Duration = Duration::from_secs(2);
        let now = Instant::now();
        if !force {
            if let Some(last) = self.auth_last_check {
                if now.duration_since(last) < POLL_INTERVAL {
                    return false;
                }
            }
        }
        self.auth_last_check = Some(now);

        // Only useful when we're NOT currently authenticated — there's no
        // reason to swap a working session. (An explicit `R` press still
        // refreshes data via the caller; this method just gates the auth
        // swap.)
        if self.auth_state.is_authenticated() {
            return false;
        }

        let Some(saved) = crate::auth::load_saved_session(&self.base_url) else {
            return false;
        };

        let new_state = AuthState::SessionValid {
            subject: saved.subject.clone(),
            email: saved.email.clone(),
            expires_at: saved.expires_at.clone(),
        };
        let Ok(new_client) = RemoteDataClient::new(self.base_url.clone(), Some(saved.token.clone())) else {
            return false;
        };
        self.token = Some(saved.token);
        self.auth_state = new_state;
        self.client = std::sync::Arc::new(new_client);

        // Clear stale auth-error messages so the next render shows the
        // refreshed panel instead of the old "Not signed in" copy.
        self.agents.error = None;
        self.matrix.error = None;
        self.approval_queue.last_error = None;
        // Reset per-screen refresh stamps so `auto_refresh` immediately
        // re-fetches with the new credentials rather than waiting out the
        // interval against the pre-auth `now`.
        self.agents_last_refresh = None;
        self.approvals_last_refresh = None;
        self.missions_last_refresh = None;

        true
    }

    /// Translate raw error text from a backend call into a more useful surface.
    /// 401s flip `auth_state` to `SessionExpired` and rewrite the error string
    /// so panels show a sign-in prompt instead of a cryptic "backend returned
    /// 401" line. Non-auth errors pass through unchanged.
    ///
    /// Returns the (possibly rewritten) error string. Always Some — call sites
    /// were already setting `Some(e)`, this keeps that shape.
    fn classify_error(&mut self, raw: String) -> Option<String> {
        if is_auth_error(&raw) {
            // Don't overwrite SessionFromFlag — a bad explicit token is still
            // a user-provided credential, label it as such.
            if !matches!(self.auth_state, AuthState::SessionFromFlag) {
                self.auth_state = AuthState::SessionExpired;
            }
            return Some(
                "Not signed in — press L to see how to authenticate.".to_string(),
            );
        }
        Some(raw)
    }

    /// Open a modal explaining how to sign in, or launch the in-TUI OIDC flow.
    pub fn open_signin_modal(&mut self) {
        match &self.auth_state {
            AuthState::SessionFromFlag => {
                self.modal = Some(AppModal::Info {
                    modal: InfoModal {
                        title: "Identity".to_string(),
                        lines: vec![
                            "You're signed in via --token / MC_TOKEN.".to_string(),
                            "".to_string(),
                            "If calls are failing, the explicit token is invalid.".to_string(),
                            "Clear it and run `mc auth login` to use a session.".to_string(),
                        ],
                    },
                });
            }
            AuthState::SessionValid { subject, expires_at, .. } => {
                let subject = subject.clone();
                let expires_at = expires_at.clone();
                self.modal = Some(AppModal::Info {
                    modal: InfoModal {
                        title: "Identity".to_string(),
                        lines: vec![
                            format!("Signed in as {subject}."),
                            format!("Token expires {expires_at}."),
                            "".to_string(),
                            "Use `mc auth logout` to clear the session.".to_string(),
                        ],
                    },
                });
            }
            AuthState::Anonymous | AuthState::SessionExpired => {
                if !self.config.connected {
                    // Server is not reachable — OIDC would also fail; show a diagnostic instead.
                    let base_url = self.base_url.clone();
                    self.modal = Some(AppModal::Info {
                        modal: InfoModal {
                            title: "Server Not Reachable".to_string(),
                            lines: vec![
                                format!("Cannot connect to: {base_url}"),
                                "".to_string(),
                                "Check that the server is running and MC_BASE_URL is correct.".to_string(),
                                "  mc system start    — start the local dev stack".to_string(),
                                format!("  MC_BASE_URL=https://your-server.com mc tui"),
                                "".to_string(),
                                "Press R to retry once it's up.".to_string(),
                            ],
                        },
                    });
                } else {
                    // Server is up — kick off the in-TUI OIDC flow.
                    let job_id = next_job_id();
                    self.pool.dispatch(
                        self.client.clone(),
                        WorkRequest::OidcFlow {
                            job_id,
                            base_url: self.base_url.clone(),
                            ttl_hours: 8,
                        },
                    );
                    self.modal = Some(AppModal::OidcLogin {
                        modal: OidcLoginModal {
                            state: OidcLoginState::Initiating,
                        },
                    });
                }
            }
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
                        self.agents.error = self.classify_error(e);
                    } else {
                        self.agents.error = None;
                        self.agents.replace_agents(agents);
                        self.agents_last_refresh = Some(Instant::now());
                    }
                }
                WorkResult::AgentDeleted { agent_id, ok, error, .. } => {
                    if ok {
                        self.agents.agents.retain(|a| a.id != agent_id);
                        let max = self.agents.visible_agents().len().saturating_sub(1);
                        if self.agents.agent_selection > max { self.agents.agent_selection = max; }
                        // Refresh in case the server deleted other rows we don't know about.
                        self.pool.dispatch(self.client.clone(), WorkRequest::ListAgents { job_id: next_job_id() });
                    } else {
                        self.agents.error = error;
                    }
                }
                WorkResult::AgentOpCompleted { ok, error, .. } => {
                    if ok {
                        // Re-fetch immediately so status flips show without waiting for the poll tick.
                        self.pool.dispatch(self.client.clone(), WorkRequest::ListAgents { job_id: next_job_id() });
                    } else {
                        self.agents.error = error;
                    }
                }
                WorkResult::MissionsListed { missions, error, .. } => {
                    if let Some(e) = error {
                        let classified = self.classify_error(e).unwrap_or_default();
                        self.matrix.error = Some(if is_auth_error(&classified) || classified.starts_with("Not signed in") {
                            classified
                        } else {
                            format!("missions error: {classified}")
                        });
                    } else {
                        self.matrix.error = None;
                        self.matrix.loading_missions = false;
                        let prev_id = self.matrix.visible_missions().get(self.matrix.mission_selection).map(|m| m.id.clone());
                        self.matrix.missions = missions;
                        if let Some(id) = prev_id {
                            if let Some(idx) = self.matrix.visible_missions().iter().position(|m| m.id == id) {
                                self.matrix.mission_selection = idx;
                            } else {
                                self.matrix.mission_selection = 0;
                            }
                        } else {
                            self.matrix.mission_selection = 0;
                        }
                        self.missions_last_refresh = Some(Instant::now());
                    }
                }
                WorkResult::KlustersListed { mission_id, klusters, .. } => {
                    if Some(&mission_id) == self.matrix.selected_mission_id.as_ref() {
                        self.matrix.loading_klusters = false;
                        let prev_id = self.matrix.visible_klusters().get(self.matrix.kluster_selection).map(|k| k.id.clone());
                        self.matrix.klusters = klusters;
                        if let Some(id) = prev_id {
                            if let Some(idx) = self.matrix.visible_klusters().iter().position(|k| k.id == id) {
                                self.matrix.kluster_selection = idx;
                            } else {
                                self.matrix.kluster_selection = 0;
                            }
                        } else {
                            self.matrix.kluster_selection = 0;
                        }
                    }
                }
                WorkResult::TasksListed { kluster_id, tasks, .. } => {
                    if Some(&kluster_id) == self.matrix.selected_kluster_id.as_ref() {
                        self.matrix.loading_tasks = false;
                        let prev_id = self.matrix.tasks.get(self.matrix.task_selection).map(|t| t.id);
                        self.matrix.tasks = tasks;
                        if let Some(id) = prev_id {
                            if let Some(idx) = self.matrix.tasks.iter().position(|t| t.id == id) {
                                self.matrix.task_selection = idx;
                            } else {
                                self.matrix.task_selection = 0;
                            }
                        } else {
                            self.matrix.task_selection = 0;
                        }
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
                        self.approval_queue.last_error = self.classify_error(e);
                    } else {
                        self.approval_queue.last_error = None;
                        let prev_id = self.approval_queue.pending.get(self.approval_queue.selection).map(|r| r.id);
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
                        if let Some(id) = prev_id {
                            if let Some(idx) = self.approval_queue.pending.iter().position(|r| r.id == id) {
                                self.approval_queue.selection = idx;
                            } else {
                                self.approval_queue.selection = 0;
                            }
                        } else {
                            self.approval_queue.selection = 0;
                        }
                        self.approvals_last_refresh = Some(Instant::now());
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
                WorkResult::OidcFlow(event) => {
                    match event {
                        OidcFlowEvent::Initiated { authorize_url, .. } => {
                            if let Some(AppModal::OidcLogin { modal }) = &mut self.modal {
                                modal.state = OidcLoginState::AwaitingBrowser {
                                    authorize_url,
                                    started: std::time::Instant::now(),
                                };
                            }
                        }
                        OidcFlowEvent::Complete { token, subject, expires_at, email, .. } => {
                            // Update auth state with the new session.
                            self.auth_state = AuthState::SessionValid {
                                subject: subject.clone(),
                                email: email.clone(),
                                expires_at: expires_at.clone(),
                            };
                            // Rebuild the data client with the new token.
                            self.token = Some(token.clone());
                            if let Ok(new_client) = super::data::RemoteDataClient::new(
                                self.base_url.clone(),
                                Some(token),
                            ) {
                                self.client = std::sync::Arc::new(new_client);
                            }
                            // Close the modal and schedule an immediate refresh.
                            self.modal = None;
                            self.agents_last_refresh = None;
                            self.missions_last_refresh = None;
                            self.approvals_last_refresh = None;
                            // Clear any stale auth error messages from panels.
                            self.agents.error = None;
                            self.matrix.error = None;
                            self.approval_queue.last_error = None;
                        }
                        OidcFlowEvent::TimedOut { .. } => {
                            if let Some(AppModal::OidcLogin { modal }) = &mut self.modal {
                                modal.state = OidcLoginState::TimedOut;
                            }
                        }
                        OidcFlowEvent::Failed { error, .. } => {
                            if let Some(AppModal::OidcLogin { modal }) = &mut self.modal {
                                modal.state = OidcLoginState::Failed { error };
                            }
                        }
                    }
                }
            }
        }

        self.auto_refresh();
        self.refresh_doctor_snapshot();
    }

    /// Recompute the Doctor panel snapshot from current app state. Cheap —
    /// reads existing fields, builds a small Vec. Called every tick so the
    /// panel reflects auth/connection/fetch state immediately rather than
    /// waiting for an explicit refresh.
    fn refresh_doctor_snapshot(&mut self) {
        let mut checks: Vec<DoctorCheckRow> = Vec::with_capacity(6);

        // Controlplane reachability — comes from the periodic ping.
        let (cp_status, cp_detail) = if self.config.connected {
            let lat = self
                .config
                .latency_ms
                .map(|ms| format!("{ms}ms"))
                .unwrap_or_else(|| "—".to_string());
            (DoctorStatus::Ok, format!("connected · {lat} · {}", self.base_url))
        } else {
            (DoctorStatus::Err, format!("unreachable at {}", self.base_url))
        };
        checks.push(DoctorCheckRow {
            name: "Controlplane",
            status: cp_status,
            detail: cp_detail,
            hint: if matches!(cp_status, DoctorStatus::Err) {
                Some("Check MC_BASE_URL and that the controlplane is running.".to_string())
            } else {
                None
            },
        });

        // Server version match.
        let (v_status, v_detail) = match self.config.server_version.as_deref() {
            Some(sv) if sv == self.version.as_str() => (
                DoctorStatus::Ok,
                format!("client {} · server {}", self.version, sv),
            ),
            Some(sv) => (
                DoctorStatus::Warn,
                format!("client {} · server {} (mismatch)", self.version, sv),
            ),
            None => (DoctorStatus::Unknown, format!("client {}", self.version)),
        };
        checks.push(DoctorCheckRow {
            name: "Version",
            status: v_status,
            detail: v_detail,
            hint: if matches!(v_status, DoctorStatus::Warn) {
                Some("`mc update` to align with the controlplane.".to_string())
            } else {
                None
            },
        });

        // Auth.
        let (a_status, a_detail, a_hint) = match &self.auth_state {
            AuthState::SessionValid {
                subject,
                email,
                expires_at,
            } => {
                let who = email.as_deref().unwrap_or(subject.as_str());
                (
                    DoctorStatus::Ok,
                    format!("session · {who} · expires {expires_at}"),
                    None,
                )
            }
            AuthState::SessionFromFlag => (
                DoctorStatus::Ok,
                "explicit token (--token / MC_TOKEN)".to_string(),
                None,
            ),
            AuthState::SessionExpired => (
                DoctorStatus::Err,
                "session expired".to_string(),
                Some(
                    "Run `mc auth login` in another shell — TUI auto-recovers within 2s.".to_string(),
                ),
            ),
            AuthState::Anonymous => (
                DoctorStatus::Warn,
                "not signed in".to_string(),
                Some("Press L for sign-in instructions.".to_string()),
            ),
        };
        checks.push(DoctorCheckRow {
            name: "Auth",
            status: a_status,
            detail: a_detail,
            hint: a_hint,
        });

        // Agents fetch.
        checks.push(error_to_check("Agents fetch", &self.agents.error));

        // Missions fetch.
        checks.push(error_to_check("Missions fetch", &self.matrix.error));

        // Approvals fetch.
        checks.push(error_to_check("Approvals fetch", &self.approval_queue.last_error));

        self.config.doctor = checks;
    }

    /// Re-dispatch list-fetch work for the currently visible screen if its
    /// last refresh is older than the per-screen interval. Hidden screens are
    /// untouched — switching to them triggers a fresh load via switch_to_*.
    fn auto_refresh(&mut self) {
        // Quietly notice if the operator ran `mc auth login` in another
        // shell and now has a fresh session on disk. No-op when we're
        // already authenticated.
        self.try_reauth_from_disk(false);

        let now = Instant::now();
        let stale = |last: Option<Instant>, interval: Duration| {
            last.map(|t| now.duration_since(t) >= interval).unwrap_or(false)
        };
        match self.screen {
            Screen::Agents => {
                if !self.agents.loading && stale(self.agents_last_refresh, AGENTS_REFRESH_INTERVAL) {
                    self.agents_last_refresh = Some(now); // stamp now to dedupe in-flight
                    self.pool.dispatch(self.client.clone(), WorkRequest::ListAgents { job_id: next_job_id() });
                }
            }
            Screen::Approvals => {
                if !self.approval_queue.loading && stale(self.approvals_last_refresh, APPROVALS_REFRESH_INTERVAL) {
                    self.approvals_last_refresh = Some(now);
                    self.pool.dispatch(self.client.clone(), WorkRequest::FetchApprovals { job_id: next_job_id(), mission_id: None });
                }
            }
            Screen::Missions => {
                if !self.matrix.loading_missions && stale(self.missions_last_refresh, MISSIONS_REFRESH_INTERVAL) {
                    self.missions_last_refresh = Some(now);
                    self.pool.dispatch(self.client.clone(), WorkRequest::ListMissions { job_id: next_job_id() });
                }
            }
            Screen::Feed | Screen::Secrets | Screen::Config => {}
        }
    }

    pub fn handle_key(&mut self, key: KeyEvent) {
        // Global Ctrl-Q / Ctrl-C (handled in event_loop for C) to quit
        if key.code == KeyCode::Char('q') && key.modifiers.contains(KeyModifiers::CONTROL) {
            self.should_quit = true;
            return;
        }

        // Help overlay: any key closes it and is consumed (so the user isn't
        // surprised by a side-effect from the closing key).
        if self.help_open {
            self.help_open = false;
            return;
        }
        if key.code == KeyCode::Char('?') {
            self.help_open = true;
            return;
        }

        // Modal owns input first — it can consume, confirm, or cancel.
        if self.modal.is_some() {
            self.handle_modal_key(key.code);
            return;
        }

        // Global: 'L' opens the identity / sign-in modal from anywhere.
        // Lowercase 'l' is reserved by other screens' nav patterns, so the
        // shifted form is the unambiguous binding.
        if key.code == KeyCode::Char('L') {
            self.open_signin_modal();
            return;
        }

        // Global: 'R' force-refreshes from disk. If the operator just ran
        // `mc auth login` in another shell, this picks up the new session
        // immediately rather than waiting for the next auto-refresh tick.
        // Always re-dispatches a fetch for the current panel so a manual
        // refresh works even when authenticated already.
        if key.code == KeyCode::Char('R') {
            self.try_reauth_from_disk(true);
            self.agents_last_refresh = None;
            self.approvals_last_refresh = None;
            self.missions_last_refresh = None;
            // Force an immediate refetch of the current panel.
            match &self.screen {
                Screen::Agents => {
                    self.pool.dispatch(self.client.clone(), WorkRequest::ListAgents { job_id: next_job_id() });
                }
                Screen::Missions => {
                    self.pool.dispatch(self.client.clone(), WorkRequest::ListMissions { job_id: next_job_id() });
                }
                Screen::Approvals => {
                    self.pool.dispatch(self.client.clone(), WorkRequest::FetchApprovals { job_id: next_job_id(), mission_id: None });
                }
                Screen::Feed | Screen::Secrets | Screen::Config => {}
            }
            return;
        }

        // Screen-level key routing — each screen gets first crack at nav keys
        let consumed = match &self.screen {
            Screen::Agents => {
                let c = self.agents.handle_key(key.code);
                if let Some(op) = self.agents.take_pending_op() {
                    self.open_agent_op_modal(op);
                }
                c
            }
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
                // Approve dispatches immediately.
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
                // Deny goes through a confirm modal.
                if let Some((id, action)) = self.approval_queue.take_pending_deny_confirm() {
                    self.modal = Some(AppModal::Confirm {
                        modal: ConfirmModal {
                            title: "Confirm Deny".into(),
                            message: format!("Deny approval \"{}\"?", action),
                            danger: true,
                        },
                        action: PendingAction::DenyApproval(id),
                    });
                }
                c
            }
            Screen::Secrets => {
                if key.code == KeyCode::Esc {
                    self.prev_tab();
                    return;
                }
                let requests = self.secrets.handle_key(key.code);
                for req in requests {
                    self.pool.dispatch(self.client.clone(), req);
                }
                matches!(
                    key.code,
                    KeyCode::Up
                        | KeyCode::Down
                        | KeyCode::Left
                        | KeyCode::Right
                        | KeyCode::Enter
                        | KeyCode::Backspace
                        | KeyCode::PageUp
                        | KeyCode::PageDown
                        | KeyCode::Char(' ')
                        | KeyCode::Char('a')
                        | KeyCode::Char('r')
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

    fn handle_modal_key(&mut self, code: KeyCode) {
        let Some(m) = &self.modal else { return };
        let action = match m {
            AppModal::Confirm { modal, .. } => modal.handle_key(code),
            AppModal::Info { modal } => modal.handle_key(code),
            AppModal::OidcLogin { modal } => modal.handle_key(code),
        };
        match action {
            ModalAction::Confirmed => {
                if let Some(AppModal::Confirm { action, .. }) = self.modal.take() {
                    self.dispatch_pending_action(action);
                }
            }
            ModalAction::Cancelled => { self.modal = None; }
            ModalAction::Handled | ModalAction::Passthrough => {}
        }
    }

    fn dispatch_pending_action(&mut self, action: PendingAction) {
        let req = match action {
            PendingAction::DeleteAgent(id)        => WorkRequest::DeleteAgent { job_id: next_job_id(), agent_id: id },
            PendingAction::RestartAgent(id)       => WorkRequest::RestartAgent { job_id: next_job_id(), agent_id: id },
            PendingAction::ClearAgentContext(id)  => WorkRequest::ClearAgentContext { job_id: next_job_id(), agent_id: id },
            PendingAction::DenyApproval(id) => {
                self.approval_queue.confirm_deny(id);
                if let Some((aid, approve)) = self.approval_queue.take_pending_response() {
                    return self.pool.dispatch(self.client.clone(), WorkRequest::RespondApproval {
                        job_id: next_job_id(),
                        approval_id: aid.to_string(),
                        decision: if approve { "approve".into() } else { "reject".into() },
                        note: None,
                    });
                }
                return;
            }
        };
        self.pool.dispatch(self.client.clone(), req);
    }

    fn open_agent_op_modal(&mut self, op: AgentOp) {
        let (title, message, danger, action) = match op {
            AgentOp::Delete { id, name } => (
                "Confirm Delete",
                format!("Remove agent \"{}\"?", name),
                true,
                PendingAction::DeleteAgent(id),
            ),
            AgentOp::Restart { id, name } => (
                "Confirm Restart",
                format!("Restart agent \"{}\"? Open sessions will be ended.", name),
                false,
                PendingAction::RestartAgent(id),
            ),
            AgentOp::ClearContext { id, name } => (
                "Confirm Clear Context",
                format!("Clear context for agent \"{}\"?", name),
                false,
                PendingAction::ClearAgentContext(id),
            ),
        };
        self.modal = Some(AppModal::Confirm {
            modal: ConfirmModal { title: title.to_string(), message, danger },
            action,
        });
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
        // If tree is loaded and healthy, nothing to do
        if let Some(tree) = &self.secrets.tree {
            if tree.error.is_none() { return; }
            // Tree has an error — reset so we reinitialize on re-entry
            self.secrets.tree = None;
        }
        // Always re-check profile on entry — user may have just added one in Config
        self.secrets.no_profile_error = None;

        let profile_path = dirs::home_dir()
            .map(|h| h.join(".mc").join("infisical_profiles.json"));

        let map: Option<mc_mesh_secrets::InfisicalProfileMap> = profile_path
            .and_then(|p| std::fs::read_to_string(p).ok())
            .and_then(|s| serde_json::from_str(&s).ok());

        let cfg = map.as_ref().and_then(|m| m.active_profile().cloned());

        let Some(cfg) = cfg else {
            self.secrets.no_profile_error = Some(
                "No active Infisical profile.".into(),
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
        self.config.reload_infisical_from_disk();

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
        self.agents_last_refresh = None;
        self.approvals_last_refresh = None;
        self.missions_last_refresh = None;
        self.modal = None;

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

        // Modal overlay last so it sits above everything else.
        if let Some(m) = &self.modal {
            m.render(area, f.buffer_mut());
        }

        // Help overlay sits on top of even modals — closing it returns the user
        // to whatever was underneath.
        if self.help_open {
            let (title, entries) = self.screen_help();
            f.render_widget(HelpOverlay { title, entries, global: GLOBAL_HELP }, area);
        }
    }

    fn screen_help(&self) -> (&'static str, &'static [HelpEntry]) {
        match self.screen {
            Screen::Agents => ("Agents", AGENTS_HELP),
            Screen::Missions => ("Missions", MISSIONS_HELP),
            Screen::Feed => ("Feed", FEED_HELP),
            Screen::Approvals => ("Approvals", APPROVALS_HELP),
            Screen::Secrets => ("Secrets", SECRETS_HELP),
            Screen::Config => ("Config", CONFIG_HELP),
        }
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
                Style::default().fg(theme::ACCENT).bg(theme::BG).add_modifier(Modifier::BOLD)
            } else {
                Style::default().fg(theme::TEXT_MUTED).bg(panel_bg)
            };
            spans.push(Span::styled(format!(" {label} "), style));
            spans.push(Span::styled(" │ ", Style::default().fg(theme::PANEL_BORDER).bg(panel_bg)));
        }

        // Right side: identity badge · context · connection status.
        let (dot, dot_style, status_str) = if self.config.connected {
            ("●", Style::default().fg(theme::OK).bg(panel_bg), "connected")
        } else {
            ("●", Style::default().fg(theme::ERR).bg(panel_bg), "offline")
        };

        // Identity badge — see AuthState for the four possible shapes. The
        // colour signals state at a glance; the text gives the operator
        // enough to know whether to press L.
        let (badge_text, badge_color) = match &self.auth_state {
            AuthState::SessionValid { subject, email, .. } => {
                let who = email.as_deref().unwrap_or(subject.as_str());
                (format!("{who}"), theme::OK)
            }
            AuthState::SessionFromFlag => ("--token".to_string(), theme::ACCENT),
            AuthState::SessionExpired => ("session expired · press L".to_string(), theme::ERR),
            AuthState::Anonymous => ("not signed in · press L".to_string(), theme::WARN),
        };

        let right_part = format!(
            "  {}  ·  {}  {} {}  ",
            badge_text, self.context_name, dot, status_str
        );
        let left_width: usize = spans.iter().map(|s| s.content.len()).sum();
        let pad = (area.width as usize).saturating_sub(left_width + right_part.len());

        spans.push(Span::styled(" ".repeat(pad), panel_style));
        spans.push(Span::styled(
            format!("  {}  ", badge_text),
            Style::default().fg(badge_color).bg(panel_bg).add_modifier(Modifier::BOLD),
        ));
        spans.push(Span::styled(
            "·  ".to_string(),
            Style::default().fg(theme::PANEL_BORDER).bg(panel_bg),
        ));
        spans.push(Span::styled(
            format!("{}  ", self.context_name),
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
        // Modal hints take precedence — there's nothing else the user can do.
        if self.modal.is_some() {
            let spans = vec![
                Span::styled("  y/Enter", theme::muted()),
                Span::styled(" confirm   ", theme::dim()),
                Span::styled("n/Esc", theme::muted()),
                Span::styled(" cancel", theme::dim()),
            ];
            f.render_widget(Paragraph::new(Line::from(spans)).style(theme::dim()), area);
            return;
        }
        if self.help_open {
            let spans = vec![
                Span::styled("  any key", theme::muted()),
                Span::styled(" close help", theme::dim()),
            ];
            f.render_widget(Paragraph::new(Line::from(spans)).style(theme::dim()), area);
            return;
        }

        let hints: &[(&str, &str)] = match &self.screen {
            Screen::Agents => &[
                ("Tab/S+Tab", "next/prev tab"),
                ("↑↓", "navigate"),
                ("r", "restart"),
                ("x", "clear ctx"),
                ("d", "remove"),
                ("?", "help"),
            ],
            Screen::Missions => &[
                ("Tab/S+Tab", "next/prev tab"),
                ("↑↓", "navigate"),
                ("/", "search"),
                ("Enter", "select"),
                ("?", "help"),
            ],
            Screen::Feed => &[
                ("Tab/S+Tab", "next/prev tab"),
                ("p", "pause"),
                ("c", "clear"),
                ("?", "help"),
            ],
            Screen::Approvals => &[
                ("Tab/S+Tab", "next/prev tab"),
                ("↑↓", "navigate"),
                ("y", "approve"),
                ("n", "deny"),
                ("?", "help"),
            ],
            Screen::Secrets => &[
                ("Tab/S+Tab", "next/prev tab"),
                ("↑↓", "navigate"),
                ("→/Enter", "expand"),
                ("←", "collapse"),
                ("?", "help"),
            ],
            Screen::Config if self.config.infisical_form.is_some() => &[
                ("Tab", "next field"),
                ("F2", "toggle auth mode"),
                ("Enter", "save"),
                ("Esc", "cancel"),
            ],
            Screen::Config if self.config.content_focused && self.config.nav_selection == 4 => &[
                ("↑↓", "navigate"),
                ("Enter", "switch context"),
                ("←/Esc", "nav panel"),
                ("Ctrl+Q", "quit"),
            ],
            Screen::Config if self.config.content_focused && self.config.nav_selection == 5 => &[
                ("↑↓", "navigate"),
                ("Enter", "activate"),
                ("e", "edit"),
                ("d", "delete"),
                ("n", "add"),
                ("←/Esc", "back"),
                ("Ctrl+Q", "quit"),
            ],
            Screen::Config => &[
                ("Tab/S+Tab", "next/prev tab"),
                ("↑↓", "navigate"),
                ("→", "focus panel"),
                ("?", "help"),
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

/// Translate a panel's optional error string into a [`DoctorCheckRow`]. The
/// classifier from `tick()` has already rewritten auth errors into a
/// friendly "Not signed in — press L…" message, so the row distinguishes
/// auth-class failures (mapped to `Warn`, with a remediation hint) from
/// other backend failures (`Err`).
fn error_to_check(name: &'static str, err: &Option<String>) -> DoctorCheckRow {
    match err {
        None => DoctorCheckRow {
            name,
            status: DoctorStatus::Ok,
            detail: "OK".to_string(),
            hint: None,
        },
        Some(msg) if msg.starts_with("Not signed in") => DoctorCheckRow {
            name,
            status: DoctorStatus::Warn,
            detail: msg.clone(),
            hint: Some(
                "Auto-recovers after `mc auth login`; press R to retry now.".to_string(),
            ),
        },
        Some(msg) => DoctorCheckRow {
            name,
            status: DoctorStatus::Err,
            detail: msg.clone(),
            hint: None,
        },
    }
}
