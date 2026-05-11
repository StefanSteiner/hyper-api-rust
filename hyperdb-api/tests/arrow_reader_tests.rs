// Copyright (c) 2026, Salesforce, Inc. All rights reserved.
// SPDX-License-Identifier: Apache-2.0 OR MIT

//! Integration tests for `ArrowReader` functionality.
//!
//! These tests verify that `ArrowReader` correctly reads query results
//! in Arrow IPC stream format.

mod common;
use common::{test_hyper_params, test_result_path};
use hyperdb_api::{ArrowReader, Connection, CreateMode, HyperProcess, Result};

/// Test that `ArrowReader` can read results from a simple table
#[test]
fn test_arrow_reader_basic() -> Result<()> {
    let db_path = test_result_path("test_arrow_reader_basic", "hyper")?;
    let params = test_hyper_params("test_arrow_reader_basic")?;
    let hyper = HyperProcess::new(None, Some(&params))?;
    let conn = Connection::new(&hyper, &db_path, CreateMode::CreateAndReplace)?;

    // Create and populate a table
    conn.execute_command("CREATE TABLE test_data (id INTEGER, value DOUBLE PRECISION)")?;
    conn.execute_command("INSERT INTO test_data VALUES (1, 1.5), (2, 2.5), (3, 3.5)")?;

    // Read using ArrowReader
    let reader = ArrowReader::new(&conn);
    let arrow_data = reader.table_to_arrow("test_data")?;

    // Verify we got data
    assert!(!arrow_data.is_empty(), "Arrow data should not be empty");
    assert!(
        arrow_data.len() > 100,
        "Arrow data should have reasonable size"
    );

    Ok(())
}

/// Test `query_to_arrow` with custom SELECT
#[test]
fn test_arrow_reader_query() -> Result<()> {
    let db_path = test_result_path("test_arrow_reader_query", "hyper")?;
    let params = test_hyper_params("test_arrow_reader_query")?;
    let hyper = HyperProcess::new(None, Some(&params))?;
    let conn = Connection::new(&hyper, &db_path, CreateMode::CreateAndReplace)?;

    conn.execute_command("CREATE TABLE products (id INT, name TEXT, price DOUBLE PRECISION)")?;
    conn.execute_command(
        "INSERT INTO products VALUES
        (1, 'Widget', 9.99),
        (2, 'Gadget', 19.99),
        (3, 'Gizmo', 29.99)",
    )?;

    let reader = ArrowReader::new(&conn);

    // Test query with filter
    let arrow_data = reader.query_to_arrow("SELECT * FROM products WHERE price > 15")?;
    assert!(!arrow_data.is_empty());

    // Test query with aggregation
    let arrow_data = reader.query_to_arrow("SELECT COUNT(*), SUM(price) FROM products")?;
    assert!(!arrow_data.is_empty());

    Ok(())
}

/// Test `table_columns_to_arrow`
#[test]
fn test_arrow_reader_columns() -> Result<()> {
    let db_path = test_result_path("test_arrow_reader_columns", "hyper")?;
    let params = test_hyper_params("test_arrow_reader_columns")?;
    let hyper = HyperProcess::new(None, Some(&params))?;
    let conn = Connection::new(&hyper, &db_path, CreateMode::CreateAndReplace)?;

    conn.execute_command("CREATE TABLE users (id INT, name TEXT, email TEXT, age INT)")?;
    conn.execute_command("INSERT INTO users VALUES (1, 'Alice', 'alice@example.com', 30)")?;

    let reader = ArrowReader::new(&conn);

    // Read only specific columns
    let arrow_data = reader.table_columns_to_arrow("users", &["id", "name"])?;
    assert!(!arrow_data.is_empty());

    Ok(())
}

/// Test `table_filtered_to_arrow`
#[test]
fn test_arrow_reader_filtered() -> Result<()> {
    let db_path = test_result_path("test_arrow_reader_filtered", "hyper")?;
    let params = test_hyper_params("test_arrow_reader_filtered")?;
    let hyper = HyperProcess::new(None, Some(&params))?;
    let conn = Connection::new(&hyper, &db_path, CreateMode::CreateAndReplace)?;

    conn.execute_command("CREATE TABLE orders (id INT, amount DOUBLE PRECISION, status TEXT)")?;
    conn.execute_command(
        "INSERT INTO orders VALUES
        (1, 100.0, 'pending'),
        (2, 200.0, 'shipped'),
        (3, 150.0, 'pending'),
        (4, 300.0, 'delivered')",
    )?;

    let reader = ArrowReader::new(&conn);

    // Read with filter
    let arrow_data = reader.table_filtered_to_arrow("orders", "status = 'pending'")?;
    assert!(!arrow_data.is_empty());

    Ok(())
}

