// Copyright (c) 2026, Salesforce, Inc. All rights reserved.
// SPDX-License-Identifier: Apache-2.0 OR MIT

//! Ingest Apache Iceberg tables into Hyper using hyperd's native
//! `external(..., format => 'iceberg')` scan.
//!
//! An Iceberg "table" on disk is a *directory* containing a `metadata/`
//! subdir (pointing at snapshot manifests) and one or more `data/`
//! parquet files. We hand the directory path to hyperd; hyperd resolves
//! the latest snapshot (or the one pinned by `metadata_filename` /
//! `version_as_of`), reads the relevant data files, and streams the
//! rows into the target table.
//!
//! Single SQL statement, single round-trip, zero Rust-side Arrow
//! decoding — same shape as the Parquet ingest path.

use crate::engine::Engine;
use crate::error::{ErrorCode, McpError};
use crate::ingest::IngestResult;
use crate::schema::ColumnSchema;
use crate::stats::{IngestStats, StatsTimer};
use hyperdb_api::escape_string_literal;
use std::path::Path;

/// Parameters for an Iceberg ingest. Mirrors `LoadFileParams` but with
/// Iceberg-specific knobs and no schema-override support (hyperd derives
/// the schema from the Iceberg metadata, so overrides would have no
/// obvious target column set to apply against).
#[derive(Debug, Clone)]
pub struct IcebergIngestOptions {
    /// Target table name.
    pub table: String,
    /// `"replace"` (drops + CTAS) or `"append"` (INSERT INTO ... SELECT).
    pub mode: String,
    /// Optional specific metadata filename to pin a snapshot, e.g.
    /// `"v2.metadata.json"`. If omitted, hyperd uses whatever
    /// `metadata/version-hint.text` (or the latest `vN.metadata.json`)
    /// points at.
    pub metadata_filename: Option<String>,
    /// Optional snapshot version to read as of. Mutually understandable
    /// with `metadata_filename`; hyperd handles the interaction.
    pub version_as_of: Option<i64>,
}

/// Build the SQL hyperd will execute.
///
/// - Replace: `CREATE TABLE "t" AS SELECT * FROM external('dir',
///   format => 'iceberg' [, metadata_filename => '…'] [, version_as_of => N])`.
/// - Append: `INSERT INTO "t" SELECT * FROM external(...)`.
///
/// Path and string options are quoted via [`escape_string_literal`];
/// integer options are rendered directly. No user input reaches the SQL
/// without quoting.
#[must_use]
pub fn build_iceberg_ingest_sql(
    table: &str,
    path: &str,
    opts: &IcebergIngestOptions,
    is_replace: bool,
) -> String {
    let quoted_table = format!("\"{}\"", table.replace('"', "\"\""));
    let quoted_path = escape_string_literal(path);

    let mut external_args = vec![quoted_path, "format => 'iceberg'".to_string()];
    if let Some(name) = &opts.metadata_filename {
        external_args.push(format!(
            "metadata_filename => {}",
            escape_string_literal(name)
        ));
    }
    if let Some(version) = opts.version_as_of {
        external_args.push(format!("version_as_of => {version}"));
    }
    let external = format!("external({})", external_args.join(", "));

    if is_replace {
        format!("CREATE TABLE {quoted_table} AS SELECT * FROM {external}")
    } else {
        format!("INSERT INTO {quoted_table} SELECT * FROM {external}")
    }
}

/// Resolve the Iceberg directory path: must exist and be a directory.
/// Returns the canonical absolute path hyperd should see.
fn resolve_iceberg_path(path: &str) -> Result<String, McpError> {
    let p = Path::new(path);
    if !p.exists() {
        return Err(McpError::new(
            ErrorCode::FileNotFound,
            format!("Iceberg path does not exist: {path}"),
        ));
    }
    if !p.is_dir() {
        return Err(McpError::new(
            ErrorCode::UnsupportedFormat,
            format!(
                "Iceberg path must be a directory (the table root with a `metadata/` subdir), not a file: {path}"
            ),
        ));
    }
    Ok(std::fs::canonicalize(p)
        .map_err(|e| {
            McpError::new(
                ErrorCode::FileNotFound,
                format!("Cannot resolve Iceberg path {path}: {e}"),
            )
        })?
        .to_string_lossy()
        .into_owned())
}

