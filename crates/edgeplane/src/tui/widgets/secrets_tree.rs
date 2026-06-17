use std::collections::{HashMap, HashSet};
use crossterm::event::KeyCode;
use ratatui::{prelude::*, widgets::*};

use super::theme;

// ── JobId ─────────────────────────────────────────────────────────────────────

pub type JobId = u64;

// ── data model ────────────────────────────────────────────────────────────────

#[derive(Debug, Clone, PartialEq)]
pub enum NodeKind { Folder, Secret }

/// Per-path state: what we know about a folder's contents.
#[derive(Default, Debug, Clone)]
pub struct PathState {
    pub folders: Vec<String>,
    pub secrets: Vec<String>,
    pub loaded: bool,
    pub folders_loading: bool,
    pub secrets_loading: bool,
}

impl PathState {
    pub fn is_loading(&self) -> bool { self.folders_loading || self.secrets_loading }
}

/// A resolved Infisical secret reference (for Bind mode).
#[derive(Debug, Clone)]
pub struct InfisicalRef {
    pub secret_name: String,
    pub project_id: Option<String>,
    pub environment: String,
    pub secret_path: String,
    pub infisical_profile: Option<String>,
}

/// One entry in the flattened visible list.
#[derive(Clone, Debug)]
pub struct VisibleRow {
    pub depth: usize,
    pub kind: NodeKind,
    pub name: String,
    pub parent_path: String,
    pub full_path: String,
}

// ── widget ────────────────────────────────────────────────────────────────────

#[derive(Debug, Clone, PartialEq)]
pub enum TreeMode {
    Browse,
    Bind,
}

#[derive(Debug, Clone)]
pub struct SecretsTree {
    pub project_id: String,
    pub environment: String,
    pub mode: TreeMode,
    pub paths: HashMap<String, PathState>,
    pub expanded: HashSet<String>,
    pub visible: Vec<VisibleRow>,
    pub cursor: usize,
    pub selected: HashSet<usize>,
    pub pending_folders: HashMap<String, JobId>,
    pub pending_names: HashMap<String, JobId>,
    pub error: Option<String>,
}

impl SecretsTree {
    pub fn new(project_id: &str, environment: &str, mode: TreeMode) -> Self {
        let mut tree = Self {
            project_id: project_id.to_string(),
            environment: environment.to_string(),
            mode,
            paths: HashMap::new(),
            expanded: {
                let mut s = HashSet::new();
                s.insert("/".to_string());
                s
            },
            visible: vec![],
            cursor: 0,
            selected: HashSet::new(),
            pending_folders: HashMap::new(),
            pending_names: HashMap::new(),
            error: None,
        };
        tree.paths.insert("/".to_string(), PathState {
            folders_loading: true,
            secrets_loading: true,
            ..Default::default()
        });
        tree
    }

    pub fn initial_load_ids(&mut self, folders_job: JobId, names_job: JobId) {
        self.pending_folders.insert("/".to_string(), folders_job);
        self.pending_names.insert("/".to_string(), names_job);
    }

    /// Request a load for a specific path. Returns (folders_job_id, names_job_id).
    pub fn request_load(&mut self, path: &str, next_id: &mut dyn FnMut() -> JobId) -> (JobId, JobId) {
        let fid = next_id();
        let nid = next_id();
        self.pending_folders.insert(path.to_string(), fid);
        self.pending_names.insert(path.to_string(), nid);
        let state = self.paths.entry(path.to_string()).or_default();
        state.folders_loading = true;
        state.secrets_loading = true;
        (fid, nid)
    }

    /// Deliver folder list for a completed job. Returns true if the job was accepted.
    pub fn deliver_folders(&mut self, job_id: JobId, folders: Vec<String>, error: Option<String>) -> bool {
        let path = self.pending_folders.iter()
            .find(|(_, jid)| **jid == job_id)
            .map(|(p, _)| p.clone());
        let Some(path) = path else { return false; };
        self.pending_folders.remove(&path);
        let state = self.paths.entry(path).or_default();
        state.folders_loading = false;
        if let Some(e) = error {
            self.error = Some(format!("Folder list error: {e}"));
        } else {
            state.folders = folders;
        }
        if !state.is_loading() { state.loaded = true; }
        self.rebuild_visible();
        true
    }

