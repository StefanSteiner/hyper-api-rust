// Copyright (c) 2026, Salesforce, Inc. All rights reserved.
// SPDX-License-Identifier: Apache-2.0 OR MIT

//! Hyper log tailer that parses `resource-metrics` JSON entries and enforces
//! memory / CPU backpressure thresholds.

#![allow(
    clippy::cast_precision_loss,
    reason = "stress-test diagnostic; values bounded by test duration"
)]

use std::fs::File;
use std::io::{BufRead, BufReader, Seek, SeekFrom};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::thread;
use std::time::{Duration, Instant};

use serde_json::Value;

// ---------------------------------------------------------------------------
// Snapshot of resource metrics at one point in time
// ---------------------------------------------------------------------------

/// A single resource-metrics sample parsed from the Hyper log.
#[derive(Debug, Clone)]
pub(crate) struct ResourceSample {
    #[expect(
        dead_code,
        reason = "retained for future log-correlation use; sampled even when unused by current tests"
    )]
    pub timestamp: Instant,
    /// Process physical memory in MB.
    pub process_memory_mb: f64,
    /// Process CPU utilization [0..1].
    pub process_cpu: f64,
    /// System CPU utilization [0..1].
    pub system_cpu: f64,
    /// Overall load [0..1].
    pub overall_load: f64,
    /// Scheduler load [0..1].
    pub scheduler_load: f64,
    /// Memory load [0..1].
    pub memory_load: f64,
    /// Scheduler thread count.
    pub scheduler_threads: i64,
    /// Waiting jobs in scheduler.
    pub scheduler_waiting_jobs: i64,
}

// ---------------------------------------------------------------------------
// Shared state
// ---------------------------------------------------------------------------

/// Thread-safe shared state between the monitor thread and user threads.
pub(crate) struct MonitorState {
    /// Set to `true` when resource thresholds are exceeded.
    pub backpressure: AtomicBool,
    /// Set to `true` to stop the monitor thread.
    pub stop: AtomicBool,
    /// Time-series of resource samples.
    pub samples: Mutex<Vec<ResourceSample>>,
    /// Peak memory observed (MB).
    pub peak_memory_mb: Mutex<f64>,
    /// Peak CPU observed.
    pub peak_cpu_percent: Mutex<f64>,
    /// Peak load observed.
    pub peak_load: Mutex<f64>,
}

impl MonitorState {
    pub(crate) fn new() -> Self {
        Self {
            backpressure: AtomicBool::new(false),
            stop: AtomicBool::new(false),
            samples: Mutex::new(Vec::new()),
            peak_memory_mb: Mutex::new(0.0),
            peak_cpu_percent: Mutex::new(0.0),
            peak_load: Mutex::new(0.0),
        }
    }
}

// ---------------------------------------------------------------------------
// Monitor thread
// ---------------------------------------------------------------------------

/// Spawn the resource monitor thread. Returns a join handle.
///
/// The thread tails `hyperd.log` in `log_dir`, parses resource-metrics JSON
/// entries, updates `state`, and sets the backpressure flag when thresholds
/// are exceeded.
pub(crate) fn spawn_monitor(
    log_dir: PathBuf,
    max_memory_mb: u64,
    max_cpu_percent: f64,
    state: Arc<MonitorState>,
) -> thread::JoinHandle<()> {
    thread::Builder::new()
        .name("stress-resource-monitor".into())
        .spawn(move || {
            monitor_loop(&log_dir, max_memory_mb, max_cpu_percent, &state);
        })
        .expect("Failed to spawn resource monitor thread")
}

fn monitor_loop(log_dir: &Path, max_memory_mb: u64, max_cpu_percent: f64, state: &MonitorState) {
    let log_path = log_dir.join("hyperd.log");

    // Wait for the log file to appear (Hyper may not have created it yet).
    let mut waited = Duration::ZERO;
    while !log_path.exists() {
        if state.stop.load(Ordering::Relaxed) {
            return;
        }
        thread::sleep(Duration::from_millis(200));
        waited += Duration::from_millis(200);
        if waited > Duration::from_secs(30) {
            eprintln!(
                "[stress-monitor] WARNING: hyperd.log not found at {} after 30s",
                log_path.display()
            );
            return;
        }
    }

    let file = match File::open(&log_path) {
        Ok(f) => f,
        Err(e) => {
            eprintln!(
                "[stress-monitor] Failed to open {}: {}",
                log_path.display(),
                e
            );
            return;
        }
    };

    let mut reader = BufReader::new(file);
    // Seek to end — we only care about entries produced during this run.
    let _ = reader.seek(SeekFrom::End(0));

    let mut line_buf = String::new();

    while !state.stop.load(Ordering::Relaxed) {
        line_buf.clear();
        match reader.read_line(&mut line_buf) {
            Ok(0) => {
                // No new data — sleep briefly and retry.
                thread::sleep(Duration::from_millis(250));
            }
            Ok(_) => {
                if let Some(sample) = try_parse_resource_metrics(&line_buf) {
                    // Update peaks
                    {
                        let mut peak_mem = state.peak_memory_mb.lock().unwrap();
                        if sample.process_memory_mb > *peak_mem {
                            *peak_mem = sample.process_memory_mb;
                        }
                    }
                    {
                        let mut peak_cpu = state.peak_cpu_percent.lock().unwrap();
                        let cpu_pct = sample.process_cpu * 100.0;
                        if cpu_pct > *peak_cpu {
                            *peak_cpu = cpu_pct;
                        }
                    }
                    {
                        let mut peak_load = state.peak_load.lock().unwrap();
                        if sample.overall_load > *peak_load {
                            *peak_load = sample.overall_load;
                        }
                    }

                    // Check thresholds
                    let mem_exceeded = sample.process_memory_mb > max_memory_mb as f64;
                    let cpu_exceeded = (sample.process_cpu * 100.0) > max_cpu_percent;
                    state
                        .backpressure
                        .store(mem_exceeded || cpu_exceeded, Ordering::Relaxed);

                    // Store sample
                    if let Ok(mut samples) = state.samples.lock() {
                        samples.push(sample);
                    }
                }
            }
            Err(e) => {
                eprintln!("[stress-monitor] Read error: {e}");
                thread::sleep(Duration::from_millis(500));
            }
        }
    }
}

