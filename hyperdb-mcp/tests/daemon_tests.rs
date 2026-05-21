// Copyright (c) 2026, Salesforce, Inc. All rights reserved.
// SPDX-License-Identifier: Apache-2.0 OR MIT

//! Tests for the single-instance daemon: discovery file, health protocol,
//! idle timeout, and full lifecycle integration with a real `hyperd`.
//!
//! Many tests mutate process-global environment variables (`HYPERDB_STATE_DIR`,
//! `HYPERDB_DAEMON_PORT`) to isolate their state directories. Because env vars
//! are process-global, these tests MUST run sequentially. We enforce this via a
//! shared mutex — every test that touches env vars acquires `ENV_LOCK` first.

use std::net::TcpListener;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use hyperdb_mcp::daemon::discovery::{self, DaemonInfo};
use hyperdb_mcp::daemon::health::{self, DaemonState, HealthListener};
use tempfile::TempDir;

/// Process-wide lock for tests that mutate environment variables.
/// Cargo runs tests in the same process by default — this prevents races.
static ENV_LOCK: Mutex<()> = Mutex::new(());

// ─── Unit tests: DaemonState (no env vars, safe to run in parallel) ───────────

#[test]
fn daemon_state_touch_resets_idle_duration() {
    let state = DaemonState::new();
    std::thread::sleep(Duration::from_millis(50));
    assert!(state.idle_duration() >= Duration::from_millis(50));

    state.touch();
    assert!(state.idle_duration() < Duration::from_millis(30));
}

#[test]
fn daemon_state_shutdown_flag() {
    let state = DaemonState::new();
    assert!(!state.should_shutdown());

    state.request_shutdown();
    assert!(state.should_shutdown());
}

#[test]
fn daemon_state_default_is_equivalent_to_new() {
    let default_state = DaemonState::default();
    assert!(!default_state.should_shutdown());
    assert!(default_state.idle_duration() < Duration::from_millis(100));
}

// ─── Unit tests: Health protocol (no env vars, safe to run in parallel) ───────

#[test]
fn health_listener_bind_succeeds_on_free_port() {
    let listener = HealthListener::bind(0).unwrap();
    assert_ne!(listener.port, 0);
}

#[test]
fn health_listener_second_bind_same_port_fails() {
    let listener = HealthListener::bind(0).unwrap();
    let port = listener.port;

    let result = HealthListener::bind(port);
    assert!(result.is_err());
    assert_eq!(result.unwrap_err().kind(), std::io::ErrorKind::AddrInUse);
}

#[test]
fn health_protocol_ping_pong() {
    let (port, _handle, _state) = start_health_listener();

    let response = health::send_command(port, "PING").unwrap();
    assert_eq!(response.trim(), "PONG");
}

#[test]
fn health_protocol_heartbeat_resets_idle() {
    let (port, _handle, state) = start_health_listener();

    std::thread::sleep(Duration::from_millis(50));
    assert!(state.idle_duration() >= Duration::from_millis(50));

    let response = health::send_command(port, "HEARTBEAT").unwrap();
    assert_eq!(response.trim(), "OK");

    assert!(state.idle_duration() < Duration::from_millis(30));
}

#[test]
fn health_protocol_stop_triggers_shutdown() {
    let (port, handle, state) = start_health_listener();

    assert!(!state.should_shutdown());

    let response = health::send_command(port, "STOP").unwrap();
    assert_eq!(response.trim(), "STOPPING");

    assert!(state.should_shutdown());

    // Health listener should exit its loop
    handle.join().unwrap();
}

#[test]
fn health_protocol_status_returns_json() {
    let (port, _handle, _state) = start_health_listener();

    let response = health::send_command(port, "STATUS").unwrap();
    let parsed: serde_json::Value = serde_json::from_str(response.trim()).unwrap();
    assert_eq!(parsed["pid"], 12345);
    assert_eq!(parsed["hyperd_endpoint"], "127.0.0.1:54321");
}

#[test]
fn health_protocol_unknown_command_returns_error() {
    let (port, _handle, _state) = start_health_listener();

    let response = health::send_command(port, "INVALID").unwrap();
    assert!(response.contains("ERR"));
}

