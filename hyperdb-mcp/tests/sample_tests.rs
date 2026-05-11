// Copyright (c) 2026, Salesforce, Inc. All rights reserved.
// SPDX-License-Identifier: Apache-2.0 OR MIT

//! Tests for `Engine::sample_table`: basic sampling, N clamping, missing tables.

mod common;
use common::TestEngine;

/// Sample a populated table and verify the response shape:
/// table name, `row_count`, `sample_size`, schema, and rows.
#[test]
fn sample_returns_schema_and_rows() {
    let te = TestEngine::new_ephemeral();
    te.engine
        .execute_command(
            "CREATE TABLE employees (id INT NOT NULL, name TEXT, salary DOUBLE PRECISION)",
        )
        .unwrap();
    for i in 1..=7 {
        te.engine
            .execute_command(&format!(
                "INSERT INTO employees VALUES ({i}, 'Name{i}', {})",
                50000 + i * 1000
            ))
            .unwrap();
    }

    let sample = te.engine.sample_table("employees", 5).unwrap();
    assert_eq!(sample["table"], "employees");
    assert_eq!(sample["row_count"], 7);
    assert_eq!(sample["sample_size"], 5);
    let schema = sample["schema"].as_array().unwrap();
    assert_eq!(schema.len(), 3);
    let rows = sample["rows"].as_array().unwrap();
    assert_eq!(rows.len(), 5);
}

/// Verify that N is clamped to the 1..=100 range. A request for 0 or a huge
/// value should still return valid results.
#[test]
fn sample_clamps_n() {
    let te = TestEngine::new_ephemeral();
    te.engine
        .execute_command("CREATE TABLE nums (v INT)")
        .unwrap();
    for i in 1..=10 {
        te.engine
            .execute_command(&format!("INSERT INTO nums VALUES ({i})"))
            .unwrap();
    }

    let sample = te.engine.sample_table("nums", 0).unwrap();
    assert_eq!(sample["sample_size"], 1);

    let sample = te.engine.sample_table("nums", 10_000).unwrap();
    assert_eq!(sample["sample_size"], 10);
}

/// Verify that sampling a missing table returns a `TableNotFound` error.
#[test]
fn sample_missing_table_errors() {
    let te = TestEngine::new_ephemeral();
    let result = te.engine.sample_table("does_not_exist", 5);
    assert!(result.is_err());
    let err = result.unwrap_err();
    assert_eq!(err.code, hyperdb_mcp::error::ErrorCode::TableNotFound);
}

/// Verify that sampling an empty table returns 0 rows but still includes
/// the schema so the caller can plan queries.
#[test]
fn sample_empty_table_returns_schema() {
    let te = TestEngine::new_ephemeral();
    te.engine
        .execute_command("CREATE TABLE empty_t (a INT, b TEXT)")
        .unwrap();

    let sample = te.engine.sample_table("empty_t", 5).unwrap();
    assert_eq!(sample["row_count"], 0);
    assert_eq!(sample["sample_size"], 0);
    assert_eq!(sample["schema"].as_array().unwrap().len(), 2);
}
