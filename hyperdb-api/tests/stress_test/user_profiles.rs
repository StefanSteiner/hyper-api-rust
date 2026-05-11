// Copyright (c) 2026, Salesforce, Inc. All rights reserved.
// SPDX-License-Identifier: Apache-2.0 OR MIT

//! User class definitions with Monte Carlo probability distributions.
//!
//! Each simulated user belongs to a class that determines the probability
//! of selecting each operation type on every tick.

use serde::{Deserialize, Serialize};

/// The class of a simulated user.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub(crate) enum UserClass {
    /// Mostly bulk inserts.
    Inserter,
    /// Mostly reads / queries.
    Query,
    /// Balanced mix.
    Mixed,
}

/// An operation that a user thread can perform.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub(crate) enum OpKind {
    BulkInsert,
    SingleInsert,
    SimpleSelect,
    AggregateQuery,
    ComplexJoinQuery,
    SchemaDdl,
}

impl OpKind {
    #[expect(
        dead_code,
        reason = "enumerates all OpKinds; referenced by the subset of stress binaries that want a full mix"
    )]
    pub(crate) const ALL: &'static [OpKind] = &[
        OpKind::BulkInsert,
        OpKind::SingleInsert,
        OpKind::SimpleSelect,
        OpKind::AggregateQuery,
        OpKind::ComplexJoinQuery,
        OpKind::SchemaDdl,
    ];

    #[expect(
        clippy::trivially_copy_pass_by_ref,
        reason = "signature kept for API consistency with the trait family that unifies Copy and non-Copy implementers"
    )]
    /// Returns `true` if the operation writes data.
    pub(crate) fn is_write(&self) -> bool {
        matches!(
            self,
            OpKind::BulkInsert | OpKind::SingleInsert | OpKind::SchemaDdl
        )
    }
}

/// Cumulative probability table for a user class.
///
/// Probabilities are stored as cumulative thresholds so that a single
/// uniform draw in `[0, 1)` selects the operation.
pub(crate) struct OpDistribution {
    /// Sorted (`cumulative_prob`, `OpKind`) pairs.
    table: Vec<(f64, OpKind)>,
}

impl OpDistribution {
    /// Build the distribution for a given user class.
    pub(crate) fn for_class(class: UserClass) -> Self {
        let weights: &[(OpKind, f64)] = match class {
            UserClass::Inserter => &[
                (OpKind::BulkInsert, 0.70),
                (OpKind::SingleInsert, 0.15),
                (OpKind::SimpleSelect, 0.05),
                (OpKind::AggregateQuery, 0.03),
                (OpKind::ComplexJoinQuery, 0.02),
                (OpKind::SchemaDdl, 0.05),
            ],
            UserClass::Query => &[
                (OpKind::BulkInsert, 0.05),
                (OpKind::SingleInsert, 0.05),
                (OpKind::SimpleSelect, 0.40),
                (OpKind::AggregateQuery, 0.25),
                (OpKind::ComplexJoinQuery, 0.20),
                (OpKind::SchemaDdl, 0.05),
            ],
            UserClass::Mixed => &[
                (OpKind::BulkInsert, 0.35),
                (OpKind::SingleInsert, 0.15),
                (OpKind::SimpleSelect, 0.20),
                (OpKind::AggregateQuery, 0.15),
                (OpKind::ComplexJoinQuery, 0.10),
                (OpKind::SchemaDdl, 0.05),
            ],
        };

        let mut cumulative = 0.0;
        let table: Vec<(f64, OpKind)> = weights
            .iter()
            .map(|&(op, w)| {
                cumulative += w;
                (cumulative, op)
            })
            .collect();

        Self { table }
    }

    /// Select an operation given a uniform random draw in `[0, 1)`.
    pub(crate) fn select(&self, roll: f64) -> OpKind {
        for &(threshold, op) in &self.table {
            if roll < threshold {
                return op;
            }
        }
        // Floating-point edge case — return last entry.
        self.table.last().unwrap().1
    }
}

/// Descriptor for a single simulated user thread.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub(crate) struct UserDescriptor {
    pub user_id: usize,
    pub class: UserClass,
    /// Per-user RNG seed (derived deterministically from global seed).
    pub seed: u64,
    /// Index of the primary database this user targets.
    pub database_idx: usize,
}
