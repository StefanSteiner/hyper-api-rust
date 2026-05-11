// Copyright (c) 2026, Salesforce, Inc. All rights reserved.
// SPDX-License-Identifier: Apache-2.0 OR MIT

//! Tests for Result and Rowset operations.

mod common;
use common::TestConnection;

#[test]
fn test_result_single_row_consumption() {
    let test = TestConnection::new().expect("Failed to create test connection");

    test.execute_command("CREATE TABLE FOO ( A INT )")
        .expect("Failed to create table");
    test.execute_command("INSERT INTO FOO SELECT * FROM GENERATE_SERIES(1, 1000)")
        .expect("Failed to insert data");

    let mut result = test
        .execute_query("SELECT * FROM FOO ORDER BY A")
        .expect("Failed to execute query");

    let mut expected = 1;
    while let Some(chunk) = result.next_chunk().expect("Failed to get chunk") {
        for row in &chunk {
            let value = row.get_i32(0).expect("NULL value");
            assert_eq!(value, expected);
            expected += 1;
        }
    }

    assert_eq!(expected, 1001);
}

#[test]
fn test_result_empty_result() {
    let test = TestConnection::new().expect("Failed to create test connection");

    test.execute_command("CREATE TABLE FOO ( A INT )")
        .expect("Failed to create table");

    let mut result = test
        .execute_query("SELECT * FROM FOO")
        .expect("Failed to execute query");

    let mut count = 0;
    while let Some(chunk) = result.next_chunk().expect("Failed to get chunk") {
        count += chunk.len();
    }

    assert_eq!(count, 0);
}

#[test]
fn test_result_multiple_columns() {
    let test = TestConnection::new().expect("Failed to create test connection");

    test.execute_command("CREATE TABLE FOO ( A INT, B TEXT, C DOUBLE PRECISION )")
        .expect("Failed to create table");
    test.execute_command("INSERT INTO FOO VALUES (1, 'Alice', 10.5), (2, 'Bob', 20.7)")
        .expect("Failed to insert data");

    let mut result = test
        .execute_query("SELECT * FROM FOO ORDER BY A")
        .expect("Failed to execute query");

    let mut rows: Vec<(i32, String, f64)> = Vec::new();
    while let Some(chunk) = result.next_chunk().expect("Failed to get chunk") {
        for row in &chunk {
            let a = row.get_i32(0).expect("NULL A");
            let b = row.get::<String>(1).expect("NULL B");
            let c = row.get_f64(2).expect("NULL C");
            rows.push((a, b, c));
        }
    }

    assert_eq!(rows.len(), 2);
    assert_eq!(rows[0], (1, "Alice".to_string(), 10.5));
    assert_eq!(rows[1], (2, "Bob".to_string(), 20.7));
}

#[test]
fn test_result_schema() {
    let test = TestConnection::new().expect("Failed to create test connection");

    test.execute_command("CREATE TABLE FOO ( A INT, B TEXT, C DOUBLE PRECISION )")
        .expect("Failed to create table");
    test.execute_command("INSERT INTO FOO VALUES (1, 'test', 1.0)")
        .expect("Failed to insert data");

    // Test schema by querying pg_catalog for column information
    let mut result = test
        .execute_query(
            r"SELECT a.attname as column_name
               FROM pg_catalog.pg_attribute a
               JOIN pg_catalog.pg_class c ON a.attrelid = c.oid
               JOIN pg_catalog.pg_namespace n ON c.relnamespace = n.oid
               WHERE n.nspname = 'public'
                 AND c.relname = 'foo'
                 AND a.attnum > 0
                 AND NOT a.attisdropped
               ORDER BY a.attnum",
        )
        .expect("Failed to execute query");

    let mut columns = Vec::new();
    while let Some(chunk) = result.next_chunk().expect("Failed to get chunk") {
        for row in &chunk {
            let name = row.get::<String>(0).expect("NULL column name");
            columns.push(name);
        }
    }

    assert_eq!(columns.len(), 3);
    // Column names are returned in lowercase by Hyper
    assert_eq!(columns[0], "a");
    assert_eq!(columns[1], "b");
    assert_eq!(columns[2], "c");
}

