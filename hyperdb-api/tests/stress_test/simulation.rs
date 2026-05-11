// Copyright (c) 2026, Salesforce, Inc. All rights reserved.
// SPDX-License-Identifier: Apache-2.0 OR MIT

//! Monte Carlo simulation engine — spawns user threads, selects operations
//! stochastically, orchestrates the full lifecycle.

#![allow(
    clippy::cast_precision_loss,
    reason = "stress-test diagnostic; values bounded by test duration"
)]

use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::Arc;
use std::thread;
use std::time::{Duration, Instant};

use hyperdb_api::{Connection, CreateMode, HyperProcess, ListenMode, Parameters, TransportMode};
use rand::rngs::StdRng;
use rand::{RngExt, SeedableRng};

use super::config::{SimTransport, SimulationConfig};
use super::replay::{ReplayLog, StopReason, SummaryLog};
use super::resource_monitor::{self, MonitorState};
use super::stats::{StatsCollector, StatsSummary};
use super::user_profiles::{OpDistribution, OpKind, UserClass, UserDescriptor};
use super::workload::{self, DatabaseInfo};

// ---------------------------------------------------------------------------
// Public entry point
// ---------------------------------------------------------------------------

/// Run the full stress-test simulation. Returns the summary.
pub(crate) fn run(config: SimulationConfig) -> StatsSummary {
    eprintln!("[stress] === Hyper DB Monte Carlo Stress Test ===");
    eprintln!("[stress] Config: {config:?}");

    // -- Resolve output directory --
    let output_dir = resolve_output_dir(&config);
    std::fs::create_dir_all(&output_dir).expect("Failed to create output directory");
    eprintln!("[stress] Output directory: {}", output_dir.display());

    // -- Derive seeds --
    let global_seed = config.seed.unwrap_or_else(|| {
        let s = rand::random::<u64>();
        eprintln!("[stress] Generated random global seed: {s}");
        s
    });
    let mut seed_rng = StdRng::seed_from_u64(global_seed);

    // Per-database schema seeds
    let schema_seeds: Vec<u64> = (0..config.num_databases)
        .map(|_| seed_rng.random())
        .collect();

    // Per-user descriptors
    let user_descriptors = build_user_descriptors(&config, &mut seed_rng);

    // -- Start HyperProcess --
    let (hyper, log_dir) = start_hyper(&config, &output_dir);
    eprintln!(
        "[stress] HyperProcess started (pid={:?}, log_dir={})",
        hyper.pid(),
        log_dir.display()
    );

    // -- Spawn resource monitor --
    let monitor_state = Arc::new(MonitorState::new());
    let monitor_handle = resource_monitor::spawn_monitor(
        log_dir.clone(),
        config.max_memory_mb,
        config.max_cpu_percent,
        Arc::clone(&monitor_state),
    );

    // -- Setup databases --
    eprintln!("[stress] Setting up {} databases...", config.num_databases);
    let databases = workload::setup_databases(&hyper, &config, &schema_seeds, &output_dir)
        .expect("Failed to set up databases");
    let databases = Arc::new(databases);
    eprintln!("[stress] Databases ready.");

    // -- Stats collector --
    let stats = Arc::new(StatsCollector::new());

    // -- Global stop signal --
    let stop_flag = Arc::new(AtomicBool::new(false));
    let disk_used = Arc::new(AtomicU64::new(0));

    // -- Spawn user threads --
    let sim_start = Instant::now();
    let duration = config.duration();
    let config_arc = Arc::new(config.clone());

    let handles: Vec<_> = user_descriptors
        .iter()
        .map(|desc| {
            let desc = desc.clone();
            let databases = Arc::clone(&databases);
            let stats = Arc::clone(&stats);
            let stop = Arc::clone(&stop_flag);
            let monitor = Arc::clone(&monitor_state);
            let disk = Arc::clone(&disk_used);
            let cfg = Arc::clone(&config_arc);
            let hyper_endpoint = hyper.endpoint().map(std::string::ToString::to_string);

            thread::Builder::new()
                .name(format!("stress-user-{}", desc.user_id))
                .spawn(move || {
                    user_thread_loop(
                        &desc,
                        &databases,
                        &stats,
                        &stop,
                        &monitor,
                        &disk,
                        &cfg,
                        hyper_endpoint.as_deref(),
                        sim_start,
                        duration,
                    );
                })
                .expect("Failed to spawn user thread")
        })
        .collect();

    // -- Wait for duration or early stop --
    eprintln!(
        "[stress] Simulation running for {} seconds with {} user threads...",
        config.duration_secs,
        user_descriptors.len()
    );

    let check_interval = Duration::from_secs(5);
    loop {
        thread::sleep(check_interval);

        if sim_start.elapsed() >= duration {
            eprintln!("[stress] Duration elapsed.");
            break;
        }

        if !hyper.is_running() {
            eprintln!("[stress] *** HYPER PROCESS CRASHED ***");
            break;
        }
    }

    // Signal all threads to stop
    stop_flag.store(true, Ordering::SeqCst);

    // Wait for all user threads
    for h in handles {
        let _ = h.join();
    }

    // Stop resource monitor
    monitor_state.stop.store(true, Ordering::Relaxed);
    let _ = monitor_handle.join();

    let actual_duration = sim_start.elapsed();
    let hyper_crashed = !hyper.is_running();

    // -- Compute disk usage --
    let disk_mb = compute_disk_usage(&databases) as f64 / (1024.0 * 1024.0);

    // -- Compute summary --
    let stats_collector = Arc::try_unwrap(stats).unwrap_or_else(|arc| {
        // If other refs still exist (shouldn't happen), clone the inner
        let _inner = arc.as_ref();
        // We can't clone StatsCollector, so just create a new summary from nothing
        // This path should never be hit since all threads have joined.
        panic!("Stats collector still has multiple references");
    });
    let mut summary = stats_collector.into_summary();

    let peak_mem = *monitor_state.peak_memory_mb.lock().unwrap();
    let peak_cpu = *monitor_state.peak_cpu_percent.lock().unwrap();
    let peak_load = *monitor_state.peak_load.lock().unwrap();

    summary.finalize(
        actual_duration,
        peak_mem,
        peak_cpu,
        peak_load,
        disk_mb,
        hyper_crashed,
    );

    // -- Determine stop reason --
    let stop_reason = if hyper_crashed {
        StopReason::CrashDetected
    } else if monitor_state.backpressure.load(Ordering::Relaxed) {
        StopReason::ResourceLimit
    } else {
        StopReason::DurationElapsed
    };

    // -- Write replay log --
    let replay = ReplayLog {
        version: 1,
        global_seed,
        config: config.clone(),
        user_seeds: user_descriptors,
        schema_seeds,
        stop_reason,
    };
    if let Err(e) = replay.write(&output_dir) {
        eprintln!("[stress] WARNING: Failed to write replay log: {e}");
    }

    // -- Write summary log --
    let summary_log = SummaryLog {
        config,
        results: summary.clone(),
    };
    if let Err(e) = summary_log.write(&output_dir) {
        eprintln!("[stress] WARNING: Failed to write summary: {e}");
    }

    // -- Print summary --
    print_summary(&summary);

    summary
}

