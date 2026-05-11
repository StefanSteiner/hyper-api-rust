// Copyright (c) 2026, Salesforce, Inc. All rights reserved.
// SPDX-License-Identifier: Apache-2.0 OR MIT

//! Replay log writing (seed capture) and reading (replay mode).
//!
//! Two artifacts are produced per simulation run:
//! 1. `replay.json` — compact seed-based file for deterministic reproduction
//! 2. `summary.json` — full config + aggregate results

use std::fs;
use std::path::Path;

use serde::{Deserialize, Serialize};

use super::config::SimulationConfig;
use super::stats::StatsSummary;
use super::user_profiles::UserDescriptor;

// ---------------------------------------------------------------------------
// Replay log
// ---------------------------------------------------------------------------

/// The compact replay file written after every run.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub(crate) struct ReplayLog {
    /// Format version (bump if the schema changes).
    pub version: u32,
    /// The global RNG seed that was used (or generated).
    pub global_seed: u64,
    /// Full simulation config snapshot.
    pub config: SimulationConfig,
    /// Per-user seed assignments.
    pub user_seeds: Vec<UserDescriptor>,
    /// Per-database schema seeds.
    pub schema_seeds: Vec<u64>,
    /// Why the simulation stopped.
    pub stop_reason: StopReason,
}

/// Why the simulation ended.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub(crate) enum StopReason {
    DurationElapsed,
    CrashDetected,
    ResourceLimit,
    UserAbort,
}

impl ReplayLog {
    /// Write the replay log to `<output_dir>/replay.json`.
    pub(crate) fn write(&self, output_dir: &Path) -> std::io::Result<()> {
        let path = output_dir.join("replay.json");
        let json = serde_json::to_string_pretty(self).map_err(std::io::Error::other)?;
        fs::write(&path, json)?;
        eprintln!("[stress] Replay log written to {}", path.display());
        Ok(())
    }

    /// Load a replay log from a file.
    pub(crate) fn load(path: &Path) -> std::io::Result<Self> {
        let json = fs::read_to_string(path)?;
        let log: ReplayLog = serde_json::from_str(&json)
            .map_err(|e| std::io::Error::new(std::io::ErrorKind::InvalidData, e))?;
        Ok(log)
    }
}

// ---------------------------------------------------------------------------
// Summary log
// ---------------------------------------------------------------------------

/// The summary file written after every run.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub(crate) struct SummaryLog {
    pub config: SimulationConfig,
    pub results: StatsSummary,
}

impl SummaryLog {
    /// Write the summary to `<output_dir>/summary.json`.
    pub(crate) fn write(&self, output_dir: &Path) -> std::io::Result<()> {
        let path = output_dir.join("summary.json");
        let json = serde_json::to_string_pretty(self).map_err(std::io::Error::other)?;
        fs::write(&path, json)?;
        eprintln!("[stress] Summary written to {}", path.display());
        Ok(())
    }
}
