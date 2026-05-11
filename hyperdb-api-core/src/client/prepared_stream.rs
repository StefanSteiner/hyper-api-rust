// Copyright (c) 2026, Salesforce, Inc. All rights reserved.
// SPDX-License-Identifier: Apache-2.0 OR MIT

//! Streaming prepared-statement execution.
//!
//! [`PreparedQueryStream`] is the analog of
//! [`QueryStream`](super::client::QueryStream) for prepared statements: it
//! drives the Bind/Execute/Sync response and yields rows in chunks so
//! callers can process arbitrarily large result sets with constant memory.
//!
//! Unlike `QueryStream`, the schema is known at construction time — it was
//! captured when the statement was prepared via Describe('S'), before any
//! rows are fetched. This makes `schema()` available *before* the first
//! `next_chunk` call.

use std::sync::{Arc, MutexGuard};

use crate::protocol::message::backend::Message;
use tracing::warn;

use super::cancel::Cancellable;
use super::connection::{parse_error_response, RawConnection};
use super::error::Result;
use super::row::StreamRow;
use super::statement::Column;
use super::sync_stream::SyncStream;

/// Bounded drain budget used by [`PreparedQueryStream::drop`] after
/// sending a best-effort cancel. Matches the value inlined by
/// [`QueryStream`](super::client::QueryStream) — see that type's `Drop`
/// impl for the rationale.
const POST_CANCEL_DRAIN_CAP: usize = 1024;

/// Streaming iterator for prepared-statement results without materializing
/// all rows.
///
/// Holding a `PreparedQueryStream` keeps the underlying connection locked
/// via a `MutexGuard`. Dropping the stream before fully iterating triggers
/// a server-side cancel (see [`Drop`] below) so the connection is returned
/// to a usable state.
pub struct PreparedQueryStream<'a> {
    conn: Option<MutexGuard<'a, RawConnection<SyncStream>>>,
    /// Best-effort cancel handle — see
    /// [`QueryStream`](super::client::QueryStream) for the full rationale.
    canceller: &'a dyn Cancellable,
    finished: bool,
    chunk_size: usize,
    /// Column metadata carried through from the statement's Describe pass.
    /// Shared with each [`StreamRow`](super::row::StreamRow) via `Arc` so
    /// schema-dependent getters have cheap access.
    columns: Arc<Vec<Column>>,
}

impl std::fmt::Debug for PreparedQueryStream<'_> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("PreparedQueryStream")
            .field("finished", &self.finished)
            .field("chunk_size", &self.chunk_size)
            .field("column_count", &self.columns.len())
            .finish_non_exhaustive()
    }
}

impl<'a> PreparedQueryStream<'a> {
    /// Constructs a new streaming reader. The caller must have already
    /// issued the Bind/Execute/Sync via
    /// [`RawConnection::start_execute_prepared`].
    pub(crate) fn new(
        conn: MutexGuard<'a, RawConnection<SyncStream>>,
        canceller: &'a dyn Cancellable,
        chunk_size: usize,
        columns: Arc<Vec<Column>>,
    ) -> Self {
        Self {
            conn: Some(conn),
            canceller,
            finished: false,
            chunk_size: chunk_size.max(1),
            columns,
        }
    }

    /// Returns the result schema (column metadata). Always available —
    /// the columns were captured at prepare time.
    #[must_use]
    pub fn schema(&self) -> &[Column] {
        &self.columns
    }

    /// Retrieves the next chunk of rows (up to `chunk_size`).
    ///
    /// Returns `Ok(Some(rows))` for each chunk, `Ok(None)` after the final
    /// `ReadyForQuery`, and `Err(_)` if the server sent an `ErrorResponse`.
    ///
    /// # Errors
    ///
    /// - Returns `Error` (I/O) / `Error` (closed) if the transport
    ///   fails while awaiting the next protocol message.
    /// - Returns `Error` (server) when the server sends an
    ///   `ErrorResponse` partway through the result stream.
    pub fn next_chunk(&mut self) -> Result<Option<Vec<StreamRow>>> {
        if self.finished {
            return Ok(None);
        }

        let Some(conn) = self.conn.as_mut() else {
            return Ok(None);
        };

        let mut rows = Vec::with_capacity(self.chunk_size);
        while rows.len() < self.chunk_size {
            let msg = conn.read_message()?;
            match msg {
                Message::BindComplete => {
                    // Bind succeeded — expected once at the start.
                }
                Message::DataRow(data) => {
                    rows.push(StreamRow::new(data));
                    if rows.len() >= self.chunk_size {
                        return Ok(Some(rows));
                    }
                }
                Message::CommandComplete(_) | Message::EmptyQueryResponse => {}
                Message::ReadyForQuery(_) => {
                    self.finished = true;
                    self.conn = None;
                    return if rows.is_empty() {
                        Ok(None)
                    } else {
                        Ok(Some(rows))
                    };
                }
                Message::ErrorResponse(body) => {
                    self.finished = true;
                    let err = match self.conn {
                        Some(ref mut c) => c.consume_error(&body),
                        None => parse_error_response(&body),
                    };
                    self.conn = None;
                    return Err(err);
                }
                _ => {}
            }
        }
        Ok(Some(rows))
    }
}

impl Drop for PreparedQueryStream<'_> {
    fn drop(&mut self) {
        if self.finished {
            return;
        }

        // Mirror QueryStream's drop: fire-and-forget cancel, then drain the
        // trailing `ErrorResponse(QueryCanceled) + ReadyForQuery` on a
        // bounded budget so the connection is returned to the pool cleanly.
        self.canceller.cancel();

        if let Some(ref mut conn) = self.conn {
            let _ok = conn.drain_until_ready_bounded(POST_CANCEL_DRAIN_CAP);
        }

        // Belt-and-braces in case the drain exceeds its budget: the drain
        // itself flips `desynchronized`, so a future call that sees the
        // connection will short-circuit with a clear error.
        if self.conn.as_ref().is_some_and(|c| !c.is_healthy()) {
            warn!(
                target: "hyperdb_api_core::client",
                "PreparedQueryStream dropped before completion and drain exceeded budget; \
                 connection marked desynchronized — discard and reconnect",
            );
        }
    }
}
