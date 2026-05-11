// Copyright (c) 2026, Salesforce, Inc. All rights reserved.
// SPDX-License-Identifier: Apache-2.0 OR MIT

//! Aggregated run statistics — throughput, latencies, errors, resource peaks.

#![allow(
    clippy::cast_precision_loss,
    reason = "stress-test diagnostic aggregation"
)]

use std::collections::HashMap;
use std::sync::Mutex;
use std::time::Duration;

use serde::{Deserialize, Serialize};

use super::user_profiles::OpKind;

// ---------------------------------------------------------------------------
// Per-operation outcome (produced by workload.rs)
// ---------------------------------------------------------------------------

/// The result of executing a single operation.
#[derive(Debug, Clone)]
pub(crate) struct OpOutcome {
    pub op: OpKind,
    pub success: bool,
    pub latency: Duration,
    pub rows_affected: u64,
    pub error: Option<String>,
}

// ---------------------------------------------------------------------------
// Thread-safe stats accumulator
// ---------------------------------------------------------------------------

/// Collects outcomes from all user threads.
pub(crate) struct StatsCollector {
    outcomes: Mutex<Vec<OpOutcome>>,
}

impl StatsCollector {
    pub(crate) fn new() -> Self {
        Self {
            outcomes: Mutex::new(Vec::with_capacity(100_000)),
        }
    }

    /// Record a single operation outcome (called from user threads).
    pub(crate) fn record(&self, outcome: OpOutcome) {
        if let Ok(mut v) = self.outcomes.lock() {
            v.push(outcome);
        }
    }

    /// Consume the collector and compute the final summary.
    pub(crate) fn into_summary(self) -> StatsSummary {
        let outcomes = self.outcomes.into_inner().unwrap_or_default();
        StatsSummary::compute(&outcomes)
    }
}

// ---------------------------------------------------------------------------
// Computed summary (serializable)
// ---------------------------------------------------------------------------

/// Final statistics summary, written to `summary.json`.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub(crate) struct StatsSummary {
    pub total_operations: u64,
    pub successful_operations: u64,
    pub failed_operations: u64,
    pub operations_by_type: HashMap<String, u64>,
    pub errors: Vec<String>,
    pub total_rows_inserted: u64,
    pub total_rows_queried: u64,
    pub throughput_ops_per_sec: f64,
    pub insert_rows_per_sec: f64,
    pub latency_ms: LatencyPercentiles,
    pub latency_by_op: HashMap<String, LatencyPercentiles>,
    /// Actual wall-clock duration of the simulation.
    pub actual_duration_secs: f64,
    pub peak_memory_mb: f64,
    pub peak_cpu_percent: f64,
    pub peak_load: f64,
    pub disk_used_mb: f64,
    pub hyper_crashed: bool,
}

/// Latency percentiles in milliseconds.
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub(crate) struct LatencyPercentiles {
    pub p50: f64,
    pub p95: f64,
    pub p99: f64,
    pub max: f64,
}

impl StatsSummary {
    fn compute(outcomes: &[OpOutcome]) -> Self {
        let total = outcomes.len() as u64;
        let successful = outcomes.iter().filter(|o| o.success).count() as u64;
        let failed = total - successful;

        // Count by type
        let mut by_type: HashMap<String, u64> = HashMap::new();
        let mut insert_rows = 0u64;
        let mut query_rows = 0u64;
        let mut errors = Vec::new();
        let mut latencies_by_op: HashMap<OpKind, Vec<f64>> = HashMap::new();
        let mut all_latencies = Vec::with_capacity(outcomes.len());

        for o in outcomes {
            *by_type.entry(format!("{:?}", o.op)).or_insert(0) += 1;

            let lat_ms = o.latency.as_secs_f64() * 1000.0;
            all_latencies.push(lat_ms);
            latencies_by_op.entry(o.op).or_default().push(lat_ms);

            if o.success {
                if o.op.is_write() {
                    insert_rows += o.rows_affected;
                } else {
                    query_rows += o.rows_affected;
                }
            } else if let Some(ref e) = o.error {
                if errors.len() < 100 {
                    errors.push(e.clone());
                }
            }
        }

        let latency_ms = compute_percentiles(&mut all_latencies);

        let mut latency_by_op: HashMap<String, LatencyPercentiles> = HashMap::new();
        for (op, mut lats) in latencies_by_op {
            latency_by_op.insert(format!("{op:?}"), compute_percentiles(&mut lats));
        }

        StatsSummary {
            total_operations: total,
            successful_operations: successful,
            failed_operations: failed,
            operations_by_type: by_type,
            errors,
            total_rows_inserted: insert_rows,
            total_rows_queried: query_rows,
            throughput_ops_per_sec: 0.0, // filled in later with actual duration
            insert_rows_per_sec: 0.0,    // filled in later
            latency_ms,
            latency_by_op,
            actual_duration_secs: 0.0, // filled in later
            peak_memory_mb: 0.0,       // filled in later from monitor
            peak_cpu_percent: 0.0,
            peak_load: 0.0,
            disk_used_mb: 0.0,
            hyper_crashed: false,
        }
    }

    /// Fill in duration-dependent fields after the simulation completes.
    pub(crate) fn finalize(
        &mut self,
        actual_duration: Duration,
        peak_memory_mb: f64,
        peak_cpu_percent: f64,
        peak_load: f64,
        disk_used_mb: f64,
        hyper_crashed: bool,
    ) {
        let secs = actual_duration.as_secs_f64();
        self.actual_duration_secs = secs;
        if secs > 0.0 {
            self.throughput_ops_per_sec = self.total_operations as f64 / secs;
            self.insert_rows_per_sec = self.total_rows_inserted as f64 / secs;
        }
        self.peak_memory_mb = peak_memory_mb;
        self.peak_cpu_percent = peak_cpu_percent;
        self.peak_load = peak_load;
        self.disk_used_mb = disk_used_mb;
        self.hyper_crashed = hyper_crashed;
    }
}

fn compute_percentiles(data: &mut [f64]) -> LatencyPercentiles {
    if data.is_empty() {
        return LatencyPercentiles::default();
    }
    data.sort_by(|a, b| a.partial_cmp(b).unwrap_or(std::cmp::Ordering::Equal));
    let n = data.len();
    LatencyPercentiles {
        p50: data[n * 50 / 100],
        p95: data[n * 95 / 100],
        p99: data[n * 99 / 100],
        max: data[n - 1],
    }
}
