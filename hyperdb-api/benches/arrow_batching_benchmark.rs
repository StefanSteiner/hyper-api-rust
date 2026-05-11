// Copyright (c) 2026, Salesforce, Inc. All rights reserved.
// SPDX-License-Identifier: Apache-2.0 OR MIT

//! Benchmark comparing Arrow insert with flush-per-message vs batched flushing.
//!
//! This benchmark demonstrates the performance improvement from batching Arrow IPC
//! messages before flushing to TCP, instead of flushing after every message.
//! It runs both sync and async implementations for comparison.
//!
//! Run with: cargo run -p hyperdb-api --release --example `arrow_batching_benchmark` -- [`ROW_COUNT`]
//!
//! Examples:
//!   cargo run -p hyperdb-api --release --example `arrow_batching_benchmark`             # Default 10M rows
//!   cargo run -p hyperdb-api --release --example `arrow_batching_benchmark` -- 50000000 # 50M rows

// Benchmark harness: intentional wide→narrow conversions for row-count display,
// throughput math, and indexing with bounds the benchmark itself enforces.
// Also allows a handful of idiom lints the benchmark deliberately trips:
// `drop(sink)` for readability, and `RefCell` borrows held across await in a
// single-threaded bench where the invariant is trivially upheld.
#![expect(
    clippy::cast_possible_truncation,
    clippy::cast_sign_loss,
    clippy::cast_possible_wrap,
    clippy::cast_precision_loss,
    clippy::drop_non_drop,
    clippy::await_holding_refcell_ref,
    reason = "benchmark harness: counts/indices narrow by bench-enforced invariants; single-threaded RefCell borrows across await are safe here"
)]

use std::env;
use std::io::{self, Write};
use std::sync::Arc;
use std::time::{Duration, Instant};

use arrow::array::{Float64Array, Int32Array, StringArray};
use arrow::datatypes::{DataType, Field, Schema};
use arrow::ipc::writer::StreamWriter;
use arrow::record_batch::RecordBatch;

use hyperdb_api::{
    ArrowInserter, AsyncArrowInserter, AsyncConnection, Catalog, Connection, CreateMode,
    HyperProcess, Parameters, Result, SqlType, TableDefinition, TransportMode,
};

const DEFAULT_ROW_COUNT: usize = 10_000_000;
const BATCH_SIZE: usize = 100_000;