    /// Deliver secret names for a completed job. Returns true if accepted.
    pub fn deliver_names(&mut self, job_id: JobId, names: Vec<String>, error: Option<String>) -> bool {
        let path = self.pending_names.iter()
            .find(|(_, jid)| **jid == job_id)
            .map(|(p, _)| p.clone());
        let Some(path) = path else { return false; };
        self.pending_names.remove(&path);
        let state = self.paths.entry(path).or_default();
        state.secrets_loading = false;
        if let Some(e) = error {
            if self.error.is_none() { self.error = Some(format!("Secret list error: {e}")); }
        } else {
            state.secrets = names;
        }
        if !state.is_loading() { state.loaded = true; }
        self.rebuild_visible();
        true
    }

    pub fn rebuild_visible(&mut self) {
        let mut rows = vec![];
        self.dfs_collect("/", 0, &mut rows);
        self.visible = rows;
        if !self.visible.is_empty() && self.cursor >= self.visible.len() {
            self.cursor = self.visible.len() - 1;
        }
    }

    fn dfs_collect(&self, path: &str, depth: usize, out: &mut Vec<VisibleRow>) {
        let Some(state) = self.paths.get(path) else { return; };

        for folder in &state.folders {
            let full_path = if path == "/" {
                format!("/{}/", folder)
            } else {
                format!("{}{}/", path, folder)
            };
            out.push(VisibleRow {
                depth,
                kind: NodeKind::Folder,
                name: folder.clone(),
                parent_path: path.to_string(),
                full_path: full_path.clone(),
            });
            if self.expanded.contains(&full_path) {
                self.dfs_collect(&full_path, depth + 1, out);
            }
        }

        for secret in &state.secrets {
            out.push(VisibleRow {
                depth,
                kind: NodeKind::Secret,
                name: secret.clone(),
                parent_path: path.to_string(),
                full_path: format!("{}{}", path, secret),
            });
        }
    }

    pub fn is_root_loading(&self) -> bool {
        self.paths.get("/").map(|s| s.is_loading()).unwrap_or(true)
    }

    pub fn current_path(&self) -> &str {
        self.visible.get(self.cursor)
            .map(|r| r.parent_path.as_str())
            .unwrap_or("/")
    }

    // ── key handling ─────────────────────────────────────────────────────────

    pub fn handle_key(&mut self, code: KeyCode, next_id: &mut dyn FnMut() -> JobId) -> SecretsTreeAction {
        // When the tree is in an error state, only Esc (cancel) and 'r' (retry root) are honored.
        if self.error.is_some() {
            match code {
                KeyCode::Esc => return SecretsTreeAction::Cancelled,
                KeyCode::Char('r') => {
                    self.error = None;
                    let root_state = self.paths.entry("/".to_string()).or_default();
                    root_state.loaded = false;
                    let (fid, nid) = self.request_load("/", next_id);
                    return SecretsTreeAction::NeedsLoad {
                        path: "/".to_string(),
                        folders_job: fid,
                        names_job: nid,
                    };
                }
                _ => return SecretsTreeAction::None,
            }
        }
        match code {
            KeyCode::Esc => return SecretsTreeAction::Cancelled,

            KeyCode::Up => {
                if self.cursor > 0 { self.cursor -= 1; }
            }
            KeyCode::Down => {
                if self.cursor + 1 < self.visible.len() { self.cursor += 1; }
            }
            KeyCode::PageUp => {
                self.cursor = self.cursor.saturating_sub(10);
            }
            KeyCode::PageDown => {
                self.cursor = (self.cursor + 10).min(self.visible.len().saturating_sub(1));
            }

            KeyCode::Right | KeyCode::Enter => {
                if let Some(row) = self.visible.get(self.cursor).cloned() {
                    match row.kind {
                        NodeKind::Folder => {
                            return self.toggle_or_enter_folder(&row, next_id);
                        }
                        NodeKind::Secret => {
                            if let Some(multi) = self.confirm_multi_select() {
                                return multi;
                            }
                            if self.mode == TreeMode::Bind {
                                return SecretsTreeAction::Selected(self.make_ref(&row));
                            }
                        }
                    }
                }
            }

            KeyCode::Left => {
                if let Some(row) = self.visible.get(self.cursor).cloned() {
                    if row.kind == NodeKind::Folder && self.expanded.contains(&row.full_path) {
                        self.expanded.remove(&row.full_path);
                        self.rebuild_visible();
                    } else if row.depth > 0 {
                        let parent = row.parent_path.clone();
                        if let Some(idx) = self.visible.iter().position(|r| r.full_path == parent) {
                            self.cursor = idx;
                        }
                    }
                }
            }

            KeyCode::Char(' ') => {
                if let Some(row) = self.visible.get(self.cursor)
                    && row.kind == NodeKind::Secret {
                        if self.selected.contains(&self.cursor) {
                            self.selected.remove(&self.cursor);
                        } else {
                            self.selected.insert(self.cursor);
                        }
                    }
            }

            KeyCode::Char('a') => {
                for (i, row) in self.visible.iter().enumerate() {
                    if row.kind == NodeKind::Secret {
                        self.selected.insert(i);
                    }
                }
            }

            _ => {}
        }
        SecretsTreeAction::None
    }