/// Run from a replay file.
pub(crate) fn run_replay(replay_path: &Path) -> StatsSummary {
    eprintln!(
        "[stress] === REPLAY MODE — loading {} ===",
        replay_path.display()
    );
    let replay = ReplayLog::load(replay_path).expect("Failed to load replay log");
    eprintln!("[stress] Loaded replay: global_seed={}", replay.global_seed);

    // Override config seed with the replay's global seed
    let mut config = replay.config;
    config.seed = Some(replay.global_seed);

    // Run with the same config — seeds will be identical
    run(config)
}

// ---------------------------------------------------------------------------
// Internal helpers
// ---------------------------------------------------------------------------

fn resolve_output_dir(config: &SimulationConfig) -> PathBuf {
    if let Some(ref dir) = config.output_dir {
        dir.clone()
    } else {
        let dir = std::env::temp_dir().join(format!("hyper_stress_{}", std::process::id()));
        dir
    }
}

fn start_hyper(config: &SimulationConfig, output_dir: &Path) -> (HyperProcess, PathBuf) {
    let log_dir = output_dir.join("logs");
    std::fs::create_dir_all(&log_dir).expect("Failed to create log directory");

    let mut params = Parameters::new();
    params.set("log_dir", log_dir.to_string_lossy().to_string());
    params.set(
        "log_resource_usage_interval",
        format!("{}s", config.log_resource_interval_secs),
    );
    params.set(
        "log_resource_usage_mode",
        config.log_resource_mode.to_string(),
    );
    params.set("log_resource_usage_always", "1");
    params.set("memory_limit", &config.memory_limit);
    params.set_transport_mode(TransportMode::Tcp);

    match config.transport {
        SimTransport::Tcp => {
            // Default ListenMode::LibPq (TCP only)
        }
        SimTransport::Grpc => {
            // Need both TCP (for inserts) and gRPC (for queries)
            params.set_listen_mode(ListenMode::Both { grpc_port: 0 });
        }
    }

    let hyper = HyperProcess::new(None, Some(&params)).expect("Failed to start HyperProcess");

    (hyper, log_dir)
}

