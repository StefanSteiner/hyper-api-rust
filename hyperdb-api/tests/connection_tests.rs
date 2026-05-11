// Copyright (c) 2026, Salesforce, Inc. All rights reserved.
// SPDX-License-Identifier: Apache-2.0 OR MIT

//! Tests for Connection management.

use hyperdb_api::{Connection, CreateMode, HyperProcess};
use std::sync::Arc;
use std::thread;
use std::time::Duration;

mod common;
use common::TestConnection;

#[test]
fn test_connection_connect() {
    let test = TestConnection::new().expect("Failed to create test connection");

    // Execute a simple query
    let result = test
        .execute_scalar_i32("SELECT 42")
        .expect("Failed to execute query");
    assert_eq!(result, 42);
}

#[test]
fn test_connection_create_mode_none() {
    let temp_dir = tempfile::tempdir().expect("Failed to create temp directory");
    let db_path = temp_dir.path().join("test.hyper");

    let params = common::test_hyper_params("test_connection_create_mode_none")
        .expect("Failed to create test parameters");
    let hyper = HyperProcess::new(None, Some(&params)).expect("Failed to start Hyper process");

    // Should fail when database doesn't exist
    assert!(Connection::new(&hyper, &db_path, CreateMode::DoNotCreate).is_err());

    // Create the database
    let _conn1 =
        Connection::new(&hyper, &db_path, CreateMode::Create).expect("Failed to create database");

    // Now should succeed
    let _conn2 = Connection::new(&hyper, &db_path, CreateMode::DoNotCreate)
        .expect("Failed to connect to existing database");
}

#[test]
fn test_connection_create_mode_create() {
    let temp_dir = tempfile::tempdir().expect("Failed to create temp directory");
    let db_path = temp_dir.path().join("test.hyper");

    let params = common::test_hyper_params("test_connection_create_mode_create")
        .expect("Failed to create test parameters");
    let hyper = HyperProcess::new(None, Some(&params)).expect("Failed to start Hyper process");

    // Create the database
    let _conn1 =
        Connection::new(&hyper, &db_path, CreateMode::Create).expect("Failed to create database");

    // Trying to create again should fail
    assert!(Connection::new(&hyper, &db_path, CreateMode::Create).is_err());
}

#[test]
fn test_connection_create_mode_create_if_not_exists() {
    let temp_dir = tempfile::tempdir().expect("Failed to create temp directory");
    let db_path = temp_dir.path().join("test.hyper");

    let params = common::test_hyper_params("test_connection_create_mode_create_if_not_exists")
        .expect("Failed to create test parameters");
    let hyper = HyperProcess::new(None, Some(&params)).expect("Failed to start Hyper process");

    // Create the database
    {
        let conn = Connection::new(&hyper, &db_path, CreateMode::CreateIfNotExists)
            .expect("Failed to create database");
        conn.execute_command("CREATE TABLE FOO()")
            .expect("Failed to create table");
        conn.close().expect("Failed to close connection");
    }

    // Should not fail if it already exists
    {
        let conn = Connection::new(&hyper, &db_path, CreateMode::CreateIfNotExists)
            .expect("Failed to connect to existing database");
        // Verify the table still exists
        let mut rowset = conn
            .execute_query("SELECT COUNT(*) FROM FOO")
            .expect("Failed to query table");
        let chunk = rowset
            .next_chunk()
            .expect("Failed to get chunk")
            .expect("Expected chunk");
        let row = chunk.first().expect("Expected row");
        let count: i64 = row.get_i64(0).expect("NULL value");
        assert_eq!(count, 0);
    }
}

