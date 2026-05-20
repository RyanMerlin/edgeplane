use mc::config::{load_server_list, write_servers_file};
use std::sync::Mutex;
use tempfile::tempdir;

/// All tests that mutate process-wide env vars must hold this lock.
/// Prevents races when cargo runs tests in parallel threads.
static ENV: Mutex<()> = Mutex::new(());

#[test]
fn test_write_servers_file_creates_file() {
    let _guard = ENV.lock().unwrap_or_else(|p| p.into_inner());
    let dir = tempdir().unwrap();
    unsafe { std::env::set_var("MC_HOME", dir.path().to_str().unwrap()) };

    write_servers_file(&[
        "https://node-a:8008".to_string(),
        "https://node-b:8008".to_string(),
    ])
    .unwrap();

    let contents = std::fs::read_to_string(dir.path().join("servers")).unwrap();
    assert!(contents.contains("https://node-a:8008"));
    assert!(contents.contains("https://node-b:8008"));
    assert!(contents.contains("# Auto-generated"));

    unsafe { std::env::remove_var("MC_HOME") };
}

#[test]
fn test_write_servers_file_overwrites_existing() {
    let _guard = ENV.lock().unwrap_or_else(|p| p.into_inner());
    let dir = tempdir().unwrap();
    unsafe { std::env::set_var("MC_HOME", dir.path().to_str().unwrap()) };

    write_servers_file(&["https://old:8008".to_string()]).unwrap();
    write_servers_file(&["https://new:8008".to_string()]).unwrap();

    let contents = std::fs::read_to_string(dir.path().join("servers")).unwrap();
    assert!(!contents.contains("https://old:8008"));
    assert!(contents.contains("https://new:8008"));

    unsafe { std::env::remove_var("MC_HOME") };
}

#[test]
fn test_load_server_list_reads_mc_servers_env() {
    let _guard = ENV.lock().unwrap_or_else(|p| p.into_inner());
    unsafe { std::env::set_var("MC_SERVERS", "https://a:8008,https://b:8008") };
    let servers = load_server_list();
    assert_eq!(servers, vec!["https://a:8008", "https://b:8008"]);
    unsafe { std::env::remove_var("MC_SERVERS") };
}

#[test]
fn test_load_server_list_reads_servers_file() {
    let _guard = ENV.lock().unwrap_or_else(|p| p.into_inner());
    let dir = tempdir().unwrap();
    unsafe {
        std::env::remove_var("MC_SERVERS");
        std::env::set_var("MC_HOME", dir.path().to_str().unwrap());
    }

    write_servers_file(&["https://from-file:8008".to_string()]).unwrap();
    let servers = load_server_list();
    assert_eq!(servers, vec!["https://from-file:8008"]);

    unsafe { std::env::remove_var("MC_HOME") };
}

#[test]
fn test_load_server_list_falls_back_to_mc_base_url() {
    let _guard = ENV.lock().unwrap_or_else(|p| p.into_inner());
    let dir = tempdir().unwrap();
    unsafe {
        std::env::set_var("MC_HOME", dir.path().to_str().unwrap());
        std::env::remove_var("MC_SERVERS");
        std::env::set_var("MC_BASE_URL", "https://legacy:8000");
    }

    let servers = load_server_list();
    assert_eq!(servers, vec!["https://legacy:8000"]);

    unsafe {
        std::env::remove_var("MC_HOME");
        std::env::remove_var("MC_BASE_URL");
    }
}

#[tokio::test]
async fn test_probe_servers_returns_only_live() {
    let candidates = vec![
        "http://127.0.0.1:1".to_string(),
        "http://127.0.0.1:2".to_string(),
    ];
    let live = mc::discover::probe_servers(&candidates).await;
    assert!(live.is_empty());
}
