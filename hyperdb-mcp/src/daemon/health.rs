// Copyright (c) 2026, Salesforce, Inc. All rights reserved.
// SPDX-License-Identifier: Apache-2.0 OR MIT

//! TCP health listener for the daemon.
//!
//! The health listener serves two purposes:
//! 1. **Single-instance lock** — binding the port guarantees at most one daemon per user.
//! 2. **Liveness probe + heartbeat** — clients connect and send simple text commands.
//!
//! Protocol (line-based, newline-terminated):
//! - `PING\n` → `PONG hyperdb-mcp <version>\n` (liveness check; the identifying
//!   token proves it's a hyperdb-mcp daemon, not a foreign process on the same port)
//! - `HEARTBEAT\n` → `OK\n` (resets idle timer)
//! - `STOP\n` → `STOPPING\n` (triggers graceful shutdown)
//! - `STATUS\n` → JSON line with daemon info (reports the *current* hyperd
//!   endpoint, which can change after a restart).
//! - `REPORT_HYPERD_ERROR\n` → `OK\n` (sets the restart-requested flag —
//!   the monitor task picks it up on its next tick).

use std::io::{BufRead, BufReader, Read, Write};
use std::net::{TcpListener, TcpStream};
use std::sync::atomic::{AtomicBool, AtomicU16, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use tracing::{debug, warn};

use super::discovery::{DaemonInfo, DaemonRecord};

/// Identifying token included in PONG responses. Used to verify that a bound
/// port is owned by a hyperdb-mcp daemon (not a foreign service).
pub const PONG_TOKEN: &str = "hyperdb-mcp";

/// Bound on the throwaway loopback connect that wakes a parked `accept()`.
/// Generous only as a hang guard — the kernel completes a loopback connection
/// into the listen backlog without waiting for `accept()`, so in practice this
/// returns in microseconds or fails fast with `ECONNREFUSED`.
const WAKE_CONNECT_TIMEOUT: Duration = Duration::from_millis(250);

/// Construct the PONG response with the identifying token and version.
fn pong_response() -> String {
    format!("PONG {PONG_TOKEN} {}\n", crate::version::MCP_VERSION)
}

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
    pub last_activity: Mutex<Instant>,
    /// Signal to shut down the daemon.
    pub shutdown: AtomicBool,
    /// Set by clients reporting that hyperd looks dead from over there;
    /// consumed by the daemon's restart monitor.
    pub restart_requested: AtomicBool,
    /// Loopback port to connect to in order to wake a [`HealthListener::run`]
    /// loop parked in a blocking `accept()`. `0` is an unambiguous
    /// "no listener registered" sentinel: [`HealthListener::bind`] resolves an
    /// ephemeral `bind(0)` through `local_addr()`, so a bound listener's port
    /// is never `0`.
    wake_port: AtomicU16,
}

impl Default for DaemonState {
    fn default() -> Self {
        Self::new()
    }
}

