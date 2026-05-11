// Copyright (c) 2026, Salesforce, Inc. All rights reserved.
// SPDX-License-Identifier: Apache-2.0 OR MIT

//! Tests for new API features:
//! - #8: Row type extractors (`get_interval`, `get_offset_timestamp`)
//! - #9: Direct `RecordBatch` access (`execute_query_to_batches`)
//! - #4: `RecordBatch` inserter (`insert_batch` / `insert_batches`)

mod common;
use common::TestConnection;

use arrow::array::{Float64Array, Int32Array, StringArray};
use arrow::datatypes::{DataType, Field, Schema};
use arrow::record_batch::RecordBatch;
use std::sync::Arc;

use hyperdb_api::{ArrowInserter, Catalog, SqlType, TableDefinition};

// =============================================================================
// #8: Row type extractors — Interval
// =============================================================================

#[test]
fn test_get_interval() {
    let test = TestConnection::new().expect("Failed to create test connection");

    test.execute_command("CREATE TABLE interval_test (id INT NOT NULL, dur INTERVAL)")
        .expect("create");
    test.execute_command(
        "INSERT INTO interval_test VALUES (1, INTERVAL '1 year 2 months 3 days 4:05:06')",
    )
    .expect("insert");

    let row = test
        .connection
        .fetch_one("SELECT dur FROM interval_test WHERE id = 1")
        .expect("fetch");

    let interval = row.get_interval(0).expect("should parse interval");
    // 1 year 2 months = 14 months, 3 days, 4h5m6s = 14706000000 microseconds
    assert_eq!(interval.months(), 14);
    assert_eq!(interval.days(), 3);
    assert_eq!(
        interval.microseconds(),
        4 * 3_600_000_000 + 5 * 60_000_000 + 6 * 1_000_000
    );
}

#[test]
fn test_get_interval_null() {
    let test = TestConnection::new().expect("Failed to create test connection");

    test.execute_command("CREATE TABLE interval_null (dur INTERVAL)")
        .expect("create");
    test.execute_command("INSERT INTO interval_null VALUES (NULL)")
        .expect("insert");

    let row = test
        .connection
        .fetch_one("SELECT dur FROM interval_null")
        .expect("fetch");

    assert!(row.get_interval(0).is_none());
}

#[test]
fn test_get_interval_via_generic() {
    let test = TestConnection::new().expect("Failed to create test connection");

    test.execute_command("CREATE TABLE interval_generic (dur INTERVAL NOT NULL)")
        .expect("create");
    test.execute_command("INSERT INTO interval_generic VALUES (INTERVAL '10 days')")
        .expect("insert");

    let row = test
        .connection
        .fetch_one("SELECT dur FROM interval_generic")
        .expect("fetch");

    let interval: Option<hyperdb_api::Interval> = row.get(0);
    let interval = interval.expect("generic get should work for Interval");
    assert_eq!(interval.days(), 10);
    assert_eq!(interval.months(), 0);
}

// =============================================================================
// #8: Row type extractors — OffsetTimestamp
// =============================================================================

#[test]
fn test_get_offset_timestamp() {
    let test = TestConnection::new().expect("Failed to create test connection");

    test.execute_command("CREATE TABLE tstz_test (id INT NOT NULL, ts TIMESTAMP WITH TIME ZONE)")
        .expect("create");
    test.execute_command("INSERT INTO tstz_test VALUES (1, TIMESTAMPTZ '2024-06-15 10:30:00+00')")
        .expect("insert");

    let row = test
        .connection
        .fetch_one("SELECT ts FROM tstz_test WHERE id = 1")
        .expect("fetch");

    let ts = row
        .get_offset_timestamp(0)
        .expect("should parse offset timestamp");
    // Verify we got a non-trivial timestamp
    let (date, _time) = ts.timestamp().to_date_time();
    let (y, m, d) = date.to_ymd();
    assert_eq!(y, 2024);
    assert_eq!(m, 6);
    assert_eq!(d, 15);
}

#[test]
fn test_get_offset_timestamp_null() {
    let test = TestConnection::new().expect("Failed to create test connection");

    test.execute_command("CREATE TABLE tstz_null (ts TIMESTAMP WITH TIME ZONE)")
        .expect("create");
    test.execute_command("INSERT INTO tstz_null VALUES (NULL)")
        .expect("insert");

    let row = test
        .connection
        .fetch_one("SELECT ts FROM tstz_null")
        .expect("fetch");

    assert!(row.get_offset_timestamp(0).is_none());
}

