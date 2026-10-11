// Copyright (c) 2026, Salesforce, Inc. All rights reserved.
// SPDX-License-Identifier: Apache-2.0 OR MIT

//! Shared plumbing for short-lived staging tables: collision-free names and
//! an RAII guard that drops the table on every scope exit.
//!
//! Used by the `load_file` merge path (`__hyperdb_merge_*`) and the
//! `copy_query` replace path (`__hyperdb_copy_*`). Both prefixes are listed
//! in `engine::INTERNAL_SCRATCH_PREFIXES`, so a staging table that is in
//! flight (or leaked by a hard crash) never shows up as user data.

use crate::engine::Engine;
use std::sync::atomic::{AtomicU64, Ordering};

/// Prefix of `load_file` merge staging tables.
pub(crate) const MERGE_PREFIX: &str = "__hyperdb_merge_";
/// Prefix of `copy_query` replace staging tables (new result and the
/// renamed-aside backup of the old target).
pub(crate) const COPY_PREFIX: &str = "__hyperdb_copy_";

/// Build a unique scratch-table name: `<prefix><safe_target>_<pid>_<nanos>_<n>`.
///
/// The name mixes the PID, a nanosecond timestamp and a process-local atomic
/// counter. The counter is what makes it race-free: two parallel calls for
/// the same target inside one process can land in the same nanosecond, but
/// never share a counter value, so their guards cannot cross-drop each
/// other's tables. Anything outside `[A-Za-z0-9_]` in `target` is squashed to
/// `_` so the identifier is always SQL-safe.
pub(crate) fn scratch_name(prefix: &str, target: &str) -> String {
    static COUNTER: AtomicU64 = AtomicU64::new(0);
    let counter = COUNTER.fetch_add(1, Ordering::Relaxed);
    let nanos = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_or(0, |d| d.as_nanos());
    let safe_target: String = target
        .chars()
        .map(|c| {
            if c.is_ascii_alphanumeric() || c == '_' {
                c
            } else {
                '_'
            }
        })
        .collect();
    format!(
        "{prefix}{safe_target}_{}_{nanos}_{counter}",
        std::process::id()
    )
}

/// RAII guard that drops a staging table on **every** scope exit — `Ok`,
/// `Err`, *and* panic-unwind — so an orphan scratch table can't leak into
/// the workspace.
///
/// The guard owns the exclusive engine borrow so the guarded code can run
/// `&mut self` engine operations (e.g. `execute_in_transaction`) through
/// [`Self::engine`]. Disarm with [`Self::disarm`] once the table has been
/// renamed into its final role and must no longer be dropped by name.
pub(crate) struct ScratchTableGuard<'engine> {
    engine: &'engine mut Engine,
    /// Fully-qualified (or unqualified) quoted identifier to drop.
    qualified: String,
    /// Bare table name, for logging.
    name: String,
    armed: bool,
}

impl<'engine> ScratchTableGuard<'engine> {
    /// Arm a guard for `qualified` (an already-quoted identifier such as
    /// `"db"."public"."name"` or `"name"`); `name` is only used in logs.
    pub(crate) fn new(engine: &'engine mut Engine, qualified: String, name: String) -> Self {
        Self {
            engine,
            qualified,
            name,
            armed: true,
        }
    }

    /// The guarded engine.
    pub(crate) fn engine(&mut self) -> &mut Engine {
        self.engine
    }

    /// Stop the guard from dropping the table (it was renamed away).
    pub(crate) fn disarm(&mut self) {
        self.armed = false;
    }
}

impl Drop for ScratchTableGuard<'_> {
    fn drop(&mut self) {
        if !self.armed {
            return;
        }
        // Distinguish expected cleanup from "we wouldn't have learned about
        // the leak otherwise" (panic-unwind) in the log.
        let panicking = std::thread::panicking();
        let drop_sql = format!("DROP TABLE IF EXISTS {}", self.qualified);
        match self.engine.execute_command(&drop_sql) {
            Ok(_) => {
                if panicking {
                    tracing::info!(
                        tmp = %self.name,
                        "scratch table dropped during panic unwind"
                    );
                }
            }
            Err(e) => {
                // Don't mask the original outcome's error with a cleanup
                // error — log and continue. The scratch table is benign
                // (hidden by its prefix); the user can drop it manually.
                tracing::warn!(
                    tmp = %self.name,
                    panicking,
                    error = %e,
                    "failed to drop scratch table on guard exit"
                );
            }
        }
    }
}
