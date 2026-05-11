// Copyright (c) 2026, Salesforce, Inc. All rights reserved.
// SPDX-License-Identifier: Apache-2.0 OR MIT

//! gRPC performance benchmarks for Hyper API
//!
//! These benchmarks test gRPC query performance with various data sizes
//! and transfer modes, measuring both rows/second and MB/second throughput.
//!
//! # Running this example
//!
//! ```bash
//! cargo run -p hyperdb-api --example grpc_benchmark_tests --release
//! ```

// Benchmark harness: intentional wide→narrow conversions for row-count display
// and throughput math (f64 rows/sec rounded back to u64 for formatting).
#![expect(
    clippy::cast_possible_truncation,
    clippy::cast_sign_loss,
    clippy::cast_precision_loss,
    reason = "benchmark harness: throughput math casts f64 → u64 for display"
)]

use hyperdb_api::grpc::{GrpcConfig, GrpcConnection, TransferMode};
use hyperdb_api::{HyperProcess, ListenMode, Parameters, Result};
use std::time::Instant;

// ============================================================================
// Benchmark Configuration
// ============================================================================

/// Scale points for testing (in rows)
/// Reduced for faster example execution (full benchmarks can be run separately)
const SCALE_POINTS: &[u64] = &[
    10_000,  // 10K - quick sanity check
    100_000, // 100K - small dataset
];

/// All transfer modes to test
const TRANSFER_MODES: &[(TransferMode, &str)] = &[
    (TransferMode::Sync, "SYNC"),
    (TransferMode::Adaptive, "ADAPTIVE"),
    (TransferMode::Async, "ASYNC"),
];

// ============================================================================
// Result Types
// ============================================================================

/// Benchmark result with timing and throughput metrics
#[derive(Debug, Clone)]
struct BenchmarkResult {
    mode: &'static str,
    row_count: u64,
    data_size_bytes: usize,
    elapsed_secs: f64,
    rows_per_sec: f64,
    mb_per_sec: f64,
}

impl BenchmarkResult {
    fn new(
        mode: &'static str,
        row_count: u64,
        data_size_bytes: usize,
        elapsed: std::time::Duration,
    ) -> Self {
        let elapsed_secs = elapsed.as_secs_f64();
        let rows_per_sec = row_count as f64 / elapsed_secs;
        let mb_per_sec = (data_size_bytes as f64 / 1_000_000.0) / elapsed_secs;

        BenchmarkResult {
            mode,
            row_count,
            data_size_bytes,
            elapsed_secs,
            rows_per_sec,
            mb_per_sec,
        }
    }
}

// ============================================================================
// Output Formatting
// ============================================================================

fn format_count(count: u64) -> String {
    if count >= 1_000_000_000 {
        format!("{:.1}B", count as f64 / 1_000_000_000.0)
    } else if count >= 1_000_000 {
        format!("{:.1}M", count as f64 / 1_000_000.0)
    } else if count >= 1_000 {
        format!("{:.1}K", count as f64 / 1_000.0)
    } else {
        format!("{count}")
    }
}

fn format_size(bytes: usize) -> String {
    if bytes >= 1_000_000_000 {
        format!("{:.2} GB", bytes as f64 / 1_000_000_000.0)
    } else if bytes >= 1_000_000 {
        format!("{:.2} MB", bytes as f64 / 1_000_000.0)
    } else if bytes >= 1_000 {
        format!("{:.2} KB", bytes as f64 / 1_000.0)
    } else {
        format!("{bytes} B")
    }
}

fn print_header(title: &str) {
    println!();
    println!("╔══════════════════════════════════════════════════════════════════════════════╗");
    println!("║ {title:^76} ║");
    println!("╚══════════════════════════════════════════════════════════════════════════════╝");
    println!();
}

fn print_section(title: &str) {
    println!();
    println!("┌──────────────────────────────────────────────────────────────────────────────┐");
    println!("│ {}{}│", title, " ".repeat(76 - title.len()));
    println!("└──────────────────────────────────────────────────────────────────────────────┘");
}

fn print_table_header() {
    println!();
    println!("┌────────────┬────────────┬────────────┬────────────┬──────────────┬──────────────┐");
    println!(
        "│ {:>10} │ {:>10} │ {:>10} │ {:>10} │ {:>12} │ {:>12} │",
        "Mode", "Rows", "Data Size", "Time (s)", "Rows/sec", "MB/sec"
    );
    println!("├────────────┼────────────┼────────────┼────────────┼──────────────┼──────────────┤");
}

fn print_table_row(result: &BenchmarkResult) {
    println!(
        "│ {:>10} │ {:>10} │ {:>10} │ {:>10.2} │ {:>12} │ {:>12.2} │",
        result.mode,
        format_count(result.row_count),
        format_size(result.data_size_bytes),
        result.elapsed_secs,
        format_count(result.rows_per_sec as u64),
        result.mb_per_sec
    );
}

