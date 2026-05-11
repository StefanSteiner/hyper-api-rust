// Copyright (c) 2026, Salesforce, Inc. All rights reserved.
// SPDX-License-Identifier: Apache-2.0 OR MIT

//! Streaming async query results.
//!
//! This module is the async mirror of the sync [`QueryStream`](super::client::QueryStream):
//! it yields rows in chunks so callers can process arbitrarily large result
//! sets with constant memory. The lifetime parameter ties the stream to the
//! owning [`AsyncClient`](super::async_client::AsyncClient) — the connection
//! mutex is held for the duration of iteration.

use tokio::sync::MutexGuard;
use tracing::warn;

use crate::protocol::message::backend::Message;

use super::async_connection::AsyncRawConnection;
use super::async_stream::AsyncStream;
use super::cancel::Cancellable;
use super::connection::parse_error_response;
use super::error::Result;
use super::row::StreamRow;
use super::statement::{Column, ColumnFormat};

/// Streaming iterator for query results without materializing all rows (async).
///
/// Holding an `AsyncQueryStream` keeps the underlying
/// [`AsyncRawConnection`] locked via a `MutexGuard`. Dropping the stream
/// before fully iterating triggers a server-side cancel (see [`Drop`] below)
/// so the connection is returned to a usable state.
pub struct AsyncQueryStream<'a> {
    conn: Option<MutexGuard<'a, AsyncRawConnection<AsyncStream>>>,
    /// Best-effort cancel handle. For [`AsyncClient`](super::async_client::AsyncClient)
    /// this is a thin wrapper that opens a synchronous TCP/UDS/Named-Pipe
    /// connection and sends a `CancelRequest` — the same pattern used by the
    /// sync [`Client`](super::client::Client). Using a sync cancel path
    /// lets `Drop` issue cancels without needing a tokio runtime handle.
    canceller: &'a dyn Cancellable,
    finished: bool,
    chunk_size: usize,
    schema: Option<Vec<Column>>,
    schema_read: bool,
}

impl std::fmt::Debug for AsyncQueryStream<'_> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("AsyncQueryStream")
            .field("finished", &self.finished)
            .field("chunk_size", &self.chunk_size)
            .field("schema_read", &self.schema_read)
            .finish_non_exhaustive()
    }
}

impl<'a> AsyncQueryStream<'a> {
    /// Constructs a new streaming result reader. The caller is responsible
    /// for having already issued the query bytes (see
    /// [`AsyncRawConnection::start_query_binary`]).
    pub(crate) fn new(
        conn: MutexGuard<'a, AsyncRawConnection<AsyncStream>>,
        canceller: &'a dyn Cancellable,
        chunk_size: usize,
    ) -> Self {
        Self {
            conn: Some(conn),
            canceller,
            finished: false,
            chunk_size: chunk_size.max(1),
            schema: None,
            schema_read: false,
        }
    }

    /// Returns the schema (column metadata) for the result set, once the
    /// first `RowDescription` has been read.
    #[must_use]
    pub fn schema(&self) -> Option<&[Column]> {
        self.schema.as_deref()
    }

    /// Retrieves the next chunk of rows (up to `chunk_size`).
    ///
    /// Returns `Ok(Some(rows))` for each chunk, `Ok(None)` after the final
    /// `ReadyForQuery`, and `Err(_)` if the server sent an `ErrorResponse`.
    ///
    /// # Errors
    ///
    /// - Returns `Error` (I/O) / `Error` (closed) if the async
    ///   transport fails while awaiting the next protocol message.
    /// - Returns `Error` (server) when the server sends an
    ///   `ErrorResponse` partway through the result stream.
    pub async fn next_chunk(&mut self) -> Result<Option<Vec<StreamRow>>> {
        if self.finished {
            return Ok(None);
        }

        let Some(conn) = self.conn.as_mut() else {
            return Ok(None);
        };

        let mut rows = Vec::with_capacity(self.chunk_size);
        while rows.len() < self.chunk_size {
            let msg = conn.read_message().await?;
            match msg {
                Message::RowDescription(desc) if !self.schema_read => {
                    let mut cols = Vec::new();
                    for f in desc.fields().filter_map(std::result::Result::ok) {
                        cols.push(Column::new(
                            f.name().to_string(),
                            f.type_oid(),
                            f.type_modifier(),
                            ColumnFormat::from_code(f.format()),
                        ));
                    }
                    self.schema = Some(cols);
                    self.schema_read = true;
                }
                Message::DataRow(data) => {
                    rows.push(StreamRow::new(data));
                    if rows.len() >= self.chunk_size {
                        return Ok(Some(rows));
                    }
                }
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
                        Some(ref mut c) => c.consume_error(&body).await,
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

impl Drop for AsyncQueryStream<'_> {
    fn drop(&mut self) {
        // If the stream was fully consumed, the server already emitted
        // `ReadyForQuery` and the connection is clean — nothing to do.
        if self.finished {
            return;
        }

        // Otherwise: fire a best-effort cancel so the server stops producing
        // rows for a query we no longer care about. `Cancellable::cancel` is
        // implemented synchronously (short-lived fresh TCP/UDS connection),
        // so it is safe to call from `Drop` without a runtime handle.
        self.canceller.cancel();

        // We cannot drain the trailing `ErrorResponse(QueryCanceled) +
        // ReadyForQuery` here because draining requires `await`. Without the
        // drain the connection is left desynchronized. Mark it as such so
        // the next operation short-circuits with a clear error rather than
        // hanging or misinterpreting trailing messages.
        //
        // This is a deliberate trade-off: async `Drop` is constrained, and
        // spawning a drain task (which would need `tokio::runtime::Handle`)
        // is fragile when the handle is unavailable. Marking desynchronized
        // ensures the connection is discarded cleanly on next use.
        if let Some(ref mut conn) = self.conn {
            conn.mark_desynchronized();
            warn!(
                target: "hyperdb_api_core::client",
                "AsyncQueryStream dropped before completion; \
                 connection marked desynchronized — discard and reconnect",
            );
        }
    }
}
