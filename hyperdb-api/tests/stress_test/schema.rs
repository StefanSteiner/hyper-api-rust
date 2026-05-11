// Copyright (c) 2026, Salesforce, Inc. All rights reserved.
// SPDX-License-Identifier: Apache-2.0 OR MIT

//! Table schema definitions for the stress test.
//!
//! Provides a set of table schemas with varying complexity (small, medium, wide)
//! that are deterministically generated from a seed.

use hyperdb_api::{SqlType, TableDefinition};
use rand::rngs::StdRng;
use rand::{RngExt, SeedableRng};

/// Schema complexity level.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum SchemaSize {
    /// 3 columns: id INT, name TEXT, value DOUBLE.
    Small,
    /// 8 columns: adds timestamp, category, quantity, price, description.
    Medium,
    /// 20+ columns: adds booleans, dates, more text, numerics.
    Wide,
}

impl SchemaSize {
    /// Pick a schema size from a random roll in [0.0, 1.0).
    pub(crate) fn from_roll(roll: f64) -> Self {
        if roll < 0.40 {
            SchemaSize::Small
        } else if roll < 0.75 {
            SchemaSize::Medium
        } else {
            SchemaSize::Wide
        }
    }
}

/// Generate a table definition for a given database index and table index.
///
/// The schema is deterministic given the seed.
pub(crate) fn generate_table_def(
    schema_seed: u64,
    db_idx: usize,
    table_idx: usize,
) -> TableDefinition {
    let mut rng =
        StdRng::seed_from_u64(schema_seed.wrapping_add(db_idx as u64 * 1000 + table_idx as u64));
    let size = SchemaSize::from_roll(rng.random::<f64>());
    let table_name = format!("t_{db_idx}_{table_idx}");
    build_table_def(&table_name, size)
}

/// Build a concrete [`TableDefinition`] for a given size.
pub(crate) fn build_table_def(table_name: &str, size: SchemaSize) -> TableDefinition {
    match size {
        SchemaSize::Small => small_table(table_name),
        SchemaSize::Medium => medium_table(table_name),
        SchemaSize::Wide => wide_table(table_name),
    }
}

fn small_table(name: &str) -> TableDefinition {
    TableDefinition::new(name)
        .add_required_column("id", SqlType::int())
        .add_required_column("name", SqlType::text())
        .add_nullable_column("value", SqlType::double())
}

fn medium_table(name: &str) -> TableDefinition {
    TableDefinition::new(name)
        .add_required_column("id", SqlType::int())
        .add_required_column("name", SqlType::text())
        .add_nullable_column("value", SqlType::double())
        .add_nullable_column("created_at", SqlType::timestamp())
        .add_nullable_column("category", SqlType::text())
        .add_nullable_column("quantity", SqlType::int())
        .add_nullable_column("price", SqlType::double())
        .add_nullable_column("description", SqlType::text())
}

fn wide_table(name: &str) -> TableDefinition {
    TableDefinition::new(name)
        .add_required_column("id", SqlType::int())
        .add_required_column("name", SqlType::text())
        .add_nullable_column("value", SqlType::double())
        .add_nullable_column("created_at", SqlType::timestamp())
        .add_nullable_column("category", SqlType::text())
        .add_nullable_column("quantity", SqlType::int())
        .add_nullable_column("price", SqlType::double())
        .add_nullable_column("description", SqlType::text())
        .add_nullable_column("is_active", SqlType::bool())
        .add_nullable_column("rating", SqlType::small_int())
        .add_nullable_column("score", SqlType::double())
        .add_nullable_column("notes", SqlType::text())
        .add_nullable_column("updated_at", SqlType::timestamp())
        .add_nullable_column("ref_code", SqlType::text())
        .add_nullable_column("weight", SqlType::double())
        .add_nullable_column("height", SqlType::double())
        .add_nullable_column("tag_a", SqlType::text())
        .add_nullable_column("tag_b", SqlType::text())
        .add_nullable_column("counter", SqlType::big_int())
        .add_nullable_column("fraction", SqlType::double())
}

/// Number of tables to create per database (deterministic from seed).
pub(crate) fn tables_per_database(schema_seed: u64, db_idx: usize) -> usize {
    let mut rng = StdRng::seed_from_u64(schema_seed.wrapping_add(db_idx as u64 * 7919));
    rng.random_range(2..=5)
}
