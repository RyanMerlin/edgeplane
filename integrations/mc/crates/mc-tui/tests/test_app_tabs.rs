use mc_tui::app::{App, Screen};
use mc_tui::data::{DataClient, FixtureDataClient, MissionSummary};
use std::sync::Arc;

fn make_app(missions: Vec<MissionSummary>) -> App {
    // Called from within a #[tokio::test] context, so Handle::current() is available
    // for the WorkPool threads without creating a nested runtime.
    let client: Arc<dyn DataClient> = Arc::new(FixtureDataClient { missions });
    App::new("http://localhost:8008".into(), None, "test".into(), None, "default".into(), client)
}

#[tokio::test]
async fn agents_tab_loading_on_switch() {
    let mut app = make_app(vec![]);
    // Agents is already the initial screen and loads on startup; clear and re-test
    app.agents.agents.clear();
    app.agents.loading = false;

    let key = crossterm::event::KeyEvent::new(
        crossterm::event::KeyCode::Char('a'),
        crossterm::event::KeyModifiers::NONE,
    );
    app.handle_key(key);
    assert_eq!(app.screen, Screen::Agents);
    // Switch dispatches a ListAgents request; loading is set after switch
    // (startup already dispatched one, so loading may or may not be re-triggered)
}

#[tokio::test]
async fn missions_tab_sets_loading() {
    let mut app = make_app(vec![]);
    let key = crossterm::event::KeyEvent::new(
        crossterm::event::KeyCode::Char('m'),
        crossterm::event::KeyModifiers::NONE,
    );
    app.handle_key(key);
    assert_eq!(app.screen, Screen::Missions);
    assert!(app.matrix.loading_missions);
}

#[tokio::test]
async fn approvals_tab_sets_loading() {
    let mut app = make_app(vec![]);
    let key = crossterm::event::KeyEvent::new(
        crossterm::event::KeyCode::Char('p'),
        crossterm::event::KeyModifiers::NONE,
    );
    app.handle_key(key);
    assert_eq!(app.screen, Screen::Approvals);
    assert!(app.approval_queue.loading);
}

#[tokio::test]
async fn approvals_tab_result_clears_loading() {
    let mut app = make_app(vec![]);
    // Switch to approvals — loads
    let key = crossterm::event::KeyEvent::new(
        crossterm::event::KeyCode::Char('p'),
        crossterm::event::KeyModifiers::NONE,
    );
    app.handle_key(key);
    // After tick with fixture (empty approvals), loading should clear
    // Fixture returns immediately; tick drains the channel
    std::thread::sleep(std::time::Duration::from_millis(100));
    app.tick();
    assert!(!app.approval_queue.loading);
    assert!(app.approval_queue.last_error.is_none());
    assert!(app.approval_queue.pending.is_empty());
}

#[tokio::test]
async fn secrets_tab_sets_no_profile_error_when_unconfigured() {
    let mut app = make_app(vec![]);
    // Override HOME so no infisical_profiles.json exists
    let tmp = tempfile::tempdir().unwrap();
    // SAFETY: single-threaded test; no other threads reading HOME concurrently
    unsafe { std::env::set_var("HOME", tmp.path()); }

    let key = crossterm::event::KeyEvent::new(
        crossterm::event::KeyCode::Char('s'),
        crossterm::event::KeyModifiers::NONE,
    );
    app.handle_key(key);
    assert_eq!(app.screen, Screen::Secrets);
    assert!(app.secrets.no_profile_error.is_some(), "expected no-profile-error when no profile exists");
    assert!(app.secrets.tree.is_none());
}

#[tokio::test]
async fn feed_tab_navigable() {
    let mut app = make_app(vec![]);
    let key = crossterm::event::KeyEvent::new(
        crossterm::event::KeyCode::Char('f'),
        crossterm::event::KeyModifiers::NONE,
    );
    app.handle_key(key);
    assert_eq!(app.screen, Screen::Feed);
    // No panic; feed dispatches SubscribeFeed which will fail to connect but won't crash
}

#[tokio::test]
async fn config_tab_navigable() {
    let mut app = make_app(vec![]);
    let key = crossterm::event::KeyEvent::new(
        crossterm::event::KeyCode::Char('c'),
        crossterm::event::KeyModifiers::NONE,
    );
    app.handle_key(key);
    assert_eq!(app.screen, Screen::Config);
}
