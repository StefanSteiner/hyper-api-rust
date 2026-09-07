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

/// Bound on the throwaway loopback connect that wakes an accept loop waiting
/// for readiness.
///
/// The common cases are fast: the kernel completes a loopback connection into
/// the listen backlog without waiting for `accept()` (measured at ~140 µs), and
/// a listener that has already gone away refuses the connection in ~8 ms. The
/// worst case is *not* fast, though — with a full backlog the connect blocks
/// for this entire duration rather than failing, measured at 251 ms on macOS
/// against a `listen(1)` socket with nothing accepting. That is why
/// [`DaemonState::wake_accept_loop`] runs the connect on a detached thread: a
/// 250 ms stall must not land on the tokio worker that calls
/// `request_shutdown`, nor delay the `STOPPING` reply that `handle_client`
/// writes right after it.
const WAKE_CONNECT_TIMEOUT: Duration = Duration::from_millis(250);

/// Upper bound on how long [`HealthListener::run`] waits for accept readiness
/// before re-checking `should_shutdown` under its own power.
///
/// This is a deliberate correctness floor, not a leftover of the old 5 ms poll.
/// [`DaemonState::request_shutdown`]'s wake connection is what makes shutdown
/// *prompt* (sub-millisecond), but every wake can be lost — an unregistered
/// `wake_port`, a listener sharing a `DaemonState` with another one, a connect
/// that never lands. Since every `request_shutdown` caller in the tree follows
/// it with a `join()`, a lost wake against a purely wake-driven loop is a
/// permanent hang rather than a slow shutdown. One second bounds that
/// unconditionally while still costing only one wakeup per second, versus the
/// ~174/s the 5 ms poll cost (see issue #274 for the measurements).
#[cfg(unix)]
const ACCEPT_POLL_INTERVAL: Duration = Duration::from_secs(1);

/// Windows keeps the historical 5 ms cadence: [`wait_for_accept_readiness`]
/// there is a plain sleep rather than a readiness wait, so this interval also
/// bounds how long a real client waits to be accepted. Stretching it to a
/// second would trade a shutdown bound for a second of accept latency, so
/// Windows stays exactly where it shipped and only Unix takes the win.
#[cfg(not(unix))]
const ACCEPT_POLL_INTERVAL: Duration = Duration::from_millis(5);

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
///
/// Every field is private and reached through the accessor pair that owns its
/// invariant. `shutdown` in particular *must not* be settable directly: a bare
/// `store(true)` looks identical to `request_shutdown()` through
/// `should_shutdown()`, but skips the wake, so the listener stays in its
/// readiness wait for a further `ACCEPT_POLL_INTERVAL` instead of returning
/// at once. The other two are private for the same reason — one way in, one
/// way out, so the pairing cannot be bypassed by a caller who only sees the
/// field.
#[derive(Debug)]
pub struct DaemonState {
    /// Last time any client sent a heartbeat or query.
    /// Read and written through [`Self::idle_duration`] and [`Self::touch`].
    last_activity: Mutex<Instant>,
    /// Signal to shut down the daemon. Written only by
    /// [`Self::request_shutdown`], read only by [`Self::should_shutdown`].
    shutdown: AtomicBool,
    /// Set by clients reporting that hyperd looks dead from over there;
    /// consumed by the daemon's restart monitor. Written only by
    /// [`Self::request_restart`], read-and-cleared only by
    /// [`Self::consume_restart_request`].
    restart_requested: AtomicBool,
    /// Loopback port to connect to in order to wake a [`HealthListener::run`]
    /// loop waiting for accept readiness. `0` is an unambiguous
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