#[test]
fn health_protocol_multi_command_session() {
    let (port, _handle, _state) = start_health_listener();

    let response1 = health::send_command(port, "PING").unwrap();
    assert_eq!(response1.trim(), "PONG");

    let response2 = health::send_command(port, "STATUS").unwrap();
    let parsed: serde_json::Value = serde_json::from_str(response2.trim()).unwrap();
    assert_eq!(parsed["health_port"], port);

    let response3 = health::send_command(port, "HEARTBEAT").unwrap();
    assert_eq!(response3.trim(), "OK");
}

// ─── Unit tests: idle timeout logic (no env vars) ─────────────────────────────

#[test]
fn daemon_idle_timeout_shuts_down_daemon() {
    let state = Arc::new(DaemonState::new());
    let idle_timeout = Duration::from_secs(2);

    let monitor_state = Arc::clone(&state);
    let monitor = std::thread::spawn(move || loop {
        std::thread::sleep(Duration::from_millis(100));
        if monitor_state.idle_duration() >= idle_timeout {
            monitor_state.request_shutdown();
            break;
        }
        if monitor_state.should_shutdown() {
            break;
        }
    });

    let start = Instant::now();
    monitor.join().unwrap();
    let elapsed = start.elapsed();

    assert!(state.should_shutdown());
    assert!(elapsed >= Duration::from_secs(2));
    assert!(elapsed < Duration::from_secs(4));
}

#[test]
fn daemon_heartbeat_prevents_idle_shutdown() {
    let state = Arc::new(DaemonState::new());
    let idle_timeout = Duration::from_secs(1);

    let monitor_state = Arc::clone(&state);
    let heartbeat_state = Arc::clone(&state);

    let heartbeat = std::thread::spawn(move || {
        let start = Instant::now();
        while start.elapsed() < Duration::from_millis(1500) {
            heartbeat_state.touch();
            std::thread::sleep(Duration::from_millis(200));
        }
    });

    let monitor = std::thread::spawn(move || loop {
        std::thread::sleep(Duration::from_millis(100));
        if monitor_state.idle_duration() >= idle_timeout {
            monitor_state.request_shutdown();
            break;
        }
        if monitor_state.should_shutdown() {
            break;
        }
    });

    heartbeat.join().unwrap();
    let start = Instant::now();
    monitor.join().unwrap();
    let after_heartbeat_stop = start.elapsed();

    assert!(state.should_shutdown());
    assert!(
        after_heartbeat_stop >= Duration::from_millis(800),
        "daemon should have waited for idle timeout after heartbeats stopped"
    );
}

// ─── Unit tests: Discovery file (require ENV_LOCK) ────────────────────────────

#[test]
fn discovery_file_write_and_read() {
    let _lock = ENV_LOCK.lock().unwrap();
    let tmp = TempDir::new().unwrap();
    let _guard = EnvGuard::set("HYPERDB_STATE_DIR", tmp.path().to_str().unwrap());

    let info = DaemonInfo {
        pid: 12345,
        hyperd_endpoint: "127.0.0.1:54321".to_string(),
        health_port: 7484,
        started_at: "2026-05-20T10:30:00Z".to_string(),
        version: "0.1.3".to_string(),
    };

    discovery::write_discovery_file(&info).unwrap();

    let path = tmp.path().join("daemon.json");
    assert!(path.exists());

    let contents = std::fs::read_to_string(&path).unwrap();
    let read_back: DaemonInfo = serde_json::from_str(&contents).unwrap();
    assert_eq!(read_back.pid, 12345);
    assert_eq!(read_back.hyperd_endpoint, "127.0.0.1:54321");
    assert_eq!(read_back.health_port, 7484);
    assert_eq!(read_back.version, "0.1.3");
}

