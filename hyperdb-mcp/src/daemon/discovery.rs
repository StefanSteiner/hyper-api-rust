// Copyright (c) 2026, Salesforce, Inc. All rights reserved.
// SPDX-License-Identifier: Apache-2.0 OR MIT

//! Discovery file management for the single-instance daemon.
//!
//! The daemon writes a JSON file to `~/.hyperdb/daemon.json` containing its
//! PID and the `hyperd` endpoint. Clients read this file to locate the running
//! daemon, validating liveness via a TCP health check before trusting it.

use std::io::{self, Read as _};
use std::path::{Path, PathBuf};
use std::time::Duration;

use serde::{Deserialize, Serialize};

use super::{DAEMON_PORT_SCAN_SPAN, DEFAULT_DAEMON_BASE_PORT};

const MAX_DISCOVERY_FILE_BYTES: usize = 64 * 1024;

/// Information written by the daemon so clients can discover and connect.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct DaemonInfo {
    /// OS process ID of the daemon.
    pub pid: u32,
    /// The `hyperd` libpq endpoint clients should connect to (e.g. `127.0.0.1:54321`).
    pub hyperd_endpoint: String,
    /// The TCP port the daemon's health listener is bound to.
    pub health_port: u16,
    /// ISO-8601 timestamp when the daemon started.
    pub started_at: String,
    /// Version of the daemon binary.
    pub version: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub(crate) struct DaemonBuildIdentity {
    mcp_version: String,
    executable_path: crate::diagnostics::ReportedPath,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub(crate) struct DaemonRecord {
    #[serde(flatten)]
    info: DaemonInfo,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    identity: Option<DaemonBuildIdentity>,
}

impl DaemonRecord {
    pub(super) fn with_current_identity(info: &DaemonInfo) -> io::Result<Self> {
        let executable = std::env::current_exe()?;
        Ok(Self {
            info: info.clone(),
            identity: Some(DaemonBuildIdentity {
                mcp_version: crate::version::mcp_version_string(),
                executable_path: crate::diagnostics::ReportedPath::from_os_str(
                    executable.as_os_str(),
                ),
            }),
        })
    }

    pub(crate) fn info(&self) -> &DaemonInfo {
        &self.info
    }

    pub(crate) fn identity(&self) -> Option<&DaemonBuildIdentity> {
        self.identity.as_ref()
    }
}

impl DaemonBuildIdentity {
    pub(crate) fn mcp_version(&self) -> &str {
        &self.mcp_version
    }

    pub(crate) fn executable_path(&self) -> &crate::diagnostics::ReportedPath {
        &self.executable_path
    }
}

/// Bytes carried alongside a [`RawDiscoveryRead`] classification so a caller
/// willing to try a looser shape can re-parse them without a second `read`.
///
/// `Debug` deliberately renders only the length. `RawDiscoveryRead` is
/// `Debug`-formatted into diagnostics, and a discovery file this process could
/// not parse may contain anything at all — a `Vec<u8>`'s derived `Debug` would
/// echo every one of those bytes into a doctor report or log line, just as a
/// decimal list rather than as text.
pub(crate) struct DiscoveryBytes(Vec<u8>);

impl DiscoveryBytes {
    pub(crate) fn new(bytes: Vec<u8>) -> Self {
        Self(bytes)
    }

    fn as_slice(&self) -> &[u8] {
        &self.0
    }
}

impl std::fmt::Debug for DiscoveryBytes {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(formatter, "<{} bytes>", self.0.len())
    }
}

#[derive(Debug)]
pub(crate) enum RawDiscoveryRead {
    Missing {
        path: crate::diagnostics::ReportedPath,
    },
    Unreadable {
        path: crate::diagnostics::ReportedPath,
        kind: io::ErrorKind,
    },
    Malformed {
        path: crate::diagnostics::ReportedPath,
        /// The bytes that failed to parse, carried so a caller willing to try
        /// a looser shape (see [`discover`]) can do so without a second
        /// `read` — which would cost redundant I/O and let a rewrite land
        /// between the two parses.
        contents: DiscoveryBytes,
    },
    /// The file exceeded [`MAX_DISCOVERY_FILE_BYTES`] before JSON parsing was
    /// even attempted. Distinct from [`Self::Malformed`]: the content may be
    /// perfectly well-formed JSON — it's merely larger than any legitimate
    /// discovery record should be — so callers (the doctor) should report
    /// what actually happened instead of sending the user to fix "invalid
    /// JSON" that isn't.
    Oversized {
        path: crate::diagnostics::ReportedPath,
        /// The bytes read before the cap was hit, carried for the same reason
        /// as [`Self::Malformed`]'s.
        contents: DiscoveryBytes,
    },
    Parsed {
        path: crate::diagnostics::ReportedPath,
        record: DaemonRecord,
    },
}

pub(crate) fn read_discovery_file_raw(path: &Path) -> RawDiscoveryRead {
    let reported_path = crate::diagnostics::ReportedPath::from_os_str(path.as_os_str());
    let file = match open_discovery_file(path) {
        Ok(file) => file,
        Err(error) if error.kind() == io::ErrorKind::NotFound => {
            return RawDiscoveryRead::Missing {
                path: reported_path,
            };
        }
        Err(error) => {
            return RawDiscoveryRead::Unreadable {
                path: reported_path,
                kind: error.kind(),
            };
        }
    };

    let is_regular_file = match file.metadata() {
        Ok(metadata) => metadata.file_type().is_file(),
        Err(error) => {
            return RawDiscoveryRead::Unreadable {
                path: reported_path,
                kind: error.kind(),
            };
        }
    };
    if !is_regular_file {
        return RawDiscoveryRead::Unreadable {
            path: reported_path,
            kind: io::ErrorKind::InvalidInput,
        };
    }

    let mut contents = Vec::with_capacity(MAX_DISCOVERY_FILE_BYTES + 1);
    let read_limit = u64::try_from(MAX_DISCOVERY_FILE_BYTES + 1).unwrap_or(u64::MAX);
    if let Err(error) = file.take(read_limit).read_to_end(&mut contents) {
        return RawDiscoveryRead::Unreadable {
            path: reported_path,
            kind: error.kind(),
        };
    }
    if contents.len() > MAX_DISCOVERY_FILE_BYTES {
        return RawDiscoveryRead::Oversized {
            path: reported_path,
            contents: DiscoveryBytes::new(contents),
        };
    }

    parse_discovery_contents(reported_path, contents)
}

/// Read the discovery file the way the client fast path does: follow
/// symlinks and accept any valid record size (unlike the doctor's bounded,
/// no-follow [`read_discovery_file_raw`]).
fn read_discovery_file_legacy(path: &Path) -> RawDiscoveryRead {
    let reported_path = crate::diagnostics::ReportedPath::from_os_str(path.as_os_str());
    let contents = match std::fs::read(path) {
        Ok(contents) => contents,
        Err(error) if error.kind() == io::ErrorKind::NotFound => {
            return RawDiscoveryRead::Missing {
                path: reported_path,
            };
        }
        Err(error) => {
            return RawDiscoveryRead::Unreadable {
                path: reported_path,
                kind: error.kind(),
            };
        }
    };
    parse_discovery_contents(reported_path, contents)
}