fn main() -> Result<()> {
    // Parse command line arguments
    let row_count = env::args()
        .nth(1)
        .and_then(|s| s.parse::<usize>().ok())
        .unwrap_or(DEFAULT_ROW_COUNT);

    println!("=== Arrow Batching Benchmark (Sync + Async) ===\n");

    println!("Configuration:");
    println!("  Rows to insert: {}", format_number(row_count));
    println!("  Batch size: {}", format_number(BATCH_SIZE));
    println!("  Default flush threshold: 16 MB");
    println!();

    // Create test_results directory
    std::fs::create_dir_all("test_results")?;

    // Start Hyper process
    println!("Starting Hyper process...");
    let mut params = Parameters::new();
    params.set("log_dir", "test_results");
    let hyper = HyperProcess::new(None, Some(&params))?;

    let sync_db_path = "test_results/arrow_batching_sync.hyper";
    let async_db_path = "test_results/arrow_batching_async.hyper";

    // Create table definition
    let table_def = TableDefinition::new("benchmark_data")
        .add_required_column("id", SqlType::int())
        .add_nullable_column("value", SqlType::double())
        .add_nullable_column("text", SqlType::text());

    // ===== SYNC BENCHMARKS =====
    println!("━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━");
    println!("                              SYNC MODE");
    println!("━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━\n");

    let connection = Connection::new(&hyper, sync_db_path, CreateMode::CreateAndReplace)?;
    println!("Created database: {sync_db_path}\n");

    println!("=== Sync 1: Flush Per Message (old behavior) ===");
    Catalog::new(&connection).create_table(&table_def)?;
    let sync_flush_per_msg = run_benchmark(&connection, &table_def, row_count, 1)?;
    connection.execute_command("DROP TABLE benchmark_data")?;

    println!("\n=== Sync 2: Batched Flushing (16MB threshold) ===");
    Catalog::new(&connection).create_table(&table_def)?;
    let sync_batched = run_benchmark(&connection, &table_def, row_count, 16 * 1024 * 1024)?;
    connection.execute_command("DROP TABLE benchmark_data")?;

    println!("\n=== Sync 3: Large Batch Flushing (64MB threshold) ===");
    Catalog::new(&connection).create_table(&table_def)?;
    let sync_large_batch = run_benchmark(&connection, &table_def, row_count, 64 * 1024 * 1024)?;

    drop(connection);

    // ===== ASYNC BENCHMARKS =====
    println!("\n━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━");
    println!("                             ASYNC MODE");
    println!("━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━\n");

    let rt = tokio::runtime::Runtime::new().expect("Failed to create tokio runtime");
    let (async_flush_per_msg, async_batched, async_large_batch) = rt.block_on(
        run_async_benchmarks(&hyper, async_db_path, &table_def, row_count),
    )?;

    // Print combined results
    println!("\n");
    println!("╔══════════════════════════════════════════════════════════════════════════════╗");
    println!("║                            BENCHMARK RESULTS                                 ║");
    println!("╠══════════════════════════════════════════════════════════════════════════════╣");
    println!("║ Method                        │ Time (s) │ Rows/sec    │ MB/sec │ Speedup   ║");
    println!("╠═══════════════════════════════╪══════════╪═════════════╪════════╪═══════════╣");

    let sync_baseline = sync_flush_per_msg.elapsed.as_secs_f64();
    let async_baseline = async_flush_per_msg.elapsed.as_secs_f64();

    // Sync results
    println!("║ SYNC                          │          │             │        │           ║");
    print_result_row_wide("  Flush per message", &sync_flush_per_msg, 1.0);
    print_result_row_wide(
        "  Batched (16MB)",
        &sync_batched,
        sync_baseline / sync_batched.elapsed.as_secs_f64(),
    );
    print_result_row_wide(
        "  Batched (64MB)",
        &sync_large_batch,
        sync_baseline / sync_large_batch.elapsed.as_secs_f64(),
    );

    println!("╠═══════════════════════════════╪══════════╪═════════════╪════════╪═══════════╣");

    // Async results
    println!("║ ASYNC                         │          │             │        │           ║");
    print_result_row_wide("  Flush per message", &async_flush_per_msg, 1.0);
    print_result_row_wide(
        "  Batched (16MB)",
        &async_batched,
        async_baseline / async_batched.elapsed.as_secs_f64(),
    );
    print_result_row_wide(
        "  Batched (64MB)",
        &async_large_batch,
        async_baseline / async_large_batch.elapsed.as_secs_f64(),
    );

    println!("╚══════════════════════════════════════════════════════════════════════════════╝");

    // Print database file size
    if let Ok(metadata) = std::fs::metadata(async_db_path) {
        let size_mb = metadata.len() as f64 / (1024.0 * 1024.0);
        println!("\nDatabase file size: {size_mb:.2} MB");
    }

    // Clean up before IPC vs TCP comparison
    drop(hyper);
    let _ = std::fs::remove_file(sync_db_path);
    let _ = std::fs::remove_file(async_db_path);

    // ===== IPC vs TCP TRANSPORT COMPARISON =====
    {
        #[cfg(unix)]
        let ipc_label = "IPC (Unix Sock)";
        #[cfg(windows)]
        let ipc_label = "IPC (NamedPipe)";
        #[cfg(not(any(unix, windows)))]
        let ipc_label = "IPC";

        #[cfg(unix)]
        let ipc_mode_name = "IPC Mode (Unix Socket)";
        #[cfg(windows)]
        let ipc_mode_name = "IPC Mode (Named Pipe)";
        #[cfg(not(any(unix, windows)))]
        let ipc_mode_name = "IPC Mode";

        println!("\n━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━");
        println!("                     IPC vs TCP TRANSPORT COMPARISON");
        println!("━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━\n");

        let ipc_db_path = "test_results/arrow_ipc.hyper";
        let tcp_db_path = "test_results/arrow_tcp.hyper";

        // TCP benchmark
        println!("--- TCP Mode ---");
        let mut tcp_params = Parameters::new();
        tcp_params.set("log_dir", "test_results");
        tcp_params.set_transport_mode(TransportMode::Tcp);
        let tcp_hyper = HyperProcess::new(None, Some(&tcp_params))?;
        println!("  Transport: {:?}", tcp_hyper.transport_mode());
        let tcp_conn = Connection::new(&tcp_hyper, tcp_db_path, CreateMode::CreateAndReplace)?;
        Catalog::new(&tcp_conn).create_table(&table_def)?;
        let tcp_result = run_benchmark(&tcp_conn, &table_def, row_count, 16 * 1024 * 1024)?;
        drop(tcp_conn);
        drop(tcp_hyper);
        let _ = std::fs::remove_file(tcp_db_path);

        // IPC benchmark
        println!("\n--- {ipc_mode_name} ---");
        let mut ipc_params = Parameters::new();
        ipc_params.set("log_dir", "test_results");
        ipc_params.set_transport_mode(TransportMode::Ipc);
        let ipc_hyper = HyperProcess::new(None, Some(&ipc_params))?;
        println!("  Transport: {:?}", ipc_hyper.transport_mode());
        let ipc_conn = Connection::new(&ipc_hyper, ipc_db_path, CreateMode::CreateAndReplace)?;
        Catalog::new(&ipc_conn).create_table(&table_def)?;
        let ipc_result = run_benchmark(&ipc_conn, &table_def, row_count, 16 * 1024 * 1024)?;
        drop(ipc_conn);
        drop(ipc_hyper);
        let _ = std::fs::remove_file(ipc_db_path);

        // Print IPC vs TCP comparison
        println!(
            "\n╔══════════════════════════════════════════════════════════════════════════════╗"
        );
        println!(
            "║                        IPC vs TCP COMPARISON                                 ║"
        );
        println!(
            "╠══════════════════════════════════════════════════════════════════════════════╣"
        );
        println!(
            "║ Transport       │ Time (s)  │ Rows/sec      │ MB/sec   │ vs TCP              ║"
        );
        println!(
            "╠═════════════════╪═══════════╪═══════════════╪══════════╪═════════════════════╣"
        );

        let tcp_rows_per_sec = tcp_result.rows as f64 / tcp_result.elapsed.as_secs_f64();
        let tcp_mb_per_sec =
            tcp_result.total_bytes as f64 / (1024.0 * 1024.0) / tcp_result.elapsed.as_secs_f64();
        let ipc_rows_per_sec = ipc_result.rows as f64 / ipc_result.elapsed.as_secs_f64();
        let ipc_mb_per_sec =
            ipc_result.total_bytes as f64 / (1024.0 * 1024.0) / ipc_result.elapsed.as_secs_f64();
        let speedup = tcp_result.elapsed.as_secs_f64() / ipc_result.elapsed.as_secs_f64();

        println!(
            "║ TCP             │ {:>9.3} │ {:>13.0} │ {:>8.2} │ baseline            ║",
            tcp_result.elapsed.as_secs_f64(),
            tcp_rows_per_sec,
            tcp_mb_per_sec
        );
        println!(
            "║ {:15} │ {:>9.3} │ {:>13.0} │ {:>8.2} │ {:>5.2}x {:>13} ║",
            ipc_label,
            ipc_result.elapsed.as_secs_f64(),
            ipc_rows_per_sec,
            ipc_mb_per_sec,
            speedup,
            if speedup > 1.0 { "faster" } else { "slower" }
        );
        println!(
            "╚══════════════════════════════════════════════════════════════════════════════╝"
        );

        if speedup > 1.01 {
            println!(
                "\nIPC is {:.1}% faster than TCP for Arrow inserts",
                (speedup - 1.0) * 100.0
            );
        } else if speedup < 0.99 {
            println!(
                "\nIPC is {:.1}% slower than TCP for Arrow inserts",
                (1.0 / speedup - 1.0) * 100.0
            );
        } else {
            println!("\nIPC and TCP performance are approximately equal");
        }
    }

    println!("\nBenchmark completed!");
    Ok(())
}