/// Test `execute_query_to_arrow` directly on Connection
#[test]
fn test_connection_execute_query_to_arrow() -> Result<()> {
    let db_path = test_result_path("test_conn_arrow", "hyper")?;
    let params = test_hyper_params("test_conn_arrow")?;
    let hyper = HyperProcess::new(None, Some(&params))?;
    let conn = Connection::new(&hyper, &db_path, CreateMode::CreateAndReplace)?;

    conn.execute_command("CREATE TABLE data (x INT, y INT)")?;
    conn.execute_command("INSERT INTO data VALUES (1, 10), (2, 20), (3, 30)")?;

    // Use Connection method directly
    let arrow_data = conn.execute_query_to_arrow("SELECT x, y, x + y AS sum FROM data")?;
    assert!(!arrow_data.is_empty());

    Ok(())
}

/// Test `export_table_to_arrow` directly on Connection
#[test]
fn test_connection_export_table_to_arrow() -> Result<()> {
    let db_path = test_result_path("test_conn_export", "hyper")?;
    let params = test_hyper_params("test_conn_export")?;
    let hyper = HyperProcess::new(None, Some(&params))?;
    let conn = Connection::new(&hyper, &db_path, CreateMode::CreateAndReplace)?;

    conn.execute_command("CREATE TABLE export_test (a INT, b TEXT)")?;
    conn.execute_command("INSERT INTO export_test VALUES (1, 'one'), (2, 'two')")?;

    let arrow_data = conn.export_table_to_arrow("export_test")?;
    assert!(!arrow_data.is_empty());

    Ok(())
}

/// Test `ArrowReader` with empty table
#[test]
fn test_arrow_reader_empty_table() -> Result<()> {
    let db_path = test_result_path("test_arrow_empty", "hyper")?;
    let params = test_hyper_params("test_arrow_empty")?;
    let hyper = HyperProcess::new(None, Some(&params))?;
    let conn = Connection::new(&hyper, &db_path, CreateMode::CreateAndReplace)?;

    conn.execute_command("CREATE TABLE empty_table (id INT, name TEXT)")?;
    // Don't insert any data

    let reader = ArrowReader::new(&conn);
    let arrow_data = reader.table_to_arrow("empty_table")?;

    // Even empty result should have some data (schema at minimum)
    assert!(!arrow_data.is_empty());

    Ok(())
}

/// Test `ArrowReader` with various data types
#[test]
fn test_arrow_reader_various_types() -> Result<()> {
    let db_path = test_result_path("test_arrow_types", "hyper")?;
    let params = test_hyper_params("test_arrow_types")?;
    let hyper = HyperProcess::new(None, Some(&params))?;
    let conn = Connection::new(&hyper, &db_path, CreateMode::CreateAndReplace)?;

    conn.execute_command(
        "CREATE TABLE typed_data (
            int_col INTEGER,
            bigint_col BIGINT,
            double_col DOUBLE PRECISION,
            text_col TEXT,
            bool_col BOOLEAN,
            date_col DATE
        )",
    )?;
    conn.execute_command(
        "INSERT INTO typed_data VALUES
        (1, 1000000000, 3.14159, 'hello', true, DATE '2024-01-15'),
        (NULL, NULL, NULL, NULL, NULL, NULL)",
    )?;

    let reader = ArrowReader::new(&conn);
    let arrow_data = reader.table_to_arrow("typed_data")?;
    assert!(!arrow_data.is_empty());

    Ok(())
}

/// Test `ArrowReader` with query that returns no rows (but has schema)
#[test]
fn test_arrow_reader_no_matching_rows() -> Result<()> {
    let db_path = test_result_path("test_arrow_no_match", "hyper")?;
    let params = test_hyper_params("test_arrow_no_match")?;
    let hyper = HyperProcess::new(None, Some(&params))?;
    let conn = Connection::new(&hyper, &db_path, CreateMode::CreateAndReplace)?;

    conn.execute_command("CREATE TABLE test_filter (id INT)")?;
    conn.execute_command("INSERT INTO test_filter VALUES (1), (2), (3)")?;

    let reader = ArrowReader::new(&conn);
    // Query that matches no rows
    let arrow_data = reader.query_to_arrow("SELECT * FROM test_filter WHERE id > 100")?;

    // Should still have data (schema)
    assert!(!arrow_data.is_empty());

    Ok(())
}

