pub mod app;
pub mod data;
pub mod screens;
pub mod theme;
pub mod widgets;
pub mod work;

use anyhow::Result;
use app::{App, AuthState};
use crossterm::{
    event::{self, Event, KeyCode},
    execute,
    terminal::{EnterAlternateScreen, LeaveAlternateScreen, disable_raw_mode, enable_raw_mode},
};
use data::RemoteDataClient;
use ratatui::{Terminal, backend::CrosstermBackend};
use std::sync::Arc;
use std::time::Duration;

pub struct TuiConfig {
    pub base_url: String,
    pub token: Option<String>,
    pub version: String,
    pub initial_domain: Option<String>,
    pub context_name: String,
}

pub fn run(cfg: TuiConfig) -> Result<()> {
    // Resolve auth state before raw-mode flip so any auth-related I/O can use
    // normal stderr if it ever needs to. Precedence: explicit cfg.token (CLI
    // flag / env) > saved session file > anonymous.
    let (resolved_token, auth_state) = resolve_auth(&cfg);

    enable_raw_mode()?;
    let mut stdout = std::io::stdout();
    execute!(stdout, EnterAlternateScreen)?;

    let backend = CrosstermBackend::new(stdout);
    let mut terminal = Terminal::new(backend)?;
    terminal.clear()?;

    let data_client: Arc<dyn data::DataClient> =
        Arc::new(RemoteDataClient::new(cfg.base_url.clone(), resolved_token.clone())?);
    let mut app = App::new(
        cfg.base_url,
        resolved_token,
        cfg.version,
        cfg.initial_domain,
        cfg.context_name,
        auth_state,
        data_client,
    );

    let result = event_loop(&mut terminal, &mut app);

    disable_raw_mode()?;
    execute!(terminal.backend_mut(), LeaveAlternateScreen)?;
    terminal.show_cursor()?;

    result
}

fn event_loop<B: ratatui::backend::Backend>(
    terminal: &mut Terminal<B>,
    app: &mut App,
) -> Result<()> {
    loop {
        app.draw(terminal)?;

        if event::poll(Duration::from_millis(50))? {
            if let Event::Key(key) = event::read()? {
                // Hard-stop: Ctrl+C always quits
                if key.code == KeyCode::Char('c')
                    && key.modifiers.contains(crossterm::event::KeyModifiers::CONTROL)
                {
                    break;
                }
                app.handle_key(key);
            }
        }

        if app.should_quit {
            break;
        }
    }
    Ok(())
}

/// Resolve the effective auth token and TUI auth state at startup.
///
/// Returns the token to pass to the data client (None means anonymous) and
/// the initial AuthState shown in the identity badge. The token from `cfg`
/// wins over the saved session — that path is set by `--token` / `EP_TOKEN`
/// in the parent CLI and the operator may be testing against a one-off
/// credential.
fn resolve_auth(cfg: &TuiConfig) -> (Option<String>, AuthState) {
    // Explicit token: trust it, mark as session-like (we don't know expiry).
    if let Some(tok) = cfg.token.clone() {
        return (Some(tok), AuthState::SessionFromFlag);
    }

    // Saved session: validate URL + expiry, surface details to the identity badge.
    if let Some(saved) = crate::auth::load_saved_session(&cfg.base_url) {
        let state = AuthState::SessionValid {
            subject: saved.subject.clone(),
            email: saved.email.clone(),
            expires_at: saved.expires_at.clone(),
        };
        return (Some(saved.token), state);
    }

    // Saved session file present but expired or URL-mismatched — show a
    // distinct state so the badge can prompt for re-auth.
    if session_file_exists_for(&cfg.base_url) {
        return (None, AuthState::SessionExpired);
    }

    (None, AuthState::Anonymous)
}

/// Returns true if there's a session file on disk for any URL — used purely
/// to distinguish "first launch" (Anonymous) from "had a session, it lapsed"
/// (SessionExpired).
fn session_file_exists_for(_base_url: &str) -> bool {
    crate::auth::session_file_path().exists()
}
