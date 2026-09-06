// Copyright (c) 2026, Salesforce, Inc. All rights reserved.
// SPDX-License-Identifier: Apache-2.0 OR MIT

//! Ingest Parquet and Arrow IPC files into Hyper tables.
//!
//! These are "Tier 1" (exact schema) ingest paths: types come directly from
//! the file's embedded schema, so there is no guessing.
//!
//! # Parquet path: native hyperd read
//!
//! For Parquet files we let hyperd read the file directly via the
//! `external(path, FORMAT => 'parquet')` table function. The MCP issues a
//! single `CREATE TABLE ... AS SELECT * FROM external(...)` (replace mode)
//! or `INSERT INTO ... SELECT * FROM external(...)` (append mode). This
//! pushes the entire read into hyperd's C++ Parquet reader — no Rust-side
//! Arrow decode, no per-row INSERTs, and native support for the full set of
//! compressions and encodings hyperd understands (including Snappy, which
//! isn't compiled into the Rust `parquet` crate in this build).
//!
//! Benchmarked at ~3.5M rows/sec on TPCH 1GB lineitem (6M rows / 360 MB
//! in ~1.7s) — roughly three orders of magnitude faster than the previous
//! row-by-row path.
//!
//! We still open the Parquet footer in Rust to infer column types — that's
//! cheap (footer-only metadata read) and lets us report the schema in
//! [`IngestResult`] and apply user-supplied schema overrides by wrapping
//! each overridden column in an explicit `::TYPE` cast inside the SELECT
//! projection. It is also where we reject the one column shape hyperd
//! cannot read at all — a physical `NullType` column — rather than relaying
//! the engine's "possibly corrupted file" 42804. See
//! `reject_null_type_columns`.
//!
//! # Arrow IPC path: binary COPY via `ArrowInserter`
//!
//! For Arrow IPC (Feather) files we use hyperdb-api's `ArrowInserter` /
//! `AsyncArrowInserter`, which push `RecordBatch`es straight through
//! Hyper's binary COPY protocol as an Arrow IPC Stream. No text encoding,
//! no per-row SQL — the batches we decode from the IPC File format are
//! re-framed as IPC Stream messages and sent wholesale to the server.
//!
//! User-supplied schema overrides are applied by pre-creating the target
//! table with the overridden column types and letting Hyper coerce the
//! incoming values on insert. This avoids a second Rust-side transform
//! pass over the data.

use crate::engine::Engine;
use crate::error::{ErrorCode, McpError};
use crate::ingest::{IngestOptions, IngestResult};
use crate::schema::ColumnSchema;
use crate::stats::{IngestStats, StatsTimer};
use arrow::datatypes::{DataType, Schema as ArrowSchema};
use arrow::record_batch::RecordBatch;
use hyperdb_api::AsyncConnection;
use std::path::Path;

/// Map an Arrow [`DataType`] to the corresponding Hyper SQL type name.
///
/// Unsigned integers are promoted to the next wider signed type (e.g.
/// `UInt32` → `BIGINT`) because Hyper has no unsigned integer types.
/// Unrecognized types fall back to `TEXT`.
///
/// [`DataType::Null`] is the one type that maps to something Hyper cannot
/// create: the literal `"NULL"`, which [`crate::schema::map_hyper_type`]
/// deliberately does not resolve. It used to fall through to `TEXT`, which
/// made `inspect_file` advertise a readable `TEXT` column on a file hyperd
/// rejects outright — and invited a `schema` override that cannot help. See
/// `reject_null_type_columns`.
fn arrow_type_to_hyper(dt: &DataType) -> String {
    match dt {
        DataType::Null => "NULL".into(),
        DataType::Boolean => "BOOL".into(),
        DataType::Int8 | DataType::Int16 => "SMALLINT".into(),
        DataType::Int32 | DataType::UInt16 => "INT".into(),
        DataType::Int64 | DataType::UInt32 => "BIGINT".into(),
        DataType::UInt64 => "BIGINT".into(),
        DataType::Float16 | DataType::Float32 => "DOUBLE PRECISION".into(),
        DataType::Float64 => "DOUBLE PRECISION".into(),
        DataType::Utf8 | DataType::LargeUtf8 => "TEXT".into(),
        DataType::Binary | DataType::LargeBinary => "BYTEA".into(),
        DataType::Date32 | DataType::Date64 => "DATE".into(),
        DataType::Time32(_) | DataType::Time64(_) => "TIME".into(),
        DataType::Timestamp(_, None) => "TIMESTAMP".into(),
        DataType::Timestamp(_, Some(_)) => "TIMESTAMPTZ".into(),
        DataType::Decimal128(p, s) | DataType::Decimal256(p, s) => {
            format!("NUMERIC({p}, {s})")
        }
        _ => "TEXT".into(),
    }
}