#[test]
fn discovery_file_overwrite_replaces_content() {
    let _lock = ENV_LOCK.lock().unwrap();
    let tmp = TempDir::new().unwrap();
    let _guard = EnvGuard::set("HYPERDB_STATE_DIR", tmp.path().to_str().unwrap());

    let info1 = DaemonInfo {
        pid: 100,
        hyperd_endpoint: "127.0.0.1:1111".to_string(),
        health_port: 7484,
        started_at: "2026-01-01T00:00:00Z".to_string(),
        version: "0.1.0".to_string(),
    };
    discovery::write_discovery_file(&info1).unwrap();

    let info2 = DaemonInfo {
        pid: 200,
        hyperd_endpoint: "127.0.0.1:2222".to_string(),
        health_port: 7485,
        started_at: "2026-02-02T00:00:00Z".to_string(),
        version: "0.2.0".to_string(),
    };
    discovery::write_discovery_file(&info2).unwrap();

    let path = tmp.path().join("daemon.json");
    let contents = std::fs::read_to_string(&path).unwrap();
    let read_back: DaemonInfo = serde_json::from_str(&contents).unwrap();
    assert_eq!(read_back.pid, 200);
    assert_eq!(read_back.hyperd_endpoint, "127.0.0.1:2222");
}

#[test]
fn remove_discovery_file_deletes_it() {
    let _lock = ENV_LOCK.lock().unwrap();
    let tmp = TempDir::new().unwrap();
    let _guard = EnvGuard::set("HYPERDB_STATE_DIR", tmp.path().to_str().unwrap());

    let info = DaemonInfo {
        pid: 1,
        hyperd_endpoint: "127.0.0.1:1".to_string(),
        health_port: 7484,
        started_at: "2026-01-01T00:00:00Z".to_string(),
        version: "0.0.1".to_string(),
    };
    discovery::write_discovery_file(&info).unwrap();
    let path = tmp.path().join("daemon.json");
    assert!(path.exists());

    discovery::remove_discovery_file();
    assert!(!path.exists());
}

#[test]
fn discover_returns_none_when_no_file_exists() {
    let _lock = ENV_LOCK.lock().unwrap();
    let tmp = TempDir::new().unwrap();
    let _guard = EnvGuard::set("HYPERDB_STATE_DIR", tmp.path().to_str().unwrap());

    assert!(discovery::discover().is_none());
}

#[test]
fn discover_returns_none_for_stale_file() {
    let _lock = ENV_LOCK.lock().unwrap();
    let tmp = TempDir::new().unwrap();
    let _guard = EnvGuard::set("HYPERDB_STATE_DIR", tmp.path().to_str().unwrap());

    let info = DaemonInfo {
        pid: 99999,
        hyperd_endpoint: "127.0.0.1:1".to_string(),
        health_port: 1,
        started_at: "2026-01-01T00:00:00Z".to_string(),
        version: "0.0.1".to_string(),
    };
    discovery::write_discovery_file(&info).unwrap();

    assert!(discovery::discover().is_none());

    let path = tmp.path().join("daemon.json");
    assert!(!path.exists());
}

#[test]
fn resolve_port_uses_env_var() {
    let _lock = ENV_LOCK.lock().unwrap();
    let _guard = EnvGuard::set("HYPERDB_DAEMON_PORT", "9999");
    assert_eq!(discovery::resolve_port(), 9999);
}

#[test]
fn resolve_port_uses_default_when_env_unset() {
    let _lock = ENV_LOCK.lock().unwrap();
    let _guard = EnvGuard::remove("HYPERDB_DAEMON_PORT");
    assert_eq!(
        discovery::resolve_port(),
        hyperdb_mcp::daemon::DEFAULT_DAEMON_PORT
    );
}

#[test]
fn discover_finds_live_daemon() {
    let _lock = ENV_LOCK.lock().unwrap();
    let tmp = TempDir::new().unwrap();
    let _guard = EnvGuard::set("HYPERDB_STATE_DIR", tmp.path().to_str().unwrap());

    let (port, _handle, _state) = start_health_listener();

    let info = DaemonInfo {
        pid: 12345,
        hyperd_endpoint: "127.0.0.1:54321".to_string(),
        health_port: port,
        started_at: "2026-05-20T10:30:00Z".to_string(),
        version: "0.1.3".to_string(),
    };
    discovery::write_discovery_file(&info).unwrap();

    let discovered = discovery::discover().expect("should discover live daemon");
    assert_eq!(discovered.pid, 12345);
    assert_eq!(discovered.health_port, port);
}

// ─── Integration tests: full daemon lifecycle with real hyperd ─────────────────