fn parse_discovery_contents(
    reported_path: crate::diagnostics::ReportedPath,
    contents: Vec<u8>,
) -> RawDiscoveryRead {
    match serde_json::from_slice(&contents) {
        Ok(record) => RawDiscoveryRead::Parsed {
            path: reported_path,
            record,
        },
        Err(_) => RawDiscoveryRead::Malformed {
            path: reported_path,
            contents: DiscoveryBytes::new(contents),
        },
    }
}

fn open_discovery_file(path: &Path) -> io::Result<std::fs::File> {
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt as _;

        // `O_NONBLOCK` makes FIFO/device rejection prompt, while `O_NOFOLLOW`
        // prevents a symlink swap from turning the checked input into a
        // blocking special file between path inspection and open.
        std::fs::OpenOptions::new()
            .read(true)
            .custom_flags(libc::O_NONBLOCK | libc::O_NOFOLLOW)
            .open(path)
    }
    #[cfg(not(unix))]
    {
        let metadata = std::fs::symlink_metadata(path)?;
        if !metadata.file_type().is_file() {
            return Err(io::Error::new(
                io::ErrorKind::InvalidInput,
                "daemon discovery source is not a regular file",
            ));
        }
        std::fs::File::open(path)
    }
}

/// Returns the directory used for daemon state files.
///
/// Resolution order:
/// 1. `HYPERDB_STATE_DIR` environment variable (if set)
/// 2. `~/.hyperdb/` (where `~` is `HOME` on Unix, `USERPROFILE` on Windows)
///
/// # Errors
/// Returns an error if neither the env var nor the home directory can be determined.
pub fn state_dir() -> io::Result<PathBuf> {
    if let Some(dir) = std::env::var_os("HYPERDB_STATE_DIR") {
        return Ok(PathBuf::from(dir));
    }
    let home = home_dir().ok_or_else(|| {
        io::Error::new(io::ErrorKind::NotFound, "cannot determine home directory")
    })?;
    Ok(home.join(".hyperdb"))
}

/// Returns the path to the discovery file.
///
/// # Errors
/// Returns an error if the home directory cannot be determined.
pub fn discovery_file_path() -> io::Result<PathBuf> {
    Ok(state_dir()?.join("daemon.json"))
}

/// Write the discovery file atomically (write-to-temp then rename).
///
/// # Errors
/// Returns an error if the state directory cannot be created or the file cannot be written.
pub fn write_discovery_file(info: &DaemonInfo) -> io::Result<()> {
    write_discovery_record(info)
}

pub(super) fn write_enriched_discovery_file(info: &DaemonInfo) -> io::Result<()> {
    let record = DaemonRecord::with_current_identity(info)?;
    write_discovery_record(&record)
}

fn write_discovery_record(record: &(impl Serialize + ?Sized)) -> io::Result<()> {
    let dir = state_dir()?;
    std::fs::create_dir_all(&dir)?;

    let path = dir.join("daemon.json");
    let tmp_path = dir.join("daemon.json.tmp");
    let json = serde_json::to_string_pretty(record).map_err(|e| io::Error::other(e.to_string()))?;
    std::fs::write(&tmp_path, json.as_bytes())?;
    // `std::fs::rename` already replaces an existing target atomically on
    // both Unix (`rename(2)`) and Windows (`MoveFileExW` with
    // `MOVEFILE_REPLACE_EXISTING`, falling back to `SetFileInformationByHandle`
    // with `FILE_RENAME_FLAG_REPLACE_IF_EXISTS`). Pre-deleting the target here
    // would reintroduce exactly the window this function's doc comment
    // promises not to have: a concurrent `discover()` could observe the file
    // as `Missing` mid-restart (see `try_restart_hyperd`, which rewrites this
    // file on every `hyperd` restart).
    std::fs::rename(&tmp_path, &path)?;
    Ok(())
}

/// Read the discovery file and validate that the daemon is still alive.
/// Returns `None` if no daemon is running (file missing, stale, or unreachable).
pub fn discover() -> Option<DaemonInfo> {
    let discovery_path = discovery_file_path().ok()?;
    // Preserve the historical client-discovery contract: normal discovery
    // follows symlinks and accepts any valid record size. Doctor uses the
    // separate bounded, no-follow raw reader above because it must never
    // mutate or block on a special file.
    //
    // The `RawDiscoveryRead` classification below chooses a debug-log message
    // and short-circuits a missing/unreadable file. It must never gate whether
    // a *live* daemon is discovered: an unrecognized, reshaped, or absent
    // `identity` block is forward-compatible noise to this fast path — see
    // docs/superpowers/specs/2026-08-13-hyperdb-mcp-agent-ux-design.md
    // ("Unknown fields remain forward compatible"). So when the strict
    // `DaemonRecord` parse fails, we don't give up: we re-parse the bytes the
    // classification already carries as a tolerant `DaemonInfo`, which ignores
    // unknown fields and never fails on a nested object (like `identity`) that
    // this fast path doesn't need. (The doctor's raw reader keeps the strict
    // `DaemonRecord` contract because it deliberately wants to know about a
    // malformed `identity` block.)
    let (info, from_fallback) = match read_discovery_file_legacy(&discovery_path) {
        RawDiscoveryRead::Missing { path } => {
            tracing::debug!(encoding = ?path.encoding, "daemon discovery file is missing");
            return None;
        }
        RawDiscoveryRead::Unreadable { path, kind } => {
            tracing::debug!(?kind, encoding = ?path.encoding, "daemon discovery file is unreadable");
            return None;
        }
        // `read_discovery_file_legacy` applies no size cap, so it does not
        // produce `Oversized` today. Folding it in with `Malformed` keeps that
        // from mattering: were this reader ever given a cap, a large-but-valid
        // record would take the tolerant fallback rather than silently
        // becoming an undiscoverable daemon — the exact defect this fallback
        // exists to prevent.
        RawDiscoveryRead::Oversized { path, contents }
        | RawDiscoveryRead::Malformed { path, contents } => {
            let Ok(info) = serde_json::from_slice::<DaemonInfo>(contents.as_slice()) else {
                tracing::debug!(encoding = ?path.encoding, "daemon discovery file is malformed");
                return None;
            };
            tracing::debug!(encoding = ?path.encoding, "daemon discovery file failed the strict parse; accepted as legacy DaemonInfo");
            (info, true)
        }
        RawDiscoveryRead::Parsed { path, record } => {
            tracing::debug!(encoding = ?path.encoding, "daemon discovery file parsed (enriched)");
            (record.info().clone(), false)
        }
    };

    // Validate liveness by connecting to the health port
    if is_daemon_alive(info.health_port) {
        return Some(info);
    }

    if from_fallback {
        // Never stale-clean a record we could not strictly parse. This branch
        // exists precisely because a *newer* daemon may write a record this
        // client cannot fully understand, and a single failed 300 ms PING is
        // not grounds for destroying it: the daemon rewrites `daemon.json`
        // only on `hyperd` restart, so deleting here would hide a live newer
        // daemon from every client, not just this one. Leaving it also keeps
        // the promise doctor already makes to operators about a record it
        // could not parse ("it was left unchanged").
        tracing::debug!(
            "leaving the leniently parsed discovery record in place after a failed health check"
        );
        return None;
    }

    // Stale file — daemon crashed. Clean up.
    let _ = std::fs::remove_file(&discovery_path);
    None
}

/// Remove the discovery file (called during graceful shutdown).
pub fn remove_discovery_file() {
    if let Ok(path) = discovery_file_path() {
        let _ = std::fs::remove_file(&path);
    }
}

