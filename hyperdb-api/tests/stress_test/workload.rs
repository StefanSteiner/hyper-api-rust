// Copyright (c) 2026, Salesforce, Inc. All rights reserved.
// SPDX-License-Identifier: Apache-2.0 OR MIT

//! Workload operations — data generation and SQL execution for each [`OpKind`].

#![allow(
    clippy::cast_precision_loss,
    reason = "stress-test diagnostic; values bounded by test duration"
)]

use hyperdb_api::{
    Catalog, Connection, CreateMode, HyperProcess, Inserter, SqlType, TableDefinition,
};
use rand::rngs::StdRng;
use rand::RngExt;
use std::time::Instant;

use super::config::SimulationConfig;
use super::schema;
use super::stats::OpOutcome;
use super::user_profiles::OpKind;

// ---------------------------------------------------------------------------
// Database context (shared across user threads via Arc)
// ---------------------------------------------------------------------------

/// Metadata for one of the simulation's databases.
#[derive(Debug)]
pub(crate) struct DatabaseInfo {
    /// Absolute path to the `.hyper` file.
    pub path: std::path::PathBuf,
    /// Table definitions that exist in this database.
    pub tables: Vec<TableDefinition>,
    /// Schema seed used to generate the tables.
    #[expect(
        dead_code,
        reason = "retained for diagnostic output; stored on every DB but read only by the reporter binaries"
    )]
    pub schema_seed: u64,
    /// Index in the database pool.
    pub db_idx: usize,
}

// ---------------------------------------------------------------------------
// Setup helpers
// ---------------------------------------------------------------------------

/// Create all databases and tables for the simulation.
///
/// Returns the list of [`DatabaseInfo`] for each database.
pub(crate) fn setup_databases(
    hyper: &HyperProcess,
    config: &SimulationConfig,
    schema_seeds: &[u64],
    output_dir: &std::path::Path,
) -> hyperdb_api::Result<Vec<DatabaseInfo>> {
    let mut databases = Vec::with_capacity(config.num_databases);

    #[expect(
        clippy::needless_range_loop,
        reason = "loop body indexes multiple parallel slices (schema_seeds, tables, etc.); an enumerated iterator would obscure the intent"
    )]
    for db_idx in 0..config.num_databases {
        let db_path = output_dir.join(format!("stress_{db_idx}.hyper"));
        let seed = schema_seeds[db_idx];

        // Create database and tables
        let conn = Connection::new(hyper, &db_path, CreateMode::CreateAndReplace)?;
        let catalog = Catalog::new(&conn);

        let num_tables = schema::tables_per_database(seed, db_idx);
        let mut tables = Vec::with_capacity(num_tables);

        for t_idx in 0..num_tables {
            let table_def = schema::generate_table_def(seed, db_idx, t_idx);
            catalog.create_table(&table_def)?;
            tables.push(table_def);
        }

        // Seed each database with a small amount of data so queries have something to hit
        for table_def in &tables {
            seed_table(&conn, table_def, 100)?;
        }

        conn.close()?;

        databases.push(DatabaseInfo {
            path: db_path,
            tables,
            schema_seed: seed,
            db_idx,
        });
    }

    Ok(databases)
}

/// Insert `row_count` seed rows into a table.
fn seed_table(
    conn: &Connection,
    table_def: &TableDefinition,
    row_count: usize,
) -> hyperdb_api::Result<()> {
    let mut inserter = Inserter::new(conn, table_def)?;
    for i in 0..row_count {
        add_row_direct(&mut inserter, table_def, i as i32)?;
    }
    inserter.execute()?;
    Ok(())
}

// ---------------------------------------------------------------------------
// Operation execution
// ---------------------------------------------------------------------------

/// Execute a single operation against a database, returning the outcome.
pub(crate) fn execute_op(
    conn: &Connection,
    db_info: &DatabaseInfo,
    op: OpKind,
    rng: &mut StdRng,
    config: &SimulationConfig,
) -> OpOutcome {
    let start = Instant::now();
    let result = match op {
        OpKind::BulkInsert => exec_bulk_insert(conn, db_info, rng, config),
        OpKind::SingleInsert => exec_single_insert(conn, db_info, rng),
        OpKind::SimpleSelect => exec_simple_select(conn, db_info, rng),
        OpKind::AggregateQuery => exec_aggregate_query(conn, db_info, rng),
        OpKind::ComplexJoinQuery => exec_complex_join(conn, db_info, rng),
        OpKind::SchemaDdl => exec_schema_ddl(conn, db_info, rng),
    };
    let elapsed = start.elapsed();

    match result {
        Ok(rows) => OpOutcome {
            op,
            success: true,
            latency: elapsed,
            rows_affected: rows,
            error: None,
        },
        Err(e) => OpOutcome {
            op,
            success: false,
            latency: elapsed,
            rows_affected: 0,
            error: Some(format!("{e}")),
        },
    }
}

