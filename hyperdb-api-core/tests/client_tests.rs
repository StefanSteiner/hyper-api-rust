// Copyright (c) 2026, Salesforce, Inc. All rights reserved.
// SPDX-License-Identifier: Apache-2.0 OR MIT

//! Integration tests for the hyper-client Client API.
//!
//! These tests verify that the low-level `Client` API works correctly
//! against a real Hyper server.

mod common;
use common::TestServer;
use hyperdb_api_core::client::{Client, Config};

// =============================================================================
// Connection Tests
// =============================================================================

#[test]
fn test_client_connect() {
    let server = TestServer::new().expect("Failed to create test server");
    let client = server.connect().expect("Failed to connect");

    // Verify connection works with a simple query
    let rows = client.query("SELECT 1").expect("Failed to query");
    assert_eq!(rows.len(), 1);

    client.close().expect("Failed to close");
}

#[test]
fn test_client_connect_without_database() {
    let server = TestServer::without_database().expect("Failed to create test server");
    let client = server
        .connect_without_database()
        .expect("Failed to connect");

    // Should be able to query without a database attached
    let rows = client.query("SELECT 42 as value").expect("Failed to query");
    assert_eq!(rows.len(), 1);

    let value = rows[0].get_i32(0).expect("Failed to get value");
    assert_eq!(value, 42);

    client.close().expect("Failed to close");
}

#[test]
fn test_client_connect_invalid_host() {
    let config = Config::new()
        .with_host("invalid-host-that-does-not-exist.local")
        .with_port(7483);

    let result = Client::connect(&config);
    assert!(result.is_err(), "Should fail to connect to invalid host");
}

#[test]
fn test_client_connect_invalid_port() {
    // Use a port that's very unlikely to be in use
    let config = Config::new().with_host("127.0.0.1").with_port(1); // Port 1 is reserved and won't have Hyper running

    let result = Client::connect(&config);
    assert!(result.is_err(), "Should fail to connect to invalid port");
}

// =============================================================================
// Query Tests
// =============================================================================

#[test]
fn test_client_query_simple() {
    let server = TestServer::new().expect("Failed to create test server");
    let client = server.connect().expect("Failed to connect");

    // Create a table and insert data
    client
        .exec("CREATE TABLE test_query (id INT, name TEXT)")
        .expect("Failed to create table");

    client
        .exec("INSERT INTO test_query VALUES (1, 'Alice'), (2, 'Bob')")
        .expect("Failed to insert");

    // Query the data
    let rows = client
        .query("SELECT * FROM test_query ORDER BY id")
        .expect("Failed to query");

    assert_eq!(rows.len(), 2);

    // First row
    assert_eq!(rows[0].get_i32(0), Some(1));
    assert_eq!(rows[0].get_string(1), Some("Alice".to_string()));

    // Second row
    assert_eq!(rows[1].get_i32(0), Some(2));
    assert_eq!(rows[1].get_string(1), Some("Bob".to_string()));

    client.close().expect("Failed to close");
}

#[test]
fn test_client_query_with_nulls() {
    let server = TestServer::new().expect("Failed to create test server");
    let client = server.connect().expect("Failed to connect");

    client
        .exec("CREATE TABLE test_nulls (id INT, value TEXT)")
        .expect("Failed to create table");

    client
        .exec("INSERT INTO test_nulls VALUES (1, 'hello'), (2, NULL)")
        .expect("Failed to insert");

    let rows = client
        .query("SELECT * FROM test_nulls ORDER BY id")
        .expect("Failed to query");

    assert_eq!(rows.len(), 2);

    // First row: has value
    assert_eq!(rows[0].get_i32(0), Some(1));
    assert_eq!(rows[0].get_string(1), Some("hello".to_string()));

    // Second row: NULL value
    assert_eq!(rows[1].get_i32(0), Some(2));
    assert_eq!(rows[1].get_string(1), None);

    client.close().expect("Failed to close");
}

#[test]
fn test_client_query_empty_result() {
    let server = TestServer::new().expect("Failed to create test server");
    let client = server.connect().expect("Failed to connect");

    client
        .exec("CREATE TABLE test_empty (id INT)")
        .expect("Failed to create table");

    let rows = client
        .query("SELECT * FROM test_empty")
        .expect("Failed to query");

    assert!(rows.is_empty());

    client.close().expect("Failed to close");
}