    fn toggle_or_enter_folder(&mut self, row: &VisibleRow, next_id: &mut dyn FnMut() -> JobId) -> SecretsTreeAction {
        if self.expanded.contains(&row.full_path) {
            self.expanded.remove(&row.full_path);
            self.rebuild_visible();
        } else {
            self.expanded.insert(row.full_path.clone());
            if !self.paths.get(&row.full_path).map(|s| s.loaded).unwrap_or(false)
                && !self.pending_folders.contains_key(&row.full_path) {
                let (fid, nid) = self.request_load(&row.full_path, next_id);
                return SecretsTreeAction::NeedsLoad {
                    path: row.full_path.clone(),
                    folders_job: fid,
                    names_job: nid,
                };
            }
            self.rebuild_visible();
        }
        SecretsTreeAction::None
    }

    fn confirm_multi_select(&mut self) -> Option<SecretsTreeAction> {
        if self.selected.is_empty() { return None; }
        let refs: Vec<InfisicalRef> = self.selected.iter()
            .filter_map(|&idx| self.visible.get(idx))
            .filter(|r| r.kind == NodeKind::Secret)
            .map(|r| self.make_ref(r))
            .collect();
        if refs.is_empty() { return None; }
        Some(SecretsTreeAction::SelectedMany(refs))
    }

    fn make_ref(&self, row: &VisibleRow) -> InfisicalRef {
        InfisicalRef {
            secret_name: row.name.clone(),
            project_id: Some(self.project_id.clone()),
            environment: self.environment.clone(),
            secret_path: row.parent_path.clone(),
            infisical_profile: None,
        }
    }

    // ── render ────────────────────────────────────────────────────────────────

    pub fn render(&self, f: &mut Frame, area: Rect) {
        let width = area.width.saturating_sub(4).clamp(54, 90);
        let height = area.height.saturating_sub(4).clamp(14, 28);
        let x = (area.width.saturating_sub(width)) / 2;
        let y = (area.height.saturating_sub(height)) / 2;
        let dialog = Rect::new(x, y, width, height);

        f.render_widget(Clear, dialog);

        let mode_badge = if self.mode == TreeMode::Bind { " [BIND]" } else { "" };
        let title = format!(
            " Infisical · {} · {}{} ",
            self.project_id, self.environment, mode_badge
        );
        let block = Block::default()
            .borders(Borders::ALL)
            .title(Span::styled(title, theme::accent_bold()))
            .border_style(theme::border_focused());
        let inner = block.inner(dialog);
        f.render_widget(block, dialog);

        let chunks = Layout::vertical([
                Constraint::Length(1),
                Constraint::Min(0),
                Constraint::Length(1),
            ])
            .split(inner);

        let raw = self.current_path();
        let crumb = if raw == "/" {
            "/".to_string()
        } else {
            raw.trim_start_matches('/').trim_end_matches('/').replace('/', " › ")
        };
        f.render_widget(
            Paragraph::new(Span::styled(format!(" {}", crumb), theme::dim())),
            chunks[0],
        );

        self.render_tree(f, chunks[1]);
        self.render_hint(f, chunks[2]);
    }