    /// Request shutdown and wake a health listener waiting for accept
    /// readiness.
    ///
    /// Every caller in the tree follows this with a `join()` on the listener
    /// thread (`run_daemon`'s step 7, and the test harnesses), so the wake is
    /// what makes that join return in microseconds rather than after a full
    /// `ACCEPT_POLL_INTERVAL`.
    ///
    /// # Memory ordering
    ///
    /// The `SeqCst` here is load-bearing and must not be relaxed to
    /// `Release`/`Acquire`. This half stores `shutdown` then loads
    /// `wake_port`; [`HealthListener::run`] stores `wake_port` then loads
    /// `shutdown`. That is Dekker's algorithm: each thread writes its own flag
    /// and reads the other's. Release/Acquire orders store→store and
    /// load→load, but says nothing about a store followed by a load of a
    /// *different* location, so both loads may return stale values in the same
    /// interleaving — `run` sees no shutdown and waits, while this sees no
    /// wake port and connects to nobody. Only `SeqCst` gives the single total
    /// order that rules the interleaving out.
    ///
    /// Both mainstream targets really do permit the reordering, which is worth
    /// recording because the arm64 half is easy to get wrong. On x86-64 a
    /// `Release` store is a plain `mov` with nothing between it and the
    /// following load, so the store buffer may sink it past that load; a
    /// `SeqCst` store is an `xchg`, which is a full barrier. On arm64 the
    /// store is `stlr` either way — but LLVM lowers an `Acquire` load to
    /// `ldapr` (RCpc) wherever FEAT_LRCPC is available, Apple Silicon
    /// included, and `ldapr` is specifically *not* ordered against a preceding
    /// `stlr`. Only the `ldar` (RCsc) that `SeqCst` emits carries that
    /// guarantee. Both readings are from the assembly rustc actually emits for
    /// this pairing, not from the reference manuals alone. The cost is one
    /// `xchg` on two cold paths.
    pub fn request_shutdown(&self) {
        self.shutdown.store(true, Ordering::SeqCst);
        self.wake_accept_loop();
    }

    /// Nudge the listening socket so a waiting `accept()` becomes ready and
    /// the loop re-reads `should_shutdown`.
    ///
    /// Runs on a detached thread. The connect is usually immediate, but its
    /// worst case is a full [`WAKE_CONNECT_TIMEOUT`] (see that constant), and
    /// this is called both from `run_daemon`'s async shutdown path — where it
    /// would stall a tokio worker — and from the `STOP` handler, ahead of the
    /// `STOPPING` reply. Neither can afford to wait on it, and nothing needs
    /// the result: shutdown is bounded by the loop's own readiness timeout
    /// whether or not the wake ever lands.
    ///
    /// Every failure mode is benign and ignored: no listener registered yet
    /// (`wake_port == 0`, and the loop's pre-`accept` `should_shutdown` check
    /// catches that), the listener already gone (connection refused), or a full
    /// backlog — in which case the loop still becomes ready through one of the
    /// queued connections. The wake makes shutdown prompt; correctness rests on
    /// the flag check the loop performs on every pass.
    fn wake_accept_loop(&self) {
        // Read on the caller's thread, not the spawned one: this load is the
        // second half of the `SeqCst` pairing documented on
        // `request_shutdown`, and moving it off-thread would break it.
        let port = self.wake_port.load(Ordering::SeqCst);
        if port == 0 {
            return;
        }
        let addr = std::net::SocketAddr::from(([127, 0, 0, 1], port));
        // `Builder::spawn` rather than `thread::spawn`: a best-effort wake must
        // not panic its caller if the process is out of threads. Losing the
        // wake costs at most one `ACCEPT_POLL_INTERVAL` of shutdown latency.
        let spawned = std::thread::Builder::new()
            .name("health-wake".to_string())
            .spawn(move || {
                let _ = TcpStream::connect_timeout(&addr, WAKE_CONNECT_TIMEOUT);
            });
        if let Err(error) = spawned {
            debug!(%error, "could not spawn the health listener wake connection");
        }
    }

    /// Point [`Self::wake_accept_loop`] at a live listening port. Called by
    /// [`HealthListener::run`] before its first `should_shutdown` check.
    ///
    /// `SeqCst` for the reason spelled out on [`Self::request_shutdown`]: this
    /// store is the first half of the Dekker pairing whose second half is the
    /// `shutdown` load in [`HealthListener::run`].
    fn register_wake_port(&self, port: u16) {
        self.wake_port.store(port, Ordering::SeqCst);
    }

    /// Forget the wake port once the listener has stopped, so a later
    /// `request_shutdown` does not connect to whatever has since taken it.
    fn clear_wake_port(&self) {
        self.wake_port.store(0, Ordering::SeqCst);
    }