#[test]
fn daemon_mode_engine_connects_to_shared_hyperd() {
    let _lock = ENV_LOCK.lock().unwrap();
    let daemon = TestDaemon::start();

    let tmp = TempDir::new().unwrap();
    let workspace_path = tmp.path().join("test.hyper");

    let engine =
        hyperdb_mcp::engine::Engine::new(Some(workspace_path.to_str().unwrap().to_string()))
            .expect("engine should connect to daemon");

    assert!(engine.is_running());

    let endpoint = engine.hyperd_endpoint().unwrap();
    assert_eq!(endpoint, daemon.info.hyperd_endpoint);
}

#[test]
fn daemon_mode_two_engines_share_same_hyperd() {
    let _lock = ENV_LOCK.lock().unwrap();
    let _daemon = TestDaemon::start();

    let tmp1 = TempDir::new().unwrap();
    let tmp2 = TempDir::new().unwrap();
    let path1 = tmp1.path().join("db1.hyper");
    let path2 = tmp2.path().join("db2.hyper");

    let engine1 =
        hyperdb_mcp::engine::Engine::new(Some(path1.to_str().unwrap().to_string())).unwrap();

    let engine2 =
        hyperdb_mcp::engine::Engine::new(Some(path2.to_str().unwrap().to_string())).unwrap();

    assert_eq!(
        engine1.hyperd_endpoint().unwrap(),
        engine2.hyperd_endpoint().unwrap()
    );

    engine1.execute_command("CREATE TABLE foo (x INT)").unwrap();
    engine1
        .execute_command("INSERT INTO foo VALUES (42)")
        .unwrap();

    let tables = engine2.describe_tables().unwrap();
    assert!(
        tables.iter().all(|t| t["name"] != "foo"),
        "engine2 should not see engine1's table"
    );
}

#[test]
fn daemon_mode_persistent_database_file_survives_engine_drop() {
    let _lock = ENV_LOCK.lock().unwrap();
    let _daemon = TestDaemon::start();
    let tmp = TempDir::new().unwrap();
    let path = tmp.path().join("persistent.hyper");
    let path_str = path.to_str().unwrap().to_string();

    {
        let engine = hyperdb_mcp::engine::Engine::new(Some(path_str.clone())).unwrap();
        engine
            .execute_command("CREATE TABLE survive (val TEXT)")
            .unwrap();
        engine
            .execute_command("INSERT INTO survive VALUES ('hello')")
            .unwrap();
    }

    assert!(
        path.exists(),
        "persistent .hyper file should survive engine drop"
    );
}

#[test]
fn daemon_mode_persistent_engine_data_is_queryable() {
    let _lock = ENV_LOCK.lock().unwrap();
    let daemon = TestDaemon::start();
    let tmp = TempDir::new().unwrap();
    let path = tmp.path().join("queryable.hyper");
    let path_str = path.to_str().unwrap().to_string();

    let engine = hyperdb_mcp::engine::Engine::new(Some(path_str)).unwrap();
    engine
        .execute_command("CREATE TABLE items (id INT, name TEXT)")
        .unwrap();
    engine
        .execute_command("INSERT INTO items VALUES (1, 'alpha'), (2, 'beta')")
        .unwrap();

    let rows = engine
        .execute_query_to_json("SELECT * FROM items ORDER BY id")
        .unwrap();
    assert_eq!(rows.len(), 2);
    assert_eq!(rows[0]["name"], "alpha");
    assert_eq!(rows[1]["name"], "beta");

    let resp = health::send_command(daemon.info.health_port, "PING").unwrap();
    assert_eq!(resp.trim(), "PONG");
}

#[test]
fn daemon_mode_ephemeral_database_cleaned_up_on_drop() {
    let _lock = ENV_LOCK.lock().unwrap();
    let _daemon = TestDaemon::start();

    let engine = hyperdb_mcp::engine::Engine::new(None).unwrap();
    let workspace_path = engine.workspace_path().to_path_buf();

    assert!(workspace_path.exists());

    engine
        .execute_command("CREATE TABLE ephemeral_test (id INT)")
        .unwrap();

    drop(engine);

    assert!(
        !workspace_path.exists(),
        "ephemeral .hyper file should be deleted after engine drop"
    );
}

// ─── Test helpers ─────────────────────────────────────────────────────────────