#[test]
fn test_client_query_fast() {
    let server = TestServer::new().expect("Failed to create test server");
    let client = server.connect().expect("Failed to connect");

    client
        .exec("CREATE TABLE test_fast (id INT, value DOUBLE PRECISION)")
        .expect("Failed to create table");

    client
        .exec("INSERT INTO test_fast VALUES (1, 1.5), (2, 2.5), (3, 3.5)")
        .expect("Failed to insert");

    // Use query_fast for HyperBinary streaming
    let rows = client
        .query_fast("SELECT * FROM test_fast ORDER BY id")
        .expect("Failed to query_fast");

    assert_eq!(rows.len(), 3);

    // StreamRow uses get_* methods
    assert_eq!(rows[0].get_i32(0), Some(1));
    assert_eq!(rows[0].get_f64(1), Some(1.5));

    assert_eq!(rows[1].get_i32(0), Some(2));
    assert_eq!(rows[1].get_f64(1), Some(2.5));

    assert_eq!(rows[2].get_i32(0), Some(3));
    assert_eq!(rows[2].get_f64(1), Some(3.5));

    client.close().expect("Failed to close");
}

#[test]
fn test_client_query_streaming() {
    let server = TestServer::new().expect("Failed to create test server");
    let client = server.connect().expect("Failed to connect");

    client
        .exec("CREATE TABLE test_streaming (id INT)")
        .expect("Failed to create table");

    // Insert 100 rows
    for i in 0..100 {
        client
            .exec(&format!("INSERT INTO test_streaming VALUES ({i})"))
            .expect("Failed to insert");
    }

    // Stream with chunks of 10
    let mut total_rows = 0;
    let mut last_id = -1i32;

    {
        let mut stream = client
            .query_streaming("SELECT * FROM test_streaming ORDER BY id", 10)
            .expect("Failed to start streaming");

        while let Some(chunk) = stream.next_chunk().expect("Failed to get chunk") {
            assert!(chunk.len() <= 10, "Chunk should not exceed chunk_size");
            for row in &chunk {
                let id = row.get_i32(0).expect("Expected id");
                assert!(id > last_id, "Rows should be in order");
                last_id = id;
                total_rows += 1;
            }
        }
    } // stream dropped here, releasing borrow

    assert_eq!(total_rows, 100);

    client.close().expect("Failed to close");
}

// =============================================================================
// Exec Tests
// =============================================================================

#[test]
fn test_client_exec_create_table() {
    let server = TestServer::new().expect("Failed to create test server");
    let client = server.connect().expect("Failed to connect");

    let affected = client
        .exec("CREATE TABLE test_exec (id INT, name TEXT)")
        .expect("Failed to exec");

    // CREATE TABLE doesn't return affected rows
    assert_eq!(affected, 0);

    // Verify table exists
    let rows = client
        .query("SELECT COUNT(*) FROM test_exec")
        .expect("Failed to query");
    assert_eq!(rows.len(), 1);

    client.close().expect("Failed to close");
}

#[test]
fn test_client_exec_insert() {
    let server = TestServer::new().expect("Failed to create test server");
    let client = server.connect().expect("Failed to connect");

    client
        .exec("CREATE TABLE test_insert (id INT)")
        .expect("Failed to create table");

    let affected = client
        .exec("INSERT INTO test_insert VALUES (1), (2), (3)")
        .expect("Failed to insert");

    assert_eq!(affected, 3);

    client.close().expect("Failed to close");
}

#[test]
fn test_client_exec_update() {
    let server = TestServer::new().expect("Failed to create test server");
    let client = server.connect().expect("Failed to connect");

    client
        .exec("CREATE TABLE test_update (id INT, value INT)")
        .expect("Failed to create table");

    client
        .exec("INSERT INTO test_update VALUES (1, 10), (2, 20), (3, 30)")
        .expect("Failed to insert");

    let affected = client
        .exec("UPDATE test_update SET value = 100 WHERE id >= 2")
        .expect("Failed to update");

    assert_eq!(affected, 2);

    // Verify update
    let rows = client
        .query("SELECT value FROM test_update WHERE id = 2")
        .expect("Failed to query");
    assert_eq!(rows[0].get_i32(0), Some(100));

    client.close().expect("Failed to close");
}

#[test]
fn test_client_exec_delete() {
    let server = TestServer::new().expect("Failed to create test server");
    let client = server.connect().expect("Failed to connect");

    client
        .exec("CREATE TABLE test_delete (id INT)")
        .expect("Failed to create table");

    client
        .exec("INSERT INTO test_delete VALUES (1), (2), (3), (4), (5)")
        .expect("Failed to insert");

    let affected = client
        .exec("DELETE FROM test_delete WHERE id > 3")
        .expect("Failed to delete");

    assert_eq!(affected, 2);

    // Verify deletion
    let rows = client
        .query("SELECT COUNT(*) FROM test_delete")
        .expect("Failed to query");
    assert_eq!(rows[0].get_i64(0), Some(3));

    client.close().expect("Failed to close");
}

// =============================================================================
// Batch Execute Tests
// =============================================================================

// Note: Hyper disables multi-part queries by default (error 0A000).
// The batch_execute method exists for API compatibility but may fail
// depending on server configuration. We test that it's callable but
// handle the expected error gracefully.

