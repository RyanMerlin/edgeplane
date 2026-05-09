pub mod app;
pub mod data;
pub mod screens;
pub mod theme;
pub mod widgets;
pub mod work;

use anyhow::Result;
use app::App;
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
    pub initial_mission: Option<String>,
    pub context_name: String,
}

pub fn run(cfg: TuiConfig) -> Result<()> {
    enable_raw_mode()?;
    let mut stdout = std::io::stdout();
    execute!(stdout, EnterAlternateScreen)?;

    let backend = CrosstermBackend::new(stdout);
    let mut terminal = Terminal::new(backend)?;
    terminal.clear()?;

    let data_client: Arc<dyn data::DataClient> =
        Arc::new(RemoteDataClient::new(cfg.base_url.clone(), cfg.token.clone())?);
    let mut app = App::new(
        cfg.base_url,
        cfg.token,
        cfg.version,
        cfg.initial_mission,
        cfg.context_name,
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