struct BenchmarkResult {
    elapsed: Duration,
    rows: u64,
    total_bytes: usize,
}

fn run_benchmark(
    connection: &Connection,
    table_def: &TableDefinition,
    row_count: usize,
    flush_threshold: usize,
) -> Result<BenchmarkResult> {
    let schema = Arc::new(Schema::new(vec![
        Field::new("id", DataType::Int32, false),
        Field::new("value", DataType::Float64, true),
        Field::new("text", DataType::Utf8, true),
    ]));

    let start = Instant::now();
    let mut inserter =
        ArrowInserter::new(connection, table_def)?.with_flush_threshold(flush_threshold);

    let mut total_bytes = 0usize;

    // Use a custom sink that writes directly to the inserter
    struct InserterSink<'conn, 'b> {
        inserter: &'b mut ArrowInserter<'conn>,
        total_bytes: &'b mut usize,
    }

    impl Write for InserterSink<'_, '_> {
        fn write(&mut self, buf: &[u8]) -> io::Result<usize> {
            if buf.is_empty() {
                return Ok(0);
            }
            self.inserter
                .insert_raw(buf)
                .map_err(|e| io::Error::other(e.to_string()))?;
            *self.total_bytes += buf.len();
            Ok(buf.len())
        }

        fn flush(&mut self) -> io::Result<()> {
            Ok(())
        }
    }

    let mut sink = InserterSink {
        inserter: &mut inserter,
        total_bytes: &mut total_bytes,
    };

    let mut writer =
        StreamWriter::try_new(&mut sink, &schema).expect("Failed to create StreamWriter");

    let num_batches = row_count.div_ceil(BATCH_SIZE);
    for batch_idx in 0..num_batches {
        let batch_start = batch_idx * BATCH_SIZE;
        let batch_end = (batch_start + BATCH_SIZE).min(row_count);

        let ids: Vec<i32> = (batch_start..batch_end).map(|i| i as i32).collect();
        let values: Vec<Option<f64>> = (batch_start..batch_end)
            .map(|i| Some(i as f64 * 0.1))
            .collect();
        let texts: Vec<Option<String>> = (batch_start..batch_end)
            .map(|i| Some(generate_text(i, 32)))
            .collect();

        let batch = RecordBatch::try_new(
            Arc::clone(&schema),
            vec![
                Arc::new(Int32Array::from(ids)),
                Arc::new(Float64Array::from(values)),
                Arc::new(StringArray::from(texts)),
            ],
        )
        .expect("Failed to create record batch");

        writer.write(&batch).expect("Failed to write batch");

        // Progress indicator
        if (batch_idx + 1) % 10 == 0 || batch_idx + 1 == num_batches {
            let progress = (batch_idx + 1) * 100 / num_batches;
            let rows_so_far = batch_end;
            print!(
                "\r  Progress: {:3}% ({} rows)",
                progress,
                format_number(rows_so_far)
            );
            io::stdout().flush().ok();
        }
    }

    writer.finish().expect("Failed to finish stream");
    drop(sink);

    let rows = inserter.execute()?;
    let elapsed = start.elapsed();

    println!();
    println!(
        "  Inserted {} rows in {:.3}s ({:.0} rows/sec, {:.2} MB/sec)",
        format_number(rows as usize),
        elapsed.as_secs_f64(),
        rows as f64 / elapsed.as_secs_f64(),
        total_bytes as f64 / (1024.0 * 1024.0) / elapsed.as_secs_f64()
    );

    Ok(BenchmarkResult {
        elapsed,
        rows,
        total_bytes,
    })
}