/// Check if the daemon is alive by sending PING and verifying the identifying token.
/// No longer accepts a bare TCP connect (prevents collisions with foreign services).
fn is_daemon_alive(port: u16) -> bool {
    super::health::ping_identified(port, Duration::from_millis(300), Duration::from_millis(300))
        .is_some()
}

/// Port scan configuration: a base port and the number of ports to scan.
/// When `span == 1`, the port is pinned (no scan). Used by the later
/// port-scanning stage to discover or spawn a daemon across a range.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct PortScan {
    pub base: u16,
    pub span: u16,
}

/// Resolve the daemon health port scan configuration from environment or default.
/// If `HYPERDB_DAEMON_PORT` is set and valid, returns a pinned scan (span=1) at
/// that exact port. Otherwise, returns the default base port with the full scan span.
///
/// `0` is not a valid value here even though `"0".parse::<u16>()` succeeds, and
/// is rejected into the same default fallback as unparseable input. A pinned
/// `PortScan { base: 0, span: 1 }` is unsatisfiable by construction: `bind`
/// would take an OS-assigned *ephemeral* port rather than port 0, while every
/// client's scan would keep probing port 0, get a connection error, and read
/// `ProbeResult::Refused` as "free" — so each client that missed the discovery
/// fast path would spawn another daemon-and-`hyperd` pair on another ephemeral
/// port that no scan can find, accumulating them silently.
pub fn resolve_port_scan() -> PortScan {
    if let Some(port) = std::env::var(super::ENV_DAEMON_PORT)
        .ok()
        .and_then(|v| v.parse::<u16>().ok())
        .filter(|port| *port != 0)
    {
        PortScan {
            base: port,
            span: 1,
        }
    } else {
        PortScan {
            base: DEFAULT_DAEMON_BASE_PORT,
            span: DAEMON_PORT_SCAN_SPAN,
        }
    }
}

/// Resolve the daemon health port from environment or default. Back-compat
/// wrapper for single-port callers; returns the base port from [`resolve_port_scan`].
/// New code that needs scan-aware logic should call [`resolve_port_scan`] directly.
pub fn resolve_port() -> u16 {
    resolve_port_scan().base
}

/// Cross-platform home directory resolution.
fn home_dir() -> Option<PathBuf> {
    // Try HOME (Unix) then USERPROFILE (Windows)
    std::env::var_os("HOME")
        .or_else(|| std::env::var_os("USERPROFILE"))
        .map(PathBuf::from)
}

/// Result of probing a single port: either our daemon, something else, or refused.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ProbeResult {
    /// A hyperdb-mcp daemon answered with valid STATUS.
    OurDaemon(Box<DaemonInfo>),
    /// The port accepted TCP but isn't our daemon (foreign service or broken STATUS).
    Camped,
    /// Connection refused (port is free).
    Refused,
}

/// Probe a single port to determine if it's occupied by our daemon, a foreign service, or free.
fn probe_port(port: u16) -> ProbeResult {
    let ping_timeout = Duration::from_millis(300);

    if let Some(_version) = super::health::ping_identified(port, ping_timeout, ping_timeout) {
        // PING succeeded — something is answering with our token. Now send STATUS
        // to retrieve the full daemon info. If STATUS fails we can't trust this
        // process (might be a test stub or a broken daemon), so treat it as Camped.
        match super::health::send_command_with_timeout(port, "STATUS", ping_timeout, ping_timeout) {
            Ok(response) => {
                if let Ok(info) = serde_json::from_str::<DaemonInfo>(response.trim()) {
                    ProbeResult::OurDaemon(Box::new(info))
                } else {
                    // Parsed PING but STATUS is malformed — treat as Camped.
                    ProbeResult::Camped
                }
            }
            Err(_) => ProbeResult::Camped,
        }
    } else {
        // PING failed or returned no identifying token. Distinguish "refused"
        // from "camped non-daemon" via a raw TCP connect attempt.
        let addr = std::net::SocketAddr::from(([127, 0, 0, 1], port));
        match std::net::TcpStream::connect_timeout(&addr, ping_timeout) {
            Ok(_) => ProbeResult::Camped, // TCP accepted but PING failed → foreign
            Err(_) => ProbeResult::Refused, // Connection refused → port is free
        }
    }
}

/// The outcome of scanning a port range for a running daemon or a free port to spawn on.
#[derive(Debug)]
pub enum ScanOutcome {
    /// Found a running hyperdb-mcp daemon.
    Found(Box<DaemonInfo>),
    /// No daemon found, but this port is free (can spawn here).
    FreePort(u16),
    /// All ports in the range are occupied (either by our daemon, foreign services, or both).
    AllOccupied,
}

/// Scan the configured port range to find a running daemon or identify a free port.
/// If any port in the range answers identified-PING and returns valid STATUS, we return
/// `Found` immediately (first wins). Otherwise, we return `FreePort` with the first
/// refused port encountered, or `AllOccupied` if everything is in use.
///
/// Product decision: prefer finding an existing daemon anywhere in range over
/// spawning a new one. Only spawn if no daemon exists.
pub fn scan_for_daemon(scan: PortScan) -> ScanOutcome {
    let mut first_free: Option<u16> = None;

    for offset in 0..scan.span {
        let Some(port) = scan.base.checked_add(offset) else {
            break; // Overflow guard: stop at u16::MAX
        };

        match probe_port(port) {
            ProbeResult::OurDaemon(info) => {
                // Found a running daemon — return immediately.
                return ScanOutcome::Found(info);
            }
            ProbeResult::Refused => {
                // Port is free. Remember the first one we see.
                if first_free.is_none() {
                    first_free = Some(port);
                }
            }
            ProbeResult::Camped => {
                // Port is occupied by something else. Keep scanning.
            }
        }
    }

    // No daemon found. Return the first free port, or AllOccupied if none.
    match first_free {
        Some(port) => ScanOutcome::FreePort(port),
        None => ScanOutcome::AllOccupied,
    }
}

/// Discover a running daemon via the discovery file, or by scanning the configured
/// port range. Returns `None` if no daemon is found in either place.
///
/// Used by CLI commands (status/stop) that want to find a daemon but not spawn one.
pub fn find_running_daemon() -> Option<DaemonInfo> {
    discover().or_else(|| match scan_for_daemon(resolve_port_scan()) {
        ScanOutcome::Found(info) => Some(*info),
        _ => None,
    })
}

#[cfg(test)]
mod tests {
    use std::ffi::{OsStr, OsString};
    use std::panic::{AssertUnwindSafe, catch_unwind};
    use std::sync::{Arc, Mutex};

    use serde_json::{Value, json};
    use tempfile::TempDir;

    use crate::daemon::health::{DaemonState, HealthListener};
    use crate::diagnostics::ReportedPath;

    use super::*;

    fn legacy_info() -> DaemonInfo {
        DaemonInfo {
            pid: 4242,
            hyperd_endpoint: "127.0.0.1:54321".to_string(),
            health_port: 7485,
            started_at: "2026-08-13T12:34:56Z".to_string(),
            version: "0.7.0".to_string(),
        }
    }

    fn catch_serde<T>(operation: impl FnOnce() -> serde_json::Result<T>) -> Result<T, String> {
        catch_unwind(AssertUnwindSafe(operation))
            .map_err(|_| "operation panicked".to_string())?
            .map_err(|error| error.to_string())
    }