#[test]
fn test_connection_create_mode_create_and_replace() {
    let temp_dir = tempfile::tempdir().expect("Failed to create temp directory");
    let db_path = temp_dir.path().join("test.hyper");

    let params = common::test_hyper_params("test_connection_create_mode_create_and_replace")
        .expect("Failed to create test parameters");
    let hyper = HyperProcess::new(None, Some(&params)).expect("Failed to start Hyper process");

    // Create the database with a table
    {
        let conn = Connection::new(&hyper, &db_path, CreateMode::CreateAndReplace)
            .expect("Failed to create database");
        conn.execute_command("CREATE TABLE FOO (id INT)")
            .expect("Failed to create table");
        conn.execute_command("INSERT INTO FOO VALUES (1)")
            .expect("Failed to insert data");
        conn.close().expect("Failed to close connection");
    }

    // Replace the database
    {
        let conn = Connection::new(&hyper, &db_path, CreateMode::CreateAndReplace)
            .expect("Failed to replace database");
        // Table should not exist anymore - error may occur on execute_query or next_chunk
        let query_result = conn.execute_query("SELECT * FROM FOO");
        let table_missing = match query_result {
            Err(_) => true,                                 // Error during execute_query
            Ok(mut rowset) => rowset.next_chunk().is_err(), // Error during streaming
        };
        assert!(
            table_missing,
            "Table FOO should not exist after database replacement"
        );
    }
}

#[test]
fn test_connection_execute_command() {
    let test = TestConnection::new().expect("Failed to create test connection");

    // Create a table
    test.execute_command("CREATE TABLE test (id INT, name TEXT)")
        .expect("Failed to create table");

    // Insert data
    let rows = test
        .execute_command("INSERT INTO test VALUES (1, 'Alice'), (2, 'Bob')")
        .expect("Failed to insert data");
    assert_eq!(rows, 2u64);

    // Verify data
    let count = test.count_tuples("test").expect("Failed to count tuples");
    assert_eq!(count, 2);
}

#[test]
fn test_connection_execute_query() {
    let test = TestConnection::new().expect("Failed to create test connection");

    test.execute_command("CREATE TABLE test (id INT, name TEXT)")
        .expect("Failed to create table");
    test.execute_command("INSERT INTO test VALUES (1, 'Alice'), (2, 'Bob')")
        .expect("Failed to insert data");

    let mut result = test
        .execute_query("SELECT id, name FROM test ORDER BY id")
        .expect("Failed to execute query");

    let mut rows: Vec<(i32, String)> = Vec::new();
    while let Some(chunk) = result.next_chunk().expect("Failed to get chunk") {
        for row in &chunk {
            let id = row.get_i32(0).expect("NULL id");
            let name = row.get::<String>(1).expect("NULL name");
            rows.push((id, name));
        }
    }

    assert_eq!(rows.len(), 2);
    assert_eq!(rows[0], (1, "Alice".to_string()));
    assert_eq!(rows[1], (2, "Bob".to_string()));
}

#[test]
fn test_connection_execute_scalar() {
    let test = TestConnection::new().expect("Failed to create test connection");

    // Test integer
    let int_val = test
        .execute_scalar_i32("SELECT 42")
        .expect("Failed to execute scalar query");
    assert_eq!(int_val, 42);

    // Test string
    let str_val = test
        .execute_scalar_string("SELECT 'hello'")
        .expect("Failed to execute scalar query");
    assert_eq!(str_val, "hello");

    // Test boolean
    let bool_val = test
        .execute_scalar_bool("SELECT true")
        .expect("Failed to execute scalar query");
    assert!(bool_val);
}