fn print_result_row_wide(name: &str, result: &BenchmarkResult, speedup: f64) {
    let rows_per_sec = result.rows as f64 / result.elapsed.as_secs_f64();
    let mb_per_sec = result.total_bytes as f64 / (1024.0 * 1024.0) / result.elapsed.as_secs_f64();

    println!(
        "║ {:29} │ {:8.2} │ {:11} │ {:6.1} │ {:8.2}x ║",
        name,
        result.elapsed.as_secs_f64(),
        format_number(rows_per_sec as usize),
        mb_per_sec,
        speedup
    );
}

async fn run_async_benchmarks(
    hyper: &HyperProcess,
    db_path: &str,
    table_def: &TableDefinition,
    row_count: usize,
) -> Result<(BenchmarkResult, BenchmarkResult, BenchmarkResult)> {
    let endpoint = hyper
        .endpoint()
        .expect("HyperProcess must have TCP endpoint");
    let connection =
        AsyncConnection::connect(endpoint, db_path, CreateMode::CreateAndReplace).await?;
    println!("Created database: {db_path}\n");

    // Run benchmark with flush-per-message (threshold = 1 byte forces flush every time)
    println!("=== Benchmark 1: Flush Per Message (old behavior) ===");
    connection
        .execute_command(
            "CREATE TABLE benchmark_data (id INT NOT NULL, value DOUBLE PRECISION, text TEXT)",
        )
        .await?;
    let result_flush_per_msg = run_async_benchmark(&connection, table_def, row_count, 1).await?;
    connection
        .execute_command("DROP TABLE benchmark_data")
        .await?;

    // Run benchmark with batched flushing (default 16MB threshold)
    println!("\n=== Benchmark 2: Batched Flushing (16MB threshold) ===");
    connection
        .execute_command(
            "CREATE TABLE benchmark_data (id INT NOT NULL, value DOUBLE PRECISION, text TEXT)",
        )
        .await?;
    let result_batched =
        run_async_benchmark(&connection, table_def, row_count, 16 * 1024 * 1024).await?;
    connection
        .execute_command("DROP TABLE benchmark_data")
        .await?;

    // Run benchmark with larger batch (64MB threshold)
    println!("\n=== Benchmark 3: Large Batch Flushing (64MB threshold) ===");
    connection
        .execute_command(
            "CREATE TABLE benchmark_data (id INT NOT NULL, value DOUBLE PRECISION, text TEXT)",
        )
        .await?;
    let result_large_batch =
        run_async_benchmark(&connection, table_def, row_count, 64 * 1024 * 1024).await?;

    Ok((result_flush_per_msg, result_batched, result_large_batch))
}