    /// `SeqCst` to match [`Self::request_shutdown`]'s store — see the memory
    /// ordering note there.
    pub fn should_shutdown(&self) -> bool {
        self.shutdown.load(Ordering::SeqCst)
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
    /// The listening socket is non-blocking. [`Self::run`] waits for accept
    /// readiness with a timeout rather than parking in `accept()` itself, so
    /// that a lost shutdown wake costs one `ACCEPT_POLL_INTERVAL` instead of
    /// blocking the loop in the kernel forever;
    /// `bind_leaves_the_listening_socket_nonblocking` pins that precondition.
    /// Passing `0` binds an ephemeral port, which `local_addr()` below
    /// resolves into [`Self::port`], so a bound listener's port is never `0`.
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
    ///
    /// `info` is shared (`Arc<Mutex<DaemonInfo>>`) so the listener reports the
    /// *current* hyperd endpoint after a restart — the monitor task updates the
    /// same Arc once a new hyperd is running.
    ///
    /// The loop does not poll on a timer. It waits for the listening socket to
    /// become readable — for up to `ACCEPT_POLL_INTERVAL` at a time — so an
    /// arriving client is accepted with no added latency and an idle daemon
    /// costs one wakeup per second instead of the ~174/s the old 5 ms sleep
    /// cost (see issue #274 for the measurements).
    ///
    /// [`DaemonState::request_shutdown`] makes a throwaway loopback connection
    /// to this listener, which returns the wait immediately; the loop
    /// re-checks `should_shutdown` on every pass and drops the wake connection
    /// unread. The readiness timeout is the floor underneath that: it bounds
    /// shutdown at one interval even when the wake never arrives, which
    /// matters because every `request_shutdown` caller joins this thread.
    #[expect(
        clippy::needless_pass_by_value,
        reason = "Arcs are cloned into per-connection threads"
    )]
    pub fn run(self, state: Arc<DaemonState>, info: Arc<Mutex<DaemonInfo>>) {
        // Register before the first `should_shutdown` check, so a shutdown
        // racing this setup either finds the port (and wakes us) or is seen by
        // the check below before we ever wait. The guard clears it again on
        // every exit path, including an unwind out of the loop — dropping
        // `self.listener` frees the port, and a `wake_port` still naming it
        // would send a later `request_shutdown` to whatever took it next.
        let _wake_port = WakePortRegistration::register(&state, self.port);

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
                    // Not an error: loop round, re-check `should_shutdown`,
                    // wait again.
                }
                Err(ref error) if error.kind() == std::io::ErrorKind::WouldBlock => {
                    // Nothing queued. Sleep in the kernel until the socket is
                    // readable or the interval expires, then re-check the flag.
                    wait_for_accept_readiness(&self.listener, ACCEPT_POLL_INTERVAL);
                }
                Err(error) => {
                    warn!(error = %error, "health listener accept error");
                    std::thread::sleep(Duration::from_millis(500));
                }
            }
        }
        debug!("health listener shut down");
    }
}

/// Keeps [`DaemonState::wake_port`] pointing at a live listener for exactly as
/// long as that listener is running.
///
/// A `Drop` impl rather than a call at the end of [`HealthListener::run`]:
/// the loop can unwind (`std::thread::spawn` panics when the process is out of
/// threads), and an unwind that skipped the clear would leave `wake_port`
/// naming a port the dropped listener had just released.
struct WakePortRegistration<'a> {
    state: &'a DaemonState,
}

impl<'a> WakePortRegistration<'a> {
    fn register(state: &'a DaemonState, port: u16) -> Self {
        state.register_wake_port(port);
        Self { state }
    }
}

impl Drop for WakePortRegistration<'_> {
    fn drop(&mut self) {
        self.state.clear_wake_port();
    }
}

/// Wait until `listener` has a connection queued, or `timeout` elapses.
///
/// Best-effort: the caller re-checks `should_shutdown` and retries `accept()`
/// regardless of the outcome, so a spurious early return costs one extra
/// `accept()` syscall and an error costs nothing. Returning early is always
/// safe; returning *late* is what must not happen, because the shutdown bound
/// documented on [`ACCEPT_POLL_INTERVAL`] rests on this call being finite.
#[cfg(unix)]
fn wait_for_accept_readiness(listener: &TcpListener, timeout: Duration) {
    use std::os::fd::AsRawFd;

    let deadline = Instant::now() + timeout;
    loop {
        let remaining = deadline.saturating_duration_since(Instant::now());
        if remaining.is_zero() {
            return;
        }

        let mut poll_fd = libc::pollfd {
            fd: listener.as_raw_fd(),
            events: libc::POLLIN,
            revents: 0,
        };
        // Saturating rather than wrapping: a timeout too large for a `c_int`
        // becomes the longest wait `poll` can express, never a negative value,
        // which `poll` reads as "block forever". Floored at 1 ms so a
        // sub-millisecond remainder cannot round to a zero-timeout spin.
        let timeout_ms = i32::try_from(remaining.as_millis())
            .unwrap_or(i32::MAX)
            .max(1);

        // SAFETY: `poll_fd` is a single live, correctly initialized `pollfd`,
        // and the `1` matches it; `listener` keeps the fd open for the whole
        // call. `poll` only reads `fd`/`events` and writes `revents`.
        if unsafe { libc::poll(&raw mut poll_fd, 1, timeout_ms) } >= 0 {
            // Readable, or the interval expired. Either way the caller's next
            // `accept()` decides what happened.
            return;
        }

        let error = std::io::Error::last_os_error();
        if error.kind() != std::io::ErrorKind::Interrupted {
            // Nothing a retry fixes, and returning immediately would spin: the
            // caller's `accept()` would come straight back with `WouldBlock`
            // and call this again. Sleep out the rest of the interval so a
            // persistently broken `poll` degrades to the old timer, keeping
            // the shutdown bound while costing no more than the 5 ms poll did.
            warn!(%error, "health listener readiness wait failed; falling back to a timer");
            std::thread::sleep(remaining);
            return;
        }
    }
}

