// Copyright (c) 2026, Salesforce, Inc. All rights reserved.
// SPDX-License-Identifier: Apache-2.0 OR MIT

//! Integration tests for the Iceberg ingest path. The SQL-builder unit
//! tests live alongside the code in `src/lakehouse.rs`; this file
//! covers the pieces that need a live engine — path validation, error
//! mapping from hyperd, and the mode/DDL plumbing.
//!
//! We intentionally don't ship real Iceberg fixture data (metadata
//! JSON + data parquet layouts are nontrivial and the files are big).
//! The "happy path" — pointing at a valid Iceberg directory and
//! reading rows — is covered in practice by hyperd's own test suite,
//! which this tool delegates to.

mod common;
use common::TestEngine;
use hyperdb_mcp::lakehouse::{IcebergIngestOptions, ingest_iceberg_table};
use tempfile::TempDir;

/// A non-existent path must produce a clean `FileNotFound` error
/// before any SQL is issued to hyperd. Guards against us accidentally
/// sending an unchecked path string into `external(...)`.
#[test]
fn ingest_iceberg_missing_path_errors_cleanly() {
    let mut te = TestEngine::new_ephemeral();
    let opts = IcebergIngestOptions {
        table: "t".into(),
        mode: hyperdb_mcp::lakehouse::IcebergMode::Replace,
        metadata_filename: None,
        version_as_of: None,
    };
    let Err(err) = ingest_iceberg_table(&mut te.engine, "/nonexistent/iceberg/path", &opts) else {
        panic!("missing path should fail")
    };
    let msg = err.to_string();
    assert!(
        msg.contains("does not exist") || msg.to_lowercase().contains("not found"),
        "error should say the path is missing, got: {msg}"
    );
}

/// Pointing at a plain file (not a directory) must fail with a clear
/// "must be a directory" error, not a confusing hyperd error about
/// missing `metadata/`.
#[test]
fn ingest_iceberg_file_instead_of_directory_errors_cleanly() {
    let mut te = TestEngine::new_ephemeral();
    let dir = TempDir::new().unwrap();
    let file_path = dir.path().join("not-a-dir.txt");
    std::fs::write(&file_path, b"hello").unwrap();

    let opts = IcebergIngestOptions {
        table: "t".into(),
        mode: hyperdb_mcp::lakehouse::IcebergMode::Replace,
        metadata_filename: None,
        version_as_of: None,
    };
    let Err(err) = ingest_iceberg_table(&mut te.engine, file_path.to_str().unwrap(), &opts) else {
        panic!("file path should fail")
    };
    let msg = err.to_string();
    assert!(
        msg.contains("must be a directory"),
        "error should explain Iceberg needs a directory, got: {msg}"
    );
}

/// An empty directory is not a valid Iceberg table — it has no
/// `metadata/` subdir. Hyperd itself catches this case, so we verify
/// we surface its error code/message rather than masking it. This also
/// exercises the full tx-wrapped execute path end-to-end.
#[test]
fn ingest_iceberg_empty_directory_surfaces_hyperd_error() {
    let mut te = TestEngine::new_ephemeral();
    let dir = TempDir::new().unwrap();

    let opts = IcebergIngestOptions {
        table: "t".into(),
        mode: hyperdb_mcp::lakehouse::IcebergMode::Replace,
        metadata_filename: None,
        version_as_of: None,
    };
    let Err(err) = ingest_iceberg_table(&mut te.engine, dir.path().to_str().unwrap(), &opts) else {
        panic!("empty directory should fail")
    };
    // We don't pin the exact wording — that's hyperd's to define. Just
    // verify we got *something* useful, not a panic or a type error.
    let msg = err.to_string();
    assert!(
        !msg.is_empty(),
        "empty directory error should carry a message"
    );
    // And the target table must not exist — the CTAS either didn't
    // run or got rolled back.
    let rows = te
        .engine
        .execute_query_to_json(
            "SELECT 1 AS present FROM pg_catalog.pg_tables WHERE tablename = 't'",
        )
        .unwrap();
    assert!(
        rows.is_empty(),
        "no table should have been created for a failed Iceberg ingest"
    );
}

/// Mode validation: an unknown `mode` is rejected at the parse boundary
/// instead of silently becoming a destructive replace.
#[test]
fn iceberg_mode_rejects_unknown_values() {
    use hyperdb_mcp::lakehouse::IcebergMode;
    for bad in ["blahblah", "merge", "Append", "", " append"] {
        let err = IcebergMode::parse(Some(bad)).expect_err(bad);
        assert_eq!(err.code, hyperdb_mcp::error::ErrorCode::InvalidArgument);
        assert!(err.message.contains("load_iceberg"), "{}", err.message);
    }
    assert_eq!(IcebergMode::parse(None).unwrap(), IcebergMode::Replace);
    assert_eq!(
        IcebergMode::parse(Some("append")).unwrap(),
        IcebergMode::Append
    );
}