    fn catch_raw_read(path: &Path) -> Result<RawDiscoveryRead, String> {
        catch_unwind(AssertUnwindSafe(|| read_discovery_file_raw(path)))
            .map_err(|_| "raw discovery read panicked".to_string())
    }

    fn directory_entries(path: &Path) -> Vec<OsString> {
        let mut entries = std::fs::read_dir(path)
            .unwrap()
            .map(|entry| entry.unwrap().file_name())
            .collect::<Vec<_>>();
        entries.sort();
        entries
    }

    fn run_discovery_compatibility_child(test_name: &str, child_sentinel_env: &str) {
        use std::process::{Command, Stdio};
        use std::time::Instant;

        let tmp = TempDir::new().unwrap();
        let state_dir = tmp.path().join("state");
        let child_marker = tmp.path().join("child-started");
        let mut child = Command::new(std::env::current_exe().unwrap())
            .arg("--exact")
            .arg(test_name)
            .arg("--nocapture")
            .env(child_sentinel_env, &child_marker)
            .env("HYPERDB_STATE_DIR", &state_dir)
            .stdin(Stdio::null())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()
            .unwrap();

        let deadline = Instant::now() + Duration::from_secs(5);
        let timed_out = loop {
            match child.try_wait() {
                Ok(Some(_)) => break false,
                Ok(None) if Instant::now() < deadline => {
                    std::thread::sleep(Duration::from_millis(10));
                }
                Ok(None) => break true,
                Err(error) => {
                    let _ = child.kill();
                    let output = child.wait_with_output().unwrap();
                    panic!(
                        "discovery compatibility child status failed: {error}\nstdout:\n{}\nstderr:\n{}",
                        String::from_utf8_lossy(&output.stdout),
                        String::from_utf8_lossy(&output.stderr)
                    );
                }
            }
        };

        if timed_out {
            let kill_error = child.kill().err();
            let output = child.wait_with_output().unwrap();
            panic!(
                "discovery compatibility child exceeded 5s and was killed ({kill_error:?})\nstdout:\n{}\nstderr:\n{}",
                String::from_utf8_lossy(&output.stdout),
                String::from_utf8_lossy(&output.stderr)
            );
        }

        let output = child.wait_with_output().unwrap();
        assert!(
            child_marker.is_file(),
            "exact discovery compatibility child branch did not start\nstdout:\n{}\nstderr:\n{}",
            String::from_utf8_lossy(&output.stdout),
            String::from_utf8_lossy(&output.stderr)
        );
        assert!(
            output.status.success(),
            "discovery compatibility child failed with {}\nstdout:\n{}\nstderr:\n{}",
            output.status,
            String::from_utf8_lossy(&output.stdout),
            String::from_utf8_lossy(&output.stderr)
        );
    }

    #[test]
    fn daemon_record_old_and_new_flat_wire_contract() {
        let old_wire = json!({
            "pid": 4242,
            "hyperd_endpoint": "127.0.0.1:54321",
            "health_port": 7485,
            "started_at": "2026-08-13T12:34:56Z",
            "version": "0.7.0"
        });
        let identity = DaemonBuildIdentity {
            mcp_version: "0.7.0.rabc123".to_string(),
            executable_path: ReportedPath::from_os_str(OsStr::new("/opt/hyperdb/bin/hyperdb-mcp")),
        };
        let expected_new_wire = json!({
            "pid": 4242,
            "hyperd_endpoint": "127.0.0.1:54321",
            "health_port": 7485,
            "started_at": "2026-08-13T12:34:56Z",
            "version": "0.7.0",
            "identity": {
                "mcp_version": "0.7.0.rabc123",
                "executable_path": {
                    "display": "/opt/hyperdb/bin/hyperdb-mcp",
                    "encoding": "utf8"
                }
            }
        });
        let mut failures = Vec::new();

        match catch_serde(|| serde_json::from_value::<DaemonRecord>(old_wire.clone())) {
            Ok(record) => {
                if record.info != legacy_info() {
                    failures
                        .push("old flat JSON did not preserve legacy daemon fields".to_string());
                }
                if record.identity.is_some() {
                    failures
                        .push("old flat JSON should deserialize with absent identity".to_string());
                }
                match catch_serde(|| serde_json::to_value(&record)) {
                    Ok(round_trip) if round_trip == old_wire => {}
                    Ok(round_trip) => failures.push(format!(
                        "old record did not reserialize to the exact flat wire: {round_trip}"
                    )),
                    Err(error) => failures.push(format!(
                        "old record could not be reserialized after parsing: {error}"
                    )),
                }
            }
            Err(error) => failures.push(format!("old flat JSON did not deserialize: {error}")),
        }

        let new_record = DaemonRecord {
            info: legacy_info(),
            identity: Some(identity.clone()),
        };
        match catch_serde(|| serde_json::to_value(&new_record)) {
            Ok(new_wire) => {
                if new_wire != expected_new_wire {
                    failures.push(format!(
                        "new record wire was not the exact additive flat shape: {new_wire}"
                    ));
                }
                if new_wire.get("info").is_some() {
                    failures.push("new wire nested legacy fields under `info`".to_string());
                }

                match catch_serde(|| serde_json::from_value::<DaemonRecord>(new_wire.clone())) {
                    Ok(round_trip) => {
                        if round_trip.info != legacy_info()
                            || round_trip.identity.as_ref() != Some(&identity)
                        {
                            failures.push(
                                "new build/executable identity did not round-trip".to_string(),
                            );
                        }
                    }
                    Err(error) => failures.push(format!(
                        "new build/executable identity could not be deserialized: {error}"
                    )),
                }

                match serde_json::from_value::<DaemonInfo>(new_wire) {
                    Ok(old_reader) if old_reader == legacy_info() => {}
                    Ok(old_reader) => failures.push(format!(
                        "old DaemonInfo reader changed legacy fields: {old_reader:?}"
                    )),
                    Err(error) => failures.push(format!(
                        "old DaemonInfo reader rejected additive identity: {error}"
                    )),
                }
            }
            Err(error) => failures.push(format!("new record could not be serialized: {error}")),
        }

        assert!(
            failures.is_empty(),
            "daemon record wire contract failures:\n{}",
            failures.join("\n")
        );
    }

