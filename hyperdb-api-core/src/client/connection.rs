// Copyright (c) 2026, Salesforce, Inc. All rights reserved.
// SPDX-License-Identifier: Apache-2.0 OR MIT

//! Low-level synchronous connection handling over the `PostgreSQL` wire protocol.
//!
//! [`RawConnection`] provides the message-level interface to a Hyper server:
//! startup/authentication handshake, simple and extended query execution, and
//! the COPY data path. It is generic over the transport stream `S` (TCP,
//! Unix domain socket, named pipe, or TLS-wrapped variants).
//!
//! # Wire Protocol Overview
//!
//! Communication follows the `PostgreSQL` v3 message protocol. Each message has a
//! 1-byte type tag, a 4-byte length (including itself), and a variable-length
//! body. The connection maintains separate read and write `BytesMut` buffers to
//! amortize syscall overhead.
//!
//! ## Query Protocols
//!
//! - **Simple Query**: A single `Query('Q')` message; the server responds with
//!   `RowDescription`, zero or more `DataRow`, `CommandComplete`, and
//!   `ReadyForQuery`. Results are in text format.
//!
//! - **Extended Query (`HyperBinary`)**: `Parse` / `Bind` / `Describe` /
//!   `Execute` / `Sync` sequence requesting format code 2 (`HyperBinary`,
//!   little-endian binary). Results are zero-copy-friendly `DataRow` messages.
//!   Used by [`Client::query_fast`](crate::client::Client::query_fast) and
//!   [`Client::query_streaming`](crate::client::Client::query_streaming).
//!
//! ## Connection Health
//!
//! A `desynchronized` flag tracks whether the wire protocol has fallen out of
//! sync (e.g., a bounded drain exceeded its budget). Once set, all subsequent
//! operations fast-fail. The only recovery is to drop the connection and
//! open a new one. Pool layers should check [`RawConnection::is_healthy`]
//! during recycle.

use std::io::{Read, Write};

use bytes::BytesMut;
use tracing::{debug, info, trace, warn};

use crate::protocol::message::{backend::Message, frontend};

use super::auth::{self, AuthState};
use super::error::{Error, Result};

/// Maximum number of messages [`RawConnection::consume_error`] (and its
/// async sibling) will read while draining the tail of a failed request.
///
/// A well-behaved server emits only a handful of messages after
/// `ErrorResponse` before `ReadyForQuery` (typically just the error +
/// `Z`, with the occasional `NoticeResponse` interleaved). This cap is
/// therefore orders of magnitude above anything a legitimate error path
/// produces; it exists purely as a defensive safety valve against broken
/// server implementations and stalled network paths. When exceeded, the
/// drain logs a `tracing::warn!` and returns — the next operation on the
/// connection will see the resulting desync and trigger a reconnect.
///
/// See [`RawConnection::consume_error`] for the full rationale.
pub const POST_ERROR_DRAIN_CAP: usize = 1024;

/// A raw connection to a Hyper server.
///
/// This handles the low-level protocol communication, including:
/// - Message framing (reading/writing `PostgreSQL` wire protocol messages)
/// - Authentication handshake
/// - Query execution (simple and extended query protocols)
/// - COPY protocol support
///
/// The connection is generic over the stream type `S`, allowing it to work
/// with different transport mechanisms (TCP, TLS, etc.) as long as they
/// implement `Read + Write`.
///
/// # Buffering
///
/// The connection maintains separate read and write buffers for efficient
/// I/O. Messages are buffered before being sent, and incoming data is
/// buffered until complete messages can be parsed.
#[derive(Debug)]
pub struct RawConnection<S> {
    /// The underlying I/O stream.
    stream: S,
    /// Buffer for reading incoming messages from the server.
    read_buf: BytesMut,
    /// Buffer for writing outgoing messages to the server.
    write_buf: BytesMut,
    /// Backend process ID (for cancel requests).
    process_id: i32,
    /// Secret key for authenticating cancel requests.
    secret_key: i32,
    /// Server parameters received during startup (e.g., `server_version`, `session_identifier`).
    server_params: std::collections::HashMap<String, String>,
    /// Sticky flag set when the wire protocol has fallen out of sync with
    /// the server (e.g. a bounded drain exhausted its budget before seeing
    /// `ReadyForQuery`). Once set it is never cleared — the only valid
    /// recovery is to discard this connection and open a new one.
    ///
    /// Every public method that initiates a new server request checks
    /// this flag via [`Self::ensure_healthy`] and fast-fails with a
    /// clear error instead of sending bytes into a known-poisoned
    /// channel. Pool layers should call [`Self::is_healthy`] during
    /// recycle to skip the health-probe roundtrip for connections that
    /// are already known-bad.
    desynchronized: bool,
}