fn print_table_footer() {
    println!("└────────────┴────────────┴────────────┴────────────┴──────────────┴──────────────┘");
}

fn print_error_row(mode: &str, rows: u64, error: &str) {
    println!(
        "│ {:>10} │ {:>10} │ {:^44} │",
        mode,
        format_count(rows),
        format!("ERROR: {}", &error[..error.len().min(38)])
    );
}

// ============================================================================
// Benchmark Execution
// ============================================================================

fn run_benchmark(
    conn: &mut GrpcConnection,
    query: &str,
    row_count: u64,
    mode: &'static str,
) -> Result<BenchmarkResult> {
    let start = Instant::now();
    let result = conn.execute_query(query)?;
    let elapsed = start.elapsed();
    let data_size = result.arrow_data().len();

    Ok(BenchmarkResult::new(mode, row_count, data_size, elapsed))
}

/// Simple query with 3 columns (for speed)
fn simple_query(row_count: u64) -> String {
    format!(
        "SELECT i AS id, i % 10000 AS bucket, random() AS rand FROM generate_series(1, {row_count}) AS s(i)"
    )
}

/// Complex query with 10+ mixed columns
fn complex_query(row_count: u64) -> String {
    format!(
        r"SELECT
            i AS id,
            i % 10000 AS bucket,
            i * 2 AS doubled,
            random() AS rand_float,
            CASE WHEN i % 2 = 0 THEN true ELSE false END AS is_even,
            'row_' || CAST(i AS TEXT) AS label,
            i % 100 AS small_int,
            i * 0.001 AS scaled,
            CAST(i AS TEXT) || '_suffix' AS text_col,
            i % 1000000 AS medium_int,
            CASE WHEN i % 3 = 0 THEN 'A' WHEN i % 3 = 1 THEN 'B' ELSE 'C' END AS category,
            i / 1000 AS quotient
        FROM generate_series(1, {row_count}) AS s(i)"
    )
}

// ============================================================================
// Main Benchmark Functions
// ============================================================================

/// Comprehensive benchmark comparing all transfer modes at multiple scale points
fn benchmark_all_modes_all_scales() -> Result<()> {
    print_header("gRPC TRANSFER MODE BENCHMARK");

    println!("This benchmark compares SYNC, ADAPTIVE, and ASYNC transfer modes");
    println!("at multiple data scales. Each test uses a simple 3-column query.");
    println!();
    println!(
        "Scale points: {:?}",
        SCALE_POINTS
            .iter()
            .map(|&x| format_count(x))
            .collect::<Vec<_>>()
    );
    println!();

    // Start Hyper with gRPC
    let mut params = Parameters::new();
    params.set("log_dir", "test_results");
    params.set_listen_mode(ListenMode::Grpc { port: 0 });
    // Note: grpc_threads is automatically set by HyperProcess
    let hyper = HyperProcess::new(None, Some(&params))?;
    let grpc_url = hyper.grpc_url().unwrap();

    let mut all_results: Vec<BenchmarkResult> = Vec::new();

    // Run each scale point
    for &row_count in SCALE_POINTS {
        print_section(&format!("Scale: {} rows", format_count(row_count)));
        print_table_header();

        let query = simple_query(row_count);

        for &(mode, mode_name) in TRANSFER_MODES {
            let config = GrpcConfig::new(&grpc_url).transfer_mode(mode);
            match GrpcConnection::connect_with_config(config) {
                Ok(mut conn) => match run_benchmark(&mut conn, &query, row_count, mode_name) {
                    Ok(result) => {
                        print_table_row(&result);
                        all_results.push(result);
                    }
                    Err(e) => {
                        print_error_row(mode_name, row_count, &e.to_string());
                    }
                },
                Err(e) => {
                    print_error_row(mode_name, row_count, &e.to_string());
                }
            }
        }

        print_table_footer();
    }

    // Print summary
    print_section("Summary: Best Results by Mode");
    println!();

    for &(_, mode_name) in TRANSFER_MODES {
        let mode_results: Vec<_> = all_results.iter().filter(|r| r.mode == mode_name).collect();
        if mode_results.is_empty() {
            continue;
        }

        let max_throughput = mode_results
            .iter()
            .map(|r| r.rows_per_sec)
            .fold(0.0_f64, f64::max);
        let max_mb_sec = mode_results
            .iter()
            .map(|r| r.mb_per_sec)
            .fold(0.0_f64, f64::max);

        println!(
            "  {:<10}: Peak {}/sec rows, {:.2} MB/sec",
            mode_name,
            format_count(max_throughput as u64),
            max_mb_sec
        );
    }

    Ok(())
}

