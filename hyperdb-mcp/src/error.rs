// Copyright (c) 2026, Salesforce, Inc. All rights reserved.
// SPDX-License-Identifier: Apache-2.0 OR MIT

//! Structured error types for MCP tool responses.
//!
//! Every error carries a machine-readable [`ErrorCode`] and a human-readable message.
//! Most codes also get a default `suggestion` string that helps LLMs self-correct
//! without a round-trip to the user.

use serde::Serialize;

/// Machine-readable error codes returned in MCP tool error responses.
///
/// Serialized as `SCREAMING_SNAKE_CASE` (e.g. `HYPERD_NOT_FOUND`) so LLM clients
/// can pattern-match without parsing prose.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "SCREAMING_SNAKE_CASE")]
pub enum ErrorCode {
    /// The `hyperd` binary was not found at `HYPERD_PATH` or on `PATH`.
    HyperdNotFound,
    /// A file path argument points to a nonexistent or unreadable file.
    FileNotFound,
    /// The file extension or explicit format string is not one we handle.
    UnsupportedFormat,
    /// Data doesn't match the inferred or user-provided schema.
    SchemaMismatch,
    /// SQL syntax error or reference to a nonexistent column.
    SqlError,
    /// A referenced table does not exist in the workspace.
    TableNotFound,
    /// The input data is empty (zero rows or zero columns).
    EmptyData,
    /// Workspace disk is full or write quota exceeded.
    DiskFull,
    /// Filesystem permission denied on a source or target path.
    PermissionDenied,
    /// Server is running in read-only mode and the requested operation would mutate state.
    ReadOnlyViolation,
    /// The connection to `hyperd` was lost (crash, broken pipe, EOF) or
    /// the wire protocol fell out of sync (a bounded drain exhausted
    /// without reaching `ReadyForQuery`, surfacing as a
    /// `"desynchronized"` error message from the `hyper-client` layer).
    /// Either way, the connection is unusable and the MCP server will
    /// automatically tear down the [`crate::engine::Engine`] and
    /// reconnect on the next call.
    ConnectionLost,
    /// A tool argument is malformed or violates a precondition that the
    /// caller can fix (bad alias shape, wrong mode string, reserved name,
    /// etc.). Distinct from [`Self::SchemaMismatch`] in that the argument
    /// itself is wrong, not the data it refers to.
    InvalidArgument,
    /// A resource (typically a `.hyper` file) is held by another process
    /// and cannot be opened exclusively. Surfaced when `ATTACH DATABASE`
    /// fails because another MCP server or `hyperd` owns the file.
    ResourceBusy,
    /// Catch-all for unexpected failures (panics, I/O, lock poisoning).
    InternalError,
}

/// An error type designed for MCP tool responses.
///
/// Serializes to JSON with `code`, `message`, and an optional `suggestion`.
/// The suggestion is aimed at the LLM caller — it tells the model how to retry
/// or what parameter to fix, reducing round-trips.
#[derive(Debug, Clone, Serialize)]
pub struct McpError {
    pub code: ErrorCode,
    pub message: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub suggestion: Option<String>,
}

impl McpError {
    /// Create an error with an auto-generated suggestion based on the error code.
    pub fn new(code: ErrorCode, message: impl Into<String>) -> Self {
        let message = message.into();
        let suggestion = default_suggestion(code, &message);
        Self {
            code,
            message,
            suggestion,
        }
    }

    #[must_use]
    /// Override the default suggestion with a context-specific one.
    pub fn with_suggestion(mut self, suggestion: impl Into<String>) -> Self {
        self.suggestion = Some(suggestion.into());
        self
    }
}

impl std::fmt::Display for McpError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "[{:?}] {}", self.code, self.message)
    }
}

impl std::error::Error for McpError {}

/// Returns a recovery hint for each error code. These are intentionally
/// phrased as instructions so an LLM can act on them directly.
fn default_suggestion(code: ErrorCode, _message: &str) -> Option<String> {
    match code {
        ErrorCode::HyperdNotFound => Some("Set HYPERD_PATH environment variable or ensure hyperd is on PATH".into()),
        ErrorCode::FileNotFound => Some("Verify the file path exists and is accessible".into()),
        ErrorCode::UnsupportedFormat => Some("Specify format explicitly: json, csv, parquet, or arrow_ipc".into()),
        ErrorCode::SchemaMismatch => Some("Retry with an explicit schema override".into()),
        ErrorCode::SqlError => Some("Check SQL syntax. Hyper uses the Data Cloud SQL dialect (PostgreSQL-compatible).".into()),
        ErrorCode::TableNotFound => Some("Use the describe tool to list available tables".into()),
        ErrorCode::EmptyData => None,
        ErrorCode::DiskFull => Some("Check disk space. Use the status tool to see workspace size.".into()),
        ErrorCode::PermissionDenied => Some("Check file permissions on the source or target path".into()),
        ErrorCode::ReadOnlyViolation => Some("Server is in read-only mode. Use query_data or query_file for one-shot analysis, or restart without --read-only.".into()),
        ErrorCode::ConnectionLost => Some("The hyperd connection was lost or fell out of wire-protocol sync. Retry the request — the server will tear down the engine and reconnect automatically.".into()),
        ErrorCode::InvalidArgument => Some("Check the tool argument shape and allowed values. The message identifies the offending field.".into()),
        ErrorCode::ResourceBusy => Some("The .hyper file is held by another process. Close the other MCP server (or hyperd instance) that owns it, or copy the file first and attach the copy.".into()),
        ErrorCode::InternalError => None,
    }
}