/// Convert an Arrow schema to our internal [`ColumnSchema`] representation,
/// preserving nullability from the Arrow field metadata.
#[must_use]
pub fn arrow_schema_to_columns(schema: &ArrowSchema) -> Vec<ColumnSchema> {
    schema
        .fields()
        .iter()
        .map(|f| ColumnSchema {
            name: f.name().clone(),
            hyper_type: arrow_type_to_hyper(f.data_type()),
            nullable: f.is_nullable(),
        })
        .collect()
}

/// Reject an exact-tier file that carries a physically `NullType` column,
/// naming the offenders, before any SQL is issued.
///
/// hyperd cannot decode a `NullType` column and rejects the **whole file**
/// with SQLSTATE 42804 ("a data type that cannot be read by Hyper ...
/// hinting at a corrupted file"). That error is doubly unhelpful: it points
/// at file corruption when the file is fine, and it arrives no matter what
/// the caller does. Verified against the pinned engine:
///
/// * Selecting only the *other* columns still fails with 42804 — the reader
///   rejects the file before it evaluates a projection.
/// * A `schema` override does not rescue it either. The override is applied
///   (it becomes a `::TYPE` cast in the projection, see
///   `parquet_select_projection`), but the engine never gets far enough to
///   run the cast.
///
/// Since nothing the caller can pass fixes the file, the only real remedy is
/// upstream: give the column a concrete type in whatever wrote it. Files
/// like this are common in the wild — an optional column that happens to be
/// entirely null in one partition gets written as `NullType`.
fn reject_null_type_columns(schema: &ArrowSchema, file_kind: &str) -> Result<(), McpError> {
    let offenders: Vec<&str> = schema
        .fields()
        .iter()
        .filter(|f| f.data_type() == &DataType::Null)
        .map(|f| f.name().as_str())
        .collect();
    if offenders.is_empty() {
        return Ok(());
    }
    Err(McpError::new(
        ErrorCode::UnsupportedFormat,
        format!(
            "{file_kind} file has column(s) stored with the physical NullType: \
             {offenders:?}. Hyper cannot read a NullType column and rejects the \
             entire file (SQLSTATE 42804), even when you select only the other \
             columns. A `schema` override cannot fix this — the override becomes \
             a cast in the projection, which the engine never evaluates. Re-type \
             the column where the file is written (e.g. cast it to DOUBLE) and \
             regenerate the file."
        ),
    )
    .with_suggestion(
        "Re-type the all-null column(s) at the writer — e.g. cast to DOUBLE in the \
         job that produced this file — then retry. Neither a `schema` override nor \
         selecting a subset of columns works around it.",
    ))
}

/// Serialize a slice of Arrow `RecordBatch`es to a single Arrow IPC Stream
/// byte buffer. Used to feed the async inserter, which accepts raw IPC
/// Stream bytes (not `RecordBatch` objects). A well-formed stream is
/// schema message + one or more batch messages, in that order.
fn record_batches_to_ipc_stream(batches: &[RecordBatch]) -> Result<Vec<u8>, McpError> {
    let schema = batches
        .first()
        .map(arrow::array::RecordBatch::schema)
        .ok_or_else(|| McpError::new(ErrorCode::EmptyData, "Arrow IPC file has no batches"))?;
    let mut buf = Vec::new();
    {
        let mut writer =
            arrow::ipc::writer::StreamWriter::try_new(&mut buf, &schema).map_err(|e| {
                McpError::new(
                    ErrorCode::InternalError,
                    format!("Failed to create Arrow IPC StreamWriter: {e}"),
                )
            })?;
        for batch in batches {
            writer.write(batch).map_err(|e| {
                McpError::new(
                    ErrorCode::InternalError,
                    format!("Failed to write Arrow batch: {e}"),
                )
            })?;
        }
        writer.finish().map_err(|e| {
            McpError::new(
                ErrorCode::InternalError,
                format!("Failed to finish Arrow IPC stream: {e}"),
            )
        })?;
    }
    Ok(buf)
}

/// Read only the Parquet footer and return the inferred column list.
///
/// Hyperd will re-read the file when ingesting, so this pass is just for
/// reporting the schema back to the caller and resolving schema overrides
/// against real column names. It's cheap — the parquet crate only loads
/// footer metadata, not row groups.
///
/// The footer is also where we catch physically `NullType` columns, which
/// hyperd cannot read — see `reject_null_type_columns`. Failing here means
/// every ingest path that goes through this function (`load_file`,
/// `load_files`, `query_file`, the directory watcher) stops before issuing
/// SQL instead of relaying the engine's misleading 42804.
fn infer_parquet_schema(path: &str) -> Result<Vec<ColumnSchema>, McpError> {
    let file = std::fs::File::open(path)
        .map_err(|e| McpError::new(ErrorCode::FileNotFound, format!("Cannot open file: {e}")))?;
    let reader = parquet::arrow::arrow_reader::ParquetRecordBatchReaderBuilder::try_new(file)
        .map_err(|e| {
            McpError::new(
                ErrorCode::UnsupportedFormat,
                format!("Invalid Parquet file: {e}"),
            )
        })?;
    let arrow_schema = reader.schema();
    reject_null_type_columns(arrow_schema, "Parquet")?;
    Ok(arrow_schema_to_columns(arrow_schema))
}