impl DaemonState {
    pub fn new() -> Self {
        Self {
            last_activity: Mutex::new(Instant::now()),
            shutdown: AtomicBool::new(false),
            restart_requested: AtomicBool::new(false),
            wake_port: AtomicU16::new(0),
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
    pub fn idle_duration(&self) -> Duration {
        self.last_activity.lock().expect("mutex poisoned").elapsed()
    }

    /// Request shutdown and wake a health listener parked in `accept()`.
    ///
    /// Every caller in the tree follows this with a `join()` on the listener
    /// thread (`run_daemon`'s step 7, and the test harnesses), so the wake is
    /// not an optimization — without it a blocking `accept()` would never
    /// return and the join would hang.
    pub fn request_shutdown(&self) {
        self.shutdown.store(true, Ordering::Release);
        self.wake_accept_loop();
    }

    /// Nudge the listening socket so a blocked `accept()` returns and the loop
    /// re-reads `should_shutdown`.
    ///
    /// The connection is completed by the kernel into the listen backlog, so
    /// this does not itself wait on `accept()` and cannot deadlock against the
    /// very loop it is waking. Every failure mode is benign and ignored: no
    /// listener registered yet (the loop's pre-`accept` `should_shutdown`
    /// check catches that), the listener already gone (connection refused), or
    /// a full backlog — in which case the loop still wakes on one of the
    /// queued connections and observes the flag. The wake makes shutdown
    /// prompt; correctness rests on the flag check after `accept()` returns.
    fn wake_accept_loop(&self) {
        let port = self.wake_port.load(Ordering::Acquire);
        if port == 0 {
            return;
        }
        let addr = std::net::SocketAddr::from(([127, 0, 0, 1], port));
        let _ = TcpStream::connect_timeout(&addr, WAKE_CONNECT_TIMEOUT);
    }

    /// Point [`Self::wake_accept_loop`] at a live listening port. Called by
    /// [`HealthListener::run`] before it parks in `accept()`.
    fn register_wake_port(&self, port: u16) {
        self.wake_port.store(port, Ordering::Release);
    }

    /// Forget the wake port once the listener has stopped, so a later
    /// `request_shutdown` does not connect to whatever has since taken it.
    fn clear_wake_port(&self) {
        self.wake_port.store(0, Ordering::Release);
    }

    pub fn should_shutdown(&self) -> bool {
        self.shutdown.load(Ordering::Acquire)
    }

    /// Signal that hyperd appears to have died and a restart is needed.
    pub fn request_restart(&self) {
        self.restart_requested.store(true, Ordering::Release);
    }

    /// Atomically read-and-clear the restart-request flag.
    /// Returns true if a restart was requested since the last call.
    pub fn consume_restart_request(&self) -> bool {
        self.restart_requested.swap(false, Ordering::AcqRel)
    }
}

impl HealthListener {
    /// Try to bind the health port.
    ///
    /// The listener is left in blocking mode: [`Self::run`] parks in
    /// `accept()` and is woken by [`DaemonState::request_shutdown`] rather
    /// than by polling. Passing `0` binds an ephemeral port, which
    /// `local_addr()` below resolves into [`Self::port`], so a bound
    /// listener's port is never `0`.
    ///
    /// # Errors
    /// Returns `Err` if the port is already in use (another daemon is running)
    /// or the bind fails for another reason.
    pub fn bind(port: u16) -> std::io::Result<Self> {
        let addr = std::net::SocketAddr::from(([127, 0, 0, 1], port));
        let listener = TcpListener::bind(addr)?;
        let port = listener.local_addr()?.port();
        Ok(Self { listener, port })
    }

    /// Run the health listener loop. Spawns per-connection threads until shutdown.
    /// Consumes `self` because this is intended to be called from a dedicated thread.
    ///
    /// `info` is shared (`Arc<Mutex<DaemonInfo>>`) so the listener reports the
    /// *current* hyperd endpoint after a restart — the monitor task updates the
    /// same Arc once a new hyperd is running.
    ///
    /// The loop blocks in `accept()` rather than polling a non-blocking socket
    /// on a timer. It is woken by [`DaemonState::request_shutdown`], which
    /// makes a throwaway loopback connection to this listener; the loop
    /// re-checks `should_shutdown` after every `accept()` and drops the wake
    /// connection unread. That leaves an idle daemon costing zero wakeups
    /// while also giving a real client zero accept latency — the previous
    /// 5 ms poll paid ~174 wakeups/second to make its own accept cost ~5 ms
    /// (see issue #274 for the measurements).
    #[expect(
        clippy::needless_pass_by_value,
        reason = "Arcs are cloned into per-connection threads"
    )]
    pub fn run(self, state: Arc<DaemonState>, info: Arc<Mutex<DaemonInfo>>) {
        // Register before the first `should_shutdown` check, so a shutdown
        // racing this setup either finds the port (and wakes us) or is seen by
        // the check below before we ever park.
        state.register_wake_port(self.port);

        loop {
            if state.should_shutdown() {
                break;
            }

            match accept_and_force_blocking(&self.listener) {
                Ok(AcceptedConnection::Ready(stream)) => {
                    // The shutdown wake arrives as an ordinary connection.
                    // Re-check before spending a thread on it; a genuine
                    // client arriving in the same instant is dropped, exactly
                    // as the pre-`accept` check has always dropped it.
                    if state.should_shutdown() {
                        break;
                    }
                    let state = Arc::clone(&state);
                    let info = Arc::clone(&info);
                    std::thread::spawn(move || {
                        handle_client(stream, &state, &info);
                    });
                }
                Ok(AcceptedConnection::ForceBlockingFailed(error)) => {
                    warn!(
                        error = %error,
                        "could not make accepted health connection blocking"
                    );
                }
                Err(ref error) if error.kind() == std::io::ErrorKind::Interrupted => {
                    // A parked `accept()` is a prime EINTR target. Not an
                    // error: loop round, re-check `should_shutdown`, re-park.
                }
                Err(ref error) if error.kind() == std::io::ErrorKind::WouldBlock => {
                    // Unreachable while `bind` leaves the listener blocking.
                    // Kept as a backstop that cannot spin if some future
                    // caller reintroduces a non-blocking listening socket.
                    std::thread::sleep(Duration::from_millis(5));
                }
                Err(error) => {
                    warn!(error = %error, "health listener accept error");
                    std::thread::sleep(Duration::from_millis(500));
                }
            }
        }
        state.clear_wake_port();
        debug!("health listener shut down");
    }
}

