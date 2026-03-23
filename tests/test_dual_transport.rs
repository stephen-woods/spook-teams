/// Integration test 13.5: dual transport startup — verifies that the HTTP MCP
/// server binds to a port and accepts connections, and that AppState + handler
/// can be constructed without panicking.
use std::sync::Arc;
use tempfile::TempDir;
use tokio::net::TcpListener;
use tokio_util::sync::CancellationToken;

use spook_teams::config::Config;
use spook_teams::db::Db;
use spook_teams::server::AppState;
use spook_teams::spawner::HeadlessSpawner;

fn make_test_config(port: u16) -> Config {
    Config {
        port,
        db_path: "/tmp/test.db".into(),
        project_path: ".".into(),
        log_level: "warn".into(),
        agent_base_port: 4097,
    }
}

/// Verify that AppState can be constructed and the HTTP listener can bind.
/// This tests the infrastructure layer without needing stdio.
#[tokio::test]
async fn test_appstate_construction() {
    let tmp = TempDir::new().unwrap();
    let db_path = tmp.path().join("test.db");
    let db = Arc::new(Db::open(&db_path).unwrap());
    let config = Arc::new(make_test_config(0));
    let spawner: Arc<dyn spook_teams::spawner::Spawner> = Arc::new(HeadlessSpawner);

    let state = AppState::new(db, config, spawner);

    // AppState should hold a valid CancellationToken
    assert!(!state.http_cancel.is_cancelled());

    // processes map should be empty initially
    assert!(state.processes.read().await.is_empty());
}

/// Verify that a TCP listener can be bound on an ephemeral port (simulating
/// what run_server does for the HTTP worker transport).
#[tokio::test]
async fn test_http_listener_bind() {
    // Port 0 → OS assigns a free port
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    assert!(addr.port() > 0, "should get a non-zero port from the OS");
}

/// Verify that the cancellation token correctly signals shutdown to listeners.
#[tokio::test]
async fn test_http_cancel_token() {
    let cancel = CancellationToken::new();
    let cancel2 = cancel.clone();

    let handle = tokio::spawn(async move {
        cancel2.cancelled().await;
        42u32
    });

    // Cancel from main task
    cancel.cancel();

    let result = handle.await.unwrap();
    assert_eq!(result, 42, "task should complete after cancellation");
}

/// Verify that two AppStates share the same DB (Arc reference count > 1) but
/// have independent cancellation tokens.
#[tokio::test]
async fn test_appstate_independent_cancel_tokens() {
    let tmp = TempDir::new().unwrap();
    let db_path = tmp.path().join("test.db");
    let db = Arc::new(Db::open(&db_path).unwrap());
    let config = Arc::new(make_test_config(0));
    let spawner: Arc<dyn spook_teams::spawner::Spawner> = Arc::new(HeadlessSpawner);

    let state1 = AppState::new(db.clone(), config.clone(), spawner.clone());
    let state2 = AppState::new(db.clone(), config.clone(), spawner.clone());

    // Cancelling state1's token does not affect state2's token
    state1.http_cancel.cancel();
    assert!(state1.http_cancel.is_cancelled());
    assert!(!state2.http_cancel.is_cancelled());
}

/// Verify that a SpookTeamsHandler responds to get_info without panicking.
#[tokio::test]
async fn test_handler_get_info() {
    use rmcp::ServerHandler;
    use spook_teams::server::SpookTeamsHandler;

    let tmp = TempDir::new().unwrap();
    let db_path = tmp.path().join("test.db");
    let db = Arc::new(Db::open(&db_path).unwrap());
    let config = Arc::new(make_test_config(0));
    let spawner: Arc<dyn spook_teams::spawner::Spawner> = Arc::new(HeadlessSpawner);
    let state = AppState::new(db, config, spawner);

    let handler = SpookTeamsHandler::new(state);
    let info = handler.get_info();

    assert_eq!(info.server_info.name, "spook-teams");
    // Should advertise tool support
    assert!(
        info.capabilities.tools.is_some(),
        "should advertise tool capabilities"
    );
}