/// Build the projection list for `SELECT <projection> FROM external(...)`.
///
/// When a user supplies a schema override, the overridden columns are
/// wrapped in a `::TYPE AS name` cast so the target table receives the
/// narrowed/widened type. Non-overridden columns pass through by name.
/// Column names are quoted to tolerate case, spaces, and reserved words.
fn parquet_select_projection(inferred: &[ColumnSchema], final_columns: &[ColumnSchema]) -> String {
    inferred
        .iter()
        .zip(final_columns.iter())
        .map(|(orig, col)| {
            let quoted = format!("\"{}\"", col.name.replace('"', "\"\""));
            if orig.hyper_type == col.hyper_type {
                quoted
            } else {
                format!("{quoted}::{ty} AS {quoted}", ty = col.hyper_type)
            }
        })
        .collect::<Vec<_>>()
        .join(", ")
}

/// Build the SQL statement hyperd will run to ingest the Parquet file.
///
/// - Replace mode: `CREATE TABLE "t" AS SELECT <proj> FROM external(...)`.
/// - Append mode: `INSERT INTO "t" SELECT <proj> FROM external(...)`.
///
/// The caller is responsible for issuing a preceding `DROP TABLE IF EXISTS`
/// in replace mode — hyperd's `CREATE TABLE AS` fails rather than replacing.
fn build_parquet_ingest_sql(
    table: &str,
    path: &str,
    projection: &str,
    is_replace: bool,
    target_db: Option<&str>,
) -> String {
    let quoted_table = match target_db {
        Some(db) => {
            let esc_db = db.replace('"', "\"\"");
            let esc_tbl = table.replace('"', "\"\"");
            format!("\"{esc_db}\".\"public\".\"{esc_tbl}\"")
        }
        None => format!("\"{}\"", table.replace('"', "\"\"")),
    };
    let quoted_path = hyperdb_api::escape_string_literal(path);
    if is_replace {
        format!(
            "CREATE TABLE {quoted_table} AS SELECT {projection} FROM external({quoted_path}, FORMAT => 'parquet')"
        )
    } else {
        format!(
            "INSERT INTO {quoted_table} SELECT {projection} FROM external({quoted_path}, FORMAT => 'parquet')"
        )
    }
}

/// Resolve the absolute ingest path, verifying the file exists.
///
/// Hyperd reads the Parquet file directly, so the path it receives must be
/// unambiguous — we canonicalize it here rather than passing whatever the
/// caller provided. A non-existent file fails fast with a clear error
/// before we ever reach the server.
fn resolve_parquet_path(path: &str) -> Result<(String, u64), McpError> {
    let file_path = Path::new(path);
    if !file_path.exists() {
        return Err(McpError::new(
            ErrorCode::FileNotFound,
            format!("File not found: {path}"),
        ));
    }
    let absolute = std::fs::canonicalize(file_path)
        .map_err(|e| {
            McpError::new(
                ErrorCode::FileNotFound,
                format!("Cannot resolve path {path}: {e}"),
            )
        })?
        .to_string_lossy()
        .into_owned();
    let file_size = std::fs::metadata(file_path).map_or(0, |m| m.len());
    Ok((absolute, file_size))
}

/// Count rows in a newly-created or freshly-appended table. Used for
/// reporting after `CREATE TABLE AS`, which doesn't surface an affected
/// row count the way `INSERT` does.
fn count_rows_sync(engine: &Engine, table: &str) -> Result<u64, McpError> {
    let quoted = format!("\"{}\"", table.replace('"', "\"\""));
    let sql = format!("SELECT COUNT(*) FROM {quoted}");
    let rows = engine.execute_query_to_json(&sql)?;
    rows.first()
        .and_then(|r| r.get("count"))
        .and_then(serde_json::Value::as_u64)
        .ok_or_else(|| {
            McpError::new(
                ErrorCode::InternalError,
                "Could not read row count after parquet ingest",
            )
        })
}