// ---------------------------------------------------------------------------
// Individual operation implementations
// ---------------------------------------------------------------------------

fn exec_bulk_insert(
    conn: &Connection,
    db_info: &DatabaseInfo,
    rng: &mut StdRng,
    config: &SimulationConfig,
) -> hyperdb_api::Result<u64> {
    let table_def = &db_info.tables[rng.random_range(0..db_info.tables.len())];
    let batch_size = rng.random_range(config.batch_size_min..=config.batch_size_max);

    let mut inserter = Inserter::new(conn, table_def)?;
    for i in 0..batch_size {
        let seed_val = rng.random::<i32>().wrapping_add(i as i32);
        add_row_direct(&mut inserter, table_def, seed_val)?;
    }
    let rows = inserter.execute()?;
    Ok(rows)
}

fn exec_single_insert(
    conn: &Connection,
    db_info: &DatabaseInfo,
    rng: &mut StdRng,
) -> hyperdb_api::Result<u64> {
    let table_def = &db_info.tables[rng.random_range(0..db_info.tables.len())];
    let id: i32 = rng.random();
    let col_count = table_def.column_count();

    // Build a simple INSERT VALUES statement
    let table_name = &table_def.name;
    let mut values = Vec::with_capacity(col_count);
    values.push(format!("{id}"));
    values.push(format!("'row_{}'", id.abs()));
    // Fill remaining columns with NULLs
    for _ in 2..col_count {
        values.push("NULL".to_string());
    }

    let sql = format!(
        "INSERT INTO \"{}\" VALUES ({})",
        table_name,
        values.join(", ")
    );
    conn.execute_command(&sql)
}

fn exec_simple_select(
    conn: &Connection,
    db_info: &DatabaseInfo,
    rng: &mut StdRng,
) -> hyperdb_api::Result<u64> {
    let table_def = &db_info.tables[rng.random_range(0..db_info.tables.len())];
    let table_name = &table_def.name;
    let limit: usize = rng.random_range(10..=1000);

    let sql = format!("SELECT * FROM \"{table_name}\" LIMIT {limit}");
    let mut result = conn.execute_query(&sql)?;
    let mut row_count = 0u64;
    while let Some(chunk) = result.next_chunk()? {
        row_count += chunk.len() as u64;
    }
    Ok(row_count)
}

fn exec_aggregate_query(
    conn: &Connection,
    db_info: &DatabaseInfo,
    rng: &mut StdRng,
) -> hyperdb_api::Result<u64> {
    let table_def = &db_info.tables[rng.random_range(0..db_info.tables.len())];
    let table_name = &table_def.name;

    // Pick a random aggregate
    let agg = match rng.random_range(0..4) {
        0 => format!("SELECT COUNT(*) FROM \"{table_name}\""),
        1 => format!("SELECT COUNT(*), MIN(\"id\"), MAX(\"id\") FROM \"{table_name}\""),
        2 => format!("SELECT \"name\", COUNT(*) FROM \"{table_name}\" GROUP BY \"name\" LIMIT 100"),
        _ => format!("SELECT COUNT(*), SUM(CAST(\"id\" AS BIGINT)) FROM \"{table_name}\""),
    };

    let mut result = conn.execute_query(&agg)?;
    let mut row_count = 0u64;
    while let Some(chunk) = result.next_chunk()? {
        row_count += chunk.len() as u64;
    }
    Ok(row_count)
}

fn exec_complex_join(
    conn: &Connection,
    db_info: &DatabaseInfo,
    rng: &mut StdRng,
) -> hyperdb_api::Result<u64> {
    if db_info.tables.len() < 2 {
        // Fall back to aggregate if only one table
        return exec_aggregate_query(conn, db_info, rng);
    }

    let t1_idx = rng.random_range(0..db_info.tables.len());
    let mut t2_idx = rng.random_range(0..db_info.tables.len());
    if t2_idx == t1_idx {
        t2_idx = (t1_idx + 1) % db_info.tables.len();
    }

    let t1 = &db_info.tables[t1_idx].name;
    let t2 = &db_info.tables[t2_idx].name;
    let limit: usize = rng.random_range(10..=500);

    let sql = format!(
        "SELECT a.\"id\", b.\"id\", a.\"name\" \
         FROM \"{t1}\" a JOIN \"{t2}\" b ON a.\"id\" = b.\"id\" \
         LIMIT {limit}"
    );

    let mut result = conn.execute_query(&sql)?;
    let mut row_count = 0u64;
    while let Some(chunk) = result.next_chunk()? {
        row_count += chunk.len() as u64;
    }
    Ok(row_count)
}