// ============================================================================
// Complex Column Benchmark (100M rows, 12 columns)
// ============================================================================

/// Benchmark with 100M rows and 12 mixed-type columns
fn benchmark_100m_complex() -> Result<()> {
    print_header("100M ROWS × 12 COLUMNS BENCHMARK");

    println!("This benchmark tests the maximum throughput with a realistic workload:");
    println!("  • 100,000,000 rows");
    println!("  • 12 columns of mixed types (INT, FLOAT, BOOL, TEXT, etc.)");
    println!();
    println!("Column schema:");
    println!("  1. id          BIGINT     - Row identifier");
    println!("  2. bucket      INT        - Modulo bucket (0-9999)");
    println!("  3. doubled     BIGINT     - Doubled value");
    println!("  4. rand_float  DOUBLE     - Random float");
    println!("  5. is_even     BOOL       - Even/odd flag");
    println!("  6. label       TEXT       - String with row number");
    println!("  7. small_int   INT        - Small modulo (0-99)");
    println!("  8. scaled      DOUBLE     - Scaled value");
    println!("  9. text_col    TEXT       - Another text column");
    println!(" 10. medium_int  INT        - Medium modulo");
    println!(" 11. category    TEXT       - A/B/C category");
    println!(" 12. quotient    BIGINT     - Divided value");
    println!();

    // Start Hyper with gRPC
    let mut params = Parameters::new();
    params.set("log_dir", "test_results");
    params.set_listen_mode(ListenMode::Grpc { port: 0 });
    // Note: grpc_threads is automatically set by HyperProcess
    let hyper = HyperProcess::new(None, Some(&params))?;
    let grpc_url = hyper.grpc_url().unwrap();

    let row_count: u64 = 100_000_000;
    let query = complex_query(row_count);

    // Run benchmarks first, collect results
    println!("Running benchmarks (this may take 1-2 minutes)...");
    let mut results: Vec<std::result::Result<BenchmarkResult, String>> = Vec::new();

    for &(mode, mode_name) in TRANSFER_MODES {
        print!("  {mode_name}... ");
        std::io::Write::flush(&mut std::io::stdout()).ok();

        let config = GrpcConfig::new(&grpc_url).transfer_mode(mode);
        match GrpcConnection::connect_with_config(config) {
            Ok(mut conn) => match run_benchmark(&mut conn, &query, row_count, mode_name) {
                Ok(result) => {
                    println!("done ({:.2}s)", result.elapsed_secs);
                    results.push(Ok(result));
                }
                Err(e) => {
                    println!("failed");
                    results.push(Err(format!("{mode_name}: {e}")));
                }
            },
            Err(e) => {
                println!("connection failed");
                results.push(Err(format!("{mode_name}: connection failed - {e}")));
            }
        }
    }

    // Now print the results table
    println!();
    print_table_header();

    for result in &results {
        match result {
            Ok(r) => print_table_row(r),
            Err(msg) => println!("│ {msg:^76} │"),
        }
    }

    print_table_footer();

    println!();
    println!("Note: ASYNC mode typically performs best with large datasets");
    println!("      because it allows Hyper to pipeline result preparation.");

    Ok(())
}

// ============================================================================
// Quick Sanity Check
// ============================================================================

/// Quick benchmark for CI/CD - tests 1M rows only
fn benchmark_quick() -> Result<()> {
    print_header("QUICK BENCHMARK (1M rows)");

    // Start Hyper with gRPC
    let mut params = Parameters::new();
    params.set("log_dir", "test_results");
    params.set_listen_mode(ListenMode::Grpc { port: 0 });
    // Note: grpc_threads is automatically set by HyperProcess
    let hyper = HyperProcess::new(None, Some(&params))?;
    let grpc_url = hyper.grpc_url().unwrap();

    let row_count: u64 = 10_000;
    let query = simple_query(row_count);

    print_table_header();

    for &(mode, mode_name) in TRANSFER_MODES {
        let config = GrpcConfig::new(&grpc_url).transfer_mode(mode);
        match GrpcConnection::connect_with_config(config) {
            Ok(mut conn) => match run_benchmark(&mut conn, &query, row_count, mode_name) {
                Ok(result) => print_table_row(&result),
                Err(e) => print_error_row(mode_name, row_count, &e.to_string()),
            },
            Err(e) => print_error_row(mode_name, row_count, &e.to_string()),
        }
    }

    print_table_footer();

    Ok(())
}

// ============================================================================
// Main Entry Point
// ============================================================================

fn main() -> Result<()> {
    benchmark_quick()?;
    benchmark_all_modes_all_scales()?;
    benchmark_100m_complex()?;
    Ok(())
}