/// Async twin of `count_rows_sync`.
async fn count_rows_async(conn: &AsyncConnection, table: &str) -> Result<u64, McpError> {
    let quoted = format!("\"{}\"", table.replace('"', "\"\""));
    let sql = format!("SELECT COUNT(*) FROM {quoted}");
    let row = conn.fetch_one(&sql).await.map_err(McpError::from)?;
    let count: i64 = row.get(0).ok_or_else(|| {
        McpError::new(
            ErrorCode::InternalError,
            "Could not read row count after parquet ingest",
        )
    })?;
    // A negative row count from COUNT(*) indicates a catalog inconsistency;
    // fall back to 0 rather than wrapping into a huge u64.
    Ok(u64::try_from(count).unwrap_or(0))
}

/// Ingest a Parquet file into a Hyper table using hyperd's native parquet
/// reader via `CREATE TABLE ... AS SELECT * FROM external(...)`.
///
/// This is one SQL round-trip; hyperd opens the file itself and writes the
/// data straight into the target table. No Rust-side Arrow decode, no
/// per-row INSERTs.
///
/// # Errors
///
/// - Propagates errors from `resolve_parquet_path` when `path` is
///   missing or not canonicalizable.
/// - Propagates errors from `infer_parquet_schema` (malformed footer, or a
///   physically `NullType` column hyperd cannot read — see
///   `reject_null_type_columns`) or [`crate::schema::apply_schema_override`].
/// - Propagates any transaction error from the `CREATE TABLE AS
///   SELECT` / `INSERT INTO ... SELECT` statement against hyperd.
/// - Propagates any error from the post-ingest `COUNT(*)` in
///   `count_rows_sync` when running in replace mode.
pub fn ingest_parquet_file(
    engine: &Engine,
    path: &str,
    opts: &IngestOptions,
) -> Result<IngestResult, McpError> {
    if opts.mode == "merge" {
        return crate::ingest::merge_via_temp_table(engine, opts, |tmp_opts| {
            ingest_parquet_file(engine, path, tmp_opts)
        });
    }
    let timer = StatsTimer::start();
    let (absolute_path, file_size) = resolve_parquet_path(path)?;

    let inferred = infer_parquet_schema(&absolute_path)?;
    let final_columns = match &opts.schema_override {
        Some(s) => crate::schema::apply_schema_override(inferred.clone(), s)?,
        None => inferred.clone(),
    };
    let projection = parquet_select_projection(&inferred, &final_columns);

    let is_replace = opts.mode != "append";
    let sql = build_parquet_ingest_sql(
        &opts.table,
        &absolute_path,
        &projection,
        is_replace,
        opts.target_db.as_deref(),
    );

    // Issue the ingest DDL/DML. `CREATE TABLE ... AS SELECT` auto-commits
    // (Hyper treats all DDL that way), so wrapping it in `execute_in_transaction`
    // no longer buys us rollback — but it still gives us a clean error
    // path that runs the transaction prelude + drops if needed.
    let affected = engine.execute_in_transaction(|engine| {
        if is_replace {
            let qualified = crate::ingest::qualified_table(opts);
            engine.execute_command(&format!("DROP TABLE IF EXISTS {qualified}"))?;
        }
        engine.execute_command(&sql)
    })?;

    // Row count: `CREATE TABLE AS` reports 0 affected, so for replace mode
    // we have to COUNT(*). Critically, the COUNT must run *outside* the
    // transaction above — when issued immediately after CTAS on the same
    // connection inside a transaction we've observed the count coming back
    // truncated (the low 17 bits of the true value). Running it as a fresh
    // statement avoids whatever wire-state quirk is at play. For append
    // mode `INSERT INTO ... SELECT` does report affected rows, so we use
    // those directly.
    let row_count = if is_replace {
        count_rows_sync(engine, &opts.table)?
    } else {
        affected
    };

    let elapsed = timer.elapsed_ms();
    let stats = IngestStats {
        operation: "load_file".into(),
        rows: row_count,
        elapsed_ms: elapsed,
        bytes_read: file_size,
        bytes_stored: 0,
        schema_inference_ms: Some(0),
        table: opts.table.clone(),
        file_format: Some("parquet".into()),
        warning: None,
        schema_changed: false,
    };

    Ok(IngestResult {
        rows: row_count,
        schema: final_columns,
        stats,
    })
}