/// Read the target table's schema after CTAS/INSERT so the tool can
/// report back what hyperd actually created. Delegates to
/// [`Engine::describe_table`], which uses the hyperdb-api `Catalog` and
/// works on any Hyper database (unlike `information_schema.columns`,
/// which Hyper does not expose).
fn describe_table(engine: &Engine, table: &str) -> Result<Vec<ColumnSchema>, McpError> {
    let desc = engine.describe_table(table)?;
    let cols = desc.get("columns").and_then(|v| v.as_array());
    Ok(cols
        .map(|arr| {
            arr.iter()
                .map(|c| ColumnSchema {
                    name: c
                        .get("name")
                        .and_then(|v| v.as_str())
                        .unwrap_or("")
                        .to_string(),
                    hyper_type: c
                        .get("type")
                        .and_then(|v| v.as_str())
                        .unwrap_or("")
                        .to_string(),
                    nullable: c
                        .get("nullable")
                        .and_then(serde_json::Value::as_bool)
                        .unwrap_or(true),
                })
                .collect()
        })
        .unwrap_or_default())
}

/// Count rows in the target table — used instead of `affected_rows`
/// because `CREATE TABLE AS` reports 0. Runs outside any transaction so
/// the result reflects the committed state, matching the pattern in
/// `ingest_parquet_file`.
fn count_rows(engine: &Engine, table: &str) -> Result<u64, McpError> {
    let quoted = format!("\"{}\"", table.replace('"', "\"\""));
    let sql = format!("SELECT COUNT(*) FROM {quoted}");
    let rows = engine.execute_query_to_json(&sql)?;
    rows.first()
        .and_then(|r| r.get("count"))
        .and_then(serde_json::Value::as_u64)
        .ok_or_else(|| {
            McpError::new(
                ErrorCode::InternalError,
                "Could not read row count after Iceberg ingest",
            )
        })
}

