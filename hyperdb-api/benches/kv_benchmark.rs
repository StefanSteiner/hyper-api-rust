// Copyright (c) 2026, Salesforce, Inc. All rights reserved.
// SPDX-License-Identifier: Apache-2.0 OR MIT

//! Key-value store write benchmark.
//!
//! Write paths:
//! - single-commit-per-set: one `KvStore::set` per fresh key (implicit commit)
//! - batched: `KvStore::set_batch` of `BATCH` fresh keys per transaction
//! - overwrite: one `KvStore::set` per *existing* key (implicit commit)
//!
//! Read / delete paths (each seeded untimed, then timed over existing keys):
//! - get: one `KvStore::get` per key (single `SELECT`, ~1 round-trip)
//! - delete: one `KvStore::delete` per key (single `DELETE`, ~1 round-trip)
//! - pop: one `KvStore::pop` per key — transactional
//!   (`BEGIN`+`SELECT`+`DELETE`+`COMMIT`, ~4 round-trips), so it is the slowest
//!
//! The fresh-vs-overwrite comparison exposes the upsert's two paths: a fresh
//! key does `UPDATE` (0 rows) then `INSERT` (~2 round-trips), while overwriting
//! an existing key does `UPDATE` only — the conditional `INSERT` is skipped
//! (~1 round-trip), so it runs roughly twice as fast.
//!
//! Run with:
//!   cargo run -p hyperdb-api --release --example kv_benchmark            # default 50k keys
//!   cargo run -p hyperdb-api --release --example kv_benchmark 200000     # 200k keys

// Throughput math converts a `usize` key count to `f64`; the resulting
// precision loss is irrelevant for a keys/sec figure. `allow` (not `expect`)
// because this is the only pedantic cast lint that fires here — an `expect`
// listing others would trip `unfulfilled_lint_expectations` under `-D warnings`.
#![allow(
    clippy::cast_precision_loss,
    reason = "benchmark throughput math needs usize -> f64; precision loss is cosmetic"
)]

use hyperdb_api::{Connection, CreateMode, HyperProcess, Result};
use std::env;
use std::time::Instant;

const DEFAULT_KEYS: usize = 50_000;
const BATCH: usize = 25; // within the requested 10-50 range

fn key_count() -> usize {
    env::args()
        .nth(1)
        .and_then(|s| s.parse::<usize>().ok())
        .unwrap_or(DEFAULT_KEYS)
}

fn throughput(label: &str, keys: usize, secs: f64) {
    let per_sec = if secs > 0.0 { keys as f64 / secs } else { 0.0 };
    println!("  {label:<28} {keys} keys in {secs:>7.3}s  =>  {per_sec:>12.0} keys/sec");
}

fn bench_single(conn: &Connection, keys: usize) -> Result<f64> {
    let kv = conn.kv_store("bench_single")?;
    kv.clear()?;
    let start = Instant::now();
    for i in 0..keys {
        kv.set(&format!("k{i}"), "value")?;
    }
    Ok(start.elapsed().as_secs_f64())
}

fn bench_overwrite(conn: &Connection, keys: usize) -> Result<f64> {
    let kv = conn.kv_store("bench_overwrite")?;
    kv.clear()?;
    // Seed every key first (untimed) so the timed loop only overwrites — the
    // upsert's `UPDATE` hits and the conditional `INSERT` is skipped.
    for i in 0..keys {
        kv.set(&format!("k{i}"), "seed")?;
    }
    let start = Instant::now();
    for i in 0..keys {
        kv.set(&format!("k{i}"), "value")?;
    }
    Ok(start.elapsed().as_secs_f64())
}

fn bench_batched(conn: &Connection, keys: usize) -> Result<f64> {
    let kv = conn.kv_store("bench_batched")?;
    kv.clear()?;
    let start = Instant::now();
    let mut i = 0;
    while i < keys {
        let end = (i + BATCH).min(keys);
        // Own the strings, then borrow into the &[(&str, &str)] slice.
        let owned: Vec<(String, String)> = (i..end)
            .map(|n| (format!("k{n}"), "value".to_string()))
            .collect();
        let batch: Vec<(&str, &str)> = owned
            .iter()
            .map(|(k, v)| (k.as_str(), v.as_str()))
            .collect();
        kv.set_batch(&batch)?;
        i = end;
    }
    Ok(start.elapsed().as_secs_f64())
}

/// Seeds `keys` entries into `store` (untimed helper for the read/delete paths).
fn seed(conn: &Connection, store: &str, keys: usize) -> Result<()> {
    let kv = conn.kv_store(store)?;
    kv.clear()?;
    for i in 0..keys {
        kv.set(&format!("k{i}"), "value")?;
    }
    Ok(())
}

fn bench_get(conn: &Connection, keys: usize) -> Result<f64> {
    seed(conn, "bench_get", keys)?;
    let kv = conn.kv_store("bench_get")?;
    let start = Instant::now();
    for i in 0..keys {
        // `?` propagates errors; the returned value is intentionally discarded.
        let _ = kv.get(&format!("k{i}"))?;
    }
    Ok(start.elapsed().as_secs_f64())
}

fn bench_delete(conn: &Connection, keys: usize) -> Result<f64> {
    seed(conn, "bench_delete", keys)?;
    let kv = conn.kv_store("bench_delete")?;
    let start = Instant::now();
    for i in 0..keys {
        let _ = kv.delete(&format!("k{i}"))?;
    }
    Ok(start.elapsed().as_secs_f64())
}

fn bench_pop(conn: &Connection, keys: usize) -> Result<f64> {
    seed(conn, "bench_pop", keys)?;
    let kv = conn.kv_store("bench_pop")?;
    let start = Instant::now();
    // Each pop is one transaction (BEGIN + SELECT + DELETE + COMMIT).
    for _ in 0..keys {
        let _ = kv.pop()?;
    }
    Ok(start.elapsed().as_secs_f64())
}

fn main() -> Result<()> {
    let keys = key_count();
    println!("\n=== KV Store write benchmark ({keys} keys, batch size {BATCH}) ===");

    let db_path = std::env::temp_dir().join("kv_benchmark.hyper");
    let hyper = HyperProcess::new(None, None)?;
    let conn = Connection::new(&hyper, &db_path, CreateMode::CreateAndReplace)?;

    let single_secs = bench_single(&conn, keys)?;
    throughput("single commit per set", keys, single_secs);

    let batched_secs = bench_batched(&conn, keys)?;
    throughput(&format!("batched ({BATCH}/txn)"), keys, batched_secs);

    let overwrite_secs = bench_overwrite(&conn, keys)?;
    throughput("overwrite existing keys", keys, overwrite_secs);

    let get_secs = bench_get(&conn, keys)?;
    throughput("get existing keys", keys, get_secs);

    let delete_secs = bench_delete(&conn, keys)?;
    throughput("delete existing keys", keys, delete_secs);

    let pop_secs = bench_pop(&conn, keys)?;
    throughput("pop (transactional)", keys, pop_secs);

    if batched_secs > 0.0 {
        println!(
            "\n  speedup (batched vs single):        {:.2}x",
            single_secs / batched_secs
        );
    }
    // Overwriting an existing key skips the conditional INSERT, so it should
    // beat the fresh-insert path (which pays UPDATE + INSERT per key).
    if overwrite_secs > 0.0 {
        println!(
            "  speedup (overwrite vs fresh insert): {:.2}x",
            single_secs / overwrite_secs
        );
    }
    Ok(())
}