/// Windows has no `poll` in this crate's dependency set, so the wait degrades
/// to a plain sleep and cannot be cut short by an arriving connection. That is
/// why [`ACCEPT_POLL_INTERVAL`] is 5 ms there: the interval doubles as the
/// accept latency, exactly as it did before the listener stopped polling.
#[cfg(not(unix))]
fn wait_for_accept_readiness(_listener: &TcpListener, timeout: Duration) {
    std::thread::sleep(timeout);
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
/// [`HealthListener::bind`] puts the listening socket into non-blocking mode
/// so [`HealthListener::run`] can bound its wait for readiness, which is
/// exactly the configuration that triggers the propagation.
/// `set_nonblocking(false)` below undoes it unconditionally, which is a no-op
/// (not a bug) on platforms that never had the problem, and states the
/// invariant [`handle_client`] actually depends on rather than leaving it to
/// follow from how the listener happens to be configured.
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

    /// The precondition the bounded-shutdown floor rests on. `run` never calls
    /// `accept()` on a socket that can park: it waits for readiness with an
    /// [`ACCEPT_POLL_INTERVAL`] timeout and then accepts what is already
    /// queued. Make the listening socket blocking and that inverts — `accept()`
    /// itself parks in the kernel with no timeout of any kind, and shutdown
    /// stops being bounded by anything except the wake connection landing.
    /// Asserting the flag rather than timing the loop keeps this deterministic;
    /// a CPU or wakeup-rate threshold would be a flake on a loaded CI runner.
    #[cfg(unix)]
    #[test]
    fn bind_leaves_the_listening_socket_nonblocking() {
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
            libc::O_NONBLOCK,
            "the health listening socket must stay non-blocking so run()'s accept() can \
             never park; a blocking listener makes shutdown depend entirely on the wake \
             connection landing, and a lost wake becomes a permanent hang in the join()"
        );
    }

    /// Deadline for the two tests that assert the *wake* is doing the work.
    ///
    /// Deliberately far below [`ACCEPT_POLL_INTERVAL`], because the readiness
    /// floor would otherwise make them pass with the wake deleted: the loop
    /// would simply notice the flag when its interval expired. Measured
    /// shutdown latency through the wake is well under a millisecond, so half
    /// a second is loose enough for a loaded CI runner and still an order of
    /// magnitude clear of the one-second floor it has to distinguish itself
    /// from. On Windows [`ACCEPT_POLL_INTERVAL`] is 5 ms and the two paths are
    /// indistinguishable; the assertion holds there but proves nothing.
    const PROMPT_SHUTDOWN_BOUND: Duration = Duration::from_millis(500);

    /// The promptness half of the design. `run_daemon` (and every test
    /// harness) calls `request_shutdown` and then *joins* the listener thread,
    /// and the wake is what makes that join return immediately instead of
    /// after a full [`ACCEPT_POLL_INTERVAL`].
    ///
    /// Bounded through a channel instead of a bare `join()` on purpose, so a
    /// regression fails with a message rather than stalling the suite.
    #[test]
    fn request_shutdown_wakes_a_waiting_accept_loop() {
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

        // Prove the loop serves a real client with no added latency, then idle
        // long enough that it is certainly back inside its readiness wait when
        // shutdown is requested — so the elapsed time below measures the wake
        // and not a coincidental interval boundary.
        let pong =
            send_command_with_timeout(port, "PING", Duration::from_secs(2), Duration::from_secs(2))
                .expect("the accept loop must serve a real client while waiting for readiness");
        assert!(pong.starts_with("PONG"), "unexpected PING reply: {pong:?}");
        std::thread::sleep(Duration::from_millis(100));

        let requested_at = Instant::now();
        state.request_shutdown();
        let woken = finished_rx.recv_timeout(Duration::from_secs(5));
        let elapsed = requested_at.elapsed();
        assert!(
            woken.is_ok(),
            "request_shutdown did not stop the listener within 5s ({elapsed:?})"
        );
        assert!(
            elapsed < PROMPT_SHUTDOWN_BOUND,
            "request_shutdown took {elapsed:?} to stop the listener, past the \
             {PROMPT_SHUTDOWN_BOUND:?} bound: the self-connect wake did not land and the \
             loop fell through to its {ACCEPT_POLL_INTERVAL:?} readiness floor instead"
        );
        server
            .join()
            .expect("health listener thread must not panic");
    }

    /// The floor underneath the wake, and the reason `run` may never call a
    /// blocking `accept()`.
    ///
    /// Clearing `wake_port` makes `request_shutdown` return without connecting
    /// to anything — the same observable outcome as a shutdown that races
    /// registration, a second listener having overwritten the port, or a
    /// connect that never lands. Every `request_shutdown` caller in the tree
    /// joins the listener thread, so with nothing but the wake to rely on this
    /// is not a slow shutdown but a permanent one.
    ///
    /// Red against a blocking listening socket: the loop sits in `accept()`
    /// with no timeout of any kind and the channel below never fires.
    #[test]
    fn shutdown_is_bounded_when_the_wake_cannot_be_delivered() {
        // Comfortably past `ACCEPT_POLL_INTERVAL`, so the test measures
        // "bounded at all" rather than racing the floor it is verifying.
        const LOST_WAKE_SHUTDOWN_BOUND: Duration = Duration::from_secs(5);

        let listener = HealthListener::bind(0).expect("bind test health listener");
        let port = listener.port;
        let state = Arc::new(DaemonState::new());
        let info = Arc::new(Mutex::new(daemon_info(port)));

        let run_state = Arc::clone(&state);
        let (finished_tx, finished_rx) = mpsc::channel();
        let server = std::thread::spawn(move || {
            listener.run(run_state, info);
            let _ = finished_tx.send(());
        });

        // Round-trip a real command first: that proves `run` has reached its
        // loop and registered, so the clear below genuinely un-registers a
        // live listener rather than winning a race against startup.
        let pong =
            send_command_with_timeout(port, "PING", Duration::from_secs(2), Duration::from_secs(2))
                .expect("the accept loop must serve a real client before the wake is removed");
        assert!(pong.starts_with("PONG"), "unexpected PING reply: {pong:?}");

        state.clear_wake_port();

        let requested_at = Instant::now();
        state.request_shutdown();
        let finished = finished_rx.recv_timeout(LOST_WAKE_SHUTDOWN_BOUND);
        let elapsed = requested_at.elapsed();
        assert!(
            finished.is_ok(),
            "the listener did not stop within {LOST_WAKE_SHUTDOWN_BOUND:?} ({elapsed:?}) once \
             the wake could not be delivered; shutdown must stay bounded by the loop's own \
             {ACCEPT_POLL_INTERVAL:?} readiness timeout, because run_daemon and every test \
             harness join this thread and an unbounded wait there is a permanent hang"
        );
        server
            .join()
            .expect("health listener thread must not panic");
    }

    /// The ordering hazard the pre-`accept` flag check covers: a shutdown
    /// requested before `run` has registered a wake port sends no wake at all
    /// (`wake_port` is still the `0` sentinel), so the loop must notice the
    /// flag on its own rather than waiting out an interval first.
    #[test]
    fn run_returns_promptly_when_shutdown_precedes_it() {
        let listener = HealthListener::bind(0).expect("bind test health listener");
        let port = listener.port;
        let state = Arc::new(DaemonState::new());
        let info = Arc::new(Mutex::new(daemon_info(port)));

        state.request_shutdown();

        let run_state = Arc::clone(&state);
        let (finished_tx, finished_rx) = mpsc::channel();
        let started_at = Instant::now();
        let server = std::thread::spawn(move || {
            listener.run(run_state, info);
            let _ = finished_tx.send(());
        });

        let finished = finished_rx.recv_timeout(Duration::from_secs(5));
        let elapsed = started_at.elapsed();
        assert!(
            finished.is_ok() && elapsed < PROMPT_SHUTDOWN_BOUND,
            "run() must observe a pre-existing shutdown flag before waiting on the socket; \
             it returned after {elapsed:?}, which means it waited out a readiness interval \
             ({ACCEPT_POLL_INTERVAL:?}) first"
        );
        server
            .join()
            .expect("health listener thread must not panic");
    }
}