/// Async twin of [`ingest_parquet_file`]. Uses an owned [`AsyncConnection`]
/// so it can run on a pooled connection from the watcher without touching
/// the engine's primary sync connection. Footer metadata is read on the
/// blocking pool (tiny read, but file I/O).
///
/// # Errors
///
/// - Returns [`ErrorCode::InternalError`] if the spawn-blocking task
///   panics (surfaced as a join error).
/// - Propagates the same errors as [`ingest_parquet_file`]:
///   path resolution, schema inference, transaction/ingest, and row
///   counting failures.
pub async fn ingest_parquet_file_async(
    conn: &mut AsyncConnection,
    path: &str,
    opts: &IngestOptions,
) -> Result<IngestResult, McpError> {
    let timer = StatsTimer::start();
    let (absolute_path, file_size) = resolve_parquet_path(path)?;

    let path_for_infer = absolute_path.clone();
    let override_owned = opts.schema_override.clone();
    let (inferred, final_columns): (Vec<ColumnSchema>, Vec<ColumnSchema>) =
        tokio::task::spawn_blocking(move || -> Result<_, McpError> {
            let inferred = infer_parquet_schema(&path_for_infer)?;
            let final_columns = match &override_owned {
                Some(s) => crate::schema::apply_schema_override(inferred.clone(), s)?,
                None => inferred.clone(),
            };
            Ok((inferred, final_columns))
        })
        .await
        .map_err(|e| McpError::new(ErrorCode::InternalError, format!("Task join error: {e}")))??;

    let projection = parquet_select_projection(&inferred, &final_columns);
    let is_replace = opts.mode != "append";
    let sql = build_parquet_ingest_sql(
        &opts.table,
        &absolute_path,
        &projection,
        is_replace,
        opts.target_db.as_deref(),
    );

    let txn = conn.transaction().await.map_err(McpError::from)?;
    let result: Result<u64, McpError> = async {
        if is_replace {
            let qualified = crate::ingest::qualified_table(opts);
            txn.execute_command(&format!("DROP TABLE IF EXISTS {qualified}"))
                .await
                .map_err(McpError::from)?;
        }
        txn.execute_command(&sql).await.map_err(McpError::from)
    }
    .await;

    let affected = match result {
        Ok(n) => {
            txn.commit().await.map_err(McpError::from)?;
            n
        }
        Err(e) => {
            if let Err(rb) = txn.rollback().await {
                tracing::warn!("rollback after error failed: {}", rb);
            }
            return Err(e);
        }
    };

    // The transaction was committed above (or the function returned on
    // error). This COUNT(*) runs on the bare connection — outside any
    // transaction — to mirror the sync path's source-of-truth behavior
    // and avoid any post-CTAS wire-state quirks that the sync path
    // documents.
    let row_count = if is_replace {
        count_rows_async(conn, &opts.table).await?
    } else {
        affected
    };

    let elapsed = timer.elapsed_ms();
    let stats = IngestStats {
        operation: "load_file".into(),
        rows: row_count,
        elapsed_ms: elapsed,
        bytes_read: file_size,
        bytes_stored: 0,
        schema_inference_ms: Some(0),
        table: opts.table.clone(),
        file_format: Some("parquet".into()),
        warning: None,
        schema_changed: false,
    };

    Ok(IngestResult {
        rows: row_count,
        schema: final_columns,
        stats,
    })
}

/// Reject schema overrides on the Arrow IPC ingest path.
///
/// The binary COPY Arrow protocol is schema-strict: [`hyperdb_api::ArrowInserter`]
/// validates that each incoming column's Arrow type matches the target
/// column's Hyper type exactly, and Hyper rejects any mismatch with
/// SQLSTATE 42804. Since Arrow IPC files carry exact typed schemas by
/// construction, an override would almost always be a user error — so we
/// surface it with a clear message rather than failing later at Hyper's
/// type check. If a genuine override is ever needed here, add a
/// Rust-side `arrow::compute::cast` pass over the batches.
fn reject_ipc_schema_override(opts: &IngestOptions) -> Result<(), McpError> {
    if opts.schema_override.is_some() {
        return Err(McpError::new(
            ErrorCode::SchemaMismatch,
            "Schema overrides are not supported for Arrow IPC files. \
             The embedded Arrow schema is authoritative on this path \
             because the binary COPY protocol requires an exact type \
             match between the file and the target table.",
        ));
    }
    Ok(())
}

/// Magic prefix written at the start of an Arrow IPC **File** format
/// (Feather v2). See the Arrow spec §IPC File Format. The Stream format
/// does not start with this marker.
const ARROW_IPC_FILE_MAGIC: &[u8] = b"ARROW1";

