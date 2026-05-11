// Copyright (c) 2026, Salesforce, Inc. All rights reserved.
// SPDX-License-Identifier: Apache-2.0 OR MIT

use std::sync::{Arc, Mutex};

use napi::bindgen_prelude::*;
use napi_derive::napi;
use tokio::sync::mpsc;

use crate::result::{extract_row, ResultColumnInfo, RowData};

/// A chunk result sent from the background task.
type ChunkResult = std::result::Result<Vec<RowData>, String>;

// =============================================================================
// QueryStream
// =============================================================================

/// A streaming query result for memory-efficient iteration over large result sets.
///
/// Rows are fetched in chunks on demand — only one chunk is buffered at a time.
///
/// ## Chunk-level iteration (high performance)
///
/// ```js
/// const stream = await conn.executeQueryStream('SELECT * FROM big_table');
/// let chunk;
/// while ((chunk = await stream.nextChunk()) !== null) {
///   for (const row of chunk) {
///     console.log(row.getInt32(0));
///   }
/// }
/// ```
///
/// ## Row-level async iteration (convenient)
///
/// ```js
/// const stream = await conn.executeQueryStream('SELECT * FROM big_table');
/// for await (const row of stream) {
///   console.log(row.getInt32(0));
/// }
/// ```
#[napi]
#[derive(Debug)]
pub struct QueryStream {
    receiver: Mutex<Option<mpsc::Receiver<ChunkResult>>>,
    /// Shared with the background task that populates it after the first chunk.
    schema: Arc<Mutex<Option<Vec<ResultColumnInfo>>>>,
}

#[napi]
impl QueryStream {
    /// Returns the next chunk of rows, or `null` when all rows have been consumed.
    #[napi]
    pub async fn next_chunk(&self) -> Result<Option<Vec<RowData>>> {
        let mut rx = {
            let mut guard = self
                .receiver
                .lock()
                .map_err(|e| Error::from_reason(format!("Lock poisoned: {e}")))?;
            match guard.take() {
                Some(rx) => rx,
                None => return Ok(None),
            }
        };

        let result = rx.recv().await;

        match result {
            Some(Ok(rows)) => {
                let mut guard = self
                    .receiver
                    .lock()
                    .map_err(|e| Error::from_reason(format!("Lock poisoned: {e}")))?;
                *guard = Some(rx);
                Ok(Some(rows))
            }
            Some(Err(e)) => Err(Error::from_reason(e)),
            None => Ok(None),
        }
    }

    /// Returns column metadata for this result set (populated after the first chunk).
    #[napi]
    pub fn get_schema(&self) -> Result<Option<Vec<ResultColumnInfo>>> {
        let guard = self
            .schema
            .lock()
            .map_err(|e| Error::from_reason(format!("Lock poisoned: {e}")))?;
        Ok(guard.clone())
    }

    /// Cancels the stream, releasing the background reader task.
    ///
    /// After calling this, `nextChunk()` returns `null`. Call this when you
    /// no longer need the remaining rows to free resources immediately
    /// rather than waiting for garbage collection.
    #[napi]
    pub fn cancel(&self) -> Result<()> {
        let mut guard = self
            .receiver
            .lock()
            .map_err(|e| Error::from_reason(format!("Lock poisoned: {e}")))?;
        *guard = None;
        Ok(())
    }
}

/// Spawns an async tokio task that reads chunks from the connection and
/// pushes them through a bounded channel. Backpressure comes from the
/// channel capacity — slow JS consumers block the producer automatically.
pub(crate) fn start_query_stream(
    conn: Arc<hyperdb_api::AsyncConnection>,
    sql: String,
) -> QueryStream {
    let (tx, rx) = mpsc::channel::<ChunkResult>(2);
    let schema_holder: Arc<Mutex<Option<Vec<ResultColumnInfo>>>> = Arc::new(Mutex::new(None));
    let schema_for_stream = Arc::clone(&schema_holder);

    // napi-rs 3: use `napi::bindgen_prelude::spawn` so the future runs on
    // the napi-managed tokio runtime. Raw `tokio::spawn` panics with
    // "there is no reactor running" because this function is invoked from
    // a synchronous napi callback that doesn't have a current runtime.
    napi::bindgen_prelude::spawn(async move {
        let mut rowset = match conn.execute_query(&sql).await {
            Ok(r) => r,
            Err(e) => {
                let _ = tx.send(Err(e.to_string())).await;
                return;
            }
        };

        let mut schema: Option<hyperdb_api::ResultSchema> = None;

        loop {
            let chunk = match rowset.next_chunk().await {
                Ok(Some(chunk)) => chunk,
                Ok(None) => break,
                Err(e) => {
                    let _ = tx.send(Err(e.to_string())).await;
                    return;
                }
            };

            if schema.is_none() {
                schema = rowset.schema();
                if let Some(ref s) = schema {
                    let info: Vec<ResultColumnInfo> = s
                        .columns()
                        .iter()
                        .map(|col| ResultColumnInfo {
                            name: col.name().to_string(),
                            type_name: col.sql_type().to_string(),
                            // Column count in a result schema is structurally
                            // bounded by Hyper (far below u32::MAX).
                            index: u32::try_from(col.index()).unwrap_or(u32::MAX),
                        })
                        .collect();
                    if let Ok(mut guard) = schema_holder.lock() {
                        *guard = Some(info);
                    }
                }
            }

            let Some(s) = schema.as_ref() else {
                let _ = tx.send(Err("No schema available".to_string())).await;
                return;
            };

            let rows: Vec<RowData> = chunk
                .iter()
                .map(|row| RowData {
                    values: extract_row(row, s),
                })
                .collect();

            if tx.send(Ok(rows)).await.is_err() {
                // JS consumer dropped the stream
                return;
            }
        }
    });

    QueryStream {
        receiver: Mutex::new(Some(rx)),
        schema: schema_for_stream,
    }
}