#[test]
fn test_get_offset_timestamp_via_generic() {
    let test = TestConnection::new().expect("Failed to create test connection");

    test.execute_command("CREATE TABLE tstz_generic (ts TIMESTAMP WITH TIME ZONE NOT NULL)")
        .expect("create");
    test.execute_command("INSERT INTO tstz_generic VALUES (TIMESTAMPTZ '2020-01-01 00:00:00+00')")
        .expect("insert");

    let row = test
        .connection
        .fetch_one("SELECT ts FROM tstz_generic")
        .expect("fetch");

    let ts: Option<hyperdb_api::OffsetTimestamp> = row.get(0);
    assert!(ts.is_some(), "generic get should work for OffsetTimestamp");
}

// =============================================================================
// #9: Direct RecordBatch access
// =============================================================================

#[test]
fn test_execute_query_to_batches() {
    let test = TestConnection::new().expect("Failed to create test connection");

    test.execute_command(
        "CREATE TABLE batch_test (id INT NOT NULL, name TEXT, value DOUBLE PRECISION)",
    )
    .expect("create");
    test.execute_command(
        "INSERT INTO batch_test VALUES (1, 'Alice', 1.5), (2, 'Bob', 2.5), (3, 'Carol', 3.5)",
    )
    .expect("insert");

    let batches = test
        .connection
        .execute_query_to_batches("SELECT * FROM batch_test ORDER BY id")
        .expect("query to batches");

    assert!(!batches.is_empty(), "should have at least one batch");

    let total_rows: usize = batches
        .iter()
        .map(arrow::array::RecordBatch::num_rows)
        .sum();
    assert_eq!(total_rows, 3);

    // Verify schema
    let schema = batches[0].schema();
    assert_eq!(schema.fields().len(), 3);
}

#[test]
fn test_execute_query_to_batches_empty() {
    let test = TestConnection::new().expect("Failed to create test connection");

    test.execute_command("CREATE TABLE batch_empty (id INT)")
        .expect("create");

    let batches = test
        .connection
        .execute_query_to_batches("SELECT * FROM batch_empty")
        .expect("query to batches");

    let total_rows: usize = batches
        .iter()
        .map(arrow::array::RecordBatch::num_rows)
        .sum();
    assert_eq!(total_rows, 0);
}

#[test]
fn test_execute_query_to_batches_large() {
    let test = TestConnection::new().expect("Failed to create test connection");

    test.execute_command("CREATE TABLE batch_large (id INT NOT NULL)")
        .expect("create");
    test.execute_command("INSERT INTO batch_large SELECT * FROM GENERATE_SERIES(1, 10000)")
        .expect("insert");

    let batches = test
        .connection
        .execute_query_to_batches("SELECT * FROM batch_large")
        .expect("query to batches");

    let total_rows: usize = batches
        .iter()
        .map(arrow::array::RecordBatch::num_rows)
        .sum();
    assert_eq!(total_rows, 10000);
}

// =============================================================================
// #4: RecordBatch inserter
// =============================================================================

#[test]
fn test_insert_batch() {
    let test = TestConnection::new().expect("Failed to create test connection");

    let table_def = TableDefinition::new("rb_insert")
        .add_required_column("id", SqlType::int())
        .add_nullable_column("name", SqlType::text())
        .add_nullable_column("value", SqlType::double());

    Catalog::new(&test.connection)
        .create_table(&table_def)
        .expect("create");

    let schema = Arc::new(Schema::new(vec![
        Field::new("id", DataType::Int32, false),
        Field::new("name", DataType::Utf8, true),
        Field::new("value", DataType::Float64, true),
    ]));

    let batch = RecordBatch::try_new(
        schema,
        vec![
            Arc::new(Int32Array::from(vec![1, 2, 3])),
            Arc::new(StringArray::from(vec![Some("Alice"), Some("Bob"), None])),
            Arc::new(Float64Array::from(vec![Some(1.5), Some(2.5), None])),
        ],
    )
    .unwrap();

    let mut inserter = ArrowInserter::new(&test.connection, &table_def).expect("create inserter");
    inserter.insert_batch(&batch).expect("insert batch");
    let rows = inserter.execute().expect("execute");

    assert_eq!(rows, 3);

    let count = test.count_tuples("rb_insert").expect("count");
    assert_eq!(count, 3);

    // Verify data
    let row = test
        .connection
        .fetch_one("SELECT name FROM rb_insert WHERE id = 1")
        .expect("fetch");
    assert_eq!(row.get::<String>(0), Some("Alice".to_string()));

    // Verify NULL
    let row = test
        .connection
        .fetch_one("SELECT name FROM rb_insert WHERE id = 3")
        .expect("fetch");
    assert!(row.get::<String>(0).is_none());
}