    /// Exercises the actual scan, health check, and cleanup logic across a
    /// process boundary so `discover()`'s `HYPERDB_STATE_DIR` env override
    /// can't race other tests in this binary.
    fn run_identity_forward_compat_scenario() {
        assert!(
            std::env::var_os("HYPERDB_STATE_DIR").is_some(),
            "child scenario requires an isolated state directory"
        );

        // Three real-world shapes a client fast path must still discover a
        // live daemon through, per the forward-compatibility contract in
        // docs/superpowers/specs/2026-08-13-hyperdb-mcp-agent-ux-design.md
        // ("Unknown fields remain forward compatible"). None of these is
        // valid input for `DaemonBuildIdentity`, so a strict `DaemonRecord`
        // parse of the whole file must fail for every one of them — that's
        // asserted below as the fixture sanity check.
        let shapes: [(&str, Value); 3] = [
            ("identity retyped as a string", json!("not-an-object")),
            (
                "executable_path reshaped",
                json!({
                    "mcp_version": "0.7.0.rabc123",
                    "executable_path": "/opt/hyperdb/bin/hyperdb-mcp"
                }),
            ),
            (
                "identity missing executable_path",
                json!({ "mcp_version": "0.7.0.rabc123" }),
            ),
        ];

        let mut failures = Vec::new();
        for (index, (label, identity_json)) in shapes.into_iter().enumerate() {
            let pid = 9_000 + u32::try_from(index).expect("fixture index fits in u32");
            let health_listener = HealthListener::bind(0).unwrap();
            let info = DaemonInfo {
                pid,
                hyperd_endpoint: "127.0.0.1:54321".to_string(),
                health_port: health_listener.port,
                started_at: "2026-08-13T12:34:56Z".to_string(),
                version: "0.7.0".to_string(),
            };

            let wire = json!({
                "pid": info.pid,
                "hyperd_endpoint": info.hyperd_endpoint,
                "health_port": info.health_port,
                "started_at": info.started_at,
                "version": info.version,
                "identity": identity_json,
            });

            if serde_json::from_value::<DaemonRecord>(wire.clone()).is_ok() {
                failures.push(format!(
                    "{label}: fixture unexpectedly satisfied the strict DaemonRecord parse; \
                     it no longer exercises the forward-compatibility regression"
                ));
            }

            std::fs::create_dir_all(state_dir().unwrap()).unwrap();
            let path = discovery_file_path().unwrap();
            std::fs::write(&path, serde_json::to_vec(&wire).unwrap()).unwrap();

            let health_state = Arc::new(DaemonState::new());
            let health_info = Arc::new(Mutex::new(info.clone()));
            let run_state = Arc::clone(&health_state);
            let run_info = Arc::clone(&health_info);
            let health_server =
                std::thread::spawn(move || health_listener.run(run_state, run_info));

            match discover() {
                Some(discovered) if discovered == info => {}
                Some(other) => failures.push(format!(
                    "{label}: discover() returned different facts than the live daemon: {other:?}"
                )),
                None => failures.push(format!(
                    "{label}: discover() returned None for a live, healthy daemon whose only \
                     defect is an unrecognized `identity` block"
                )),
            }

            health_state.request_shutdown();
            health_server.join().unwrap();
            let _ = std::fs::remove_file(&path);
        }

        assert!(
            failures.is_empty(),
            "identity forward-compatibility discover failures:\n{}",
            failures.join("\n")
        );
    }

    #[test]
    fn discover_tolerates_forward_incompatible_identity_shapes() {
        const CHILD_SENTINEL_ENV: &str = "HYPERDB_MCP_IDENTITY_FORWARD_COMPAT_CHILD";
        const TEST_NAME: &str =
            "daemon::discovery::tests::discover_tolerates_forward_incompatible_identity_shapes";

        let _process_guard = crate::diagnostics::real_network_test_guard();
        if let Some(marker) = std::env::var_os(CHILD_SENTINEL_ENV) {
            std::fs::write(std::path::PathBuf::from(marker), b"started").unwrap();
            run_identity_forward_compat_scenario();
            return;
        }
        run_discovery_compatibility_child(TEST_NAME, CHILD_SENTINEL_ENV);
    }

    /// Returns a port that nothing is listening on, by binding one and
    /// releasing it immediately. Anything that later camps on it still fails
    /// `ping_identified` (no PONG token), so the port reads as dead either way.
    fn released_port() -> u16 {
        let listener = std::net::TcpListener::bind(("127.0.0.1", 0)).unwrap();
        listener.local_addr().unwrap().port()
    }

    fn run_lenient_stale_cleanup_scenario() {
        assert!(
            std::env::var_os("HYPERDB_STATE_DIR").is_some(),
            "child scenario requires an isolated state directory"
        );

        let path = discovery_file_path().unwrap();
        let mut failures = Vec::new();

        // A record whose `identity` block this client cannot understand, but
        // whose legacy fields are perfectly readable — exactly what a *newer*
        // daemon looks like to an older client, which is the only reason the
        // lenient fallback exists.
        let lenient_wire = json!({
            "pid": 9_100,
            "hyperd_endpoint": "127.0.0.1:54321",
            "health_port": released_port(),
            "started_at": "2026-08-13T12:34:56Z",
            "version": "0.7.0",
            "identity": "not-an-object",
        });
        if serde_json::from_value::<DaemonRecord>(lenient_wire.clone()).is_ok() {
            failures.push(
                "fixture satisfied the strict DaemonRecord parse; it no longer reaches the \
                 lenient fallback this test is about"
                    .to_string(),
            );
        }
        if serde_json::from_value::<DaemonInfo>(lenient_wire.clone()).is_err() {
            failures.push(
                "fixture is not readable as a legacy DaemonInfo either; it would be rejected \
                 before the branch this test pins"
                    .to_string(),
            );
        }

        std::fs::create_dir_all(state_dir().unwrap()).unwrap();
        let lenient_bytes = serde_json::to_vec(&lenient_wire).unwrap();
        std::fs::write(&path, &lenient_bytes).unwrap();

        if let Some(info) = discover() {
            failures.push(format!(
                "discover() reported a live daemon on a released port: {info:?}"
            ));
        }
        // The pin: a failed health check must not destroy a record this
        // client could only read leniently. The daemon rewrites `daemon.json`
        // only on `hyperd` restart, so deleting it here would hide a live,
        // newer daemon from every client on the machine.
        match std::fs::read(&path) {
            Ok(after) if after == lenient_bytes => {}
            Ok(_) => {
                failures.push("discover() changed the leniently parsed record's bytes".to_string());
            }
            Err(error) => failures.push(format!(
                "discover() removed the leniently parsed discovery record it could not \
                 strictly parse: {error}"
            )),
        }

        // Contrast: a strictly parseable record on a dead port is still
        // stale-cleaned, so the guard above is narrow rather than a blanket
        // disabling of cleanup.
        let mut strict_info = legacy_info();
        strict_info.pid = 9_101;
        strict_info.health_port = released_port();
        write_discovery_file(&strict_info).unwrap();
        if let Some(info) = discover() {
            failures.push(format!(
                "discover() reported a live daemon for the strict record on a released port: {info:?}"
            ));
        }
        if path.exists() {
            failures.push(
                "discover() did not stale-clean a strictly parsed record on a dead port"
                    .to_string(),
            );
        }

        assert!(
            failures.is_empty(),
            "lenient stale-cleanup failures:\n{}",
            failures.join("\n")
        );
    }

    /// A record that only the lenient `DaemonInfo` fallback could read must
    /// survive a failed health check. Before the fallback existed, a strict
    /// parse failure returned from `discover()` before ever reaching the
    /// stale-cleanup `remove_file`, so tolerating the record must not also
    /// start deleting it.
    #[test]
    fn discover_preserves_a_leniently_parsed_stale_record() {
        const CHILD_SENTINEL_ENV: &str = "HYPERDB_MCP_LENIENT_STALE_CLEANUP_CHILD";
        const TEST_NAME: &str =
            "daemon::discovery::tests::discover_preserves_a_leniently_parsed_stale_record";

        let _process_guard = crate::diagnostics::real_network_test_guard();
        if let Some(marker) = std::env::var_os(CHILD_SENTINEL_ENV) {
            std::fs::write(std::path::PathBuf::from(marker), b"started").unwrap();
            run_lenient_stale_cleanup_scenario();
            return;
        }
        run_discovery_compatibility_child(TEST_NAME, CHILD_SENTINEL_ENV);
    }