// ---------------------------------------------------------------------------
// Log line parsing
// ---------------------------------------------------------------------------

/// Try to parse a single log line as a `resource-metrics` entry.
///
/// Hyper's JSON log format puts each entry on its own line with fields like:
/// `{"k":"resource-metrics","v":{...},"ts":"...","lvl":"info"}`
fn try_parse_resource_metrics(line: &str) -> Option<ResourceSample> {
    let line = line.trim();
    if line.is_empty() || !line.contains("resource-metrics") {
        return None;
    }

    let root: Value = serde_json::from_str(line).ok()?;

    // Check this is a resource-metrics entry
    if root.get("k")?.as_str()? != "resource-metrics" {
        return None;
    }

    let v = root.get("v")?;

    let mut sample = ResourceSample {
        timestamp: Instant::now(),
        process_memory_mb: 0.0,
        process_cpu: 0.0,
        system_cpu: 0.0,
        overall_load: 0.0,
        scheduler_load: 0.0,
        memory_load: 0.0,
        scheduler_threads: 0,
        scheduler_waiting_jobs: 0,
    };

    // Parse memory metrics — v.memory.process_physical_memory_mb is a direct f64
    sample.process_memory_mb =
        extract_metric_value(v, "memory", "process_physical_memory_mb").unwrap_or(0.0);

    // Parse CPU metrics — v.cpu.process_cpu_utilization / system_cpu_utilization
    // Note: on macOS these are empty objects `{}`, so CPU will be 0.
    sample.process_cpu = extract_metric_value(v, "cpu", "process_cpu_utilization").unwrap_or(0.0);

    sample.system_cpu = extract_metric_value(v, "cpu", "system_cpu_utilization").unwrap_or(0.0);

    // Parse load metrics — v.load.overall_load, scheduler_load, memory_load
    sample.overall_load = extract_metric_value(v, "load", "overall_load").unwrap_or(0.0);

    sample.scheduler_load = extract_metric_value(v, "load", "scheduler_load").unwrap_or(0.0);

    sample.memory_load = extract_metric_value(v, "load", "memory_load").unwrap_or(0.0);

    // Parse scheduler metrics — v["scheduler-thread-count"]
    // scheduler_thread_count is {active: N, inactive: M} — sum them
    sample.scheduler_threads = extract_scheduler_thread_count(v);

    // scheduler_waiting_jobs_count is a direct number
    sample.scheduler_waiting_jobs =
        extract_metric_value(v, "scheduler-thread-count", "scheduler_waiting_jobs_count")
            .map_or(0, |f| f as i64);

    Some(sample)
}

/// Extract a numeric value from a nested metrics structure.
///
/// The resource-metrics JSON has a structure like:
/// `{"memory": {"process_physical_memory_mb": {"value": 123.4, ...}}, ...}`
fn extract_metric_value(v: &Value, section: &str, metric: &str) -> Option<f64> {
    let section_val = v.get(section)?;
    let metric_val = section_val.get(metric)?;

    // Could be a direct number or an object with a "value" field
    if let Some(n) = metric_val.as_f64() {
        return Some(n);
    }
    if let Some(obj) = metric_val.as_object() {
        if let Some(val) = obj.get("value").and_then(serde_json::Value::as_f64) {
            return Some(val);
        }
    }
    None
}

/// Fallback: try to find the metric at top level of the value object.
#[expect(
    dead_code,
    reason = "test helper; referenced by subset of stress-test binaries in this crate"
)]
fn extract_simple_metric(v: &Value, metric: &str) -> Option<f64> {
    if let Some(val) = v.get(metric) {
        if let Some(n) = val.as_f64() {
            return Some(n);
        }
        if let Some(obj) = val.as_object() {
            return obj.get("value").and_then(serde_json::Value::as_f64);
        }
    }
    None
}

/// Extract total scheduler thread count from `v["scheduler-thread-count"]["scheduler_thread_count"]`.
///
/// The field is an object like `{"active": 14, "inactive": 7}` — we sum them.
fn extract_scheduler_thread_count(v: &Value) -> i64 {
    let Some(section) = v.get("scheduler-thread-count") else {
        return 0;
    };
    let Some(tc) = section.get("scheduler_thread_count") else {
        return 0;
    };
    if let Some(obj) = tc.as_object() {
        let active = obj
            .get("active")
            .and_then(serde_json::Value::as_i64)
            .unwrap_or(0);
        let inactive = obj
            .get("inactive")
            .and_then(serde_json::Value::as_i64)
            .unwrap_or(0);
        return active + inactive;
    }
    tc.as_i64().unwrap_or(0)
}

// ---------------------------------------------------------------------------
// Public helpers
// ---------------------------------------------------------------------------

/// Check whether the monitor has detected a Hyper crash (process gone).
#[expect(
    dead_code,
    reason = "test helper; referenced by subset of stress-test binaries in this crate"
)]
pub(crate) fn check_hyper_alive(hyper: &hyperdb_api::HyperProcess) -> bool {
    hyper.is_running()
}