fn exec_schema_ddl(
    conn: &Connection,
    db_info: &DatabaseInfo,
    rng: &mut StdRng,
) -> hyperdb_api::Result<u64> {
    let temp_table = format!("stress_temp_{}_{}", db_info.db_idx, rng.random::<u32>());

    // Create a temp table, insert a row, query it, drop it
    conn.execute_command(&format!(
        "CREATE TEMPORARY TABLE \"{temp_table}\" (id INT, val TEXT)"
    ))?;
    conn.execute_command(&format!(
        "INSERT INTO \"{temp_table}\" VALUES (1, 'stress')"
    ))?;
    let mut result = conn.execute_query(&format!("SELECT * FROM \"{temp_table}\""))?;
    let mut rows = 0u64;
    while let Some(chunk) = result.next_chunk()? {
        rows += chunk.len() as u64;
    }
    conn.execute_command(&format!("DROP TABLE \"{temp_table}\""))?;
    Ok(rows)
}

// ---------------------------------------------------------------------------
// Row generation
// ---------------------------------------------------------------------------

/// Add a row directly to the inserter using typed methods.
///
/// This avoids trait-object boxing and ensures correct `HyperBinary` encoding
/// for each column type.
fn add_row_direct(
    inserter: &mut Inserter<'_>,
    table_def: &TableDefinition,
    seed_val: i32,
) -> hyperdb_api::Result<()> {
    for col_idx in 0..table_def.column_count() {
        let col = &table_def.columns()[col_idx];
        let col_name = &col.name;
        let sql_type = col.sql_type();

        // id column is always the seed value
        if col_name == "id" {
            inserter.add_i32(seed_val)?;
            continue;
        }

        // For nullable columns, occasionally insert NULL (10% chance)
        if col.nullable && (seed_val.wrapping_mul(col_idx as i32 + 7)) % 10 == 0 {
            inserter.add_null()?;
            continue;
        }

        match sql_type {
            Some(SqlType::Int) => {
                inserter.add_i32(seed_val.wrapping_add(col_idx as i32))?;
            }
            Some(SqlType::SmallInt) => {
                inserter.add_i16((seed_val.wrapping_add(col_idx as i32) % 32000) as i16)?;
            }
            Some(SqlType::BigInt) => {
                inserter.add_i64(i64::from(seed_val) * 1000 + col_idx as i64)?;
            }
            Some(SqlType::Double) => {
                inserter.add_f64(f64::from(seed_val) * 1.5 + col_idx as f64)?;
            }
            Some(SqlType::Float) => {
                inserter.add_f32(seed_val as f32 * 1.5 + col_idx as f32)?;
            }
            Some(SqlType::Text | SqlType::Varchar { .. }) => {
                inserter.add_str(&format!("v_{}_{}", seed_val.abs(), col_idx))?;
            }
            Some(SqlType::Bool) => {
                inserter.add_bool(seed_val % 2 == 0)?;
            }
            Some(SqlType::Timestamp | SqlType::TimestampTz) => {
                let days = i64::from(seed_val.abs() % 3650);
                let base_days = 7305i64; // days from 2000-01-01 to 2020-01-01
                let us_per_day = 86_400_000_000i64;
                let ts = hyperdb_api::Timestamp::from_microseconds(
                    (base_days + days) * us_per_day + 43_200_000_000,
                );
                inserter.add_timestamp(ts)?;
            }
            Some(SqlType::Date) => {
                let days = seed_val.abs() % 3650;
                let base_days = 7305i32;
                inserter.add_date(hyperdb_api::Date::from_days(base_days + days))?;
            }
            _ => {
                // Fallback: insert NULL for unknown types
                inserter.add_null()?;
            }
        }
    }
    inserter.end_row()?;
    Ok(())
}

/// Estimate the byte size of a single row for disk budget tracking.
pub(crate) fn estimate_row_bytes(table_def: &TableDefinition) -> usize {
    let mut bytes = 0;
    for col in table_def.columns() {
        bytes += match col.sql_type() {
            Some(SqlType::Int | SqlType::SmallInt) => 4,
            Some(SqlType::BigInt) => 8,
            Some(SqlType::Double | SqlType::Float) => 8,
            Some(SqlType::Bool) => 1,
            Some(SqlType::Timestamp | SqlType::TimestampTz | SqlType::Date) => 8,
            Some(SqlType::Text | SqlType::Varchar { .. }) => 32, // avg estimate
            _ => 8,
        };
    }
    bytes
}