fn build_user_descriptors(config: &SimulationConfig, seed_rng: &mut StdRng) -> Vec<UserDescriptor> {
    let mut descriptors = Vec::with_capacity(config.total_users());
    let mut user_id = 0;

    // Inserter users
    for _ in 0..config.inserter_users {
        descriptors.push(UserDescriptor {
            user_id,
            class: UserClass::Inserter,
            seed: seed_rng.random(),
            database_idx: user_id % config.num_databases,
        });
        user_id += 1;
    }

    // Query users
    for _ in 0..config.query_users {
        descriptors.push(UserDescriptor {
            user_id,
            class: UserClass::Query,
            seed: seed_rng.random(),
            database_idx: user_id % config.num_databases,
        });
        user_id += 1;
    }

    // Mixed users
    for _ in 0..config.mixed_users {
        descriptors.push(UserDescriptor {
            user_id,
            class: UserClass::Mixed,
            seed: seed_rng.random(),
            database_idx: user_id % config.num_databases,
        });
        user_id += 1;
    }

    descriptors
}

fn user_thread_loop(
    desc: &UserDescriptor,
    databases: &[DatabaseInfo],
    stats: &StatsCollector,
    stop: &AtomicBool,
    monitor: &MonitorState,
    disk_used: &AtomicU64,
    config: &SimulationConfig,
    hyper_endpoint: Option<&str>,
    sim_start: Instant,
    duration: Duration,
) {
    let mut rng = StdRng::seed_from_u64(desc.seed);
    let dist = OpDistribution::for_class(desc.class);
    let think_range = config.think_time.to_range();

    let db_info = &databases[desc.database_idx];

    // Open a connection to the user's primary database
    let endpoint = if let Some(ep) = hyper_endpoint {
        ep.to_string()
    } else {
        eprintln!(
            "[stress-user-{}] No endpoint available, exiting",
            desc.user_id
        );
        return;
    };

    let db_path_str = db_info.path.to_string_lossy();
    let conn = match Connection::connect(&endpoint, &db_path_str, CreateMode::DoNotCreate) {
        Ok(c) => c,
        Err(e) => {
            eprintln!("[stress-user-{}] Failed to connect: {}", desc.user_id, e);
            return;
        }
    };

    let mut op_count = 0u64;

    loop {
        // Check stop conditions
        if stop.load(Ordering::Relaxed) || sim_start.elapsed() >= duration {
            break;
        }

        // Backpressure: if thresholds exceeded, sleep and retry
        if monitor.backpressure.load(Ordering::Relaxed) {
            thread::sleep(Duration::from_millis(500));
            continue;
        }

        // Roll an operation
        let roll: f64 = rng.random();
        let mut op = dist.select(roll);

        // Disk budget check: if over budget, skip writes
        if op.is_write() {
            let current_disk = disk_used.load(Ordering::Relaxed);
            if current_disk >= config.max_total_disk_bytes {
                // Switch to a read operation instead
                op = OpKind::SimpleSelect;
            }
        }

        // Execute the operation
        let outcome = workload::execute_op(&conn, db_info, op, &mut rng, config);

        // Update disk usage estimate for inserts
        if outcome.success && op.is_write() && outcome.rows_affected > 0 {
            // Rough estimate: rows * avg_row_size
            let table_def = &db_info.tables[0]; // approximate
            let row_bytes = workload::estimate_row_bytes(table_def) as u64;
            disk_used.fetch_add(outcome.rows_affected * row_bytes, Ordering::Relaxed);
        }

        stats.record(outcome);
        op_count += 1;

        // Think time
        if think_range.start < think_range.end {
            let sleep_ms = rng.random_range(
                think_range.start.as_millis() as u64..think_range.end.as_millis() as u64,
            );
            if sleep_ms > 0 {
                thread::sleep(Duration::from_millis(sleep_ms));
            }
        }

        // Periodic progress (every 1000 ops)
        if op_count % 1000 == 0 {
            eprintln!(
                "[stress-user-{}] {} ops completed ({:.1}s elapsed)",
                desc.user_id,
                op_count,
                sim_start.elapsed().as_secs_f64()
            );
        }
    }

    eprintln!(
        "[stress-user-{}] Finished: {} ops total",
        desc.user_id, op_count
    );

    // Close connection gracefully
    if let Err(e) = conn.close() {
        eprintln!(
            "[stress-user-{}] Warning: connection close error: {}",
            desc.user_id, e
        );
    }
}

