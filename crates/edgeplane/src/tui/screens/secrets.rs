use crossterm::event::KeyCode;
use ratatui::{prelude::*, widgets::*};

use crate::tui::theme;
use crate::tui::widgets::secrets_tree::{SecretsTree, SecretsTreeAction, TreeMode};
use crate::tui::work::{JobId, WorkRequest, next_job_id};

// ── state ─────────────────────────────────────────────────────────────────────

pub struct SecretsState {
    /// The active Infisical config, loaded when the screen is first entered.
    pub cfg: Option<edgeplaned_secrets::InfisicalConfig>,
    /// Active profile project_id (pulled from cfg or prompted).
    pub project_id: Option<String>,
    /// Active profile environment (pulled from cfg default_environment).
    pub environment: String,
    /// The tree widget, initialized once project/environment are known.
    pub tree: Option<SecretsTree>,
    /// Error shown when no profile is configured.
    pub no_profile_error: Option<String>,
}

impl Default for SecretsState {
    fn default() -> Self {
        Self {
            cfg: None,
            project_id: None,
            environment: "prod".to_string(),
            tree: None,
            no_profile_error: None,
        }
    }
}

impl SecretsState {
    /// Initialize (or re-initialize) the tree for a given project/environment.
    /// Returns the initial (folders_job, names_job) pair so the caller can dispatch.
    pub fn init_tree(&mut self, project_id: String, environment: String) -> Option<(JobId, JobId)> {
        self.project_id = Some(project_id.clone());
        self.environment = environment.clone();

        let mut tree = SecretsTree::new(&project_id, &environment, TreeMode::Browse);
        let fid = next_job_id();
        let nid = next_job_id();
        tree.initial_load_ids(fid, nid);
        self.tree = Some(tree);
        Some((fid, nid))
    }

    /// Handle a key event. Returns pending work requests that should be dispatched.
    pub fn handle_key(&mut self, code: KeyCode) -> Vec<WorkRequest> {
        let Some(tree) = &mut self.tree else {
            return vec![];
        };
        let cfg = match &self.cfg {
            Some(c) => c.clone(),
            None => return vec![],
        };
        let project_id = self.project_id.clone().unwrap_or_default();
        let environment = self.environment.clone();

        let mut id_gen = || next_job_id();
        let action = tree.handle_key(code, &mut id_gen);

        match action {
            SecretsTreeAction::NeedsLoad {
                path,
                folders_job,
                names_job,
            } => {
                vec![
                    WorkRequest::LoadSecretFolders {
                        job_id: folders_job,
                        project_id: project_id.clone(),
                        environment: environment.clone(),
                        path: path.clone(),
                        cfg: cfg.clone(),
                    },
                    WorkRequest::LoadSecretNames {
                        job_id: names_job,
                        project_id,
                        environment,
                        path,
                        cfg,
                    },
                ]
            }
            SecretsTreeAction::Cancelled => {
                // Esc in browse mode: clear any error so user can retry navigation
                if let Some(t) = &mut self.tree {
                    t.error = None;
                }
                vec![]
            }
            _ => vec![],
        }
    }
}

// ── widget ────────────────────────────────────────────────────────────────────

pub struct SecretsScreen<'a> {
    pub state: &'a SecretsState,
}

impl Widget for SecretsScreen<'_> {
    fn render(self, area: Rect, buf: &mut Buffer) {
        // Draw the outer chrome: full-screen block
        let block = Block::default()
            .borders(Borders::ALL)
            .border_type(BorderType::Rounded)
            .border_style(theme::border_focused())
            .title(Span::styled(" Secrets Browser ", theme::accent_bold()))
            .style(theme::normal());

        let inner = block.inner(area);
        block.render(area, buf);

        if let Some(ref err) = self.state.no_profile_error {
            let chunks = Layout::vertical([
                Constraint::Fill(1),
                Constraint::Length(5),
                Constraint::Fill(1),
            ])
            .split(inner);
            let lines = vec![
                Line::from(Span::styled(err.as_str(), theme::err())),
                Line::from(""),
                Line::from(Span::styled(
                    "Go to Config → Identity → Infisical, then press n to add a profile.",
                    theme::muted(),
                )),
                Line::from(""),
                Line::from(Span::styled(
                    "Or run: edgeplane secrets infisical add <name> --service-token <token> --activate",
                    theme::dim(),
                )),
            ];
            Paragraph::new(lines)
                .alignment(Alignment::Center)
                .render(chunks[1], buf);
            return;
        }

        if self.state.tree.is_none() {
            let chunks = Layout::vertical([
                Constraint::Fill(1),
                Constraint::Length(1),
                Constraint::Fill(1),
            ])
            .split(inner);
            Paragraph::new(Span::styled("  Loading profile…", theme::muted()))
                .alignment(Alignment::Center)
                .render(chunks[1], buf);
        }
        // The SecretsTree renders itself as a dialog overlay — it needs a Frame,
        // so the actual tree render is called from App::render() directly.
    }
}

/// Render the tree overlay on top of the screen background.
/// Call this after rendering SecretsScreen.
pub fn render_tree_overlay(state: &SecretsState, f: &mut Frame, area: Rect) {
    if let Some(ref tree) = state.tree {
        tree.render(f, area);
    }
}