    fn render_tree(&self, f: &mut Frame, area: Rect) {
        if let Some(err) = &self.error {
            let lines = vec![
                Line::from(Span::styled(err.as_str(), theme::danger())),
                Line::from(""),
                Line::from(Span::styled("press r to retry · esc to close", theme::muted())),
            ];
            f.render_widget(Paragraph::new(lines), area);
            return;
        }

        if self.is_root_loading() {
            let frame = std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .map(|d| (d.subsec_millis() / 250) as usize % 4)
                .unwrap_or(0);
            let spinner = ["⠋", "⠙", "⠹", "⠸"][frame];
            f.render_widget(
                Paragraph::new(Span::styled(format!("  {} Loading…", spinner), theme::muted())),
                area,
            );
            return;
        }

        if self.visible.is_empty() {
            f.render_widget(
                Paragraph::new(Span::styled("  (empty — no secrets or folders found)", theme::inactive())),
                area,
            );
            return;
        }

        let items: Vec<ListItem> = self.visible.iter().enumerate().map(|(i, row)| {
            let is_cursor = i == self.cursor;
            let is_selected = self.selected.contains(&i);

            let guide = "│   ".repeat(row.depth);
            let (icon, base_style) = match row.kind {
                NodeKind::Folder => {
                    let expanded = self.expanded.contains(&row.full_path);
                    let loading = self.pending_folders.contains_key(&row.full_path)
                        || self.pending_names.contains_key(&row.full_path);
                    let icon = if loading { "⠸ " } else if expanded { "▾ " } else { "▸ " };
                    (icon, if is_cursor { theme::selected() } else { theme::accent() })
                }
                NodeKind::Secret => {
                    let icon = if is_selected { "[✓] " } else { "[ ] " };
                    let style = if is_cursor { theme::selected() } else if is_selected { theme::ok() } else { theme::normal() };
                    (icon, style)
                }
            };

            ListItem::new(Line::from(vec![
                Span::styled(format!("  {}", guide), theme::dim()),
                Span::styled(format!("{}{}", icon, row.name), base_style),
            ]))
        }).collect();

        let list = List::new(items).highlight_style(theme::selected());
        let mut state = ListState::default();
        state.select(Some(self.cursor));
        f.render_stateful_widget(list, area, &mut state);
    }

    fn render_hint(&self, f: &mut Frame, area: Rect) {
        let sel_count = self.selected.len();
        let hint = if sel_count > 0 {
            format!("{} selected  space:toggle  a:all  enter:confirm  esc:cancel", sel_count)
        } else {
            "↑↓ move  →/enter:expand  ←:collapse  esc:close".to_string()
        };
        f.render_widget(
            Paragraph::new(hint).style(theme::muted()),
            area,
        );
    }
}

// ── action ────────────────────────────────────────────────────────────────────

#[allow(dead_code)]
pub enum SecretsTreeAction {
    None,
    Cancelled,
    NeedsLoad { path: String, folders_job: JobId, names_job: JobId },
    Selected(InfisicalRef),
    SelectedMany(Vec<InfisicalRef>),
}

#[cfg(test)]
mod tests {
    use super::*;
    use crossterm::event::KeyCode;

    fn next_id_factory() -> impl FnMut() -> JobId {
        let mut n: JobId = 100;
        move || { n += 1; n }
    }

    #[test]
    fn deliver_folders_with_stale_job_id_is_noop() {
        let mut tree = SecretsTree::new("p", "prod", TreeMode::Browse);
        tree.initial_load_ids(1, 2);
        // Stale id (never registered) — should not crash, should not mutate state.
        let accepted = tree.deliver_folders(9999, vec!["unexpected".into()], None);
        assert!(!accepted);
        assert!(tree.paths.get("/").map(|s| s.folders.is_empty()).unwrap_or(true));
    }

    #[test]
    fn retry_after_error_clears_state_and_requests_load() {
        let mut tree = SecretsTree::new("p", "prod", TreeMode::Browse);
        tree.error = Some("boom".into());
        let mut next = next_id_factory();
        let action = tree.handle_key(KeyCode::Char('r'), &mut next);
        assert!(tree.error.is_none(), "error must be cleared on retry");
        match action {
            SecretsTreeAction::NeedsLoad { path, .. } => assert_eq!(path, "/"),
            _ => panic!("retry must request a fresh root load"),
        }
    }

    #[test]
    fn keys_other_than_esc_and_r_are_ignored_when_errored() {
        let mut tree = SecretsTree::new("p", "prod", TreeMode::Browse);
        tree.error = Some("err".into());
        let mut next = next_id_factory();
        // Down arrow must not move cursor or panic.
        let _ = tree.handle_key(KeyCode::Down, &mut next);
        assert_eq!(tree.cursor, 0);
        assert!(tree.error.is_some(), "error stays until cleared by 'r' or Esc");
    }
}