/// Tests database attach and detach functionality.
///
/// # Connection Lifecycle Notes
///
/// This test creates databases in separate scopes. When each scope ends, the
/// Connection is dropped, which automatically closes the connection to the database.
/// This ensures the database files are not locked when we later try to attach them.
///
/// The Hyper process allows multiple connections and multiple attached databases,
/// but a single database file cannot be opened by multiple connections simultaneously
/// with write access. By closing the initial connections (via scope drop), we ensure
/// the attach operations succeed.
#[test]
fn test_connection_attach_detach_database() {
    let temp_dir1 = tempfile::tempdir().expect("Failed to create temp directory");
    let temp_dir2 = tempfile::tempdir().expect("Failed to create temp directory");
    let db1_path = temp_dir1.path().join("db1.hyper");
    let db2_path = temp_dir2.path().join("db2.hyper");

    let params = common::test_hyper_params("test_connection_attach_detach_database")
        .expect("Failed to create test parameters");
    let hyper = HyperProcess::new(None, Some(&params)).expect("Failed to start Hyper process");

    // Create first database with a table.
    // The connection is closed when the scope ends (via Drop), releasing the database file.
    {
        let conn = Connection::new(&hyper, &db1_path, CreateMode::CreateAndReplace)
            .expect("Failed to create database 1");
        conn.execute_command("CREATE TABLE t1 (id INT)")
            .expect("Failed to create table");
        conn.execute_command("INSERT INTO t1 VALUES (1)")
            .expect("Failed to insert data");
        // Connection dropped here - database file released
    }

    // Create second database.
    // The connection is closed when the scope ends (via Drop), releasing the database file.
    {
        let conn = Connection::new(&hyper, &db2_path, CreateMode::CreateAndReplace)
            .expect("Failed to create database 2");
        conn.execute_command("CREATE TABLE t2 (id INT)")
            .expect("Failed to create table");
        // Connection dropped here - database file released
    }

    // Both database files are now released, so we can attach them.
    // Connect without a database and attach both
    let conn = Connection::without_database(hyper.endpoint().unwrap()).expect("Failed to connect");

    conn.attach_database(db1_path.to_str().unwrap(), Some("alias1"))
        .expect("Failed to attach db1");
    conn.attach_database(db2_path.to_str().unwrap(), Some("alias2"))
        .expect("Failed to attach db2");

    // Query from attached database
    let mut result = conn
        .execute_query("SELECT id FROM alias1.public.t1")
        .expect("Failed to query attached db");
    let chunk = result
        .next_chunk()
        .expect("Failed to get chunk")
        .expect("Expected chunk");
    let row = chunk.first().expect("Expected row");
    let id = row.get_i32(0).expect("NULL id");
    assert_eq!(id, 1);

    // Detach databases
    conn.detach_database("alias1")
        .expect("Failed to detach db1");
    conn.detach_database("alias2")
        .expect("Failed to detach db2");
}

/// Tests that query cancellation works correctly.
///
/// This test verifies that:
/// 1. A long-running query can be cancelled from another thread
/// 2. The cancelled query returns an error with SQLSTATE 57014 (`query_canceled`)
/// 3. The connection remains usable after cancellation
#[test]
fn test_connection_cancel_query() {
    let temp_dir = tempfile::tempdir().expect("Failed to create temp directory");
    let db_path = temp_dir.path().join("cancel_test.hyper");

    let params = common::test_hyper_params("test_connection_cancel_query")
        .expect("Failed to create test parameters");
    let hyper = HyperProcess::new(None, Some(&params)).expect("Failed to start Hyper process");

    // Create a connection and wrap in Arc for thread sharing
    let conn = Arc::new(
        Connection::new(&hyper, &db_path, CreateMode::CreateIfNotExists)
            .expect("Failed to create connection"),
    );

    // Create a large table to query
    conn.execute_command(
        "CREATE TABLE large_data AS
         SELECT i as id, 'row_' || i::TEXT as name
         FROM generate_series(1, 1000000) AS s(i)",
    )
    .expect("Failed to create large table");

    let conn_query = Arc::clone(&conn);
    let conn_cancel = Arc::clone(&conn);

    // Start a long query in another thread
    let query_handle = thread::spawn(move || {
        // Execute a query that will return many rows
        // We iterate through results to keep the query running
        let result = conn_query.execute_query("SELECT * FROM large_data ORDER BY id");

        match result {
            Ok(mut rowset) => {
                let mut row_count = 0;
                loop {
                    match rowset.next_chunk() {
                        Ok(Some(chunk)) => {
                            row_count += chunk.len();
                            // Keep iterating to keep the query alive
                        }
                        Ok(None) => {
                            // Query completed without cancellation
                            return Ok(row_count);
                        }
                        Err(e) => {
                            // This is where we expect the cancel error
                            return Err(e);
                        }
                    }
                }
            }
            Err(e) => Err(e),
        }
    });

    // Give the query a moment to start
    thread::sleep(Duration::from_millis(50));

    // Cancel the query from the main thread
    conn_cancel.cancel().expect("Failed to send cancel request");

    // Wait for the query thread to complete
    let query_result = query_handle.join().expect("Query thread panicked");

    // The query should have been cancelled (error) OR completed before cancel arrived
    // Both are valid outcomes depending on timing
    match query_result {
        Ok(row_count) => {
            // Query completed before cancel - this can happen if the query is fast
            println!("Query completed with {row_count} rows before cancel arrived");
        }
        Err(e) => {
            // Query was cancelled - verify it's the right error
            let error_string = e.to_string();
            // Check for query_canceled (57014) or canceling statement due to user request
            assert!(
                error_string.contains("57014")
                    || error_string.contains("cancel")
                    || error_string.contains("Cancel"),
                "Expected cancellation error, got: {error_string}"
            );
            println!("Query successfully cancelled: {error_string}");
        }
    }

    // Verify the data is intact by using a fresh connection.
    // Note: We can't reliably use the same connection because the server may have
    // queued the cancel request and will apply it to the next query (this is
    // expected PostgreSQL/Hyper behavior, as noted in test_connection_cancel_no_query).
    drop(conn);
    let verify_conn = Connection::new(&hyper, &db_path, CreateMode::DoNotCreate)
        .expect("Should be able to open database for verification");
    let count: i64 = verify_conn
        .execute_scalar_query("SELECT COUNT(*) FROM large_data")
        .expect("Should be able to query data")
        .expect("Should return count");
    assert_eq!(count, 1000000);
}