/// Converts a `hyperdb_api::Error` into an [`McpError`] by sniffing the message text
/// to pick the most specific error code. Falls back to [`ErrorCode::InternalError`].
impl From<hyperdb_api::Error> for McpError {
    fn from(err: hyperdb_api::Error) -> Self {
        let msg = err.to_string();
        let lower = msg.to_lowercase();
        if is_connection_lost(&msg) {
            McpError::new(ErrorCode::ConnectionLost, msg)
        } else if msg.contains("Multi-part queries") || msg.contains("0A000") {
            // Hyper only allows one statement per call. Rewrite the error
            // with an actionable suggestion instead of the cryptic code.
            McpError::new(ErrorCode::SqlError, msg)
                .with_suggestion("Hyper only accepts one SQL statement per call. Split your query into separate execute/query calls — one per statement.")
        } else if msg.contains("22003")
            || lower.contains("numeric overflow")
            || lower.contains("out of range")
        {
            // SQLSTATE 22003: numeric_value_out_of_range. Happens at COPY/INSERT
            // time when a value exceeds its column type. With the partial
            // name-keyed schema override now in place, widening a single column
            // is a one-line retry.
            McpError::new(ErrorCode::SchemaMismatch, msg).with_suggestion(
                "A numeric value exceeded its column's range. Retry with a partial schema override that widens the offending column, e.g. schema: {\"Population\": \"BIGINT\"} or {\"Amount\": \"NUMERIC(38,0)\"}. The override is a partial dictionary keyed by column name — unlisted columns keep their inferred type. Call inspect_file first if you don't know which column is too narrow.")
        } else if msg.contains("22P02") || lower.contains("invalid input syntax") {
            // SQLSTATE 22P02: invalid_text_representation. A value couldn't be
            // parsed into its column type (e.g. non-date string in a DATE
            // column). Usually the safe fix is to keep the column as TEXT and
            // cast later in SQL.
            McpError::new(ErrorCode::SchemaMismatch, msg).with_suggestion(
                "A value could not be parsed into its column type. Retry with a partial schema override forcing TEXT for the offending column, e.g. schema: {\"Id\": \"TEXT\"}, and cast in SQL as needed.")
        } else if msg.contains("syntax error")
            || (msg.contains("does not exist") && msg.contains("column"))
        {
            McpError::new(ErrorCode::SqlError, msg)
        } else if msg.contains("No such file") || msg.contains("not found") {
            McpError::new(ErrorCode::FileNotFound, msg)
        } else if is_resource_busy(&msg) {
            // ATTACH on a .hyper file already held by another hyperd
            // surfaces as a "database is in use" / "already attached" /
            // "could not lock" error. Route to ResourceBusy so the LLM
            // sees an actionable recovery hint instead of InternalError.
            McpError::new(ErrorCode::ResourceBusy, msg)
        } else {
            McpError::new(ErrorCode::InternalError, msg)
        }
    }
}

/// Classify an error message as one where the underlying connection is no
/// longer usable and the caller should recycle it. Used to decide whether
/// the [`crate::engine::Engine`] should be torn down and reinitialized
/// before the next call.
///
/// Covers two distinct failure modes:
///
/// 1. **Transport-level disappearance** — OS broken-pipe / reset / refused
///    plus the generic end-of-file and "connection closed" responses the
///    `PostgreSQL` client produces when `hyperd` crashes or is killed
///    mid-transaction.
///
/// 2. **Wire-protocol desync** — the `hyper-client` layer marks a
///    connection `desynchronized` when its bounded drain exhausts the
///    `POST_ERROR_DRAIN_CAP` budget without reaching `ReadyForQuery` or
///    hits an I/O error mid-drain. Subsequent operations on that
///    connection fast-fail with an
///    `ErrorKind::Connection` whose message contains `"desynchronized"`.
///    The socket is technically still open but the wire state is corrupt
///    and the only valid recovery is the same as #1: discard the
///    connection and reconnect. Recognizing the signal here is what
///    makes the mcp server's auto-reconnect path kick in for that case
///    instead of returning the drain-poisoned error to callers forever.
#[must_use]
pub fn is_connection_lost(msg: &str) -> bool {
    let lower = msg.to_lowercase();
    // Transport-level disappearance.
    lower.contains("broken pipe")
        || lower.contains("connection reset")
        || lower.contains("connection refused")
        || lower.contains("connection closed")
        || lower.contains("unexpected eof")
        || lower.contains("end of file")
        || lower.contains("unexpectedly closed")
        || lower.contains("socket is not connected")
        // Wire-protocol desync (see function-level comment).
        || lower.contains("desynchronized")
}

/// Classify a hyperd error message as "the file is already opened by
/// somebody else" so the registry can surface a clear
/// [`ErrorCode::ResourceBusy`] instead of a generic internal error.
/// Matches the wording hyperd uses when a `.hyper` file is locked or
/// already attached by another process.
fn is_resource_busy(msg: &str) -> bool {
    let lower = msg.to_lowercase();
    lower.contains("already attached")
        || lower.contains("database is in use")
        || lower.contains("could not lock")
        || lower.contains("already in use")
        || lower.contains("file is locked")
}