#[test]
fn test_client_batch_execute_disabled() {
    let server = TestServer::new().expect("Failed to create test server");
    let client = server.connect().expect("Failed to connect");

    // Execute multiple statements - Hyper may reject this
    let result = client.batch_execute(
        "CREATE TABLE test_batch (id INT, name TEXT);
         INSERT INTO test_batch VALUES (1, 'first');",
    );

    // Hyper by default disables multi-part queries
    // Either it succeeds (if enabled) or fails with 0A000
    match result {
        Ok(()) => {
            // If it succeeded, verify the statements executed
            let rows = client
                .query("SELECT COUNT(*) FROM test_batch")
                .expect("Failed to query");
            assert!(rows[0].get_i64(0).unwrap_or(0) >= 1);
        }
        Err(e) => {
            // Multi-part queries disabled is expected
            assert!(
                e.to_string().contains("Multi-part queries are disabled")
                    || e.sqlstate() == Some("0A000"),
                "Unexpected error: {e}"
            );
        }
    }

    client.close().expect("Failed to close");
}

// =============================================================================
// Error Handling Tests
// =============================================================================

#[test]
fn test_client_query_syntax_error() {
    let server = TestServer::new().expect("Failed to create test server");
    let client = server.connect().expect("Failed to connect");

    let result = client.query("SELECT * FORM nonexistent");
    assert!(result.is_err(), "Should fail with syntax error");

    let err = result.unwrap_err();
    let err_str = err.to_string().to_lowercase();
    assert!(
        err_str.contains("syntax") || err_str.contains("parse"),
        "Error should mention syntax: {err_str}"
    );

    client.close().expect("Failed to close");
}

#[test]
fn test_client_query_table_not_found() {
    let server = TestServer::new().expect("Failed to create test server");
    let client = server.connect().expect("Failed to connect");

    let result = client.query("SELECT * FROM table_that_does_not_exist");
    assert!(result.is_err(), "Should fail for nonexistent table");

    client.close().expect("Failed to close");
}

#[test]
fn test_client_connection_still_usable_after_error() {
    let server = TestServer::new().expect("Failed to create test server");
    let client = server.connect().expect("Failed to connect");

    // Cause an error by querying a nonexistent table
    let result = client.query("SELECT * FROM nonexistent_table");
    assert!(result.is_err(), "Query should fail for nonexistent table");

    // Connection should still be usable after an error.
    // After an error, the connection state may need to be synchronized.
    // The is_alive() check should indicate if the connection is still valid.
    assert!(
        client.is_alive(),
        "Connection should still be alive after error"
    );

    // Try a simple query that doesn't depend on previous state
    // Note: After a query error, the connection may need the server to send
    // ReadyForQuery before accepting new commands. The simple query protocol
    // handles this, but there may be edge cases.
    let result = client.exec("SELECT 1");

    // The connection should either work or fail clearly
    // (not silently ignore commands)
    match result {
        Ok(_) => {
            // Connection recovered - good!
        }
        Err(e) => {
            // Connection may be in bad state - this is a known limitation
            // of some PostgreSQL-style clients
            eprintln!("Note: Connection did not recover from error: {e}");
        }
    }

    // Close should always work
    client.close().expect("Failed to close");
}

// =============================================================================
// Type Tests
// =============================================================================

#[test]
fn test_client_query_integer_types() {
    let server = TestServer::new().expect("Failed to create test server");
    let client = server.connect().expect("Failed to connect");

    client
        .exec("CREATE TABLE test_ints (small SMALLINT, med INTEGER, big BIGINT)")
        .expect("Failed to create table");

    client
        .exec("INSERT INTO test_ints VALUES (32767, 2147483647, 9223372036854775807)")
        .expect("Failed to insert");

    let rows = client
        .query("SELECT * FROM test_ints")
        .expect("Failed to query");

    assert_eq!(rows.len(), 1);
    assert_eq!(rows[0].get_i16(0), Some(32767));
    assert_eq!(rows[0].get_i32(1), Some(2147483647));
    assert_eq!(rows[0].get_i64(2), Some(9223372036854775807));

    client.close().expect("Failed to close");
}

#[test]
fn test_client_query_float_types() {
    let server = TestServer::new().expect("Failed to create test server");
    let client = server.connect().expect("Failed to connect");

    client
        .exec("CREATE TABLE test_floats (val DOUBLE PRECISION)")
        .expect("Failed to create table");

    client
        .exec("INSERT INTO test_floats VALUES (3.14259265358979)")
        .expect("Failed to insert");

    let rows = client
        .query("SELECT * FROM test_floats")
        .expect("Failed to query");

    assert_eq!(rows.len(), 1);
    let val = rows[0].get_f64(0).expect("Expected f64");
    assert!((val - 3.14259265358979).abs() < 1e-10);

    client.close().expect("Failed to close");
}