fn compute_disk_usage(databases: &[DatabaseInfo]) -> u64 {
    let mut total = 0u64;
    for db in databases {
        if let Ok(meta) = std::fs::metadata(&db.path) {
            total += meta.len();
        }
    }
    total
}

fn print_summary(summary: &StatsSummary) {
    eprintln!("\n[stress] ========== SIMULATION RESULTS ==========");
    eprintln!("[stress] Duration:    {:.1}s", summary.actual_duration_secs);
    eprintln!(
        "[stress] Operations:  {} total ({} ok, {} failed)",
        summary.total_operations, summary.successful_operations, summary.failed_operations
    );
    eprintln!(
        "[stress] Throughput:  {:.1} ops/sec, {:.0} insert-rows/sec",
        summary.throughput_ops_per_sec, summary.insert_rows_per_sec
    );
    eprintln!(
        "[stress] Latency:     p50={:.1}ms  p95={:.1}ms  p99={:.1}ms  max={:.1}ms",
        summary.latency_ms.p50,
        summary.latency_ms.p95,
        summary.latency_ms.p99,
        summary.latency_ms.max
    );
    eprintln!("[stress] Peak memory: {:.0} MB", summary.peak_memory_mb);
    eprintln!("[stress] Peak CPU:    {:.1}%", summary.peak_cpu_percent);
    eprintln!("[stress] Peak load:   {:.2}", summary.peak_load);
    eprintln!("[stress] Disk used:   {:.1} MB", summary.disk_used_mb);
    if summary.hyper_crashed {
        eprintln!("[stress] *** HYPER CRASHED DURING SIMULATION ***");
    }
    if !summary.errors.is_empty() {
        eprintln!("[stress] Errors ({}):", summary.failed_operations);
        for (i, e) in summary.errors.iter().enumerate().take(10) {
            eprintln!("[stress]   {}: {}", i + 1, e);
        }
        if summary.errors.len() > 10 {
            eprintln!("[stress]   ... and {} more", summary.errors.len() - 10);
        }
    }
    eprintln!("[stress] ============================================\n");
}