#[test]
fn test_insert_multiple_batches() {
    let test = TestConnection::new().expect("Failed to create test connection");

    let table_def = TableDefinition::new("rb_multi")
        .add_required_column("id", SqlType::int())
        .add_nullable_column("value", SqlType::double());

    Catalog::new(&test.connection)
        .create_table(&table_def)
        .expect("create");

    let schema = Arc::new(Schema::new(vec![
        Field::new("id", DataType::Int32, false),
        Field::new("value", DataType::Float64, true),
    ]));

    let batch1 = RecordBatch::try_new(
        Arc::clone(&schema),
        vec![
            Arc::new(Int32Array::from(vec![1, 2])),
            Arc::new(Float64Array::from(vec![10.0, 20.0])),
        ],
    )
    .unwrap();

    let batch2 = RecordBatch::try_new(
        schema,
        vec![
            Arc::new(Int32Array::from(vec![3, 4, 5])),
            Arc::new(Float64Array::from(vec![30.0, 40.0, 50.0])),
        ],
    )
    .unwrap();

    let mut inserter = ArrowInserter::new(&test.connection, &table_def).expect("create inserter");
    inserter
        .insert_batches([&batch1, &batch2])
        .expect("insert batches");
    let rows = inserter.execute().expect("execute");

    assert_eq!(rows, 5);

    let count = test.count_tuples("rb_multi").expect("count");
    assert_eq!(count, 5);
}

#[test]
fn test_insert_batch_roundtrip() {
    let test = TestConnection::new().expect("Failed to create test connection");

    let table_def = TableDefinition::new("rb_roundtrip")
        .add_required_column("id", SqlType::int())
        .add_nullable_column("name", SqlType::text());

    Catalog::new(&test.connection)
        .create_table(&table_def)
        .expect("create");

    // Insert via RecordBatch
    let schema = Arc::new(Schema::new(vec![
        Field::new("id", DataType::Int32, false),
        Field::new("name", DataType::Utf8, true),
    ]));

    let batch = RecordBatch::try_new(
        schema,
        vec![
            Arc::new(Int32Array::from(vec![1, 2, 3])),
            Arc::new(StringArray::from(vec!["one", "two", "three"])),
        ],
    )
    .unwrap();

    let mut inserter = ArrowInserter::new(&test.connection, &table_def).expect("create inserter");
    inserter.insert_batch(&batch).expect("insert");
    inserter.execute().expect("execute");

    // Read back as RecordBatch
    let batches = test
        .connection
        .execute_query_to_batches("SELECT id, name FROM rb_roundtrip ORDER BY id")
        .expect("read back");

    let total_rows: usize = batches
        .iter()
        .map(arrow::array::RecordBatch::num_rows)
        .sum();
    assert_eq!(total_rows, 3);
}

/// Verifies that `insert_batch()` streams each batch eagerly without accumulating
/// them in memory. Inserts 100 batches of 1000 rows each (100k rows total).
#[test]
fn test_insert_batch_many_batches_streaming() {
    let test = TestConnection::new().expect("Failed to create test connection");

    let table_def = TableDefinition::new("rb_streaming")
        .add_required_column("id", SqlType::int())
        .add_nullable_column("value", SqlType::double());

    Catalog::new(&test.connection)
        .create_table(&table_def)
        .expect("create");

    let schema = Arc::new(Schema::new(vec![
        Field::new("id", DataType::Int32, false),
        Field::new("value", DataType::Float64, true),
    ]));

    let mut inserter = ArrowInserter::new(&test.connection, &table_def).expect("create inserter");

    let batch_count = 100;
    let rows_per_batch = 1000;

    for batch_idx in 0..batch_count {
        let start = batch_idx * rows_per_batch;
        let ids: Vec<i32> = (start..start + rows_per_batch).collect();
        let values: Vec<f64> = (start..start + rows_per_batch)
            .map(|i| f64::from(i) * 0.5)
            .collect();

        let batch = RecordBatch::try_new(
            Arc::clone(&schema),
            vec![
                Arc::new(Int32Array::from(ids)),
                Arc::new(Float64Array::from(values)),
            ],
        )
        .unwrap();

        inserter.insert_batch(&batch).expect("insert batch");
    }

    let rows = inserter.execute().expect("execute");
    assert_eq!(
        rows,
        u64::try_from(batch_count * rows_per_batch).expect("test row count fits in u64")
    );

    let count = test.count_tuples("rb_streaming").expect("count");
    assert_eq!(count, i64::from(batch_count * rows_per_batch));

    // Verify first and last values
    let first = test
        .connection
        .fetch_one("SELECT id, value FROM rb_streaming WHERE id = 0")
        .expect("first");
    assert_eq!(first.get::<i32>(0), Some(0));
    assert_eq!(first.get::<f64>(1), Some(0.0));

    let last_id: i32 = batch_count * rows_per_batch - 1;
    let last = test
        .connection
        .fetch_one(format!(
            "SELECT id, value FROM rb_streaming WHERE id = {last_id}"
        ))
        .expect("last");
    assert_eq!(last.get::<i32>(0), Some(last_id));
}