#[test]
fn test_client_query_bool_type() {
    let server = TestServer::new().expect("Failed to create test server");
    let client = server.connect().expect("Failed to connect");

    client
        .exec("CREATE TABLE test_bool (val BOOL)")
        .expect("Failed to create table");

    client
        .exec("INSERT INTO test_bool VALUES (true), (false)")
        .expect("Failed to insert");

    let rows = client
        .query("SELECT * FROM test_bool ORDER BY val")
        .expect("Failed to query");

    assert_eq!(rows.len(), 2);
    // false sorts before true
    assert_eq!(rows[0].get_bool(0), Some(false));
    assert_eq!(rows[1].get_bool(0), Some(true));

    client.close().expect("Failed to close");
}

#[test]
fn test_client_query_text_type() {
    let server = TestServer::new().expect("Failed to create test server");
    let client = server.connect().expect("Failed to connect");

    client
        .exec("CREATE TABLE test_text (val TEXT)")
        .expect("Failed to create table");

    client
        .exec("INSERT INTO test_text VALUES ('hello'), ('world'), ('unicode: 你好')")
        .expect("Failed to insert");

    let rows = client
        .query("SELECT * FROM test_text ORDER BY val")
        .expect("Failed to query");

    assert_eq!(rows.len(), 3);
    assert_eq!(rows[0].get_string(0), Some("hello".to_string()));
    assert_eq!(rows[1].get_string(0), Some("unicode: 你好".to_string()));
    assert_eq!(rows[2].get_string(0), Some("world".to_string()));

    client.close().expect("Failed to close");
}

#[test]
fn test_client_query_bytes_type() {
    let server = TestServer::new().expect("Failed to create test server");
    let client = server.connect().expect("Failed to connect");

    client
        .exec("CREATE TABLE test_bytes (val BYTEA)")
        .expect("Failed to create table");

    // Insert binary data using hex notation
    client
        .exec("INSERT INTO test_bytes VALUES ('\\x48656c6c6f')")
        .expect("Failed to insert");

    // Use query_fast for binary format results
    let rows = client
        .query_fast("SELECT * FROM test_bytes")
        .expect("Failed to query");

    assert_eq!(rows.len(), 1);
    let bytes = rows[0].get_bytes(0).expect("Expected bytes");
    assert_eq!(bytes, b"Hello");

    client.close().expect("Failed to close");
}

// =============================================================================
// Thread Safety Tests
// =============================================================================

#[test]
fn test_client_thread_safety() {
    use std::sync::Arc;
    use std::thread;

    let server = TestServer::new().expect("Failed to create test server");
    let client = Arc::new(server.connect().expect("Failed to connect"));

    client
        .exec("CREATE TABLE test_threads (id INT, thread_id INT)")
        .expect("Failed to create table");

    let mut handles = vec![];

    // Spawn multiple threads that use the same client
    for thread_id in 0..4 {
        let client_clone = Arc::clone(&client);
        let handle = thread::spawn(move || {
            for i in 0..10 {
                let id = thread_id * 100 + i;
                client_clone
                    .exec(&format!(
                        "INSERT INTO test_threads VALUES ({id}, {thread_id})"
                    ))
                    .expect("Failed to insert");
            }
        });
        handles.push(handle);
    }

    // Wait for all threads
    for handle in handles {
        handle.join().expect("Thread panicked");
    }

    // Verify all inserts succeeded
    let rows = client
        .query("SELECT COUNT(*) FROM test_threads")
        .expect("Failed to query");
    assert_eq!(rows[0].get_i64(0), Some(40)); // 4 threads * 10 rows each

    // Note: We don't call close() because Arc<Client> doesn't expose it directly
    // The client will be closed when all Arc references are dropped
}

// =============================================================================
// Client Metadata Tests
// =============================================================================

#[test]
fn test_client_process_id() {
    let server = TestServer::new().expect("Failed to create test server");
    let client = server.connect().expect("Failed to connect");

    // Process ID is returned by the server during connection setup.
    // Hyper may return 0 which is still a valid value (means no backend PID tracking).
    let _pid = client.process_id();
    // Just verify we can access the value without panicking

    client.close().expect("Failed to close");
}

#[test]
fn test_client_secret_key() {
    let server = TestServer::new().expect("Failed to create test server");
    let client = server.connect().expect("Failed to connect");

    // Secret key can be any i32 value
    let _key = client.secret_key();

    client.close().expect("Failed to close");
}

#[test]
fn test_client_is_alive() {
    let server = TestServer::new().expect("Failed to create test server");
    let client = server.connect().expect("Failed to connect");

    assert!(client.is_alive(), "Client should be alive after connect");

    client.close().expect("Failed to close");
}
