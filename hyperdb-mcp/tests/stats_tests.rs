// Copyright (c) 2026, Salesforce, Inc. All rights reserved.
// SPDX-License-Identifier: Apache-2.0 OR MIT

//! Tests for the performance telemetry structs: verifying that derived
//! throughput metrics are computed correctly from raw counters, and that
//! the structs serialize to the expected JSON shape.

use hyperdb_mcp::stats::{IngestStats, QueryStats, StatsTimer};

/// Verify `IngestStats` derived metrics with known values:
/// - 1M rows in 500ms → 2M rows/sec
/// - 100MB read in 500ms → 200 MB/sec
/// - 60MB stored from 100MB read → 0.6 compression ratio
#[test]
fn ingest_stats_computes_throughput() {
    let stats = IngestStats {
        operation: "load_file".into(),
        rows: 1_000_000,
        elapsed_ms: 500,
        bytes_read: 100_000_000,
        bytes_stored: 60_000_000,
        schema_inference_ms: Some(10),
        table: "orders".into(),
        file_format: Some("csv".into()),
        warning: None,
        schema_changed: false,
    };
    assert_eq!(stats.rows_per_sec(), 2_000_000);
    assert!((stats.ingest_throughput_mb_sec() - 200.0).abs() < 0.1);
    assert!((stats.compression_ratio() - 0.6).abs() < 0.01);
}

/// Verify `QueryStats` scan rate: 10M rows scanned in 200ms → 50M rows/sec.
#[test]
fn query_stats_computes_scan_rate() {
    let stats = QueryStats {
        operation: "query".into(),
        rows_returned: 100,
        rows_scanned: 10_000_000,
        elapsed_ms: 200,
        result_size_bytes: 5000,
        tables_touched: vec!["orders".into()],
    };
    assert_eq!(stats.scan_rate_rows_sec(), 50_000_000);
}

/// Verify that `StatsTimer` measures real wall-clock time by sleeping for at
/// least 10ms and confirming the elapsed value is >= 10.
#[test]
fn stats_timer_measures_elapsed() {
    let timer = StatsTimer::start();
    std::thread::sleep(std::time::Duration::from_millis(10));
    let elapsed = timer.elapsed_ms();
    assert!(elapsed >= 10);
}

/// Verify that `QueryStats` serializes to JSON with the expected field names
/// and values, matching the format returned in MCP tool responses.
#[test]
fn stats_serialize_to_json() {
    let stats = QueryStats {
        operation: "query".into(),
        rows_returned: 5,
        rows_scanned: 1000,
        elapsed_ms: 50,
        result_size_bytes: 200,
        tables_touched: vec!["data".into()],
    };
    let json = serde_json::to_value(&stats).unwrap();
    assert_eq!(json["operation"], "query");
    assert_eq!(json["rows_returned"], 5);
}
