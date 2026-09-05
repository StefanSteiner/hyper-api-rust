// Copyright (c) 2026, Salesforce, Inc. All rights reserved.
// SPDX-License-Identifier: Apache-2.0 OR MIT

//! Unit coverage for the benchmark harness helpers in `benches/common.rs`.
//!
//! The benches themselves are registered as **examples** (`autobenches =
//! false`), so `cargo test --benches` runs nothing and a `#[test]` placed
//! inside a bench file would never execute. This target pulls the same
//! `common.rs` in through the same `#[path]` include the benches use, so the
//! helpers get real, `make test`-visible coverage.
//!
//! The point of these tests is to pin the **unit**. Every `MB`-labelled
//! helper here reports decimal megabytes (10^6). They used to divide by
//! 1024², emitting MiB under an `MB` label, which put two different units in
//! a single `MB/sec` column of `docs/BENCHMARK_GUIDE.md`. The assertions
//! below fail if anyone reintroduces a binary divisor.

// Mirrors the relevant crate-level expectation every bench that includes
// `common.rs` declares. Only `cast_precision_loss` fires from this target —
// the truncation/sign/wrap casts live in the bench binaries, not in
// `common.rs` — and `expect` rejects a lint that never triggers.
#![expect(
    clippy::cast_precision_loss,
    reason = "benchmark harness: throughput math needs f64"
)]

#[path = "../benches/common.rs"]
mod common;

use common::{BYTES_PER_MB, BenchRecord, ResourceStats, fmt_count, fmt_mb, fmt_rate, fmt_size};

/// Tolerance-based float equality — the workspace denies `clippy::float_cmp`.
#[track_caller]
fn assert_close(actual: f64, expected: f64) {
    assert!(
        (actual - expected).abs() < 1e-9,
        "expected {expected}, got {actual}"
    );
}

/// One megabyte is 10^6 bytes, not 2^20.
#[test]
fn bytes_per_mb_is_decimal() {
    assert_close(BYTES_PER_MB, 1_000_000.0);
    assert!(
        (BYTES_PER_MB - 1_048_576.0).abs() > 1.0,
        "MB must not be MiB"
    );
}

/// `fmt_mb` divides by 10^6: exactly 1 MB in 1 s is 1.0 MB/s.
#[test]
fn fmt_mb_is_decimal_per_second() {
    assert_eq!(fmt_mb(1_000_000, 1.0), "1.0 MB/s");
    assert_eq!(fmt_mb(2_600_000, 2.0), "1.3 MB/s");
    assert_eq!(fmt_mb(45_000_000, 0.5), "90.0 MB/s");
    // Under the old binary divisor this read "0.9 MB/s".
    assert_eq!(fmt_mb(1_048_576, 1.0), "1.0 MB/s");
}

/// A non-positive duration has no defined rate.
#[test]
fn fmt_mb_rejects_non_positive_elapsed() {
    assert_eq!(fmt_mb(1_000_000, 0.0), "—");
    assert_eq!(fmt_mb(1_000_000, -1.0), "—");
}

/// `BenchRecord::mb_per_sec` is decimal, and agrees with `fmt_mb`.
#[test]
fn bench_record_mb_per_sec_is_decimal() {
    let record = BenchRecord {
        workload: "insert.bulk".to_string(),
        flavor: "sync",
        variant: String::new(),
        rows: 1_000,
        bytes: 24_000_000,
        elapsed_secs: 2.0,
    };
    // 24 MB over 2 s = 12 MB/s decimal (11.44 under the old MiB divisor).
    assert_close(record.mb_per_sec(), 12.0);
    assert_close(record.rows_per_sec(), 500.0);
}

/// Guards the divide-by-zero path rather than emitting `inf`.
#[test]
fn bench_record_zero_elapsed_is_zero_not_infinite() {
    let record = BenchRecord {
        workload: "query.full_scan".to_string(),
        flavor: "async",
        variant: "1 connection".to_string(),
        rows: 10,
        bytes: 240,
        elapsed_secs: 0.0,
    };
    assert_close(record.mb_per_sec(), 0.0);
    assert_close(record.rows_per_sec(), 0.0);
}

/// The `memory_*_mb` accessors report decimal MB from raw byte samples.
#[test]
fn resource_stats_memory_is_decimal_mb() {
    let stats = ResourceStats {
        cpu_samples: vec![10.0, 30.0],
        memory_samples: vec![1_000_000, 3_000_000],
        sample_count: 2,
    };
    assert_close(stats.memory_min_mb(), 1.0);
    assert_close(stats.memory_max_mb(), 3.0);
    assert_close(stats.memory_avg_mb(), 2.0);
    assert!((stats.cpu_avg() - 20.0).abs() < 1e-5);
    assert!((stats.cpu_max() - 30.0).abs() < 1e-5);
}

/// Empty sample sets must not panic or divide by zero.
#[test]
fn resource_stats_empty_samples_are_zero() {
    let stats = ResourceStats::default();
    assert_close(stats.memory_avg_mb(), 0.0);
    assert_close(stats.memory_max_mb(), 0.0);
    assert_close(stats.memory_min_mb(), 0.0);
    assert!(stats.cpu_avg().abs() < 1e-5);
}

/// The sibling formatters were already decimal; assert it so the whole
/// module keeps one unit system.
#[test]
fn sibling_formatters_are_decimal() {
    assert_eq!(fmt_size(1_000_000), "1.00 MB");
    assert_eq!(fmt_size(1_000_000_000), "1.00 GB");
    assert_eq!(fmt_size(2_500), "2.50 KB");
    assert_eq!(fmt_size(999), "999 B");

    assert_eq!(fmt_count(1_000), "1.0K");
    assert_eq!(fmt_count(1_500_000), "1.50M");
    assert_eq!(fmt_count(2_000_000_000), "2.00B");
    assert_eq!(fmt_count(42), "42");

    assert_eq!(fmt_rate(1_000.0), "1.00 K/s");
    assert_eq!(fmt_rate(2_500_000.0), "2.50 M/s");
    assert_eq!(fmt_rate(1e9), "1.00 B/s");
}

/// `gen_id` must reject a row index that cannot fit the `id INT` column
/// rather than silently wrapping to a duplicate or negative ID.
#[test]
#[should_panic(expected = "benchmark row IDs must fit the `id INT` column")]
fn gen_id_panics_instead_of_wrapping_past_i32() {
    let _ = common::gen_id(i64::from(i32::MAX), 1);
}

/// Deterministic generators stay stable, so numbers compare across benches.
#[test]
fn row_generators_are_deterministic() {
    assert_eq!(common::gen_id(1_000, 24), 1_024);
    assert_eq!(common::gen_sensor_id(1_024), 4);
    assert_close(common::gen_value(10), 1.0);
    assert_eq!(common::gen_timestamp(1), 1_700_000_001_000);
}