/// Open an Arrow IPC file (either File/Feather format or Stream format)
/// and return the inferred column schema plus the decoded batches. Used
/// by both the sync and async ingest paths.
///
/// We auto-detect the sub-format by peeking the first 6 bytes: the File
/// format opens with `ARROW1`, the Stream format does not. This matters
/// because hyperdb-mcp's own export path emits raw IPC Stream bytes —
/// the same shape Hyper's wire protocol speaks — so round-tripping
/// `export → load_file` would otherwise fail on the Stream side with
/// "Arrow file does not contain correct footer". Accepting both also
/// matches what external producers emit: pyarrow's `new_file` writes
/// File format, `new_stream` writes Stream format, and both are common.
fn read_arrow_ipc_file(path: &str) -> Result<(Vec<ColumnSchema>, Vec<RecordBatch>), McpError> {
    use std::io::Read;

    let mut file = std::fs::File::open(path)
        .map_err(|e| McpError::new(ErrorCode::FileNotFound, format!("Cannot open file: {e}")))?;

    let mut magic = [0u8; 6];
    let read = file.read(&mut magic).map_err(|e| {
        McpError::new(
            ErrorCode::UnsupportedFormat,
            format!("Cannot read Arrow IPC header: {e}"),
        )
    })?;
    // Rewind so whichever reader we pick sees the full file.
    use std::io::Seek;
    file.rewind().map_err(|e| {
        McpError::new(
            ErrorCode::InternalError,
            format!("Cannot rewind file handle: {e}"),
        )
    })?;

    let is_file_format = read == 6 && magic == ARROW_IPC_FILE_MAGIC;

    if is_file_format {
        let reader = arrow::ipc::reader::FileReader::try_new(file, None).map_err(|e| {
            McpError::new(
                ErrorCode::UnsupportedFormat,
                format!("Invalid Arrow IPC file: {e}"),
            )
        })?;
        let arrow_schema = reader.schema();
        reject_null_type_columns(&arrow_schema, "Arrow IPC")?;
        let inferred = arrow_schema_to_columns(&arrow_schema);
        let batches: Vec<RecordBatch> = reader.collect::<Result<Vec<_>, _>>().map_err(|e| {
            McpError::new(
                ErrorCode::InternalError,
                format!("Arrow IPC read error: {e}"),
            )
        })?;
        Ok((inferred, batches))
    } else {
        let reader = arrow::ipc::reader::StreamReader::try_new(file, None).map_err(|e| {
            McpError::new(
                ErrorCode::UnsupportedFormat,
                format!("Invalid Arrow IPC stream: {e}"),
            )
        })?;
        let arrow_schema = reader.schema();
        reject_null_type_columns(&arrow_schema, "Arrow IPC")?;
        let inferred = arrow_schema_to_columns(&arrow_schema);
        let batches: Vec<RecordBatch> = reader.collect::<Result<Vec<_>, _>>().map_err(|e| {
            McpError::new(
                ErrorCode::InternalError,
                format!("Arrow IPC read error: {e}"),
            )
        })?;
        Ok((inferred, batches))
    }
}

/// Ingest an Arrow IPC (Feather) file into a Hyper table via the binary
/// COPY protocol.
///
/// The file is read with `FileReader` to get `RecordBatch`es, then streamed
/// through [`hyperdb_api::ArrowInserter`] — which re-frames each batch as an
/// Arrow IPC Stream message and pushes it over COPY. No per-row SQL, no
/// text encoding.
///
/// Schema overrides are rejected on this path — see
/// `reject_ipc_schema_override` for the reasoning.
///
/// # Errors
///
/// - Propagates [`ErrorCode::InvalidArgument`] from
///   `reject_ipc_schema_override` when `opts.schema_override` is set.
/// - Returns [`ErrorCode::FileNotFound`] if `path` does not exist.
/// - Returns [`ErrorCode::UnsupportedFormat`] when the file cannot be
///   parsed as an Arrow IPC file or stream, or carries a physically
///   `NullType` column hyperd cannot read (see `read_arrow_ipc_file` and
///   `reject_null_type_columns`), and [`ErrorCode::InternalError`] when a
///   batch fails to decode.
/// - Propagates transaction errors from [`Engine::create_table`] and
///   from [`hyperdb_api::ArrowInserter`] operations (COPY setup, batch
///   insert, or execute).
pub fn ingest_arrow_ipc_file(
    engine: &Engine,
    path: &str,
    opts: &IngestOptions,
) -> Result<IngestResult, McpError> {
    if opts.mode == "merge" {
        return crate::ingest::merge_via_temp_table(engine, opts, |tmp_opts| {
            ingest_arrow_ipc_file(engine, path, tmp_opts)
        });
    }
    let timer = StatsTimer::start();
    reject_ipc_schema_override(opts)?;

    let file_path = Path::new(path);
    if !file_path.exists() {
        return Err(McpError::new(
            ErrorCode::FileNotFound,
            format!("File not found: {path}"),
        ));
    }
    let file_size = std::fs::metadata(file_path).map_or(0, |m| m.len());

    let (columns, batches) = read_arrow_ipc_file(path)?;

    let is_replace = opts.mode != "append";
    // Arrow IPC uses the binary COPY protocol which resolves table names via
    // the search path. When targeting a non-primary database, temporarily
    // redirect the search path for the duration of the transaction.
    let _search_guard = if let Some(ref db) = opts.target_db {
        Some(engine.scoped_search_path(db)?)
    } else {
        None
    };
    let row_count = engine.execute_in_transaction(|engine| {
        engine.create_table_in(&opts.table, &columns, is_replace, opts.target_db.as_deref())?;

        // Stream RecordBatches through the binary COPY protocol. Each
        // batch is written to an IPC Stream segment internally — no
        // text encoding, no per-row SQL.
        let mut inserter =
            hyperdb_api::ArrowInserter::from_table(engine.connection(), opts.table.as_str())
                .map_err(McpError::from)?;
        inserter
            .insert_batches(batches.iter())
            .map_err(McpError::from)?;
        inserter.execute().map_err(McpError::from)
    })?;

    let elapsed = timer.elapsed_ms();
    let stats = IngestStats {
        operation: "load_file".into(),
        rows: row_count,
        elapsed_ms: elapsed,
        bytes_read: file_size,
        bytes_stored: 0,
        schema_inference_ms: Some(0),
        table: opts.table.clone(),
        file_format: Some("arrow_ipc".into()),
        warning: None,
        schema_changed: false,
    };

    Ok(IngestResult {
        rows: row_count,
        schema: columns,
        stats,
    })
}