#[test]
fn test_result_null_values() {
    let test = TestConnection::new().expect("Failed to create test connection");

    test.execute_command("CREATE TABLE FOO ( A INT, B TEXT )")
        .expect("Failed to create table");
    test.execute_command("INSERT INTO FOO VALUES (1, 'Alice'), (2, NULL)")
        .expect("Failed to insert data");

    let mut result = test
        .execute_query("SELECT * FROM FOO ORDER BY A")
        .expect("Failed to execute query");

    let mut rows: Vec<(i32, Option<String>)> = Vec::new();
    while let Some(chunk) = result.next_chunk().expect("Failed to get chunk") {
        for row in &chunk {
            let a = row.get_i32(0).expect("NULL A");
            let b = row.get::<String>(1);
            rows.push((a, b));
        }
    }

    assert_eq!(rows.len(), 2);
    assert_eq!(rows[0], (1, Some("Alice".to_string())));
    assert_eq!(rows[1], (2, None));
}

#[test]
fn test_result_chunked_consumption() {
    let test = TestConnection::new().expect("Failed to create test connection");

    test.execute_command("CREATE TABLE FOO ( A INT )")
        .expect("Failed to create table");
    test.execute_command("INSERT INTO FOO SELECT * FROM GENERATE_SERIES(1, 1000)")
        .expect("Failed to insert data");

    let mut result = test
        .execute_query("SELECT * FROM FOO ORDER BY A")
        .expect("Failed to execute query");

    let mut expected = 1;
    let mut row_count = 0;

    while let Some(chunk) = result.next_chunk().expect("Failed to get chunk") {
        for row in &chunk {
            row_count += 1;
            let value = row.get_i32(0).expect("NULL value");
            assert_eq!(value, expected);
            expected += 1;
        }
    }

    assert_eq!(expected, 1001);
    assert!(row_count > 0);
}

#[test]
#[expect(
    clippy::approx_constant,
    reason = "test literal chosen for readability; not intended as an approximation"
)]
fn test_result_scalar_queries() {
    let test = TestConnection::new().expect("Failed to create test connection");

    // Integer scalar
    let int_val = test
        .execute_scalar_i32("SELECT 42")
        .expect("Failed to execute scalar query");
    assert_eq!(int_val, 42);

    // String scalar
    let str_val = test
        .execute_scalar_string("SELECT 'hello'")
        .expect("Failed to execute scalar query");
    assert_eq!(str_val, "hello");

    // Boolean scalar
    let bool_val = test
        .execute_scalar_bool("SELECT true")
        .expect("Failed to execute scalar query");
    assert!(bool_val);

    // Double scalar - use CAST to ensure DOUBLE PRECISION type
    let mut result = test
        .execute_query("SELECT 3.14::DOUBLE PRECISION")
        .expect("Failed to query");
    let chunk = result
        .next_chunk()
        .expect("Failed to get chunk")
        .expect("Expected chunk");
    let row = chunk.first().expect("Expected row");
    let double_val = row.get_f64(0).expect("NULL double");
    assert!((double_val - 3.14).abs() < 0.001);
}

#[test]
#[expect(
    clippy::approx_constant,
    reason = "test literal chosen for readability; not intended as an approximation"
)]
fn test_result_type_conversion() {
    let test = TestConnection::new().expect("Failed to create test connection");

    test.execute_command("CREATE TABLE FOO ( A INT, B TEXT, C DOUBLE PRECISION, D BOOLEAN )")
        .expect("Failed to create table");
    test.execute_command("INSERT INTO FOO VALUES (42, 'test', 3.14, true)")
        .expect("Failed to insert data");

    let mut result = test
        .connection
        .execute_query("SELECT * FROM FOO")
        .expect("Failed to execute query");

    let chunk = result
        .next_chunk()
        .expect("Failed to get chunk")
        .expect("Expected chunk");
    let row = chunk.first().expect("Expected row");

    // Test type conversions
    let a_int = row.get_i32(0).expect("NULL A");
    assert_eq!(a_int, 42);

    let b_str = row.get::<String>(1).expect("NULL B");
    assert_eq!(b_str, "test");

    let c_f64 = row.get_f64(2).expect("NULL C");
    assert!((c_f64 - 3.14).abs() < 0.001);

    let d_bool = row.get::<bool>(3).expect("NULL D");
    assert!(d_bool);
}

#[test]
fn test_result_row_iterator() {
    let test = TestConnection::new().expect("Failed to create test connection");

    test.execute_command("CREATE TABLE FOO ( A INT, B TEXT )")
        .expect("Failed to create table");
    test.execute_command("INSERT INTO FOO VALUES (1, 'one'), (2, 'two'), (3, 'three')")
        .expect("Failed to insert data");

    let mut result = test
        .execute_query("SELECT * FROM FOO ORDER BY A")
        .expect("Failed to execute query");

    let mut count = 0;
    while let Some(chunk) = result.next_chunk().expect("Failed to get chunk") {
        for row in &chunk {
            let a: Option<i32> = row.get(0);
            let b: Option<String> = row.get(1);
            count += 1;
            match count {
                1 => {
                    assert_eq!(a, Some(1));
                    assert_eq!(b, Some("one".to_string()));
                }
                2 => {
                    assert_eq!(a, Some(2));
                    assert_eq!(b, Some("two".to_string()));
                }
                3 => {
                    assert_eq!(a, Some(3));
                    assert_eq!(b, Some("three".to_string()));
                }
                _ => panic!("Unexpected row"),
            }
        }
    }

    assert_eq!(count, 3);
}

