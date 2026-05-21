// Copyright (c) 2026, Salesforce, Inc. All rights reserved.
// SPDX-License-Identifier: Apache-2.0 OR MIT

//! Daemon main loop: spawns `hyperd`, runs health listener, monitors idle timeout.

use std::sync::Arc;
use std::time::Duration;

use tokio::signal;
use tracing::info;

use hyperdb_api::{HyperProcess, Parameters, TransportMode};

use super::discovery::{self, DaemonInfo};
use super::health::{DaemonState, HealthListener};
use super::{DEFAULT_IDLE_TIMEOUT_SECS, ENV_IDLE_TIMEOUT};

/// Configuration for the daemon process.
#[derive(Debug)]
pub struct DaemonConfig {
    pub port: u16,
    pub idle_timeout: Duration,
}

impl DaemonConfig {
    pub fn from_args(port: u16, idle_timeout_secs: Option<u64>) -> Self {
        let idle_timeout_secs = idle_timeout_secs
            .or_else(|| {
                std::env::var(ENV_IDLE_TIMEOUT)
                    .ok()
                    .and_then(|v| v.parse().ok())
            })
            .unwrap_or(DEFAULT_IDLE_TIMEOUT_SECS);

        Self {
            port,
            idle_timeout: Duration::from_secs(idle_timeout_secs),
        }
    }
}

/// Run the daemon. This function blocks until shutdown is triggered.
///
/// # Errors
/// Returns an error if the health port cannot be bound, `hyperd` fails to start,
/// or the discovery file cannot be written.
pub async fn run_daemon(config: DaemonConfig) -> Result<(), Box<dyn std::error::Error>> {
    // Step 1: Bind health port (single-instance lock)
    let listener = HealthListener::bind(config.port).map_err(|e| {
        if e.kind() == std::io::ErrorKind::AddrInUse {
            format!(
                "Another hyperdb daemon is already running on port {}. \
                 Use `hyperdb-mcp daemon status` to check or `hyperdb-mcp daemon stop` to stop it.",
                config.port
            )
        } else {
            format!("Failed to bind health port {}: {e}", config.port)
        }
    })?;
    let bound_port = listener.port;
    info!(port = bound_port, "daemon health listener bound");

    // Step 2: Spawn HyperProcess with TCP transport (shared across clients)
    let log_dir = discovery::state_dir()?.join("logs");
    std::fs::create_dir_all(&log_dir)?;

    let mut params = Parameters::new();
    params.set("log_file_max_count", "2");
    params.set("log_file_size_limit", "100M");
    params.set("log_dir", log_dir.to_string_lossy().as_ref());
    params.set_transport_mode(TransportMode::Tcp);

    let hyper = HyperProcess::new(None, Some(&params))?;
    let endpoint = hyper
        .endpoint()
        .ok_or("hyperd did not report an endpoint")?
        .to_string();
    info!(endpoint = %endpoint, "hyperd started");

    // Step 3: Write discovery file
    let info = DaemonInfo {
        pid: std::process::id(),
        hyperd_endpoint: endpoint.clone(),
        health_port: bound_port,
        started_at: chrono::Utc::now().to_rfc3339(),
        version: env!("CARGO_PKG_VERSION").to_string(),
    };
    discovery::write_discovery_file(&info)?;
    info!(path = %discovery::discovery_file_path()?.display(), "discovery file written");

    // Step 4: Start health listener in background thread
    let state = Arc::new(DaemonState::new());
    let health_state = Arc::clone(&state);
    let health_info = info.clone();
    let health_handle = std::thread::spawn(move || {
        listener.run(health_state, health_info);
    });

    // Step 5: Monitor idle timeout + OS signals
    let idle_timeout = config.idle_timeout;
    let shutdown_state = Arc::clone(&state);

    tokio::select! {
        () = async {
            loop {
                tokio::time::sleep(Duration::from_secs(10)).await;
                if shutdown_state.idle_duration() >= idle_timeout {
                    info!(
                        idle_secs = idle_timeout.as_secs(),
                        "idle timeout reached, shutting down"
                    );
                    shutdown_state.request_shutdown();
                    break;
                }
                if shutdown_state.should_shutdown() {
                    break;
                }
            }
        } => {}
        () = shutdown_signal() => {
            info!("received shutdown signal");
            state.request_shutdown();
        }
    }

    // Step 6: Graceful shutdown
    info!("shutting down daemon");
    discovery::remove_discovery_file();
    drop(hyper); // closes callback connection → hyperd exits
    let _ = health_handle.join();

    Ok(())
}

async fn shutdown_signal() {
    let ctrl_c = signal::ctrl_c();

    #[cfg(unix)]
    {
        let mut sigterm =
            signal::unix::signal(signal::unix::SignalKind::terminate()).expect("sigterm handler");
        tokio::select! {
            _ = ctrl_c => {}
            _ = sigterm.recv() => {}
        }
    }

    #[cfg(not(unix))]
    {
        ctrl_c.await.ok();
    }
}
