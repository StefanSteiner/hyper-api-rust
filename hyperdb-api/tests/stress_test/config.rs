// Copyright (c) 2026, Salesforce, Inc. All rights reserved.
// SPDX-License-Identifier: Apache-2.0 OR MIT

//! Simulation configuration for the Hyper DB stress test.
//!
//! All tunables are collected in [`SimulationConfig`], which can be built
//! from environment variables or loaded from a replay file.

use std::ops::Range;
use std::path::PathBuf;
use std::time::Duration;

use serde::{Deserialize, Serialize};

// ---------------------------------------------------------------------------
// Transport / format enums (serde-friendly mirrors of hyperdb-api types)
// ---------------------------------------------------------------------------

/// Which protocol the simulation uses for connections.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub(crate) enum SimTransport {
    /// TCP (libpq wire protocol).
    Tcp,
    /// gRPC (Arrow results, read-only — writes still go over TCP).
    Grpc,
}

/// Data format used for COPY inserts over TCP.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub(crate) enum SimDataFormat {
    HyperBinary,
    ArrowStream,
}

// ---------------------------------------------------------------------------
// Duration range helper (serde-friendly)
// ---------------------------------------------------------------------------

/// A half-open range of durations expressed in milliseconds (for serde).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub(crate) struct DurationRangeMs {
    pub min_ms: u64,
    pub max_ms: u64,
}

impl DurationRangeMs {
    pub(crate) fn new(min: Duration, max: Duration) -> Self {
        Self {
            min_ms: min.as_millis() as u64,
            max_ms: max.as_millis() as u64,
        }
    }

    pub(crate) fn to_range(&self) -> Range<Duration> {
        Duration::from_millis(self.min_ms)..Duration::from_millis(self.max_ms)
    }
}

// ---------------------------------------------------------------------------
// Main config
// ---------------------------------------------------------------------------

/// Complete configuration for a single simulation run.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub(crate) struct SimulationConfig {
    // -- duration --
    /// How long the simulation should run.
    pub duration_secs: u64,

    // -- transport --
    pub transport: SimTransport,
    pub data_format: SimDataFormat,

    // -- database population --
    /// Number of separate `.hyper` database files.
    pub num_databases: usize,
    /// Maximum total disk space (bytes) across all databases.
    pub max_total_disk_bytes: u64,

    // -- user population --
    pub inserter_users: usize,
    pub query_users: usize,
    pub mixed_users: usize,

    // -- resource limits (backpressure thresholds) --
    pub max_memory_mb: u64,
    pub max_cpu_percent: f64,

    // -- Hyper tuning --
    /// `log_resource_usage_interval` passed to hyperd (seconds).
    pub log_resource_interval_secs: u64,
    /// `log_resource_usage_mode` bitmask (default 2047 = All).
    pub log_resource_mode: u16,
    /// Hyper `memory_limit` setting (e.g. "4g", "80%").
    pub memory_limit: String,

    // -- Monte Carlo tuning --
    /// Global RNG seed. `None` → random.
    pub seed: Option<u64>,
    /// Random pause between operations per user thread.
    pub think_time: DurationRangeMs,
    /// Rows per bulk-insert batch.
    pub batch_size_min: usize,
    pub batch_size_max: usize,
    /// Query complexity level (1 = simple, higher = joins/aggs).
    pub query_complexity_min: usize,
    pub query_complexity_max: usize,

    // -- output --
    /// Directory for `.hyper` files, `summary.json`, `replay.json`.
    /// If `None`, a temp directory is created.
    pub output_dir: Option<PathBuf>,
}

impl SimulationConfig {
    /// Total number of simulated user threads.
    pub(crate) fn total_users(&self) -> usize {
        self.inserter_users + self.query_users + self.mixed_users
    }

    /// The effective duration as a [`Duration`].
    pub(crate) fn duration(&self) -> Duration {
        Duration::from_secs(self.duration_secs)
    }

    /// Build a config from environment variables, falling back to defaults.
    pub(crate) fn from_env() -> Self {
        fn env_u64(key: &str, default: u64) -> u64 {
            std::env::var(key)
                .ok()
                .and_then(|v| v.parse().ok())
                .unwrap_or(default)
        }
        fn env_usize(key: &str, default: usize) -> usize {
            std::env::var(key)
                .ok()
                .and_then(|v| v.parse().ok())
                .unwrap_or(default)
        }
        fn env_f64(key: &str, default: f64) -> f64 {
            std::env::var(key)
                .ok()
                .and_then(|v| v.parse().ok())
                .unwrap_or(default)
        }
        fn env_str(key: &str, default: &str) -> String {
            std::env::var(key).unwrap_or_else(|_| default.to_string())
        }

        let transport = match env_str("STRESS_TRANSPORT", "tcp").to_lowercase().as_str() {
            "grpc" => SimTransport::Grpc,
            _ => SimTransport::Tcp,
        };
        let data_format = match env_str("STRESS_DATA_FORMAT", "hyperbinary")
            .to_lowercase()
            .as_str()
        {
            "arrow" | "arrowstream" => SimDataFormat::ArrowStream,
            _ => SimDataFormat::HyperBinary,
        };

        SimulationConfig {
            duration_secs: env_u64("STRESS_DURATION", 300), // 5 min default
            transport,
            data_format,
            num_databases: env_usize("STRESS_DATABASES", 3),
            max_total_disk_bytes: env_u64("STRESS_MAX_DISK_MB", 2048) * 1024 * 1024,
            inserter_users: env_usize("STRESS_INSERTER_USERS", 4),
            query_users: env_usize("STRESS_QUERY_USERS", 3),
            mixed_users: env_usize("STRESS_MIXED_USERS", 2),
            max_memory_mb: env_u64("STRESS_MAX_MEMORY_MB", 4096),
            max_cpu_percent: env_f64("STRESS_MAX_CPU_PERCENT", 90.0),
            log_resource_interval_secs: env_u64("STRESS_LOG_INTERVAL", 5),
            log_resource_mode: env_u64("STRESS_LOG_MODE", 2047) as u16,
            memory_limit: env_str("STRESS_MEMORY_LIMIT", "80%"),
            seed: std::env::var("STRESS_SEED")
                .ok()
                .and_then(|v| v.parse().ok()),
            think_time: DurationRangeMs::new(
                Duration::from_millis(env_u64("STRESS_THINK_MIN_MS", 10)),
                Duration::from_millis(env_u64("STRESS_THINK_MAX_MS", 200)),
            ),
            batch_size_min: env_usize("STRESS_BATCH_MIN", 100),
            batch_size_max: env_usize("STRESS_BATCH_MAX", 10_000),
            query_complexity_min: 1,
            query_complexity_max: env_usize("STRESS_QUERY_COMPLEXITY_MAX", 3),
            output_dir: std::env::var("STRESS_OUTPUT_DIR").ok().map(PathBuf::from),
        }
    }
}

impl Default for SimulationConfig {
    fn default() -> Self {
        Self::from_env()
    }
}