#[test]
fn test_result_is_null() {
    let test = TestConnection::new().expect("Failed to create test connection");

    test.execute_command("CREATE TABLE FOO ( A INT, B TEXT )")
        .expect("Failed to create table");
    test.execute_command("INSERT INTO FOO VALUES (1, 'value'), (2, NULL)")
        .expect("Failed to insert data");

    let mut result = test
        .execute_query("SELECT * FROM FOO ORDER BY A")
        .expect("Failed to execute query");

    let chunk = result
        .next_chunk()
        .expect("Failed to get chunk")
        .expect("Expected chunk");

    // Validate chunk dimensions before accessing indices to prevent panics
    assert!(
        chunk.len() >= 2,
        "Expected at least 2 rows, got {}",
        chunk.len()
    );

    // First row - not null (row 0, column 1)
    assert!(chunk[0].get::<String>(1).is_some());
    // Second row - null (row 1, column 1)
    assert!(chunk[1].get::<String>(1).is_none());
}

#[test]
fn test_result_rows_iterator() {
    let test = TestConnection::new().expect("Failed to create test connection");

    test.execute_command("CREATE TABLE FOO ( A INT, B TEXT )")
        .expect("Failed to create table");
    test.execute_command("INSERT INTO FOO VALUES (1, 'one'), (2, 'two'), (3, 'three')")
        .expect("Failed to insert data");

    // Test the rows() iterator - C++-like pattern
    let result = test
        .execute_query("SELECT * FROM FOO ORDER BY A")
        .expect("Failed to execute query");

    let mut count = 0;
    for row in result.rows() {
        let row = row.expect("Failed to get row");
        let a: Option<i32> = row.get(0);
        let b: Option<String> = row.get(1);
        count += 1;
        match count {
            1 => {
                assert_eq!(a, Some(1));
                assert_eq!(b, Some("one".to_string()));
            }
            2 => {
                assert_eq!(a, Some(2));
                assert_eq!(b, Some("two".to_string()));
            }
            3 => {
                assert_eq!(a, Some(3));
                assert_eq!(b, Some("three".to_string()));
            }
            _ => panic!("Unexpected row"),
        }
    }

    assert_eq!(count, 3);
}

#[test]
fn test_result_rows_iterator_with_try_for_each() {
    let test = TestConnection::new().expect("Failed to create test connection");

    test.execute_command("CREATE TABLE FOO ( A INT )")
        .expect("Failed to create table");
    test.execute_command("INSERT INTO FOO SELECT * FROM GENERATE_SERIES(1, 100)")
        .expect("Failed to insert data");

    // Test try_for_each pattern
    let result = test
        .execute_query("SELECT * FROM FOO ORDER BY A")
        .expect("Failed to execute query");

    let mut sum = 0i64;
    result
        .rows()
        .try_for_each(|row| {
            let row = row?;
            let value: i32 = row.get(0).unwrap_or(0);
            sum += i64::from(value);
            Ok::<_, hyperdb_api::Error>(())
        })
        .expect("Iteration failed");

    // Sum of 1..100 = 5050
    assert_eq!(sum, 5050);
}

#[test]
fn test_result_rows_iterator_collect() {
    let test = TestConnection::new().expect("Failed to create test connection");

    test.execute_command("CREATE TABLE FOO ( A INT )")
        .expect("Failed to create table");
    test.execute_command("INSERT INTO FOO VALUES (1), (2), (3)")
        .expect("Failed to insert data");

    // Test collecting all rows
    let result = test
        .execute_query("SELECT * FROM FOO ORDER BY A")
        .expect("Failed to execute query");

    let rows: Vec<_> = result
        .rows()
        .collect::<Result<Vec<_>, _>>()
        .expect("Collection failed");

    assert_eq!(rows.len(), 3);
    assert_eq!(rows[0].get_i32(0), Some(1));
    assert_eq!(rows[1].get_i32(0), Some(2));
    assert_eq!(rows[2].get_i32(0), Some(3));
}