/// Test `ArrowReader` with schema-qualified table name
#[test]
fn test_arrow_reader_schema_qualified() -> Result<()> {
    let db_path = test_result_path("test_arrow_schema", "hyper")?;
    let params = test_hyper_params("test_arrow_schema")?;
    let hyper = HyperProcess::new(None, Some(&params))?;
    let conn = Connection::new(&hyper, &db_path, CreateMode::CreateAndReplace)?;

    // Create a custom schema and table
    conn.execute_command("CREATE SCHEMA custom_schema")?;
    conn.execute_command("CREATE TABLE custom_schema.test_table (id INT, value TEXT)")?;
    conn.execute_command("INSERT INTO custom_schema.test_table VALUES (1, 'test')")?;

    let reader = ArrowReader::new(&conn);
    let arrow_data = reader.table_to_arrow("custom_schema.test_table")?;
    assert!(!arrow_data.is_empty());

    Ok(())
}

/// Test that multiple sequential reads work correctly
#[test]
fn test_arrow_reader_multiple_reads() -> Result<()> {
    let db_path = test_result_path("test_arrow_multi", "hyper")?;
    let params = test_hyper_params("test_arrow_multi")?;
    let hyper = HyperProcess::new(None, Some(&params))?;
    let conn = Connection::new(&hyper, &db_path, CreateMode::CreateAndReplace)?;

    conn.execute_command("CREATE TABLE multi_test (id INT)")?;
    conn.execute_command("INSERT INTO multi_test VALUES (1), (2), (3)")?;

    let reader = ArrowReader::new(&conn);

    // Multiple reads should all succeed
    let data1 = reader.table_to_arrow("multi_test")?;
    let data2 = reader.table_to_arrow("multi_test")?;
    let data3 = reader.query_to_arrow("SELECT COUNT(*) FROM multi_test")?;

    assert!(!data1.is_empty());
    assert!(!data2.is_empty());
    assert!(!data3.is_empty());

    // Same query should return same size
    assert_eq!(data1.len(), data2.len());

    Ok(())
}

/// Test `ArrowReader` with large result set
#[test]
fn test_arrow_reader_large_result() -> Result<()> {
    let db_path = test_result_path("test_arrow_large", "hyper")?;
    let params = test_hyper_params("test_arrow_large")?;
    let hyper = HyperProcess::new(None, Some(&params))?;
    let conn = Connection::new(&hyper, &db_path, CreateMode::CreateAndReplace)?;

    // Create table with 10000 rows using generate_series
    conn.execute_command("CREATE TABLE large_table (id INT, value DOUBLE PRECISION)")?;
    conn.execute_command(
        "INSERT INTO large_table
        SELECT i, i * 0.1
        FROM generate_series(1, 10000) AS s(i)",
    )?;

    let reader = ArrowReader::new(&conn);
    let arrow_data = reader.table_to_arrow("large_table")?;

    // Should have substantial data
    assert!(arrow_data.len() > 10000);

    Ok(())
}

/// Test that invalid queries return errors
#[test]
fn test_arrow_reader_invalid_query() -> Result<()> {
    let db_path = test_result_path("test_arrow_invalid", "hyper")?;
    let params = test_hyper_params("test_arrow_invalid")?;
    let hyper = HyperProcess::new(None, Some(&params))?;
    let conn = Connection::new(&hyper, &db_path, CreateMode::CreateAndReplace)?;

    let reader = ArrowReader::new(&conn);

    // Query non-existent table should fail
    let result = reader.table_to_arrow("nonexistent_table");
    assert!(result.is_err(), "Query on nonexistent table should fail");

    // Query with non-existent column should fail
    let db_path2 = test_result_path("test_arrow_invalid2", "hyper")?;
    let conn2 = Connection::new(&hyper, &db_path2, CreateMode::CreateAndReplace)?;
    conn2.execute_command("CREATE TABLE test_table (id INT)")?;
    let reader2 = ArrowReader::new(&conn2);
    let result = reader2.query_to_arrow("SELECT nonexistent_column FROM test_table");
    assert!(result.is_err(), "Query with nonexistent column should fail");

    Ok(())
}