impl<S> RawConnection<S>
where
    S: Read + Write,
{
    /// Creates a new raw connection from a stream.
    ///
    /// Initializes read and write buffers with default capacity (64 KB each).
    /// The connection is not yet authenticated - call `startup()` to begin
    /// the connection handshake.
    ///
    /// # Arguments
    ///
    /// * `stream` - The I/O stream (must implement `Read + Write`)
    pub fn new(stream: S) -> Self {
        RawConnection {
            stream,
            read_buf: BytesMut::with_capacity(64 * 1024),
            write_buf: BytesMut::with_capacity(64 * 1024),
            process_id: 0,
            secret_key: 0,
            server_params: std::collections::HashMap::new(),
            desynchronized: false,
        }
    }

    /// Returns `true` if this connection is still in a known-good state
    /// and safe to use for new requests.
    ///
    /// Once the wire protocol falls out of sync with the server (see the
    /// `desynchronized` field on [`RawConnection`]), this returns `false`
    /// permanently — the only recovery is to drop this connection and
    /// open a new one. Pool implementations should consult this before
    /// running a recycle health probe to avoid spending a roundtrip on a
    /// connection that is already known to be bad.
    pub fn is_healthy(&self) -> bool {
        !self.desynchronized
    }

    /// Fast-fails with an explicit [`ErrorKind::Connection`] error if the
    /// wire has fallen out of sync with the server, before any bytes are
    /// written to the stream. Called from the entry point of every public
    /// method that initiates a new server request — simple queries,
    /// streaming queries, prepared statement execution, COPY in/out,
    /// etc. — so that a desynchronized connection produces a clear
    /// "connection unusable" diagnostic at the API boundary rather than a
    /// cryptic protocol-parse error deep inside the message loop of the
    /// *next* unrelated operation.
    pub(crate) fn ensure_healthy(&self) -> Result<()> {
        if self.desynchronized {
            return Err(Error::new(
                crate::client::error::ErrorKind::Connection,
                "connection is desynchronized from the server and cannot be reused; \
                 discard it and open a new one",
            ));
        }
        Ok(())
    }

    /// Reserves capacity in the write buffer to avoid reallocations.
    ///
    /// Call this before bulk operations to pre-allocate buffer space.
    /// This is useful for high-throughput scenarios where buffer growth
    /// would cause performance overhead.
    pub fn reserve_write_buffer(&mut self, additional: usize) {
        self.write_buf.reserve(additional);
    }

    /// Returns the process ID assigned by the server.
    pub fn process_id(&self) -> i32 {
        self.process_id
    }

    /// Returns the secret key for cancel requests.
    pub fn secret_key(&self) -> i32 {
        self.secret_key
    }

    /// Returns a reference to the underlying stream.
    pub fn stream(&self) -> &S {
        &self.stream
    }

    /// Returns a mutable reference to the underlying stream.
    pub fn stream_mut(&mut self) -> &mut S {
        &mut self.stream
    }

    /// Returns a server parameter value by name.
    ///
    /// Server parameters are sent by the server during connection startup.
    /// Common parameters include:
    /// - `server_version` - The server version string
    /// - `server_encoding` - The server's character encoding
    /// - `client_encoding` - The client's character encoding
    pub fn parameter_status(&self, name: &str) -> Option<&str> {
        self.server_params
            .get(name)
            .map(std::string::String::as_str)
    }

    /// Sends a startup message and performs initial handshake.
    ///
    /// # Errors
    ///
    /// - Returns [`Error`] (auth) when the server requests an
    ///   auth method (cleartext, MD5, SASL) and no password is supplied,
    ///   when the offered SASL mechanisms exclude SCRAM-SHA-256, or when
    ///   SCRAM state is missing at the SASL-continue / SASL-final step.
    /// - Returns [`Error`] (server) when the server sends an `ErrorResponse`
    ///   during startup (for example, unknown user or database).
    /// - Returns [`Error`] (protocol) if a message arrives out of the
    ///   expected startup sequence.
    /// - Returns [`Error`] (I/O) on wire-protocol read/write failure.
    pub fn startup(&mut self, params: &[(&str, &str)], password: Option<&str>) -> Result<()> {
        // Send startup message
        frontend::startup_message(params, &mut self.write_buf)?;
        self.flush()?;

        // Handle authentication
        let mut auth_state: Option<AuthState> = None;

        loop {
            let msg = self.read_message()?;
            match msg {
                Message::AuthenticationOk => {
                    info!(target: "hyperdb_api", "connection-auth-success");
                }
                Message::AuthenticationCleartextPassword => {
                    debug!(target: "hyperdb_api", method = "cleartext", "connection-auth-method");
                    let password = password.ok_or_else(|| {
                        Error::authentication(
                            "server requested cleartext password but none provided",
                        )
                    })?;
                    frontend::password_message(password, &mut self.write_buf)?;
                    self.flush()?;
                }
                Message::AuthenticationMd5Password(body) => {
                    debug!(target: "hyperdb_api", method = "MD5", "connection-auth-method");
                    let password = password.ok_or_else(|| {
                        Error::authentication("server requested MD5 password but none provided")
                    })?;
                    let user = params
                        .iter()
                        .find(|(k, _)| *k == "user")
                        .map_or("", |(_, v)| *v);

                    let md5_response = auth::compute_md5_password(user, password, &body.salt());
                    frontend::password_message(&md5_response, &mut self.write_buf)?;
                    self.flush()?;
                }
                Message::AuthenticationSasl(body) => {
                    debug!(target: "hyperdb_api", method = "SCRAM-SHA-256", "connection-auth-method");
                    let password = password.ok_or_else(|| {
                        Error::authentication(
                            "server requested SASL authentication but no password provided",
                        )
                    })?;

                    // Check for SCRAM-SHA-256
                    let mechanisms: Vec<&str> = body.mechanisms().collect();
                    if !mechanisms.contains(&"SCRAM-SHA-256") {
                        return Err(Error::authentication(format!(
                            "server offered unsupported SASL mechanisms: {mechanisms:?}"
                        )));
                    }

                    // Start SCRAM-SHA-256 exchange
                    let (state, client_first) = auth::scram_client_first(password)?;
                    auth_state = Some(state);

                    frontend::sasl_initial_response(
                        "SCRAM-SHA-256",
                        &client_first,
                        &mut self.write_buf,
                    )?;
                    self.flush()?;
                }
                Message::AuthenticationSaslContinue(body) => {
                    let state = auth_state.take().ok_or_else(|| {
                        Error::authentication("received SASL continue without initial state")
                    })?;

                    let server_first = body.data();
                    let (new_state, client_final) = auth::scram_client_final(state, server_first)?;
                    auth_state = Some(new_state);

                    frontend::sasl_response(&client_final, &mut self.write_buf)?;
                    self.flush()?;
                }
                Message::AuthenticationSaslFinal(body) => {
                    let state = auth_state.take().ok_or_else(|| {
                        Error::authentication("received SASL final without state")
                    })?;

                    // Verify server signature
                    auth::scram_verify_server(state, body.data())?;
                }
                Message::BackendKeyData(data) => {
                    self.process_id = data.process_id();
                    self.secret_key = data.secret_key();
                }
                Message::ParameterStatus(body) => {
                    // Store server parameters
                    if let (Ok(name), Ok(value)) = (body.name(), body.value()) {
                        self.server_params
                            .insert(name.to_string(), value.to_string());
                    }
                }
                Message::ReadyForQuery(_) => {
                    // Connection is ready
                    return Ok(());
                }
                Message::ErrorResponse(body) => {
                    // Startup typically fails the connection outright, but we
                    // still drain in case the server sent any trailing
                    // messages so parity with the async startup path is
                    // preserved. The drain is bounded implicitly: a post-
                    // startup-error server either sends ReadyForQuery or
                    // closes immediately, so drain_until_ready returns fast.
                    return Err(self.consume_error(&body));
                }
                _ => {
                    return Err(Error::protocol("unexpected message during startup"));
                }
            }
        }
    }

    /// Sends a simple query and returns all messages until `ReadyForQuery`.
    ///
    /// # Errors
    ///
    /// - Returns [`Error`] (server) when the server sends an `ErrorResponse`
    ///   (SQL error, constraint violation, etc.).
    /// - Returns [`Error`] (I/O) on transport read/write failure.
    /// - Returns [`Error`] (closed) if the server closes the connection
    ///   mid-query.
    /// - Returns [`Error`] (connection) if the connection has already
    ///   been marked unhealthy by a prior failure.
    pub fn simple_query(&mut self, query: &str) -> Result<Vec<Message>> {
        self.ensure_healthy()?;
        frontend::query(query, &mut self.write_buf)?;
        self.flush()?;

        let mut messages = Vec::new();
        loop {
            let msg = self.read_message()?;
            match &msg {
                Message::ReadyForQuery(_) => {
                    messages.push(msg);
                    return Ok(messages);
                }
                Message::ErrorResponse(body) => {
                    return Err(self.consume_error(body));
                }
                _ => {
                    messages.push(msg);
                }
            }
        }
    }

    /// Sends a query using extended protocol with binary format results.
    ///
    /// This uses the `PostgreSQL` extended query protocol (Parse/Bind/Execute/Sync)
    /// with `HyperBinary` format (format code 2) for maximum performance.
    ///
    /// Returns all messages until `ReadyForQuery`.
    ///
    /// # Errors
    ///
    /// Same failure modes as [`Self::simple_query`] — server-side SQL
    /// errors surface as [`Error`] (server), transport failures as
    /// [`Error`] (I/O) / [`Error`] (closed), and an unhealthy prior state
    /// as [`Error`] (connection).
    pub fn query_binary(&mut self, query: &str) -> Result<Vec<Message>> {
        self.ensure_healthy()?;
        // HyperBinary format code
        const HYPER_BINARY_FORMAT: i16 = 2;

        // Parse: prepare an unnamed statement
        frontend::parse("", query, &[], &mut self.write_buf)?;

        // Bind: bind unnamed portal with HyperBinary result format
        // Empty arrays for param_formats and params (no parameters)
        // Single result_format of 2 (HyperBinary) applies to all columns
        frontend::bind(
            "",
            "",
            &[],
            &[],
            &[HYPER_BINARY_FORMAT],
            &mut self.write_buf,
        )?;

        // Describe: get column metadata (optional but useful)
        frontend::describe(b'P', "", &mut self.write_buf)?;

        // Execute: run the unnamed portal with no row limit
        frontend::execute("", 0, &mut self.write_buf)?;

        // Sync: end the extended query sequence
        frontend::sync(&mut self.write_buf);

        self.flush()?;

        let mut messages = Vec::new();
        loop {
            let msg = self.read_message()?;
            match &msg {
                Message::ReadyForQuery(_) => {
                    messages.push(msg);
                    return Ok(messages);
                }
                Message::ErrorResponse(body) => {
                    return Err(self.consume_error(body));
                }
                _ => {
                    messages.push(msg);
                }
            }
        }
    }

    /// Starts a binary query but leaves result consumption to the caller.
    ///
    /// This is useful for streaming scenarios where you want to pull messages
    /// incrementally instead of materializing the full result set up front.
    ///
    /// # Errors
    ///
    /// - Returns [`Error`] (connection) if the connection has been
    ///   marked unhealthy.
    /// - Returns [`Error`] (I/O) if writing the Parse/Bind/Execute/Sync
    ///   sequence to the transport fails.
    pub fn start_query_binary(&mut self, query: &str) -> Result<()> {
        self.ensure_healthy()?;
        const HYPER_BINARY_FORMAT: i16 = 2;

        frontend::parse("", query, &[], &mut self.write_buf)?;
        frontend::bind(
            "",
            "",
            &[],
            &[],
            &[HYPER_BINARY_FORMAT],
            &mut self.write_buf,
        )?;
        frontend::describe(b'P', "", &mut self.write_buf)?;
        frontend::execute("", 0, &mut self.write_buf)?;
        frontend::sync(&mut self.write_buf);

        self.flush()
    }

    /// Starts a simple query but leaves result consumption to the caller.
    ///
    /// This is useful for streaming scenarios where you want to pull messages
    /// incrementally instead of materializing the full result set up front.
    ///
    /// # Errors
    ///
    /// Same failure modes as [`Self::start_query_binary`] —
    /// [`Error`] (connection) for unhealthy state, [`Error`] (I/O) for
    /// transport failure.
    pub fn start_simple_query(&mut self, query: &str) -> Result<()> {
        self.ensure_healthy()?;
        frontend::query(query, &mut self.write_buf)?;
        self.flush()
    }

    /// Starts an **execute** of a prepared statement but leaves result
    /// consumption to the caller.
    ///
    /// Sends `Bind` + `Execute(unnamed_portal, 0)` + `Sync`, then
    /// returns. The caller drives a message loop that reads
    /// `BindComplete`, any `DataRow`s, then `CommandComplete` +
    /// `ReadyForQuery` — the same shape used by
    /// [`Self::start_query_binary`].
    ///
    /// Format codes (`PostgreSQL` wire protocol):
    /// - **Parameters**: format `1` (standard PG binary, big-endian).
    ///   Hyper's server-side Bind decodes bound parameters as standard
    ///   PG binary regardless of the format code we advertise. The
    ///   caller is responsible for supplying parameter bytes in BE.
    /// - **Results**: format `2` (`HyperBinary`, little-endian). Hyper
    ///   supports this as a separate protocol extension; the row
    ///   decoders in [`super::row::StreamRow`] and the hyperdb-api `Row`
    ///   type all expect LE, so requesting LE at Bind time avoids an
    ///   extra conversion pass.
    ///
    /// `max_rows = 0` means "send all rows" — we pace on the client side
    /// by reading `DataRows` in chunks from the read buffer.
    ///
    /// # Errors
    ///
    /// - Returns [`Error`] (connection) if the connection has been
    ///   marked unhealthy.
    /// - Returns [`Error`] (I/O) if writing the Bind/Execute/Sync
    ///   sequence to the transport fails.
    pub fn start_execute_prepared(
        &mut self,
        statement_name: &str,
        params: &[Option<&[u8]>],
        column_count: usize,
    ) -> Result<()> {
        self.ensure_healthy()?;

        const PG_BINARY_FORMAT: i16 = 1;
        const HYPER_BINARY_FORMAT: i16 = 2;
        let param_formats: Vec<i16> = vec![PG_BINARY_FORMAT; params.len()];
        let result_formats: Vec<i16> = vec![HYPER_BINARY_FORMAT; column_count];

        frontend::bind(
            "", // unnamed portal
            statement_name,
            &param_formats,
            params,
            &result_formats,
            &mut self.write_buf,
        )?;
        frontend::execute("", 0, &mut self.write_buf)?;
        frontend::sync(&mut self.write_buf);

        self.flush()
    }

    /// Reads a single message from the server.
    ///
    /// # Errors
    ///
    /// - Returns [`Error`] (I/O) if reading from the transport fails or
    ///   if [`Message::parse`] reports a malformed frame.
    /// - Returns [`Error`] (closed) when the transport reaches EOF
    ///   (server closed the connection).
    pub fn read_message(&mut self) -> Result<Message> {
        loop {
            if let Some(msg) = Message::parse(&mut self.read_buf).map_err(Error::io)? {
                return Ok(msg);
            }

            // Need more data — read directly into the spare capacity of
            // `read_buf`, no temporary buffer or `extend_from_slice` memcpy.
            //
            // The 64 KiB read-window matches the typical TCP loopback
            // segment size and the default DataRow streaming chunk. On
            // Windows TCP loopback the per-`WSARecv` overhead is several
            // times higher than Linux/macOS `recv`, so a tight ceiling here
            // dominates wall time on long scans. The previous 8 KiB ceiling
            // forced an 8× syscall amplification on this path.
            //
            // Implementation: `resize` extends the buffer with zeroed bytes
            // (single memset, ~50 GB/s on modern CPUs), `read` writes into
            // the new tail, then `truncate` shrinks to the actual byte
            // count. This is safe Rust and results in exactly one memset
            // per syscall — no heap alloc, no extra memcpy.
            let prev_len = self.read_buf.len();
            self.read_buf.resize(prev_len + 64 * 1024, 0);
            let n = self.stream.read(&mut self.read_buf[prev_len..])?;
            if n == 0 {
                self.read_buf.truncate(prev_len);
                warn!(target: "hyperdb_api", "connection-closed");
                return Err(Error::closed());
            }
            self.read_buf.truncate(prev_len + n);
        }
    }

    /// Drains messages from the server until a [`Message::ReadyForQuery`] is
    /// seen, discarding them. Call this after receiving an
    /// [`Message::ErrorResponse`] to stay in sync with the wire protocol.
    ///
    /// Per the `PostgreSQL` wire protocol, every query (simple or extended) ends
    /// with `ReadyForQuery`, even if the statement failed. Without this drain,
    /// the `ReadyForQuery` (and any other trailing messages) remain in the
    /// read buffer and get consumed by the next operation's response parser,
    /// which misinterprets them — classic wire desync.
    ///
    /// This is the **unbounded** variant, safe to use in the standard
    /// error path where the server has already sent `ErrorResponse` and
    /// `ReadyForQuery` is guaranteed to arrive within a few messages. In
    /// exceptional cases where the drain might take arbitrarily long —
    /// most notably the `Drop` path for a streaming result that the caller
    /// abandoned mid-way — prefer
    /// [`drain_until_ready_bounded`](Self::drain_until_ready_bounded) to
    /// avoid blocking indefinitely on an unresponsive server.
    ///
    /// Drain errors (connection already closed, I/O failure mid-drain) are
    /// logged via `tracing::warn!` and then swallowed. The caller's original
    /// error is more informative to surface, and a dead connection will be
    /// reported on the next real operation anyway.
    pub fn drain_until_ready(&mut self) {
        let _ = self.drain_until_ready_bounded(usize::MAX);
    }

    /// Bounded version of [`drain_until_ready`](Self::drain_until_ready) that
    /// stops after reading at most `max_messages` messages. Returns `true`
    /// when `ReadyForQuery` was observed within that budget; `false` if the
    /// budget was exhausted first or an I/O error occurred before reaching it.
    ///
    /// # Why we do not send `Sync` before draining
    ///
    /// A natural question is whether to send a `Sync` message first to prompt
    /// the server to emit `ReadyForQuery` sooner. The answer for Hyper is
    /// **no** — it would actively corrupt the next query.
    ///
    /// Per the Hyper server state machine (see `LibpqConnection::handleSync`
    /// and `handleQueryDone`), every query — simple or extended — already
    /// ends with exactly one `ReadyForQuery` emission. After an error or
    /// normal completion the server returns to its main loop. If we then
    /// send a `Sync`, `handleSync` would emit an **additional**
    /// `ReadyForQuery` that no current operation is reading, and the next
    /// query's response parser would consume that stale `ReadyForQuery`
    /// as its own terminator — the symptom is that query returning an
    /// empty result with "Query returned no rows".
    ///
    /// For the abandoned-stream case (a long-running query that the client
    /// stopped reading), `Sync` also does not help: Hyper processes the
    /// incoming byte stream in order, so `Sync` is only handled *after*
    /// the in-flight `Execute` finishes emitting all its `DataRow`s plus
    /// its own `CommandComplete` and `ReadyForQuery`. By that point the
    /// drain has already reached `ReadyForQuery`, and the `Sync` produces
    /// the same extra `ReadyForQuery` contamination described above.
    ///
    /// The canonical way to abort a running query is to open a *separate*
    /// connection and send `CancelRequest` with the original connection's
    /// process id and secret. That is exactly what
    /// [`QueryStream`](super::client::QueryStream)'s `Drop` impl does
    /// (via the [`Cancellable`](super::cancel::Cancellable) trait)
    /// before calling this bounded drain. Cancel-then-drain converges on
    /// `ReadyForQuery` within a handful of messages because the server
    /// stops producing new `DataRow`s once it observes the cancel.
    ///
    /// # Poisoned connections
    ///
    /// When this returns `false` the connection is in an indeterminate
    /// state. Callers should treat it as poisoned and not return it to a
    /// connection pool — the next operation will see residual bytes from
    /// whatever was still streaming. The bounded variant exists precisely
    /// to prevent indefinite blocking in contexts like `Drop` impls where
    /// we don't own the thread's time and can't afford to wait for a
    /// multi-million-row query result to finish before returning from a
    /// destructor.
    ///
    /// All drain errors are logged via `tracing::warn!` so state-related
    /// issues are observable in logs even though they don't interrupt the
    /// caller's control flow.
    pub fn drain_until_ready_bounded(&mut self, max_messages: usize) -> bool {
        for i in 0..max_messages {
            match self.read_message() {
                Ok(Message::ReadyForQuery(_)) => return true,
                Ok(_) => {}
                Err(e) => {
                    warn!(
                        target: "hyperdb_api_core::client",
                        error = %e,
                        messages_read = i,
                        "drain_until_ready: read error mid-drain (likely closed connection); \
                         connection marked desynchronized",
                    );
                    // Whether the underlying error is a closed socket, a
                    // partial read, or a corrupt frame, any subsequent
                    // `read_message` on this connection is operating on
                    // unknown state. Mark it so pool layers and upper APIs
                    // can short-circuit instead of piling another failed
                    // operation on top.
                    self.desynchronized = true;
                    return false;
                }
            }
        }
        warn!(
            target: "hyperdb_api_core::client",
            max_messages,
            "drain_until_ready_bounded: exhausted budget without seeing ReadyForQuery; \
             connection marked desynchronized and should not be reused",
        );
        // Budget exhausted — residual messages still on the wire. The
        // next read on this connection will almost certainly misparse
        // them as belonging to an unrelated operation. Mark it so the
        // failure surfaces at a well-defined API boundary instead.
        self.desynchronized = true;
        false
    }

    /// Convenience: parse a server [`Message::ErrorResponse`] body into an
    /// [`Error`] and drain the rest of the response through the trailing
    /// [`Message::ReadyForQuery`] so the connection is safe to reuse.
    ///
    /// Callers should almost always prefer this over calling
    /// [`drain_until_ready`](Self::drain_until_ready) or
    /// [`drain_until_ready_bounded`](Self::drain_until_ready_bounded) by
    /// hand, because forgetting the drain is exactly the bug it exists to
    /// prevent.
    ///
    /// # Drain budget
    ///
    /// Uses a bounded drain with a [`POST_ERROR_DRAIN_CAP`]-message budget
    /// rather than the unbounded [`drain_until_ready`](Self::drain_until_ready).
    /// A well-behaved server emits only a handful of messages after
    /// `ErrorResponse` before `ReadyForQuery` — typically just the error
    /// itself plus the `Z`, occasionally with a few `NoticeResponse`
    /// messages interleaved — so the cap is orders of magnitude above
    /// anything a legitimate error path produces. The cap exists purely
    /// as a defensive safety valve against pathological server behavior
    /// (a broken backend that never emits `ReadyForQuery`) and
    /// misbehaved network paths (stalled reads that would otherwise hang
    /// the caller indefinitely, particularly visible in async contexts).
    ///
    /// If the cap is exceeded, `drain_until_ready_bounded` logs a
    /// `tracing::warn!` and marks the connection desynchronized; the
    /// next operation on it will surface a transport-level failure and
    /// trigger reconnect higher up. That is strictly better than
    /// blocking forever with no observable symptom.
    ///
    /// # Example
    ///
    /// ```ignore
    /// match msg {
    ///     Message::ErrorResponse(body) => {
    ///         return Err(self.consume_error(&body));
    ///     }
    ///     // ...
    /// }
    /// ```
    pub fn consume_error(
        &mut self,
        body: &crate::protocol::message::backend::ErrorResponseBody,
    ) -> Error {
        let err = parse_error_response(body);
        let _ = self.drain_until_ready_bounded(POST_ERROR_DRAIN_CAP);
        err
    }

    /// Flushes the write buffer to the server.
    ///
    /// # Errors
    ///
    /// Returns [`Error`] (I/O) if writing the buffered bytes or flushing
    /// the underlying transport fails.
    pub fn flush(&mut self) -> Result<()> {
        if !self.write_buf.is_empty() {
            self.stream.write_all(&self.write_buf)?;
            self.stream.flush()?;
            self.write_buf.clear();
        }
        Ok(())
    }

    /// Sends a terminate message and closes the connection.
    ///
    /// # Errors
    ///
    /// Returns [`Error`] (I/O) if writing the `Terminate` frame or
    /// flushing the transport fails.
    pub fn terminate(&mut self) -> Result<()> {
        frontend::terminate(&mut self.write_buf);
        self.flush()
    }

    /// Returns a mutable reference to the write buffer.
    pub fn write_buf(&mut self) -> &mut BytesMut {
        &mut self.write_buf
    }

    /// Initiates a COPY IN operation with `HyperBinary` format.
    ///
    /// This sends a COPY ... FROM STDIN query and waits for `CopyInResponse`.
    /// After this returns successfully, the caller should send data using
    /// `send_copy_data` and then call `finish_copy` or `cancel_copy`.
    ///
    /// # Errors
    ///
    /// Same failure modes as [`Self::start_copy_in_with_format`].
    pub fn start_copy_in(&mut self, table_name: &str, columns: &[&str]) -> Result<()> {
        self.start_copy_in_with_format(table_name, columns, "HYPERBINARY")
    }

    /// Initiates a COPY IN operation with a specified format.
    ///
    /// This sends a COPY ... FROM STDIN query and waits for `CopyInResponse`.
    /// After this returns successfully, the caller should send data using
    /// `send_copy_data` and then call `finish_copy` or `cancel_copy`.
    ///
    /// # Arguments
    ///
    /// * `table_name` - The target table name (should be properly quoted if needed)
    /// * `columns` - Column names to insert into
    /// * `format` - The data format string: "HYPERBINARY" or "ARROWSTREAM"
    ///
    /// # Example
    ///
    /// ```no_run
    /// # use hyperdb_api_core::client::connection::RawConnection;
    /// # use std::net::TcpStream;
    /// # fn example(conn: &mut RawConnection<TcpStream>) -> hyperdb_api_core::client::Result<()> {
    /// // For HyperBinary format (default)
    /// conn.start_copy_in_with_format("my_table", &["col1", "col2"], "HYPERBINARY")?;
    ///
    /// // For Arrow IPC stream format
    /// conn.start_copy_in_with_format("my_table", &["col1", "col2"], "ARROWSTREAM")?;
    /// # Ok(())
    /// # }
    /// ```
    ///
    /// # Errors
    ///
    /// - Returns [`Error`] (connection) if the connection has been
    ///   marked unhealthy by a prior failure.
    /// - Returns [`Error`] (server) if the server rejects the generated
    ///   `COPY ... FROM STDIN` statement (missing table, column
    ///   mismatch, etc.).
    /// - Returns [`Error`] (I/O) on wire-protocol I/O failure.
    pub fn start_copy_in_with_format(
        &mut self,
        table_name: &str,
        columns: &[&str],
        format: &str,
    ) -> Result<()> {
        self.ensure_healthy()?;
        // Build COPY command with specified format
        let column_list = if columns.is_empty() {
            String::new()
        } else {
            format!(
                " ({})",
                columns
                    .iter()
                    .map(|c| format!("\"{}\"", c.replace('"', "\"\"")))
                    .collect::<Vec<_>>()
                    .join(", ")
            )
        };

        let query = format!("COPY {table_name}{column_list} FROM STDIN WITH (FORMAT {format})");

        frontend::query(&query, &mut self.write_buf)?;
        self.flush()?;

        // Wait for CopyInResponse
        loop {
            let msg = self.read_message()?;
            match msg {
                Message::CopyInResponse(_) => {
                    // Ready to receive data
                    return Ok(());
                }
                Message::ErrorResponse(body) => {
                    return Err(self.consume_error(&body));
                }
                _ => {
                    // Ignore other messages (like NoticeResponse)
                }
            }
        }
    }

    /// Initiates a COPY IN operation from a raw SQL query string.
    ///
    /// The query must be a complete `COPY ... FROM STDIN ...` statement.
    ///
    /// # Errors
    ///
    /// Same failure modes as [`Self::start_copy_in_with_format`]: unhealthy
    /// connection, server-side SQL rejection, or transport I/O failure.
    pub fn start_copy_in_raw(&mut self, query: &str) -> Result<()> {
        self.ensure_healthy()?;
        frontend::query(query, &mut self.write_buf)?;
        self.flush()?;

        loop {
            let msg = self.read_message()?;
            match msg {
                Message::CopyInResponse(_) => {
                    return Ok(());
                }
                Message::ErrorResponse(body) => {
                    return Err(self.consume_error(&body));
                }
                _ => {}
            }
        }
    }

    /// Sends COPY data to the server.
    ///
    /// The data should be in `HyperBinary` format.
    ///
    /// # Errors
    ///
    /// Currently infallible — frame construction is pure. The `Result`
    /// return type is preserved for forward compatibility.
    pub fn send_copy_data(&mut self, data: &[u8]) -> Result<()> {
        frontend::copy_data(data, &mut self.write_buf);
        // Don't flush immediately for better batching
        // Caller can call flush() explicitly if needed
        Ok(())
    }

    /// Sends COPY data directly to the stream without internal buffering.
    ///
    /// This writes the `CopyData` message directly to the TCP stream, letting
    /// the kernel's TCP stack handle buffering. Use `flush_stream()` periodically
    /// to ensure data is sent.
    ///
    /// This is more efficient for streaming large amounts of data as it avoids
    /// copying data into an intermediate buffer.
    ///
    /// # Errors
    ///
    /// - Returns [`Error`] (protocol) if `data.len() + 4` exceeds
    ///   `u32::MAX` (the PostgreSQL per-message length cap).
    /// - Returns [`Error`] (I/O) if flushing buffered bytes or writing
    ///   the header/payload directly to the transport fails.
    pub fn send_copy_data_direct(&mut self, data: &[u8]) -> Result<()> {
        // First flush any pending buffered data
        if !self.write_buf.is_empty() {
            self.stream.write_all(&self.write_buf)?;
            self.write_buf.clear();
        }

        // Write CopyData message header + data directly to stream
        // Message format: 'd' (1 byte) + length (4 bytes BigEndian) + data
        let msg_len = u32::try_from(4 + data.len())
            .map_err(|_| Error::protocol("CopyData payload exceeds u32::MAX bytes"))?;
        let len_be = msg_len.to_be_bytes();
        let header = [b'd', len_be[0], len_be[1], len_be[2], len_be[3]];
        self.stream.write_all(&header)?;
        self.stream.write_all(data)?;
        Ok(())
    }

    /// Flushes the TCP stream without clearing the write buffer.
    ///
    /// Use this with `send_copy_data_direct()` to periodically ensure
    /// data is sent to the server.
    ///
    /// # Errors
    ///
    /// Returns [`Error`] (I/O) if flushing the underlying transport
    /// fails.
    pub fn flush_stream(&mut self) -> Result<()> {
        self.stream.flush()?;
        Ok(())
    }

    /// Finishes a COPY IN operation successfully.
    ///
    /// This sends `CopyDone` and waits for `CommandComplete`.
    /// Returns the number of rows inserted.
    ///
    /// # Errors
    ///
    /// - Returns [`Error`] (server) if the server emits an `ErrorResponse`
    ///   during finalization (e.g. constraint violation from the
    ///   accumulated rows).
    /// - Returns [`Error`] (I/O) on wire-protocol read/write failure.
    pub fn finish_copy(&mut self) -> Result<u64> {
        // Ensure all data is sent
        self.flush()?;

        // Send CopyDone
        frontend::copy_done(&mut self.write_buf);
        self.flush()?;

        // Wait for CommandComplete and ReadyForQuery
        let mut row_count = 0u64;
        loop {
            let msg = self.read_message()?;
            match msg {
                Message::CommandComplete(body) => {
                    if let Ok(tag) = body.tag() {
                        // Parse row count from tag like "COPY 1234"
                        if let Some(count_str) = tag.strip_prefix("COPY ") {
                            if let Ok(count) = count_str.trim().parse() {
                                row_count = count;
                            }
                        }
                    }
                }
                Message::ReadyForQuery(_) => {
                    return Ok(row_count);
                }
                Message::ErrorResponse(body) => {
                    return Err(self.consume_error(&body));
                }
                _ => {
                    // Ignore other messages
                }
            }
        }
    }

    /// Cancels a COPY IN operation.
    ///
    /// This sends `CopyFail` and waits for the error response.
    ///
    /// # Errors
    ///
    /// Returns [`Error`] (I/O) if flushing the buffer or writing the
    /// `CopyFail` frame fails, or [`Error`] (closed) if the server
    /// drops the connection before returning `ReadyForQuery`.
    pub fn cancel_copy(&mut self, reason: &str) -> Result<()> {
        // Ensure buffer is clear
        self.flush()?;

        // Send CopyFail
        frontend::copy_fail(reason, &mut self.write_buf);
        self.flush()?;

        // Wait for ErrorResponse and ReadyForQuery
        loop {
            let msg = self.read_message()?;
            match msg {
                Message::ReadyForQuery(_) => {
                    return Ok(());
                }
                Message::ErrorResponse(_) => {
                    // Expected - the server confirms the cancel
                }
                _ => {
                    // Ignore other messages
                }
            }
        }
    }

    /// Executes a COPY ... TO STDOUT query and returns all output data.
    ///
    /// This is used for queries like:
    /// `COPY (SELECT ...) TO STDOUT WITH (format arrowstream)`
    ///
    /// The method:
    /// 1. Sends the query
    /// 2. Waits for `CopyOutResponse`
    /// 3. Collects all `CopyData` messages
    /// 4. Waits for `CopyDone`, `CommandComplete`, and `ReadyForQuery`
    ///
    /// # Arguments
    ///
    /// * `query` - The COPY TO STDOUT query to execute
    ///
    /// # Returns
    ///
    /// The raw bytes from all `CopyData` messages concatenated together.
    ///
    /// # Example
    ///
    /// ```no_run
    /// # use hyperdb_api_core::client::connection::RawConnection;
    /// # use std::net::TcpStream;
    /// # fn example(conn: &mut RawConnection<TcpStream>) -> hyperdb_api_core::client::Result<()> {
    /// let arrow_data = conn.copy_out(
    ///     "COPY (SELECT * FROM my_table) TO STDOUT WITH (format arrowstream)"
    /// )?;
    /// # Ok(())
    /// # }
    /// ```
    ///
    /// # Errors
    ///
    /// - Returns [`Error`] (connection) if the connection has been
    ///   marked unhealthy.
    /// - Returns [`Error`] (server) when the server rejects the COPY TO
    ///   STDOUT statement via `ErrorResponse`.
    /// - Returns [`Error`] (I/O) / [`Error`] (closed) on transport
    ///   read/write failure.
    pub fn copy_out(&mut self, query: &str) -> Result<Vec<u8>> {
        self.ensure_healthy()?;
        // Send the query
        frontend::query(query, &mut self.write_buf)?;
        self.flush()?;

        let mut data = Vec::new();
        let mut in_copy_out = false;

        // Process messages
        loop {
            let msg = self.read_message()?;
            match msg {
                Message::CopyOutResponse(_) => {
                    // Server is ready to send COPY data
                    in_copy_out = true;
                }
                Message::CopyData(body) if in_copy_out => {
                    // Accumulate the copy data
                    data.extend_from_slice(body.data());
                }
                Message::CopyDone => {
                    // COPY data transfer complete
                    in_copy_out = false;
                }
                Message::CommandComplete(_) => {
                    // Command finished
                }
                Message::ReadyForQuery(_) => {
                    // Connection is ready for next command
                    return Ok(data);
                }
                Message::ErrorResponse(body) => {
                    return Err(self.consume_error(&body));
                }
                _ => {
                    // Ignore other messages (like NoticeResponse)
                }
            }
        }
    }

    /// Streams COPY OUT data directly to a writer without buffering all data in memory.
    ///
    /// Returns the total number of bytes written.
    ///
    /// # Errors
    ///
    /// Same failure modes as [`Self::copy_out`], plus [`Error`] (I/O)
    /// wrapping any error from `writer.write_all` when the target
    /// writer cannot accept a COPY chunk.
    pub fn copy_out_to_writer(
        &mut self,
        query: &str,
        writer: &mut dyn std::io::Write,
    ) -> Result<u64> {
        self.ensure_healthy()?;
        frontend::query(query, &mut self.write_buf)?;
        self.flush()?;

        let mut total_bytes: u64 = 0;
        let mut in_copy_out = false;

        loop {
            let msg = self.read_message()?;
            match msg {
                Message::CopyOutResponse(_) => {
                    in_copy_out = true;
                }
                Message::CopyData(body) if in_copy_out => {
                    let chunk = body.data();
                    writer.write_all(chunk).map_err(|e| {
                        Error::new(
                            super::error::ErrorKind::Io,
                            format!("Failed to write COPY data: {e}"),
                        )
                    })?;
                    total_bytes += chunk.len() as u64;
                }
                Message::CopyDone => {
                    in_copy_out = false;
                }
                Message::CommandComplete(_) => {}
                Message::ReadyForQuery(_) => {
                    return Ok(total_bytes);
                }
                Message::ErrorResponse(body) => {
                    return Err(self.consume_error(&body));
                }
                _ => {}
            }
        }
    }
}