/// Async twin of [`ingest_arrow_ipc_file`]. File I/O and Arrow decode run
/// on the blocking pool; the COPY stream is driven on the async connection.
/// [`hyperdb_api::AsyncArrowInserter`] accepts raw IPC Stream bytes, so we
/// serialize the decoded batches into one buffer and send it in a single
/// `insert_data` call.
///
/// # Errors
///
/// - Propagates [`ErrorCode::InvalidArgument`] from
///   `reject_ipc_schema_override` when a schema override is supplied.
/// - Returns [`ErrorCode::FileNotFound`] if `path` does not exist.
/// - Returns [`ErrorCode::InternalError`] if the spawn-blocking Arrow
///   decode task panics (join error).
/// - Propagates Arrow decode errors from `read_arrow_ipc_file` /
///   `record_batches_to_ipc_stream`, including the
///   [`ErrorCode::UnsupportedFormat`] rejection of a physically `NullType`
///   column (see `reject_null_type_columns`).
/// - Propagates transaction errors from the async `CREATE TABLE` and
///   from [`hyperdb_api::AsyncArrowInserter`] operations.
pub async fn ingest_arrow_ipc_file_async(
    conn: &mut AsyncConnection,
    path: &str,
    opts: &IngestOptions,
) -> Result<IngestResult, McpError> {
    let timer = StatsTimer::start();
    reject_ipc_schema_override(opts)?;

    let file_path = Path::new(path);
    if !file_path.exists() {
        return Err(McpError::new(
            ErrorCode::FileNotFound,
            format!("File not found: {path}"),
        ));
    }
    let file_size = std::fs::metadata(file_path).map_or(0, |m| m.len());

    let path_owned = path.to_string();
    let (columns, ipc_stream): (Vec<ColumnSchema>, Vec<u8>) =
        tokio::task::spawn_blocking(move || -> Result<_, McpError> {
            let (columns, batches) = read_arrow_ipc_file(&path_owned)?;
            let ipc_stream = record_batches_to_ipc_stream(&batches)?;
            Ok((columns, ipc_stream))
        })
        .await
        .map_err(|e| McpError::new(ErrorCode::InternalError, format!("Task join error: {e}")))??;

    let is_replace = opts.mode != "append";
    let table_def = crate::schema::build_table_def(&opts.table, &columns)?;

    let txn = conn.transaction().await.map_err(McpError::from)?;
    let result: Result<u64, McpError> = async {
        crate::ingest::create_table_async(
            txn.connection(),
            &opts.table,
            &columns,
            is_replace,
            opts.target_db.as_deref(),
        )
        .await?;

        // AsyncArrowInserter resolves table names via the connection's
        // search path. When targeting a non-primary DB, qualify the
        // TableDefinition with that database. (Same trick the sync
        // Arrow IPC path uses via scoped_search_path.)
        let mut inserter = hyperdb_api::AsyncArrowInserter::new(txn.connection(), &table_def)
            .map_err(McpError::from)?;
        inserter
            .insert_data(&ipc_stream)
            .await
            .map_err(McpError::from)?;
        inserter.execute().await.map_err(McpError::from)
    }
    .await;

    let row_count = match result {
        Ok(n) => {
            txn.commit().await.map_err(McpError::from)?;
            n
        }
        Err(e) => {
            if let Err(rb) = txn.rollback().await {
                tracing::warn!("rollback after error failed: {}", rb);
            }
            return Err(e);
        }
    };

    let elapsed = timer.elapsed_ms();
    let stats = IngestStats {
        operation: "load_file".into(),
        rows: row_count,
        elapsed_ms: elapsed,
        bytes_read: file_size,
        bytes_stored: 0,
        schema_inference_ms: Some(0),
        table: opts.table.clone(),
        file_format: Some("arrow_ipc".into()),
        warning: None,
        schema_changed: false,
    };

    Ok(IngestResult {
        rows: row_count,
        schema: columns,
        stats,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use arrow::array::{ArrayRef, Int32Array, NullArray};
    use arrow::datatypes::{Field, Schema};
    use std::sync::Arc;

    /// Write a one-batch Parquet file from `(name, nullable, array)` triples
    /// into `dir` and return its path.
    fn write_parquet(dir: &tempfile::TempDir, cols: Vec<(&str, bool, ArrayRef)>) -> String {
        let schema = Arc::new(Schema::new(
            cols.iter()
                .map(|(name, nullable, array)| {
                    Field::new(*name, array.data_type().clone(), *nullable)
                })
                .collect::<Vec<_>>(),
        ));
        let batch = RecordBatch::try_new(
            Arc::clone(&schema),
            cols.into_iter().map(|(_, _, array)| array).collect(),
        )
        .expect("build record batch");
        let path = dir.path().join("t.parquet");
        let file = std::fs::File::create(&path).expect("create parquet");
        let mut writer =
            parquet::arrow::ArrowWriter::try_new(file, schema, None).expect("open writer");
        writer.write(&batch).expect("write batch");
        writer.close().expect("close writer");
        path.to_string_lossy().into_owned()
    }

    /// A physical `NullType` column must be reported as `NULL`, not `TEXT`.
    /// `TEXT` was a lie: it promised a readable column on a file hyperd
    /// refuses outright, and `inspect_file`'s output is documented as safe to
    /// feed back as a `schema` override.
    #[test]
    fn null_arrow_type_maps_to_null_not_text() {
        assert_eq!(arrow_type_to_hyper(&DataType::Null), "NULL");

        let schema = Schema::new(vec![
            Field::new("a", DataType::Int32, false),
            Field::new("congestion_surcharge", DataType::Null, true),
        ]);
        let columns = arrow_schema_to_columns(&schema);
        assert_eq!(columns[0].hyper_type, "INT");
        assert_eq!(columns[1].hyper_type, "NULL");
    }

    /// The reported `NULL` must stay unresolvable by
    /// [`crate::schema::map_hyper_type`]. If it ever gained a mapping,
    /// `build_table_def` would create a column for a type the engine cannot
    /// read, and copying `inspect_file`'s output into a `schema` override
    /// would look like it worked.
    #[test]
    fn null_is_not_a_resolvable_override_type() {
        assert!(crate::schema::map_hyper_type("NULL").is_none());
    }

    /// Footer inspection must reject a physically `NullType` Parquet column,
    /// naming it, before any SQL is built.
    #[test]
    fn parquet_footer_rejects_physical_null_type_column() {
        let dir = tempfile::tempdir().expect("tempdir");
        let path = write_parquet(
            &dir,
            vec![
                ("a", false, Arc::new(Int32Array::from(vec![1, 2, 3]))),
                ("congestion_surcharge", true, Arc::new(NullArray::new(3))),
            ],
        );

        let err = infer_parquet_schema(&path).expect_err("NullType column must be rejected");
        assert_eq!(err.code, ErrorCode::UnsupportedFormat);
        assert!(
            err.message.contains("congestion_surcharge"),
            "must name the offending column: {}",
            err.message
        );
        assert!(
            err.message.contains("NullType"),
            "must name the physical type: {}",
            err.message
        );
        assert!(
            err.message.contains("`schema` override cannot fix"),
            "must say an override cannot rescue the file: {}",
            err.message
        );
        let suggestion = err.suggestion.expect("suggestion");
        assert!(
            suggestion.contains("writer") && suggestion.contains("DOUBLE"),
            "must point at re-typing upstream: {suggestion}"
        );
    }

    /// The rejection must not fire on ordinary columns — including a
    /// genuinely nullable one that happens to hold only nulls, which is a
    /// normal typed column and reads fine.
    #[test]
    fn parquet_footer_accepts_typed_columns_holding_nulls() {
        let dir = tempfile::tempdir().expect("tempdir");
        let path = write_parquet(
            &dir,
            vec![
                ("a", false, Arc::new(Int32Array::from(vec![1, 2, 3]))),
                (
                    "b",
                    true,
                    Arc::new(Int32Array::from(vec![None, None, None])),
                ),
            ],
        );

        let columns = infer_parquet_schema(&path).expect("typed columns must be accepted");
        assert_eq!(columns.len(), 2);
        assert_eq!(columns[1].hyper_type, "INT");
        assert!(columns[1].nullable);
    }
}