    #[test]
    fn raw_discovery_read_is_non_mutating_and_distinguishes_io() {
        const SECRET_SENTINEL: &str = "RAW_DISCOVERY_SECRET_MUST_NOT_LEAK";

        let tmp = TempDir::new().unwrap();
        let missing_path = tmp.path().join("missing.json");
        let unreadable_path = tmp.path().join("directory-not-file");
        std::fs::create_dir(&unreadable_path).unwrap();

        let malformed_path = tmp.path().join("malformed.json");
        let malformed_bytes = format!("{{\"secret\":\"{SECRET_SENTINEL}\"").into_bytes();
        std::fs::write(&malformed_path, &malformed_bytes).unwrap();

        let parsed_path = tmp.path().join("parsed.json");
        let parsed_bytes = serde_json::to_vec(&json!({
            "pid": 4242,
            "hyperd_endpoint": "127.0.0.1:54321",
            "health_port": 7485,
            "started_at": "2026-08-13T12:34:56Z",
            "version": "0.7.0"
        }))
        .unwrap();
        std::fs::write(&parsed_path, &parsed_bytes).unwrap();

        let entries_before = directory_entries(tmp.path());
        let mut failures = Vec::new();

        match catch_raw_read(&missing_path) {
            Ok(RawDiscoveryRead::Missing { path })
                if path == ReportedPath::from_os_str(missing_path.as_os_str()) => {}
            Ok(other) => failures.push(format!(
                "missing path was not reported as Missing with its ReportedPath: {other:?}"
            )),
            Err(error) => failures.push(format!("missing path read failed: {error}")),
        }

        match catch_raw_read(&unreadable_path) {
            Ok(RawDiscoveryRead::Unreadable { path, kind }) => {
                if path != ReportedPath::from_os_str(unreadable_path.as_os_str()) {
                    failures.push("unreadable path did not use ReportedPath".to_string());
                }
                if kind == io::ErrorKind::NotFound {
                    failures.push("non-NotFound I/O was misclassified as missing".to_string());
                }
            }
            Ok(other) => failures.push(format!(
                "directory read error was not distinguished as Unreadable: {other:?}"
            )),
            Err(error) => failures.push(format!("unreadable path read failed: {error}")),
        }

        match catch_raw_read(&malformed_path) {
            Ok(state) => {
                let rendered = format!("{state:?}");
                if rendered.contains(SECRET_SENTINEL) {
                    failures.push("malformed state leaked discovery contents".to_string());
                }
                // `Malformed` carries the unparsed bytes so `discover()` can
                // retry them without a second `read`. A derived `Debug` on a
                // `Vec<u8>` would print every byte as a decimal list — the same
                // leak, merely unsearchable — so check for that form too.
                let sentinel_as_debug_bytes = SECRET_SENTINEL
                    .as_bytes()
                    .iter()
                    .map(u8::to_string)
                    .collect::<Vec<_>>()
                    .join(", ");
                if rendered.contains(&sentinel_as_debug_bytes) {
                    failures.push(
                        "malformed state leaked discovery contents as a debug byte list"
                            .to_string(),
                    );
                }
                match state {
                    RawDiscoveryRead::Malformed { path, .. }
                        if path == ReportedPath::from_os_str(malformed_path.as_os_str()) => {}
                    other => failures.push(format!(
                        "malformed JSON was not reported as Malformed with its ReportedPath: {other:?}"
                    )),
                }
            }
            Err(error) => failures.push(format!("malformed path read failed: {error}")),
        }

        match catch_raw_read(&parsed_path) {
            Ok(RawDiscoveryRead::Parsed { path, record }) => {
                if path != ReportedPath::from_os_str(parsed_path.as_os_str()) {
                    failures.push("parsed path did not use ReportedPath".to_string());
                }
                if record.info != legacy_info() || record.identity.is_some() {
                    failures.push(
                        "old flat discovery JSON did not parse as a legacy record".to_string(),
                    );
                }
            }
            Ok(other) => failures.push(format!(
                "valid old discovery JSON was not reported as Parsed: {other:?}"
            )),
            Err(error) => failures.push(format!("parsed path read failed: {error}")),
        }

        #[cfg(unix)]
        {
            use std::os::unix::ffi::OsStringExt;

            use crate::diagnostics::PathEncoding;

            let non_utf8_path = tmp
                .path()
                .join(OsString::from_vec(b"missing-\xff.json".to_vec()));
            match catch_raw_read(&non_utf8_path) {
                Ok(RawDiscoveryRead::Missing { path }) if path.encoding == PathEncoding::Lossy => {}
                Ok(other) => failures.push(format!(
                    "non-UTF-8 path was not safely reported as lossy Missing: {other:?}"
                )),
                Err(error) => failures.push(format!("non-UTF-8 path read failed: {error}")),
            }
        }

        if missing_path.exists() {
            failures.push("raw read created the missing discovery path".to_string());
        }
        if !unreadable_path.is_dir() {
            failures.push("raw read removed or replaced the unreadable path".to_string());
        }
        match std::fs::read(&malformed_path) {
            Ok(bytes) if bytes == malformed_bytes => {}
            _ => failures.push("raw read changed or deleted malformed discovery bytes".to_string()),
        }
        match std::fs::read(&parsed_path) {
            Ok(bytes) if bytes == parsed_bytes => {}
            _ => failures.push("raw read changed or deleted parsed discovery bytes".to_string()),
        }
        if directory_entries(tmp.path()) != entries_before {
            failures.push("raw read changed the discovery directory entries".to_string());
        }

        assert!(
            failures.is_empty(),
            "raw discovery read contract failures:\n{}",
            failures.join("\n")
        );
    }

    #[test]
    fn raw_discovery_reports_oversized_valid_json_as_oversized() {
        const MAX_EXPECTED_DISCOVERY_BYTES: usize = 64 * 1024;

        let tmp = TempDir::new().unwrap();
        let base_bytes = serde_json::to_vec(&json!({
            "pid": 4242,
            "hyperd_endpoint": "127.0.0.1:54321",
            "health_port": 7485,
            "started_at": "2026-08-13T12:34:56Z",
            "version": "0.7.0",
            "ignored_padding": ""
        }))
        .unwrap();
        let marker = b"\"ignored_padding\":\"\"";
        let marker_start = base_bytes
            .windows(marker.len())
            .position(|window| window == marker)
            .unwrap();
        let padding_offset = marker_start + marker.len() - 1;
        let sized_fixture = |target_len: usize| {
            let mut bytes = base_bytes.clone();
            bytes.splice(
                padding_offset..padding_offset,
                vec![b'x'; target_len - bytes.len()],
            );
            bytes
        };
        let limit_bytes = sized_fixture(MAX_EXPECTED_DISCOVERY_BYTES);
        let oversized_bytes = sized_fixture(MAX_EXPECTED_DISCOVERY_BYTES + 1);
        let limit_path = tmp.path().join("limit-daemon.json");
        let oversized_path = tmp.path().join("oversized-daemon.json");
        std::fs::write(&limit_path, &limit_bytes).unwrap();
        std::fs::write(&oversized_path, &oversized_bytes).unwrap();

        let mut failures = Vec::new();
        if serde_json::from_slice::<DaemonRecord>(&limit_bytes).is_err()
            || serde_json::from_slice::<DaemonRecord>(&oversized_bytes).is_err()
        {
            failures.push("fixed-limit fixtures were not independently valid JSON".to_string());
        }
        match read_discovery_file_raw(&limit_path) {
            RawDiscoveryRead::Parsed { path: reported, .. }
                if reported == ReportedPath::from_os_str(limit_path.as_os_str()) => {}
            other => failures.push(format!(
                "valid JSON at the exact fixed limit was not accepted: {other:?}"
            )),
        }
        match read_discovery_file_raw(&oversized_path) {
            RawDiscoveryRead::Oversized { path: reported, .. }
                if reported == ReportedPath::from_os_str(oversized_path.as_os_str()) => {}
            other => failures.push(format!(
                "oversized valid JSON was not honestly classified as Oversized (not Malformed): {other:?}"
            )),
        }
        for (label, path, expected) in [
            ("limit", &limit_path, &limit_bytes),
            ("oversized", &oversized_path, &oversized_bytes),
        ] {
            match std::fs::read(path) {
                Ok(after) if after == *expected => {}
                Ok(_) => failures.push(format!("{label} discovery bytes were modified")),
                Err(error) => failures.push(format!(
                    "{label} discovery file disappeared after raw read: {error}"
                )),
            }
        }

        assert!(
            failures.is_empty(),
            "oversized raw discovery failures:\n{}",
            failures.join("\n")
        );
    }