/// Parses an error response into an Error.
pub(crate) fn parse_error_response(
    body: &crate::protocol::message::backend::ErrorResponseBody,
) -> Error {
    let mut severity = String::from("ERROR");
    let mut code = String::from("00000");
    let mut message = String::from("unknown error");

    for field in body.fields().filter_map(|r| {
        r.map_err(|e| trace!(target: "hyperdb_api_core::client", error = %e, "dropped error parsing error response field")).ok()
    }) {
        match field.type_() {
            b'S' | b'V' => {
                if let Ok(s) = field.value() {
                    severity = s.to_string();
                }
            }
            b'C' => {
                if let Ok(s) = field.value() {
                    code = s.to_string();
                }
            }
            b'M' => {
                if let Ok(s) = field.value() {
                    message = s.to_string();
                }
            }
            _ => {}
        }
    }

    Error::db(&severity, &code, &message)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Cursor;

    /// Minimal `Read + Write` harness. Discards all writes and hands back
    /// empty reads so we can construct a `RawConnection` without touching
    /// the network. Adequate for exercising pure-state logic like the
    /// `desynchronized` flag and `ensure_healthy` — not for anything that
    /// actually needs a live server response.
    struct NullStream;
    impl std::io::Read for NullStream {
        fn read(&mut self, _: &mut [u8]) -> std::io::Result<usize> {
            Ok(0)
        }
    }
    impl std::io::Write for NullStream {
        fn write(&mut self, buf: &[u8]) -> std::io::Result<usize> {
            Ok(buf.len())
        }
        fn flush(&mut self) -> std::io::Result<()> {
            Ok(())
        }
    }

    /// Fresh connections are healthy and `ensure_healthy` is a no-op.
    #[test]
    fn fresh_connection_is_healthy() {
        let conn = RawConnection::new(NullStream);
        assert!(conn.is_healthy());
        assert!(conn.ensure_healthy().is_ok());
    }

    /// Once `desynchronized` is set, `is_healthy` reports false and
    /// `ensure_healthy` returns a `Connection`-kind error whose message
    /// explicitly names the desync so log consumers can grep for it.
    #[test]
    fn desynchronized_connection_fails_health_check() {
        let mut conn = RawConnection::new(NullStream);
        conn.desynchronized = true;
        assert!(!conn.is_healthy());
        let err = conn.ensure_healthy().expect_err("must fail-fast");
        assert_eq!(err.kind(), crate::client::error::ErrorKind::Connection);
        assert!(
            err.to_string().to_lowercase().contains("desynchron"),
            "error message should mention desynchronization; got: {err}",
        );
    }

    /// `drain_until_ready_bounded` with budget `0` returns false without
    /// reading anything, and marks the connection desynchronized. This is
    /// the cheapest way to exercise the "budget exhausted" code path
    /// without a live protocol stream. Uses a `Cursor` over an empty
    /// buffer so the underlying stream is well-defined.
    #[test]
    fn zero_budget_drain_marks_desynchronized() {
        let mut conn = RawConnection::new(Cursor::new(Vec::<u8>::new()));
        assert!(conn.is_healthy());
        let ok = conn.drain_until_ready_bounded(0);
        assert!(!ok, "zero-budget drain must return false");
        assert!(
            !conn.is_healthy(),
            "drain failure must mark connection desynchronized",
        );
    }

    /// Once desynchronized, the main public request APIs all fail-fast
    /// with the `ensure_healthy` error instead of sending bytes into a
    /// known-poisoned wire. Spot-check one sync query method here; the
    /// check itself (`self.ensure_healthy()?`) is a trivial first line
    /// at every entry point so extending coverage to every API wouldn't
    /// catch additional bug classes.
    #[test]
    fn desynchronized_connection_fast_fails_simple_query() {
        let mut conn = RawConnection::new(Cursor::new(Vec::<u8>::new()));
        conn.desynchronized = true;
        // `Message` doesn't implement `Debug`, so we can't use
        // `expect_err`; match the result directly instead.
        let Err(err) = conn.simple_query("SELECT 1") else {
            panic!("desynced simple_query must fail-fast")
        };
        assert_eq!(err.kind(), crate::client::error::ErrorKind::Connection);
        assert!(err.to_string().to_lowercase().contains("desynchron"));
    }
}
