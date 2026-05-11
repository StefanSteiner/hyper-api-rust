// Copyright (c) 2026, Salesforce, Inc. All rights reserved.
// SPDX-License-Identifier: Apache-2.0 OR MIT

//! Streaming async prepared-statement execution.
//!
//! Async mirror of [`PreparedQueryStream`](super::prepared_stream::PreparedQueryStream).
//! See that module's docs for the design rationale.

use std::sync::Arc;

use crate::protocol::message::backend::Message;
use tokio::sync::MutexGuard;
use tracing::warn;

use super::async_connection::AsyncRawConnection;
use super::async_stream::AsyncStream;
use super::cancel::Cancellable;
use super::connection::parse_error_response;
use super::error::Result;
use super::row::StreamRow;
use super::statement::Column;

/// Streaming iterator for prepared-statement results without materializing
/// all rows (async).
///
/// Holding an `AsyncPreparedQueryStream` keeps the underlying connection
/// locked via a `MutexGuard`. Dropping the stream before fully iterating
/// triggers a best-effort server-side cancel and marks the connection
/// desynchronized — same Drop semantics as
/// [`AsyncQueryStream`](super::async_stream_query::AsyncQueryStream).
pub struct AsyncPreparedQueryStream<'a> {
    conn: Option<MutexGuard<'a, AsyncRawConnection<AsyncStream>>>,
    canceller: &'a dyn Cancellable,
    finished: bool,
    chunk_size: usize,
    /// Columns captured at prepare time.
    columns: Arc<Vec<Column>>,
}

impl std::fmt::Debug for AsyncPreparedQueryStream<'_> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("AsyncPreparedQueryStream")
            .field("finished", &self.finished)
            .field("chunk_size", &self.chunk_size)
            .field("column_count", &self.columns.len())
            .finish_non_exhaustive()
    }
}

impl<'a> AsyncPreparedQueryStream<'a> {
    pub(crate) fn new(
        conn: MutexGuard<'a, AsyncRawConnection<AsyncStream>>,
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
                Message::BindComplete => {}
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

impl Drop for AsyncPreparedQueryStream<'_> {
    fn drop(&mut self) {
        if self.finished {
            return;
        }

        // Drop can't await; mark the connection desynchronized and issue
        // the sync CancelRequest on a fresh transport — same pattern as
        // AsyncQueryStream::drop.
        self.canceller.cancel();

        if let Some(ref mut conn) = self.conn {
            conn.mark_desynchronized();
            warn!(
                target: "hyperdb_api_core::client",
                "AsyncPreparedQueryStream dropped before completion; \
                 connection marked desynchronized — discard and reconnect",
            );
        }
    }
}