    fn run_oversized_discovery_compatibility_scenario() {
        const RAW_DOCTOR_LIMIT_BYTES: usize = 64 * 1024;

        assert!(
            std::env::var_os("HYPERDB_STATE_DIR").is_some(),
            "child scenario requires an isolated state directory"
        );
        let health_listener = HealthListener::bind(0).unwrap();
        let health_port = health_listener.port;
        let oversized_info = DaemonInfo {
            pid: 5_252,
            hyperd_endpoint: "127.0.0.1:54321".to_string(),
            health_port,
            started_at: "2026-08-13T12:34:56Z".to_string(),
            version: "v".repeat(RAW_DOCTOR_LIMIT_BYTES + 1),
        };
        write_discovery_file(&oversized_info).unwrap();
        let path = discovery_file_path().unwrap();
        let original_bytes = std::fs::read(&path).unwrap();
        let mut failures = Vec::new();

        if original_bytes.len() <= RAW_DOCTOR_LIMIT_BYTES {
            failures.push(format!(
                "public writer produced only {} bytes, expected more than {RAW_DOCTOR_LIMIT_BYTES}",
                original_bytes.len()
            ));
        }
        match serde_json::from_slice::<DaemonInfo>(&original_bytes) {
            Ok(parsed) if parsed == oversized_info => {}
            Ok(_) => {
                failures.push("oversized public DaemonInfo did not round-trip exactly".to_string());
            }
            Err(error) => failures.push(format!(
                "public writer did not produce valid oversized DaemonInfo JSON: {error}"
            )),
        }
        match read_discovery_file_raw(&path) {
            RawDiscoveryRead::Oversized { path: reported, .. }
                if reported == ReportedPath::from_os_str(path.as_os_str()) => {}
            RawDiscoveryRead::Missing { .. } => {
                failures
                    .push("doctor raw reader reported the oversized record missing".to_string());
            }
            RawDiscoveryRead::Unreadable { kind, .. } => failures.push(format!(
                "doctor raw reader reported the oversized record unreadable: {kind:?}"
            )),
            RawDiscoveryRead::Malformed { .. } => {
                failures.push(
                    "doctor raw reader misclassified the well-formed oversized record as malformed"
                        .to_string(),
                );
            }
            RawDiscoveryRead::Oversized { .. } => {
                failures.push("doctor raw reader reported the wrong oversized path".to_string());
            }
            RawDiscoveryRead::Parsed { .. } => {
                failures.push("doctor raw reader accepted the oversized record".to_string());
            }
        }
        match std::fs::read(&path) {
            Ok(after) if after == original_bytes => {}
            Ok(_) => failures.push("doctor raw reader changed oversized bytes".to_string()),
            Err(error) => failures.push(format!(
                "doctor raw reader removed the oversized record: {error}"
            )),
        }

        let health_state = Arc::new(DaemonState::new());
        let health_info = Arc::new(Mutex::new(oversized_info.clone()));
        let run_state = Arc::clone(&health_state);
        let run_info = Arc::clone(&health_info);
        let health_server = std::thread::spawn(move || health_listener.run(run_state, run_info));

        match discover() {
            Some(info) if info == oversized_info => {}
            Some(_) => {
                failures
                    .push("normal discover returned different live oversized facts".to_string());
            }
            None => failures
                .push("normal discover did not accept the live oversized record".to_string()),
        }
        match std::fs::read(&path) {
            Ok(after) if after == original_bytes => {}
            Ok(_) => failures.push("live discover changed oversized bytes".to_string()),
            Err(error) => failures.push(format!(
                "live discover removed the oversized record: {error}"
            )),
        }

        health_state.request_shutdown();
        health_server.join().unwrap();
        if discover().is_some() {
            failures.push("stopped oversized record was incorrectly retained as live".to_string());
        }
        if path.exists() {
            failures
                .push("normal discover did not stale-clean the valid oversized record".to_string());
        }

        assert!(
            failures.is_empty(),
            "oversized legacy discover failures:\n{}",
            failures.join("\n")
        );
    }

    #[test]
    fn discover_preserves_legacy_oversized_stale_cleanup() {
        const CHILD_SENTINEL_ENV: &str = "HYPERDB_MCP_OVERSIZED_DISCOVERY_COMPATIBILITY_CHILD";
        const TEST_NAME: &str =
            "daemon::discovery::tests::discover_preserves_legacy_oversized_stale_cleanup";

        let _process_guard = crate::diagnostics::real_network_test_guard();
        if let Some(marker) = std::env::var_os(CHILD_SENTINEL_ENV) {
            std::fs::write(std::path::PathBuf::from(marker), b"started").unwrap();
            run_oversized_discovery_compatibility_scenario();
            return;
        }
        run_discovery_compatibility_child(TEST_NAME, CHILD_SENTINEL_ENV);
    }