/// Starts a health listener on a random port and returns the port, join handle,
/// and shared state. Does NOT touch env vars — safe for parallel use.
fn start_health_listener() -> (u16, std::thread::JoinHandle<()>, Arc<DaemonState>) {
    let listener = HealthListener::bind(0).unwrap();
    let port = listener.port;
    let state = Arc::new(DaemonState::new());
    let run_state = Arc::clone(&state);

    let info = DaemonInfo {
        pid: 12345,
        hyperd_endpoint: "127.0.0.1:54321".to_string(),
        health_port: port,
        started_at: "2026-05-20T10:30:00Z".to_string(),
        version: "0.1.3".to_string(),
    };

    let handle = std::thread::spawn(move || {
        listener.run(run_state, info);
    });

    // Give the listener a moment to start accepting
    std::thread::sleep(Duration::from_millis(50));

    (port, handle, state)
}

/// A real daemon running in a background thread for integration tests.
/// Sets `HYPERDB_STATE_DIR` and `HYPERDB_DAEMON_PORT` to isolated values.
/// Caller MUST hold `ENV_LOCK` before calling `start()`.
struct TestDaemon {
    info: DaemonInfo,
    _state_dir_guard: EnvGuard,
    _port_guard: EnvGuard,
}

impl TestDaemon {
    fn start() -> Self {
        let tmp = TempDir::new().unwrap();
        // Leak the TempDir so it persists for the lifetime of the test.
        let tmp = Box::leak(Box::new(tmp));

        let state_dir_guard = EnvGuard::set("HYPERDB_STATE_DIR", tmp.path().to_str().unwrap());

        let port = find_free_port();
        let port_guard = EnvGuard::set("HYPERDB_DAEMON_PORT", &port.to_string());

        // Start the daemon in a background tokio runtime
        let daemon_port = port;
        std::thread::spawn(move || {
            let rt = tokio::runtime::Runtime::new().unwrap();
            rt.block_on(async {
                let config = hyperdb_mcp::daemon::run::DaemonConfig {
                    port: daemon_port,
                    idle_timeout: Duration::from_secs(300),
                };
                let _ = hyperdb_mcp::daemon::run::run_daemon(config).await;
            });
        });

        // Wait for daemon to become ready
        let start = Instant::now();
        loop {
            if let Some(info) = discovery::discover() {
                return Self {
                    info,
                    _state_dir_guard: state_dir_guard,
                    _port_guard: port_guard,
                };
            }
            assert!(
                start.elapsed() <= Duration::from_secs(15),
                "TestDaemon did not start within 15 seconds"
            );
            std::thread::sleep(Duration::from_millis(100));
        }
    }
}

impl Drop for TestDaemon {
    fn drop(&mut self) {
        let _ = health::send_command(self.info.health_port, "STOP");
        std::thread::sleep(Duration::from_millis(200));
    }
}

/// Find a free TCP port by binding to port 0 and reading the assigned port.
fn find_free_port() -> u16 {
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    listener.local_addr().unwrap().port()
}

/// RAII guard that sets/removes an environment variable and restores it on drop.
struct EnvGuard {
    key: String,
    previous: Option<String>,
}

impl EnvGuard {
    fn set(key: &str, value: &str) -> Self {
        let previous = std::env::var(key).ok();
        // SAFETY: Callers hold ENV_LOCK, ensuring no concurrent env var access.
        unsafe { std::env::set_var(key, value) };
        Self {
            key: key.to_string(),
            previous,
        }
    }

    fn remove(key: &str) -> Self {
        let previous = std::env::var(key).ok();
        // SAFETY: Callers hold ENV_LOCK, ensuring no concurrent env var access.
        unsafe { std::env::remove_var(key) };
        Self {
            key: key.to_string(),
            previous,
        }
    }
}

impl Drop for EnvGuard {
    fn drop(&mut self) {
        match &self.previous {
            // SAFETY: Callers hold ENV_LOCK for the lifetime of this guard.
            Some(val) => unsafe { std::env::set_var(&self.key, val) },
            // SAFETY: Callers hold ENV_LOCK for the lifetime of this guard.
            None => unsafe { std::env::remove_var(&self.key) },
        }
    }
}
