// Copyright (c) 2026, Salesforce, Inc. All rights reserved.
// SPDX-License-Identifier: Apache-2.0 OR MIT

//! TCP health listener for the daemon.
//!
//! The health listener serves two purposes:
//! 1. **Single-instance lock** — binding the port guarantees at most one daemon per user.
//! 2. **Liveness probe + heartbeat** — clients connect and send simple text commands.
//!
//! Protocol (line-based, newline-terminated):
//! - `PING\n` → `PONG\n` (liveness check)
//! - `HEARTBEAT\n` → `OK\n` (resets idle timer)
//! - `STOP\n` → `STOPPING\n` (triggers graceful shutdown)
//! - `STATUS\n` → JSON line with daemon info

use std::io::{BufRead, BufReader, Write};
use std::net::{TcpListener, TcpStream};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::time::Instant;

use tracing::{debug, warn};

use super::discovery::DaemonInfo;

/// Handle to the health listener, used to check binding success and manage lifecycle.
#[derive(Debug)]
pub struct HealthListener {
    listener: TcpListener,
    pub port: u16,
}

/// Shared state between the health listener and the daemon main loop.
#[derive(Debug)]
pub struct DaemonState {
    /// Last time any client sent a heartbeat or query.
    pub last_activity: std::sync::Mutex<Instant>,
    /// Signal to shut down the daemon.
    pub shutdown: AtomicBool,
}

impl Default for DaemonState {
    fn default() -> Self {
        Self::new()
    }
}

impl DaemonState {
    pub fn new() -> Self {
        Self {
            last_activity: std::sync::Mutex::new(Instant::now()),
            shutdown: AtomicBool::new(false),
        }
    }

    /// Record activity (resets idle timer).
    ///
    /// # Panics
    /// Panics if the internal mutex is poisoned.
    pub fn touch(&self) {
        *self.last_activity.lock().expect("mutex poisoned") = Instant::now();
    }

    /// Duration since the last activity.
    ///
    /// # Panics
    /// Panics if the internal mutex is poisoned.
    pub fn idle_duration(&self) -> std::time::Duration {
        self.last_activity.lock().expect("mutex poisoned").elapsed()
    }

    pub fn request_shutdown(&self) {
        self.shutdown.store(true, Ordering::Release);
    }

    pub fn should_shutdown(&self) -> bool {
        self.shutdown.load(Ordering::Acquire)
    }
}

impl HealthListener {
    /// Try to bind the health port.
    ///
    /// # Errors
    /// Returns `Err` if the port is already in use (another daemon is running)
    /// or the bind fails for another reason.
    pub fn bind(port: u16) -> std::io::Result<Self> {
        let addr = std::net::SocketAddr::from(([127, 0, 0, 1], port));
        let listener = TcpListener::bind(addr)?;
        listener.set_nonblocking(true)?;
        let port = listener.local_addr()?.port();
        Ok(Self { listener, port })
    }

    /// Run the health listener loop. Spawns per-connection threads until shutdown.
    /// Consumes `self` because this is intended to be called from a dedicated thread.
    #[expect(
        clippy::needless_pass_by_value,
        reason = "Arc and DaemonInfo are cloned into per-connection threads"
    )]
    pub fn run(self, state: Arc<DaemonState>, info: DaemonInfo) {
        loop {
            if state.should_shutdown() {
                break;
            }

            match self.listener.accept() {
                Ok((stream, _addr)) => {
                    let state = Arc::clone(&state);
                    let info = info.clone();
                    std::thread::spawn(move || {
                        handle_client(stream, &state, &info);
                    });
                }
                Err(ref e) if e.kind() == std::io::ErrorKind::WouldBlock => {
                    std::thread::sleep(std::time::Duration::from_millis(100));
                }
                Err(e) => {
                    warn!(error = %e, "health listener accept error");
                    std::thread::sleep(std::time::Duration::from_millis(500));
                }
            }
        }
        debug!("health listener shut down");
    }
}

#[expect(
    clippy::needless_pass_by_value,
    reason = "TcpStream must be owned for BufReader"
)]
fn handle_client(stream: TcpStream, state: &DaemonState, info: &DaemonInfo) {
    let _ = stream.set_read_timeout(Some(std::time::Duration::from_secs(5)));
    let mut reader = BufReader::new(&stream);
    let mut writer = &stream;
    let mut line = String::new();

    loop {
        line.clear();
        match reader.read_line(&mut line) {
            Ok(0) => break,
            Ok(_) => {
                let cmd = line.trim();
                let response = match cmd {
                    "PING" => "PONG\n".to_string(),
                    "HEARTBEAT" => {
                        state.touch();
                        "OK\n".to_string()
                    }
                    "STOP" => {
                        state.request_shutdown();
                        "STOPPING\n".to_string()
                    }
                    "STATUS" => {
                        let json = serde_json::to_string(info).unwrap_or_default();
                        format!("{json}\n")
                    }
                    _ => "ERR unknown command\n".to_string(),
                };
                if writer.write_all(response.as_bytes()).is_err() {
                    break;
                }
            }
            Err(_) => break,
        }
    }
}

/// Send a command to the daemon's health port and return the response.
///
/// # Errors
/// Returns an error if the connection fails or the response cannot be read.
pub fn send_command(port: u16, command: &str) -> std::io::Result<String> {
    let addr = std::net::SocketAddr::from(([127, 0, 0, 1], port));
    let mut stream = TcpStream::connect_timeout(&addr, std::time::Duration::from_secs(2))?;
    stream.set_read_timeout(Some(std::time::Duration::from_secs(5)))?;

    let msg = format!("{command}\n");
    stream.write_all(msg.as_bytes())?;
    stream.flush()?;

    let mut reader = BufReader::new(&stream);
    let mut response = String::new();
    reader.read_line(&mut response)?;
    Ok(response)
}