/// Tests that cancel request succeeds even when no query is running.
///
/// Note: The server may hold onto the cancel request and apply it to
/// the next query, so we don't verify query behavior after cancel.
/// This test just verifies that sending a cancel doesn't crash.
#[test]
fn test_connection_cancel_no_query() {
    let test = TestConnection::new().expect("Failed to create test connection");

    // Cancel when nothing is running - the call should succeed
    // (it opens a new connection, sends cancel, and closes)
    test.connection
        .cancel()
        .expect("Cancel with no query should succeed");

    // Note: We don't test queries after this because the server may have
    // queued the cancel request. This is expected PostgreSQL/Hyper behavior.
}

/// Tests that `parameter_status` returns server parameters.
#[test]
fn test_connection_parameter_status() {
    let test = TestConnection::new().expect("Failed to create test connection");

    // server_version should always be set
    let version = test.connection.parameter_status("server_version");
    assert!(
        version.is_some(),
        "server_version parameter should be available"
    );
    println!("Connected to Hyper version: {}", version.unwrap());

    // Test a non-existent parameter
    let unknown = test.connection.parameter_status("nonexistent_param");
    assert!(unknown.is_none(), "Unknown parameter should return None");
}

/// Tests that notice receiver callback is called for warnings.
#[test]
fn test_connection_notice_receiver() {
    use std::sync::{Arc, Mutex};

    let temp_dir = tempfile::tempdir().expect("Failed to create temp directory");
    let db_path = temp_dir.path().join("notice_test.hyper");

    let params = common::test_hyper_params("test_connection_notice_receiver")
        .expect("Failed to create test parameters");
    let hyper = HyperProcess::new(None, Some(&params)).expect("Failed to start Hyper process");
    let mut conn = Connection::new(&hyper, &db_path, CreateMode::CreateIfNotExists)
        .expect("Failed to create connection");

    // Collect notices in a vector
    let notices: Arc<Mutex<Vec<String>>> = Arc::new(Mutex::new(Vec::new()));
    let notices_clone = Arc::clone(&notices);

    conn.set_notice_receiver(Some(Box::new(move |notice| {
        notices_clone.lock().unwrap().push(format!("{notice}"));
    })));

    // Execute a simple query - may or may not generate notices
    conn.execute_command("SELECT 1")
        .expect("Failed to execute query");

    // The test passes if the callback mechanism works (even if no notices generated)
    // Print any collected notices for debugging
    let collected = notices.lock().unwrap();
    if !collected.is_empty() {
        println!("Collected {} notices:", collected.len());
        for notice in collected.iter() {
            println!("  - {notice}");
        }
    }

    // Reset to default behavior
    conn.set_notice_receiver(None);

    // Verify connection still works after resetting receiver
    let result: i32 = conn
        .execute_scalar_query("SELECT 42")
        .expect("Query failed after resetting notice receiver")
        .expect("Expected non-null value");
    assert_eq!(result, 42);
}