/// Outcome of [`accept_and_force_blocking`] once a connection has actually
/// been accepted (as opposed to `accept()` itself erroring, which is
/// propagated through the outer `io::Result`).
enum AcceptedConnection {
    /// Accepted and confirmed blocking; ready to hand to [`handle_client`].
    Ready(TcpStream),
    /// Accepted, but the attempt to clear the non-blocking flag failed. The
    /// connection is dropped; the caller logs and moves on to the next
    /// `accept()`.
    ForceBlockingFailed(std::io::Error),
}

/// Accept one connection and force it into blocking mode.
///
/// On BSD-derived kernels — macOS and other BSDs, but **not** Linux, which
/// keeps a newly accepted socket's blocking mode independent of the
/// listener's — `accept()` propagates the listening socket's `O_NONBLOCK`
/// flag to the accepted socket. Left non-blocking, the accepted stream would
/// return `WouldBlock` from `read_line` in [`handle_client`] in microseconds —
/// typically before the client has even written its first byte — tearing the
/// connection down before it ever received a command.
///
/// [`HealthListener::bind`] now leaves the listening socket *blocking*, so
/// there is no `O_NONBLOCK` to inherit and this `set_nonblocking(false)` is a
/// no-op on every platform today. It is kept deliberately, as the invariant
/// [`handle_client`] actually depends on — that the stream it receives is
/// blocking — rather than as a consequence of how the listener happens to be
/// configured. `accept_and_force_blocking_clears_nonblocking_flag` pins it by
/// making the listener non-blocking on purpose, so the guarantee is still
/// tested under the condition that motivated it.
fn accept_and_force_blocking(listener: &TcpListener) -> std::io::Result<AcceptedConnection> {
    let (stream, _addr) = listener.accept()?;
    match stream.set_nonblocking(false) {
        Ok(()) => Ok(AcceptedConnection::Ready(stream)),
        Err(error) => Ok(AcceptedConnection::ForceBlockingFailed(error)),
    }
}

fn status_json(info: &Mutex<DaemonInfo>) -> String {
    let snapshot = info.lock().expect("DaemonInfo mutex poisoned").clone();
    match DaemonRecord::with_current_identity(&snapshot) {
        Ok(record) => serde_json::to_string(&record).unwrap_or_default(),
        Err(error) => {
            warn!(%error, "could not collect daemon executable identity for STATUS");
            serde_json::to_string(&snapshot).unwrap_or_default()
        }
    }
}