/// Ingest an Iceberg table directory into a Hyper table.
///
/// Issues a single `CREATE TABLE AS SELECT` (replace) or
/// `INSERT INTO ... SELECT` (append) against hyperd's
/// `external(..., format => 'iceberg')` reader. Hyperd does all the
/// metadata resolution, snapshot selection, and data-file scanning; we
/// just wait for the row count.
///
/// # Errors
///
/// - Propagates errors from `resolve_iceberg_path` when `path`
///   cannot be canonicalized or is not a directory.
/// - Propagates transaction errors from `DROP TABLE IF EXISTS` or the
///   `CREATE TABLE AS SELECT` / `INSERT INTO ... SELECT` statement —
///   typically Iceberg metadata errors, snapshot resolution failures,
///   or Hyper wire errors.
/// - Returns [`ErrorCode::InternalError`] if the post-ingest
///   `COUNT(*)` cannot be read back (bubbled from `count_rows`).
pub fn ingest_iceberg_table(
    engine: &Engine,
    path: &str,
    opts: &IcebergIngestOptions,
) -> Result<IngestResult, McpError> {
    let timer = StatsTimer::start();
    let absolute_path = resolve_iceberg_path(path)?;

    let is_replace = opts.mode != "append";
    let sql = build_iceberg_ingest_sql(&opts.table, &absolute_path, opts, is_replace);

    // Same tx-shape as parquet ingest. The COUNT(*) *must* run outside
    // the transaction to avoid the post-CTAS wire-state quirk that
    // truncates the returned count — see `ingest_parquet_file` for the
    // long version.
    let affected = engine.execute_in_transaction(|engine| {
        if is_replace {
            let quoted_table = format!("\"{}\"", opts.table.replace('"', "\"\""));
            engine.execute_command(&format!("DROP TABLE IF EXISTS {quoted_table}"))?;
        }
        engine.execute_command(&sql)
    })?;

    let row_count = if is_replace {
        count_rows(engine, &opts.table)?
    } else {
        affected
    };

    let schema = describe_table(engine, &opts.table).unwrap_or_default();
    let elapsed = timer.elapsed_ms();
    let stats = IngestStats {
        operation: "load_iceberg".into(),
        rows: row_count,
        elapsed_ms: elapsed,
        bytes_read: 0,
        bytes_stored: 0,
        schema_inference_ms: Some(0),
        table: opts.table.clone(),
        file_format: Some("iceberg".into()),
        warning: None,
        schema_changed: false,
    };

    Ok(IngestResult {
        rows: row_count,
        schema,
        stats,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn builds_replace_sql_with_minimal_options() {
        let opts = IcebergIngestOptions {
            table: "my_table".into(),
            mode: "replace".into(),
            metadata_filename: None,
            version_as_of: None,
        };
        let sql = build_iceberg_ingest_sql("my_table", "/abs/table", &opts, true);
        assert_eq!(
            sql,
            "CREATE TABLE \"my_table\" AS SELECT * FROM external('/abs/table', format => 'iceberg')"
        );
    }

    #[test]
    fn builds_append_sql_with_metadata_filename() {
        let opts = IcebergIngestOptions {
            table: "t".into(),
            mode: "append".into(),
            metadata_filename: Some("v2.metadata.json".into()),
            version_as_of: None,
        };
        let sql = build_iceberg_ingest_sql("t", "/abs/t", &opts, false);
        assert_eq!(
            sql,
            "INSERT INTO \"t\" SELECT * FROM external('/abs/t', format => 'iceberg', metadata_filename => 'v2.metadata.json')"
        );
    }

    #[test]
    fn builds_sql_with_version_as_of() {
        let opts = IcebergIngestOptions {
            table: "t".into(),
            mode: "replace".into(),
            metadata_filename: None,
            version_as_of: Some(7),
        };
        let sql = build_iceberg_ingest_sql("t", "/abs/t", &opts, true);
        assert_eq!(
            sql,
            "CREATE TABLE \"t\" AS SELECT * FROM external('/abs/t', format => 'iceberg', version_as_of => 7)"
        );
    }

    #[test]
    fn builds_sql_with_both_metadata_and_version() {
        let opts = IcebergIngestOptions {
            table: "t".into(),
            mode: "replace".into(),
            metadata_filename: Some("v3.metadata.json".into()),
            version_as_of: Some(42),
        };
        let sql = build_iceberg_ingest_sql("t", "/abs/t", &opts, true);
        assert_eq!(
            sql,
            "CREATE TABLE \"t\" AS SELECT * FROM external('/abs/t', format => 'iceberg', metadata_filename => 'v3.metadata.json', version_as_of => 42)"
        );
    }

    #[test]
    fn escapes_single_quotes_in_path() {
        let opts = IcebergIngestOptions {
            table: "t".into(),
            mode: "replace".into(),
            metadata_filename: None,
            version_as_of: None,
        };
        // Path containing a single quote — SQL injection guard.
        let sql = build_iceberg_ingest_sql("t", "/abs/it's/t", &opts, true);
        assert!(
            sql.contains("'/abs/it''s/t'"),
            "single quote in path must be escaped; got: {sql}"
        );
    }

    #[test]
    fn escapes_single_quotes_in_metadata_filename() {
        let opts = IcebergIngestOptions {
            table: "t".into(),
            mode: "replace".into(),
            metadata_filename: Some("v1.metadata'.json".into()),
            version_as_of: None,
        };
        let sql = build_iceberg_ingest_sql("t", "/abs/t", &opts, true);
        assert!(
            sql.contains("metadata_filename => 'v1.metadata''.json'"),
            "single quote in metadata_filename must be escaped; got: {sql}"
        );
    }

    #[test]
    fn escapes_quotes_in_table_name() {
        let opts = IcebergIngestOptions {
            table: "weird\"name".into(),
            mode: "replace".into(),
            metadata_filename: None,
            version_as_of: None,
        };
        let sql = build_iceberg_ingest_sql("weird\"name", "/abs/t", &opts, true);
        assert!(
            sql.contains("\"weird\"\"name\""),
            "double-quote in table name must be escaped; got: {sql}"
        );
    }
}