    #[cfg(unix)]
    fn run_symlink_discovery_compatibility_scenario() {
        assert!(
            std::env::var_os("HYPERDB_STATE_DIR").is_some(),
            "child scenario requires an isolated state directory"
        );
        let health_listener = HealthListener::bind(0).unwrap();
        let health_port = health_listener.port;
        let mut linked_info = legacy_info();
        linked_info.pid = 6_363;
        linked_info.health_port = health_port;
        write_discovery_file(&linked_info).unwrap();
        let link_path = discovery_file_path().unwrap();
        let target_path = link_path.with_file_name("legacy-daemon-target.json");
        std::fs::rename(&link_path, &target_path).unwrap();
        std::os::unix::fs::symlink(&target_path, &link_path).unwrap();
        let target_bytes = std::fs::read(&target_path).unwrap();
        let mut failures = Vec::new();

        match read_discovery_file_raw(&link_path) {
            RawDiscoveryRead::Unreadable {
                path: reported,
                kind,
            } if reported == ReportedPath::from_os_str(link_path.as_os_str())
                && kind != io::ErrorKind::NotFound => {}
            other => failures.push(format!(
                "doctor raw reader did not reject the symlink without following it: {other:?}"
            )),
        }
        match std::fs::symlink_metadata(&link_path) {
            Ok(metadata) if metadata.file_type().is_symlink() => {}
            Ok(metadata) => failures.push(format!(
                "doctor raw reader replaced the link with {:?}",
                metadata.file_type()
            )),
            Err(error) => failures.push(format!(
                "doctor raw reader removed the discovery symlink: {error}"
            )),
        }
        match std::fs::read(&target_path) {
            Ok(after) if after == target_bytes => {}
            Ok(_) => failures.push("doctor raw reader changed symlink target bytes".to_string()),
            Err(error) => failures.push(format!(
                "doctor raw reader removed the symlink target: {error}"
            )),
        }

        let health_state = Arc::new(DaemonState::new());
        let health_info = Arc::new(Mutex::new(linked_info.clone()));
        let run_state = Arc::clone(&health_state);
        let run_info = Arc::clone(&health_info);
        let health_server = std::thread::spawn(move || health_listener.run(run_state, run_info));

        match discover() {
            Some(info) if info == linked_info => {}
            other => failures.push(format!(
                "normal discover did not follow the live valid symlink: {other:?}"
            )),
        }
        match std::fs::symlink_metadata(&link_path) {
            Ok(metadata) if metadata.file_type().is_symlink() => {}
            Ok(_) => failures.push("live discover replaced the discovery symlink".to_string()),
            Err(error) => failures.push(format!(
                "live discover removed the discovery symlink: {error}"
            )),
        }

        health_state.request_shutdown();
        health_server.join().unwrap();
        if discover().is_some() {
            failures.push("stopped symlinked record was incorrectly retained as live".to_string());
        }
        match std::fs::symlink_metadata(&link_path) {
            Err(error) if error.kind() == io::ErrorKind::NotFound => {}
            Ok(_) => failures
                .push("normal discover did not remove the stale discovery symlink".to_string()),
            Err(error) => failures.push(format!(
                "stale discovery symlink cleanup failed unexpectedly: {error}"
            )),
        }
        match std::fs::read(&target_path) {
            Ok(after) if after == target_bytes => {}
            Ok(_) => failures.push("normal discover changed symlink target bytes".to_string()),
            Err(error) => failures.push(format!(
                "normal discover removed the symlink target instead of the link: {error}"
            )),
        }

        assert!(
            failures.is_empty(),
            "symlink legacy discover failures:\n{}",
            failures.join("\n")
        );
    }

    #[cfg(unix)]
    #[test]
    fn discover_preserves_legacy_symlink_stale_cleanup() {
        const CHILD_SENTINEL_ENV: &str = "HYPERDB_MCP_SYMLINK_DISCOVERY_COMPATIBILITY_CHILD";
        const TEST_NAME: &str =
            "daemon::discovery::tests::discover_preserves_legacy_symlink_stale_cleanup";

        let _process_guard = crate::diagnostics::real_network_test_guard();
        if let Some(marker) = std::env::var_os(CHILD_SENTINEL_ENV) {
            std::fs::write(std::path::PathBuf::from(marker), b"started").unwrap();
            run_symlink_discovery_compatibility_scenario();
            return;
        }
        run_discovery_compatibility_child(TEST_NAME, CHILD_SENTINEL_ENV);
    }

    #[cfg(unix)]
    #[test]
    fn raw_discovery_rejects_fifo_without_blocking() {
        use std::ffi::CString;
        use std::os::unix::ffi::OsStrExt as _;
        use std::os::unix::fs::FileTypeExt as _;
        use std::path::PathBuf;
        use std::process::{Command, Stdio};
        use std::time::{Duration, Instant};

        const CHILD_PATH_ENV: &str = "HYPERDB_MCP_RAW_DISCOVERY_FIFO_CHILD";
        const CHILD_MARKER_ENV: &str = "HYPERDB_MCP_RAW_DISCOVERY_FIFO_MARKER";
        const TEST_NAME: &str =
            "daemon::discovery::tests::raw_discovery_rejects_fifo_without_blocking";

        if let Some(path) = std::env::var_os(CHILD_PATH_ENV) {
            let path = PathBuf::from(path);
            let marker = PathBuf::from(
                std::env::var_os(CHILD_MARKER_ENV)
                    .expect("FIFO child marker path must accompany child path"),
            );
            std::fs::write(marker, b"started").unwrap();
            let mut failures = Vec::new();
            match read_discovery_file_raw(&path) {
                RawDiscoveryRead::Unreadable { kind, .. } if kind != io::ErrorKind::NotFound => {}
                other => failures.push(format!(
                    "FIFO was not rejected as a non-NotFound unreadable discovery source: {other:?}"
                )),
            }
            match std::fs::symlink_metadata(&path) {
                Ok(metadata) if metadata.file_type().is_fifo() => {}
                Ok(_) => {
                    failures.push("raw read replaced the FIFO with another file type".to_string());
                }
                Err(error) => {
                    failures.push(format!("raw read removed the FIFO: {error}"));
                }
            }
            assert!(
                failures.is_empty(),
                "FIFO child failures:\n{}",
                failures.join("\n")
            );
            return;
        }

        let tmp = TempDir::new().unwrap();
        let fifo_path = tmp.path().join("daemon.fifo");
        let child_marker = tmp.path().join("child-started");
        let c_path = CString::new(fifo_path.as_os_str().as_bytes()).unwrap();
        // SAFETY: `c_path` is a live, NUL-terminated path and mode contains only
        // ordinary permission bits. The return code is checked before use.
        let result = unsafe { libc::mkfifo(c_path.as_ptr(), 0o600) };
        assert_eq!(result, 0, "mkfifo failed: {}", io::Error::last_os_error());

        let mut child = Command::new(std::env::current_exe().unwrap())
            .arg("--exact")
            .arg(TEST_NAME)
            .arg("--nocapture")
            .env(CHILD_PATH_ENV, &fifo_path)
            .env(CHILD_MARKER_ENV, &child_marker)
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .spawn()
            .unwrap();

        let deadline = Instant::now() + Duration::from_secs(2);
        let child_status = loop {
            match child.try_wait().unwrap() {
                Some(status) => break Some(status),
                None if Instant::now() < deadline => {
                    std::thread::sleep(Duration::from_millis(10));
                }
                None => {
                    child.kill().unwrap();
                    child.wait().unwrap();
                    break None;
                }
            }
        };

        let mut failures = Vec::new();
        match child_status {
            Some(status) if status.success() => {}
            Some(status) => failures.push(format!(
                "bounded FIFO child rejected the contract with status {status}"
            )),
            None => failures.push(
                "raw discovery read blocked on a FIFO past the two-second child watchdog"
                    .to_string(),
            ),
        }
        match std::fs::read(&child_marker) {
            Ok(marker) if marker == b"started" => {}
            Ok(marker) => failures.push(format!(
                "FIFO child wrote an unexpected start marker: {marker:?}"
            )),
            Err(error) => failures.push(format!(
                "FIFO child never reached the raw reader; exact filter may be wrong: {error}"
            )),
        }
        match std::fs::symlink_metadata(&fifo_path) {
            Ok(metadata) if metadata.file_type().is_fifo() => {}
            Ok(_) => {
                failures.push("watchdog run replaced the FIFO with another file type".to_string());
            }
            Err(error) => failures.push(format!("watchdog run removed the FIFO: {error}")),
        }

        assert!(
            failures.is_empty(),
            "FIFO raw discovery failures:\n{}",
            failures.join("\n")
        );
    }
}