#[expect(
    clippy::needless_pass_by_value,
    reason = "TcpStream must be owned for BufReader"
)]
fn handle_client(stream: TcpStream, state: &DaemonState, info: &Mutex<DaemonInfo>) {
    let _ = stream.set_read_timeout(Some(Duration::from_secs(5)));
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
                    "PING" => pong_response(),
                    "HEARTBEAT" => {
                        state.touch();
                        "OK\n".to_string()
                    }
                    "STOP" => {
                        state.request_shutdown();
                        "STOPPING\n".to_string()
                    }
                    "STATUS" => format!("{}\n", status_json(info)),
                    "REPORT_HYPERD_ERROR" => {
                        state.request_restart();
                        "OK\n".to_string()
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
/// Uses generous timeouts (2s connect, 5s read) suitable for `STOP`/`STATUS`
/// where the caller is willing to wait. Use [`send_command_with_timeout`] for
/// best-effort fire-and-forget calls (e.g. heartbeat, error reporting).
///
/// # Errors
/// Returns an error if the connection fails or the response cannot be read.
pub fn send_command(port: u16, command: &str) -> std::io::Result<String> {
    send_command_with_timeout(
        port,
        command,
        Duration::from_secs(2),
        Duration::from_secs(5),
    )
}

/// Best-effort fire-and-forget: tell the running daemon that hyperd appears to
/// be dead from this client's perspective. Uses short timeouts (200ms each) so
/// the calling tool handler isn't stalled if the daemon itself is slow.
/// Errors are logged at debug level and otherwise ignored.
pub fn report_hyperd_error_to_daemon(health_port: u16) {
    let timeout = Duration::from_millis(200);
    match send_command_with_timeout(health_port, "REPORT_HYPERD_ERROR", timeout, timeout) {
        Ok(response) => {
            debug!(response = %response.trim(), "reported hyperd error to daemon");
        }
        Err(e) => {
            debug!(error = %e, "could not report hyperd error to daemon (best-effort)");
        }
    }
}

/// Send a command with caller-specified connect and I/O timeouts.
///
/// The supplied `read_timeout` also bounds writes so every phase of the
/// request is finite without changing this helper's public signature.
///
/// # Errors
/// Returns an error if the connection fails or the response cannot be read
/// within the supplied timeouts.
pub fn send_command_with_timeout(
    port: u16,
    command: &str,
    connect_timeout: Duration,
    read_timeout: Duration,
) -> std::io::Result<String> {
    let addr = std::net::SocketAddr::from(([127, 0, 0, 1], port));
    let mut stream = TcpStream::connect_timeout(&addr, connect_timeout)?;
    let io_deadline = Instant::now().checked_add(read_timeout).ok_or_else(|| {
        std::io::Error::new(
            std::io::ErrorKind::InvalidInput,
            "health command I/O timeout overflows deadline",
        )
    })?;

    let msg = format!("{command}\n");
    let mut written = 0;
    while written < msg.len() {
        stream.set_write_timeout(Some(remaining_io_time(io_deadline)?))?;
        match stream.write(&msg.as_bytes()[written..]) {
            Ok(0) => {
                return Err(std::io::Error::new(
                    std::io::ErrorKind::WriteZero,
                    "health command write returned zero bytes",
                ));
            }
            Ok(count) => {
                written += count;
                remaining_io_time(io_deadline)?;
            }
            Err(error) => return Err(normalize_expired_io_error(error, io_deadline)),
        }
    }

    const MAX_HEALTH_RESPONSE_BYTES: usize = 64 * 1024;
    let mut response = Vec::new();
    loop {
        stream.set_read_timeout(Some(remaining_io_time(io_deadline)?))?;
        let mut byte = [0];
        match stream.read(&mut byte) {
            Ok(0) if response.is_empty() => {
                return Err(std::io::Error::new(
                    std::io::ErrorKind::UnexpectedEof,
                    "health command connection closed before any response was sent",
                ));
            }
            Ok(0) => break,
            Ok(_) if response.len() == MAX_HEALTH_RESPONSE_BYTES => {
                return Err(std::io::Error::new(
                    std::io::ErrorKind::InvalidData,
                    "health response exceeds the 64 KiB limit",
                ));
            }
            Ok(_) => {
                response.push(byte[0]);
                remaining_io_time(io_deadline)?;
                if byte[0] == b'\n' {
                    break;
                }
            }
            Err(error) => return Err(normalize_expired_io_error(error, io_deadline)),
        }
    }

    String::from_utf8(response).map_err(|error| {
        std::io::Error::new(
            std::io::ErrorKind::InvalidData,
            format!("health response is not valid UTF-8: {error}"),
        )
    })
}

fn remaining_io_time(io_deadline: Instant) -> std::io::Result<Duration> {
    let remaining = io_deadline.saturating_duration_since(Instant::now());
    if remaining.is_zero() {
        Err(std::io::Error::new(
            std::io::ErrorKind::TimedOut,
            "health command I/O deadline expired",
        ))
    } else {
        Ok(remaining)
    }
}

fn normalize_expired_io_error(error: std::io::Error, io_deadline: Instant) -> std::io::Error {
    if matches!(
        error.kind(),
        std::io::ErrorKind::TimedOut | std::io::ErrorKind::WouldBlock
    ) || Instant::now() >= io_deadline
    {
        std::io::Error::new(
            std::io::ErrorKind::TimedOut,
            "health command I/O deadline expired",
        )
    } else {
        error
    }
}

/// Send PING and verify the response contains the identifying token. Returns
/// `Some(version)` if the responding daemon is a hyperdb-mcp daemon (the version
/// string is the daemon's `MCP_VERSION`), or `None` if connection fails, the
/// response lacks the expected token, or read times out. An empty version string
/// (`Some(String::new())`) is returned if the PONG prefix matches but no version
/// token is present (graceful degradation for forward/backward compat).
///
/// This is the primitive for liveness checks now that bare TCP connect is
/// insufficient (a foreign service on the same port would cause collisions).
pub fn ping_identified(
    port: u16,
    connect_timeout: Duration,
    read_timeout: Duration,
) -> Option<String> {
    let response = send_command_with_timeout(port, "PING", connect_timeout, read_timeout).ok()?;
    // Validate by exact tokens, not a string prefix: a prefix check on
    // "PONG hyperdb-mcp" would also match a foreign reply like
    // "PONG hyperdb-mcpEVIL 1.0.0". Require the first two whitespace-separated
    // tokens to be exactly "PONG" and the token, so only our daemon passes.
    let mut tokens = response.split_whitespace();
    if tokens.next() != Some("PONG") || tokens.next() != Some(PONG_TOKEN) {
        return None;
    }
    // The 3rd token is the daemon's version; absent ⇒ accept with empty
    // version (future-proofing for a token-only reply).
    Some(tokens.next().unwrap_or("").to_string())
}

#[cfg(test)]
mod tests {
    use std::panic::{AssertUnwindSafe, catch_unwind, resume_unwind};
    use std::sync::mpsc::{self, Receiver, Sender};
    use std::thread::JoinHandle;

    use serde_json::{Value, json};

    use crate::diagnostics::ReportedPath;

    use super::*;

    struct TestPeer<T> {
        port: u16,
        stop: Option<Sender<()>>,
        handle: Option<JoinHandle<Result<T, String>>>,
    }

    impl<T> TestPeer<T> {
        fn finish(mut self) -> Result<T, String> {
            if let Some(stop) = self.stop.take() {
                let _ = stop.send(());
            }
            self.handle
                .take()
                .expect("test peer handle must exist")
                .join()
                .map_err(|payload| format!("test peer panicked: {payload:?}"))?
        }
    }

    impl<T> Drop for TestPeer<T> {
        fn drop(&mut self) {
            if let Some(stop) = self.stop.take() {
                let _ = stop.send(());
            }
            if let Some(handle) = self.handle.take() {
                let _ = handle.join();
            }
        }
    }

    fn spawn_test_peer<T, F>(script: F) -> TestPeer<T>
    where
        T: Send + 'static,
        F: FnOnce(TcpStream, Receiver<()>) -> Result<T, String> + Send + 'static,
    {
        let listener = TcpListener::bind(("127.0.0.1", 0)).expect("bind test health peer");
        listener
            .set_nonblocking(true)
            .expect("make test health peer nonblocking");
        let port = listener.local_addr().expect("test peer address").port();
        let (stop_tx, stop_rx) = mpsc::channel();
        let handle = std::thread::spawn(move || {
            let accept_deadline = Instant::now() + Duration::from_secs(2);
            loop {
                if stop_rx.try_recv().is_ok() {
                    return Err("test peer stopped before accepting a connection".to_string());
                }
                match listener.accept() {
                    Ok((stream, _)) => {
                        stream.set_nonblocking(false).map_err(|error| {
                            format!("make accepted test health peer blocking: {error}")
                        })?;
                        return script(stream, stop_rx);
                    }
                    Err(error) if error.kind() == std::io::ErrorKind::WouldBlock => {
                        if Instant::now() >= accept_deadline {
                            return Err("test peer timed out waiting for a connection".to_string());
                        }
                        std::thread::sleep(Duration::from_millis(2));
                    }
                    Err(error) => return Err(format!("test peer accept failed: {error}")),
                }
            }
        });

        TestPeer {
            port,
            stop: Some(stop_tx),
            handle: Some(handle),
        }
    }

    fn read_test_command(stream: &TcpStream) -> Result<String, String> {
        let reader_stream = stream
            .try_clone()
            .map_err(|error| format!("clone test peer stream: {error}"))?;
        reader_stream
            .set_read_timeout(Some(Duration::from_secs(1)))
            .map_err(|error| format!("set test peer read timeout: {error}"))?;
        let mut request = String::new();
        BufReader::new(reader_stream)
            .read_line(&mut request)
            .map_err(|error| format!("read test health command: {error}"))?;
        Ok(request)
    }

    fn daemon_info(health_port: u16) -> DaemonInfo {
        DaemonInfo {
            pid: 4242,
            hyperd_endpoint: "127.0.0.1:54321".to_string(),
            health_port,
            started_at: "2026-08-13T12:34:56Z".to_string(),
            version: "0.7.0".to_string(),
        }
    }

    fn expected_status(info: &DaemonInfo) -> Value {
        let executable = std::env::current_exe().unwrap();
        let executable_path = ReportedPath::from_os_str(executable.as_os_str());

        json!({
            "pid": info.pid,
            "hyperd_endpoint": info.hyperd_endpoint,
            "health_port": info.health_port,
            "started_at": info.started_at,
            "version": info.version,
            "identity": {
                "mcp_version": crate::version::mcp_version_string(),
                "executable_path": executable_path
            }
        })
    }

    fn check_status_json(
        label: &str,
        response: &str,
        expected: &Value,
        failures: &mut Vec<String>,
    ) {
        match serde_json::from_str::<Value>(response.trim()) {
            Ok(actual) => {
                if actual != *expected {
                    failures.push(format!(
                        "{label} was not the exact flat enriched record: {actual}"
                    ));
                }
                if actual.get("info").is_some() {
                    failures.push(format!("{label} nested legacy fields under `info`"));
                }
            }
            Err(error) => failures.push(format!("{label} was not JSON: {error}")),
        }
    }

    #[test]
    fn health_status_returns_flat_enriched_record() {
        let _network_guard = crate::diagnostics::real_network_test_guard();
        let public_run_signature: fn(HealthListener, Arc<DaemonState>, Arc<Mutex<DaemonInfo>>) =
            HealthListener::run;
        std::hint::black_box(public_run_signature);

        let listener = HealthListener::bind(0).unwrap();
        let port = listener.port;
        let state = Arc::new(DaemonState::new());
        let info = Arc::new(Mutex::new(daemon_info(port)));
        let initial_expected = expected_status(&info.lock().unwrap());
        let mut failures = Vec::new();

        match catch_unwind(AssertUnwindSafe(|| status_json(info.as_ref()))) {
            Ok(response) => check_status_json(
                "private STATUS serializer",
                &response,
                &initial_expected,
                &mut failures,
            ),
            Err(_) => failures.push("private STATUS serializer is not implemented".to_string()),
        }

        let run_state = Arc::clone(&state);
        let run_info = Arc::clone(&info);
        let handle = std::thread::spawn(move || listener.run(run_state, run_info));

        let initial_response = send_command(port, "STATUS").unwrap();
        check_status_json(
            "initial STATUS response",
            &initial_response,
            &initial_expected,
            &mut failures,
        );

        {
            let mut current = info.lock().unwrap();
            current.hyperd_endpoint = "127.0.0.1:60000".to_string();
        }
        let updated_expected = expected_status(&info.lock().unwrap());
        let updated_response = send_command(port, "STATUS").unwrap();
        check_status_json(
            "STATUS response after shared DaemonInfo update",
            &updated_response,
            &updated_expected,
            &mut failures,
        );

        let _ = send_command(port, "STOP");
        handle.join().unwrap();

        assert!(
            failures.is_empty(),
            "health STATUS contract failures:\n{}",
            failures.join("\n")
        );
    }

    #[test]
    fn slow_drip_response_honors_absolute_io_deadline() {
        const IO_TIMEOUT: Duration = Duration::from_millis(100);
        const DRIP_INTERVAL: Duration = Duration::from_millis(20);
        const DRIP_BYTES: usize = 40;
        const GENEROUS_COMPLETION_BOUND: Duration = Duration::from_millis(500);

        let peer = spawn_test_peer(|mut stream, stop| {
            let request = read_test_command(&stream)?;
            if request != "PING\n" {
                return Err(format!("unexpected health command: {request:?}"));
            }

            for _ in 0..DRIP_BYTES {
                if stop.try_recv().is_ok() {
                    return Ok(false);
                }
                if stream.write_all(b"x").is_err() {
                    return Ok(false);
                }
                std::thread::sleep(DRIP_INTERVAL);
            }
            Ok(true)
        });

        let call = catch_unwind(AssertUnwindSafe(|| {
            let started = Instant::now();
            let result =
                send_command_with_timeout(peer.port, "PING", Duration::from_secs(1), IO_TIMEOUT);
            (result, started.elapsed())
        }));
        let completed_full_drip = peer
            .finish()
            .expect("slow-drip peer must shut down cleanly");
        let (result, elapsed) = match call {
            Ok(outcome) => outcome,
            Err(payload) => resume_unwind(payload),
        };

        assert_eq!(
            result.as_ref().err().map(std::io::Error::kind),
            Some(std::io::ErrorKind::TimedOut),
            "a peer that makes progress without terminating a line must hit the one I/O deadline; got {result:?} after {elapsed:?}"
        );
        assert!(
            elapsed < GENEROUS_COMPLETION_BOUND,
            "100ms I/O budget was extended to {elapsed:?} by slow-drip progress"
        );
        assert!(
            !completed_full_drip,
            "the client waited for the peer's entire 800ms drip instead of enforcing its absolute deadline"
        );
    }

    #[test]
    fn oversized_newline_free_response_is_rejected() {
        const MAX_HEALTH_RESPONSE_BYTES: usize = 64 * 1024;
        const OVERSIZED_RESPONSE_BYTES: usize = MAX_HEALTH_RESPONSE_BYTES + 1;

        let peer = spawn_test_peer(|mut stream, _stop| {
            let request = read_test_command(&stream)?;
            if request != "PING\n" {
                return Err(format!("unexpected health command: {request:?}"));
            }
            stream
                .set_write_timeout(Some(Duration::from_secs(1)))
                .map_err(|error| format!("set test peer write timeout: {error}"))?;
            let response = vec![b'x'; OVERSIZED_RESPONSE_BYTES];
            let mut emitted = 0;
            while emitted < response.len() {
                match stream.write(&response[emitted..]) {
                    Ok(0) => {
                        return Err(format!(
                            "oversized health peer wrote zero bytes after {emitted} bytes"
                        ));
                    }
                    Ok(written) => emitted += written,
                    Err(error) => {
                        return Err(format!(
                            "oversized health peer stopped after {emitted} bytes: {error}"
                        ));
                    }
                }
            }
            Ok(emitted)
        });

        let call = catch_unwind(AssertUnwindSafe(|| {
            send_command_with_timeout(
                peer.port,
                "PING",
                Duration::from_secs(1),
                Duration::from_secs(1),
            )
        }));
        let response_bytes = peer
            .finish()
            .expect("oversized-response peer must shut down cleanly");
        let result = match call {
            Ok(outcome) => outcome,
            Err(payload) => resume_unwind(payload),
        };
        let outcome = match &result {
            Ok(response) => format!("accepted {} bytes", response.len()),
            Err(error) => format!("returned {:?}: {error}", error.kind()),
        };

        assert_eq!(response_bytes, OVERSIZED_RESPONSE_BYTES);
        assert_eq!(
            result.as_ref().err().map(std::io::Error::kind),
            Some(std::io::ErrorKind::InvalidData),
            "newline-free health responses beyond the 64 KiB protocol limit must be rejected; {outcome}"
        );
    }

    /// Asserts the accepted socket's actual blocking state via `fcntl`,
    /// rather than inferring it from read-timing behavior. On Linux an
    /// accepted socket is blocking regardless of the listener's mode, so a
    /// behavioral test (send late, expect it to still be read) passes
    /// trivially there even with `accept_and_force_blocking`'s
    /// `set_nonblocking(false)` reverted — it would only catch a regression
    /// on the BSD-derived kernels (macOS and other BSDs) that actually
    /// propagate `O_NONBLOCK` to accepted sockets. Reading the `O_NONBLOCK`
    /// flag directly makes the test verify the real contract everywhere,
    /// rather than a platform-dependent behavioral proxy for it.
    ///
    /// The listener is put into non-blocking mode explicitly. `bind` leaves it
    /// blocking now that [`HealthListener::run`] parks in `accept()`, so
    /// without this the accepted socket would be blocking for free and the
    /// assertion would hold no matter what `accept_and_force_blocking` did —
    /// re-opening the verification gap #273 closed.
    #[cfg(unix)]
    #[test]
    fn accept_and_force_blocking_clears_nonblocking_flag() {
        use std::os::unix::io::AsRawFd;

        let listener = HealthListener::bind(0).expect("bind test health listener");
        listener
            .listener
            .set_nonblocking(true)
            .expect("recreate the O_NONBLOCK-propagating listener this test is about");
        let port = listener.port;

        let client = std::thread::spawn(move || {
            // Held for the duration of the accept below; dropped (and thus
            // closed) only once this thread returns.
            std::net::TcpStream::connect(("127.0.0.1", port)).expect("connect test client")
        });

        let accept_deadline = Instant::now() + Duration::from_secs(2);
        let accepted = loop {
            match accept_and_force_blocking(&listener.listener) {
                Ok(AcceptedConnection::Ready(stream)) => break stream,
                Ok(AcceptedConnection::ForceBlockingFailed(error)) => {
                    panic!("force-blocking the accepted test connection failed: {error}")
                }
                Err(ref error) if error.kind() == std::io::ErrorKind::WouldBlock => {
                    assert!(
                        Instant::now() < accept_deadline,
                        "timed out waiting to accept the test client connection"
                    );
                    std::thread::sleep(Duration::from_millis(2));
                }
                Err(error) => panic!("accept failed: {error}"),
            }
        };
        client.join().expect("test client thread must not panic");

        // SAFETY: `accepted` is a live, owned, valid socket for the duration
        // of this call; `F_GETFL` only reads flags and mutates nothing.
        let flags = unsafe { libc::fcntl(accepted.as_raw_fd(), libc::F_GETFL) };
        assert!(
            flags >= 0,
            "fcntl(F_GETFL) on the accepted test connection failed: {}",
            std::io::Error::last_os_error()
        );
        assert_eq!(
            flags & libc::O_NONBLOCK,
            0,
            "accepted health connection must be blocking (O_NONBLOCK must be clear); \
             reverting accept_and_force_blocking's set_nonblocking(false) call would \
             leave O_NONBLOCK set on BSD-derived kernels that propagate it from the \
             listening socket"
        );
    }

    /// A peer that reads the request and then closes without writing anything
    /// must be reported as a failed command, not a successful empty response.
    /// `daemon_stop` in `main.rs` treats `Ok(_)` as "the daemon responded" and
    /// exits 0, so an `Ok("")` here would silently mask a peer that never
    /// answered at all.
    #[test]
    fn peer_close_without_any_response_is_an_error_not_empty_success() {
        let peer = spawn_test_peer(|stream, _stop| {
            let request = read_test_command(&stream)?;
            if request != "PING\n" {
                return Err(format!("unexpected health command: {request:?}"));
            }
            // Close the connection immediately, writing nothing at all.
            drop(stream);
            Ok(())
        });

        let call = catch_unwind(AssertUnwindSafe(|| {
            send_command_with_timeout(
                peer.port,
                "PING",
                Duration::from_secs(1),
                Duration::from_secs(1),
            )
        }));
        peer.finish()
            .expect("silent-close peer must shut down cleanly");
        let result = match call {
            Ok(outcome) => outcome,
            Err(payload) => resume_unwind(payload),
        };

        assert_eq!(
            result.as_ref().err().map(std::io::Error::kind),
            Some(std::io::ErrorKind::UnexpectedEof),
            "a peer that closes having sent nothing must be reported as an error, not {result:?}"
        );
    }

    /// The structural property that makes a busy accept poll impossible: with
    /// a blocking listening socket, `accept()` parks in the kernel and there
    /// is nothing for a sleep to pace. Asserting the flag rather than timing
    /// the loop keeps this deterministic — a CPU or wakeup-rate threshold
    /// would be a flake on a loaded CI runner. Red against the previous
    /// `bind`, which called `set_nonblocking(true)`.
    #[cfg(unix)]
    #[test]
    fn bind_leaves_the_listening_socket_blocking() {
        use std::os::unix::io::AsRawFd;

        let listener = HealthListener::bind(0).expect("bind test health listener");

        // SAFETY: `listener.listener` is a live, owned, valid socket for the
        // duration of this call; `F_GETFL` only reads flags and mutates nothing.
        let flags = unsafe { libc::fcntl(listener.listener.as_raw_fd(), libc::F_GETFL) };
        assert!(
            flags >= 0,
            "fcntl(F_GETFL) on the health listening socket failed: {}",
            std::io::Error::last_os_error()
        );
        assert_eq!(
            flags & libc::O_NONBLOCK,
            0,
            "the health listening socket must stay blocking so run()'s accept() parks \
             instead of returning WouldBlock on a timer; a non-blocking listener puts the \
             5 ms idle poll back"
        );
    }

    /// The safety half of the blocking-accept design. `run_daemon` (and every
    /// test harness) calls `request_shutdown` and then *joins* the listener
    /// thread, so a parked `accept()` that is never woken hangs the daemon's
    /// shutdown forever rather than merely delaying it.
    ///
    /// Bounded through a channel instead of a bare `join()` on purpose: a
    /// missing wake is an infinite park, and this must fail with a message
    /// rather than hang the suite.
    #[test]
    fn request_shutdown_wakes_a_parked_accept() {
        let listener = HealthListener::bind(0).expect("bind test health listener");
        let port = listener.port;
        let state = Arc::new(DaemonState::new());
        let info = Arc::new(Mutex::new(daemon_info(port)));

        let run_state = Arc::clone(&state);
        let run_info = Arc::clone(&info);
        let (finished_tx, finished_rx) = mpsc::channel();
        let server = std::thread::spawn(move || {
            listener.run(run_state, run_info);
            let _ = finished_tx.send(());
        });

        // Prove the parked loop still serves a real client — a blocking
        // accept() that never woke on a genuine connection would fail here —
        // then idle well past the 5 ms cadence the old poll ran at, so the
        // loop is certainly parked when shutdown is requested.
        let pong =
            send_command_with_timeout(port, "PING", Duration::from_secs(2), Duration::from_secs(2))
                .expect("a parked blocking accept() must still accept a real client");
        assert!(pong.starts_with("PONG"), "unexpected PING reply: {pong:?}");
        std::thread::sleep(Duration::from_millis(100));

        let requested_at = Instant::now();
        state.request_shutdown();
        let woken = finished_rx.recv_timeout(Duration::from_secs(5));
        let elapsed = requested_at.elapsed();
        assert!(
            woken.is_ok(),
            "request_shutdown did not wake the parked accept() within 5s ({elapsed:?}); \
             without the self-connect wake the listener thread never returns and \
             run_daemon's join() hangs"
        );
        server
            .join()
            .expect("health listener thread must not panic");
    }

    /// The ordering hazard the pre-`accept` flag check covers: a shutdown
    /// requested before `run` has registered a wake port sends no wake at all
    /// (`wake_port` is still the `0` sentinel), so the loop must notice the
    /// flag on its own and never park.
    #[test]
    fn run_returns_promptly_when_shutdown_precedes_it() {
        let listener = HealthListener::bind(0).expect("bind test health listener");
        let port = listener.port;
        let state = Arc::new(DaemonState::new());
        let info = Arc::new(Mutex::new(daemon_info(port)));

        state.request_shutdown();

        let run_state = Arc::clone(&state);
        let (finished_tx, finished_rx) = mpsc::channel();
        let server = std::thread::spawn(move || {
            listener.run(run_state, info);
            let _ = finished_tx.send(());
        });

        assert!(
            finished_rx.recv_timeout(Duration::from_secs(5)).is_ok(),
            "run() must observe a pre-existing shutdown flag before parking in accept(); \
             it parked instead and no wake was ever sent"
        );
        server
            .join()
            .expect("health listener thread must not panic");
    }
}
