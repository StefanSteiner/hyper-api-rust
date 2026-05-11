// Copyright (c) 2026, Salesforce, Inc. All rights reserved.
// SPDX-License-Identifier: Apache-2.0 OR MIT

//! Tests for the `inspect_file` dry-run module.
//!
//! The inspector must report the *same* schema `load_file` would use (so the
//! partial override an LLM builds from the report aligns with the columns the
//! ingest pipeline will actually see) plus per-column diagnostics that make
//! overflow and parse errors obvious before they happen.

use hyperdb_mcp::inspect::inspect_source;
use std::fmt::Write as _;
use std::io::Write;

fn write_tmp(ext: &str, content: &[u8]) -> tempfile::NamedTempFile {
    let mut f = tempfile::Builder::new()
        .suffix(&format!(".{ext}"))
        .tempfile()
        .expect("create temp file");
    f.write_all(content).expect("write temp file");
    f.flush().ok();
    f
}

/// CSV inspection returns the same widened schema the ingest pipeline would
/// produce, plus per-column min/max captured from a full scan. This is the
/// core safety check the LLM relies on before issuing a schema override.
#[test]
fn inspect_csv_reports_widened_types_and_min_max() {
    let mut csv = String::from("id,population\n");
    for i in 0..1200 {
        let _ = writeln!(csv, "{i},{}\n", 100 + i);
    }
    csv.push_str("9999,8000000000\n");
    let file = write_tmp("csv", csv.as_bytes());

    let report = inspect_source(file.path().to_str().unwrap(), 3).expect("inspect");
    assert_eq!(report.file_format, "csv");
    assert_eq!(report.row_count, 1201);
    let pop = report
        .columns
        .iter()
        .position(|c| c.name == "population")
        .unwrap();
    assert_eq!(report.columns[pop].hyper_type, "BIGINT");
    assert_eq!(report.stats[pop].max_i128, Some(8_000_000_000));
    assert_eq!(report.stats[pop].min_i128, Some(100));
    assert_eq!(report.stats[pop].null_count, 0);
    assert!(!report.stats[pop].sample_values.is_empty());
}

/// Nulls and empty cells must be counted, not silently dropped — the caller
/// uses `null_count` to decide whether NOT NULL is safe on an override.
#[test]
fn inspect_csv_counts_nulls_and_empty_cells() {
    let csv = "name,age\nalice,30\nbob,\ncarol,NULL\n";
    let file = write_tmp("csv", csv.as_bytes());
    let report = inspect_source(file.path().to_str().unwrap(), 5).unwrap();
    let age = report.columns.iter().position(|c| c.name == "age").unwrap();
    assert_eq!(report.stats[age].null_count, 2);
}

/// The inspection should surface the JSON shape the MCP tool returns without
/// panicking on mixed numeric/float columns. This locks in the schema +
/// diagnostics wire format consumed by clients.
#[test]
fn inspect_csv_to_json_contains_expected_fields() {
    let csv = "a,b\n1,2.5\n2,3.25\n";
    let file = write_tmp("csv", csv.as_bytes());
    let report = inspect_source(file.path().to_str().unwrap(), 2).unwrap();
    let val = report.to_json();
    assert_eq!(val["file_format"], "csv");
    assert_eq!(val["row_count"], 2);
    let cols = val["columns"].as_array().unwrap();
    assert_eq!(cols.len(), 2);
    // b is a float column, so min/max should be reported as floats, not ints.
    let b = cols.iter().find(|c| c["name"] == "b").unwrap();
    assert_eq!(b["type"], "DOUBLE PRECISION");
    assert!(b.get("min_f64").is_some());
    assert!(b.get("max_f64").is_some());
}

/// Nonexistent paths must surface a `FileNotFound` error with a useful message
/// — not a generic `InternalError` that the caller would mistake for a bug.
#[test]
fn inspect_missing_file_returns_file_not_found() {
    let err = inspect_source("/nonexistent/path/really_not_here.csv", 5).unwrap_err();
    assert_eq!(err.code, hyperdb_mcp::error::ErrorCode::FileNotFound);
}

/// A `.json` file containing a top-level array of objects inspects as
/// JSON, not CSV. Schema matches what `infer_json_schema` produces so
/// an LLM's override built from this report will align with what
/// `load_file` actually creates.
#[test]
fn inspect_json_array_file_reports_json_schema() {
    let file = write_tmp(
        "json",
        b"[{\"id\":1,\"name\":\"Alice\"},{\"id\":2,\"name\":\"Bob\"},{\"id\":3,\"name\":\"Carol\"}]",
    );
    let report = inspect_source(file.path().to_str().unwrap(), 2).expect("inspect");
    assert_eq!(report.file_format, "json");
    assert_eq!(report.row_count, 3);
    let id = report.columns.iter().position(|c| c.name == "id").unwrap();
    assert_eq!(report.columns[id].hyper_type, "INT");
    assert_eq!(report.stats[id].null_count, 0);
    assert_eq!(report.stats[id].sample_values.len(), 2);
    let name = report
        .columns
        .iter()
        .position(|c| c.name == "name")
        .unwrap();
    assert_eq!(report.columns[name].hyper_type, "TEXT");
}

/// A `.jsonl` file (newline-delimited JSON) is recognized and reports
/// `file_format: "jsonl"` to distinguish it from the array variant —
/// useful for LLMs explaining what the file shape actually is.
#[test]
fn inspect_jsonl_file_reports_jsonl_format() {
    let file = write_tmp(
        "jsonl",
        b"{\"a\":1,\"b\":\"x\"}\n{\"a\":2,\"b\":\"y\"}\n{\"a\":3,\"b\":null}\n",
    );
    let report = inspect_source(file.path().to_str().unwrap(), 3).expect("inspect");
    assert_eq!(report.file_format, "jsonl");
    assert_eq!(report.row_count, 3);
    let b = report.columns.iter().position(|c| c.name == "b").unwrap();
    // Third row has b=null → the stats layer counts one null.
    assert_eq!(report.stats[b].null_count, 1);
}

/// A `.log` file containing JSONL sniffs as JSON (via the shared
/// `detect_file_format` helper) so `inspect_file` and `load_file`
/// never disagree about what a file is. This is the hyperd-log case
/// the whole content-sniff fix was designed for.
#[test]
fn inspect_log_extension_sniffs_as_jsonl() {
    let file = write_tmp(
        "log",
        b"{\"k\":\"start\",\"n\":1}\n{\"k\":\"done\",\"n\":2}\n",
    );
    let report = inspect_source(file.path().to_str().unwrap(), 2).expect("inspect");
    assert_eq!(
        report.file_format, "jsonl",
        "`.log` with JSONL content should sniff as jsonl, not csv"
    );
    assert_eq!(report.row_count, 2);
}