async fn run_async_benchmark(
    connection: &AsyncConnection,
    table_def: &TableDefinition,
    row_count: usize,
    flush_threshold: usize,
) -> Result<BenchmarkResult> {
    let schema = Arc::new(Schema::new(vec![
        Field::new("id", DataType::Int32, false),
        Field::new("value", DataType::Float64, true),
        Field::new("text", DataType::Utf8, true),
    ]));

    let start = Instant::now();
    let mut inserter =
        AsyncArrowInserter::new(connection, table_def)?.with_flush_threshold(flush_threshold);

    let mut total_bytes = 0usize;
    let num_batches = row_count.div_ceil(BATCH_SIZE);

    // Use a shared buffer wrapped in Rc<RefCell> to allow StreamWriter to write
    // while we can still access the data
    let buf = std::rc::Rc::new(std::cell::RefCell::new(Vec::new()));

    // Custom writer that appends to our shared buffer
    struct SharedBufWriter(std::rc::Rc<std::cell::RefCell<Vec<u8>>>);
    impl Write for SharedBufWriter {
        fn write(&mut self, data: &[u8]) -> io::Result<usize> {
            self.0.borrow_mut().extend_from_slice(data);
            Ok(data.len())
        }
        fn flush(&mut self) -> io::Result<()> {
            Ok(())
        }
    }

    let mut writer = StreamWriter::try_new(SharedBufWriter(std::rc::Rc::clone(&buf)), &schema)
        .expect("Failed to create StreamWriter");

    for batch_idx in 0..num_batches {
        let batch_start = batch_idx * BATCH_SIZE;
        let batch_end = (batch_start + BATCH_SIZE).min(row_count);

        let ids: Vec<i32> = (batch_start..batch_end).map(|i| i as i32).collect();
        let values: Vec<Option<f64>> = (batch_start..batch_end)
            .map(|i| Some(i as f64 * 0.1))
            .collect();
        let texts: Vec<Option<String>> = (batch_start..batch_end)
            .map(|i| Some(generate_text(i, 32)))
            .collect();

        let batch = RecordBatch::try_new(
            Arc::clone(&schema),
            vec![
                Arc::new(Int32Array::from(ids)),
                Arc::new(Float64Array::from(values)),
                Arc::new(StringArray::from(texts)),
            ],
        )
        .expect("Failed to create record batch");

        // Write batch to the StreamWriter
        writer.write(&batch).expect("Failed to write batch");

        // Send accumulated data to inserter and clear buffer
        {
            let mut b = buf.borrow_mut();
            if !b.is_empty() {
                inserter.insert_raw(&b).await?;
                total_bytes += b.len();
                b.clear();
            }
        }

        // Progress indicator
        if (batch_idx + 1) % 10 == 0 || batch_idx + 1 == num_batches {
            let progress = (batch_idx + 1) * 100 / num_batches;
            let rows_so_far = batch_end;
            print!(
                "\r  Progress: {:3}% ({} rows)",
                progress,
                format_number(rows_so_far)
            );
            io::stdout().flush().ok();
        }
    }

    // Finish the stream and send any remaining data
    writer.finish().expect("Failed to finish stream");
    {
        let b = buf.borrow();
        if !b.is_empty() {
            inserter.insert_raw(&b).await?;
            total_bytes += b.len();
        }
    }

    let rows = inserter.execute().await?;
    let elapsed = start.elapsed();

    println!();
    println!(
        "  Inserted {} rows in {:.3}s ({:.0} rows/sec, {:.2} MB/sec)",
        format_number(rows as usize),
        elapsed.as_secs_f64(),
        rows as f64 / elapsed.as_secs_f64(),
        total_bytes as f64 / (1024.0 * 1024.0) / elapsed.as_secs_f64()
    );

    Ok(BenchmarkResult {
        elapsed,
        rows,
        total_bytes,
    })
}

fn generate_text(seed: usize, max_len: usize) -> String {
    let len = (seed % max_len) + 1;
    let mut result = String::with_capacity(len);
    for i in 0..len {
        let char_code = (seed.wrapping_mul(31).wrapping_add(i)) % 62;
        let c = if char_code < 26 {
            (b'a' + char_code as u8) as char
        } else if char_code < 52 {
            (b'A' + (char_code - 26) as u8) as char
        } else {
            (b'0' + (char_code - 52) as u8) as char
        };
        result.push(c);
    }
    result
}

fn format_number(n: usize) -> String {
    let s = n.to_string();
    let mut result = String::new();
    for (i, c) in s.chars().rev().enumerate() {
        if i > 0 && i % 3 == 0 {
            result.push(',');
        }
        result.push(c);
    }
    result.chars().rev().collect()
}
