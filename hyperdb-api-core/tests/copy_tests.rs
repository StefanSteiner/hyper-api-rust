// Copyright (c) 2026, Salesforce, Inc. All rights reserved.
// SPDX-License-Identifier: Apache-2.0 OR MIT

//! Integration tests for the COPY protocol in hyper-client.
//!
//! These tests verify that the low-level COPY API works correctly
//! against a real Hyper server using `HyperBinary` format.

// Test harness: fixed small row counts and indices narrow by construction.
#![expect(
    clippy::cast_possible_truncation,
    clippy::cast_possible_wrap,
    clippy::cast_sign_loss,
    reason = "test harness: row counts and loop indices narrow by test-enforced invariants"
)]

mod common;
use bytes::BytesMut;
use common::TestServer;
use hyperdb_api_core::protocol::copy;

// =============================================================================
// COPY IN Tests (Client -> Server)
// =============================================================================

#[test]
fn test_copy_in_basic() {
    let server = TestServer::new().expect("Failed to create test server");
    let client = server.connect().expect("Failed to connect");

    // Create a simple table with NOT NULL columns
    client
        .exec("CREATE TABLE test_copy (id INT NOT NULL, name TEXT NOT NULL)")
        .expect("Failed to create table");

    // Build HyperBinary data
    let mut buf = BytesMut::new();
    copy::write_header(&mut buf);

    // For NOT NULL columns, use write_*_not_null (no null indicator)
    // Row 1: id=1, name="Alice"
    copy::write_i32_not_null(&mut buf, 1);
    copy::write_varbinary_not_null(&mut buf, b"Alice");

    // Row 2: id=2, name="Bob"
    copy::write_i32_not_null(&mut buf, 2);
    copy::write_varbinary_not_null(&mut buf, b"Bob");

    // Row 3: id=3, name="Charlie"
    copy::write_i32_not_null(&mut buf, 3);
    copy::write_varbinary_not_null(&mut buf, b"Charlie");

    // Start COPY and send data
    let mut writer = client
        .copy_in("test_copy", &["id", "name"])
        .expect("Failed to start COPY");

    writer.send(&buf).expect("Failed to send data");
    let rows_inserted = writer.finish().expect("Failed to finish COPY");

    assert_eq!(rows_inserted, 3);

    // Verify data was inserted
    let rows = client
        .query("SELECT id, name FROM test_copy ORDER BY id")
        .expect("Failed to query");

    assert_eq!(rows.len(), 3);
    assert_eq!(rows[0].get_i32(0), Some(1));
    assert_eq!(rows[0].get_string(1), Some("Alice".to_string()));
    assert_eq!(rows[1].get_i32(0), Some(2));
    assert_eq!(rows[1].get_string(1), Some("Bob".to_string()));
    assert_eq!(rows[2].get_i32(0), Some(3));
    assert_eq!(rows[2].get_string(1), Some("Charlie".to_string()));

    client.close().expect("Failed to close");
}

#[test]
fn test_copy_in_with_nulls() {
    let server = TestServer::new().expect("Failed to create test server");
    let client = server.connect().expect("Failed to connect");

    // Create a table with nullable column
    client
        .exec("CREATE TABLE test_copy_nulls (id INT NOT NULL, value TEXT)")
        .expect("Failed to create table");

    // Build HyperBinary data with NULLs
    let mut buf = BytesMut::new();
    copy::write_header(&mut buf);

    // Row 1: id=1, value="hello"
    // - id is NOT NULL, so use write_i32_not_null
    // - value is NULLABLE, so use write_varbinary (includes null indicator)
    copy::write_i32_not_null(&mut buf, 1);
    copy::write_varbinary(&mut buf, b"hello"); // nullable column with value

    // Row 2: id=2, value=NULL
    copy::write_i32_not_null(&mut buf, 2);
    copy::write_null(&mut buf); // NULL value for nullable column

    // Row 3: id=3, value="world"
    copy::write_i32_not_null(&mut buf, 3);
    copy::write_varbinary(&mut buf, b"world");

    let mut writer = client
        .copy_in("test_copy_nulls", &["id", "value"])
        .expect("Failed to start COPY");

    writer.send(&buf).expect("Failed to send data");
    let rows_inserted = writer.finish().expect("Failed to finish COPY");

    assert_eq!(rows_inserted, 3);

    // Verify NULLs
    let rows = client
        .query("SELECT id, value FROM test_copy_nulls ORDER BY id")
        .expect("Failed to query");

    assert_eq!(rows.len(), 3);
    assert_eq!(rows[0].get_string(1), Some("hello".to_string()));
    assert_eq!(rows[1].get_string(1), None); // NULL
    assert_eq!(rows[2].get_string(1), Some("world".to_string()));

    client.close().expect("Failed to close");
}

#[test]
fn test_copy_in_multiple_chunks() {
    let server = TestServer::new().expect("Failed to create test server");
    let client = server.connect().expect("Failed to connect");

    client
        .exec("CREATE TABLE test_copy_chunks (id INT NOT NULL)")
        .expect("Failed to create table");

    let mut writer = client
        .copy_in("test_copy_chunks", &["id"])
        .expect("Failed to start COPY");

    // Send header in first chunk
    let mut header_buf = BytesMut::new();
    copy::write_header(&mut header_buf);
    writer.send(&header_buf).expect("Failed to send header");

    // Send data in multiple chunks (NOT NULL column, use _not_null variant)
    for chunk_start in (0..100).step_by(10) {
        let mut buf = BytesMut::new();
        for i in chunk_start..chunk_start + 10 {
            copy::write_i32_not_null(&mut buf, i);
        }
        writer.send(&buf).expect("Failed to send chunk");
    }

    let rows_inserted = writer.finish().expect("Failed to finish COPY");
    assert_eq!(rows_inserted, 100);

    // Verify count
    let rows = client
        .query("SELECT COUNT(*) FROM test_copy_chunks")
        .expect("Failed to query");
    assert_eq!(rows[0].get_i64(0), Some(100));

    client.close().expect("Failed to close");
}

#[test]
fn test_copy_in_cancel() {
    let server = TestServer::new().expect("Failed to create test server");
    let client = server.connect().expect("Failed to connect");

    client
        .exec("CREATE TABLE test_copy_cancel (id INT NOT NULL)")
        .expect("Failed to create table");

    // Build some data
    let mut buf = BytesMut::new();
    copy::write_header(&mut buf);
    copy::write_i32(&mut buf, 1);
    copy::write_i32(&mut buf, 2);

    let mut writer = client
        .copy_in("test_copy_cancel", &["id"])
        .expect("Failed to start COPY");

    writer.send(&buf).expect("Failed to send data");

    // Cancel the COPY operation
    writer
        .cancel("Test cancellation")
        .expect("Failed to cancel");

    // Verify no data was inserted
    let rows = client
        .query("SELECT COUNT(*) FROM test_copy_cancel")
        .expect("Failed to query");
    assert_eq!(rows[0].get_i64(0), Some(0));

    client.close().expect("Failed to close");
}

#[test]
fn test_copy_in_large_batch() {
    let server = TestServer::new().expect("Failed to create test server");
    let client = server.connect().expect("Failed to connect");

    client
        .exec("CREATE TABLE test_copy_large (id INT NOT NULL, value DOUBLE PRECISION NOT NULL)")
        .expect("Failed to create table");

    // Build a large batch (NOT NULL columns, use _not_null variants)
    let mut buf = BytesMut::new();
    copy::write_header(&mut buf);

    let row_count = 10000;
    for i in 0..row_count {
        copy::write_i32_not_null(&mut buf, i);
        copy::write_f64_not_null(&mut buf, f64::from(i) * 0.1);
    }

    let mut writer = client
        .copy_in("test_copy_large", &["id", "value"])
        .expect("Failed to start COPY");

    writer.send(&buf).expect("Failed to send data");
    let rows_inserted = writer.finish().expect("Failed to finish COPY");

    assert_eq!(rows_inserted, row_count as u64);

    // Verify count and sum
    let rows = client
        .query("SELECT COUNT(*), SUM(id) FROM test_copy_large")
        .expect("Failed to query");
    assert_eq!(rows[0].get_i64(0), Some(i64::from(row_count)));

    // Sum of 0..10000 = (n-1)*n/2
    let expected_sum = i64::from((row_count - 1) * row_count / 2);
    assert_eq!(rows[0].get_i64(1), Some(expected_sum));

    client.close().expect("Failed to close");
}

// =============================================================================
// COPY OUT Tests (Server -> Client)
// =============================================================================

#[test]
fn test_copy_out_basic() {
    let server = TestServer::new().expect("Failed to create test server");
    let client = server.connect().expect("Failed to connect");

    // Create and populate a table
    client
        .exec("CREATE TABLE test_copy_out (id INT, name TEXT)")
        .expect("Failed to create table");

    client
        .exec("INSERT INTO test_copy_out VALUES (1, 'Alice'), (2, 'Bob')")
        .expect("Failed to insert");

    // Use COPY ... TO STDOUT with Arrow format
    let arrow_data = client
        .copy_out(
            "COPY (SELECT * FROM test_copy_out ORDER BY id) TO STDOUT WITH (format arrowstream)",
        )
        .expect("Failed to copy out");

    // Arrow IPC stream should start with a schema message
    // The magic bytes for Arrow IPC are 0xFFFFFFFF followed by metadata
    assert!(
        arrow_data.len() > 8,
        "Arrow data should have at least a header"
    );

    // Verify it looks like Arrow IPC (continuation indicator)
    // Arrow IPC streams start with a schema message which has a continuation indicator
    assert_eq!(
        &arrow_data[0..4],
        &[0xFF, 0xFF, 0xFF, 0xFF],
        "Should start with Arrow continuation indicator"
    );

    client.close().expect("Failed to close");
}

// =============================================================================
// COPY with Different Data Types
// =============================================================================

#[test]
fn test_copy_in_various_types() {
    let server = TestServer::new().expect("Failed to create test server");
    let client = server.connect().expect("Failed to connect");

    client
        .exec(
            "CREATE TABLE test_copy_types (
                small SMALLINT NOT NULL,
                med INTEGER NOT NULL,
                big BIGINT NOT NULL,
                dbl DOUBLE PRECISION NOT NULL,
                flag BOOL NOT NULL
            )",
        )
        .expect("Failed to create table");

    let mut buf = BytesMut::new();
    copy::write_header(&mut buf);

    // Write a row with all types (all NOT NULL, use _not_null variants)
    copy::write_i16_not_null(&mut buf, 123);
    copy::write_i32_not_null(&mut buf, 456789);
    copy::write_i64_not_null(&mut buf, 9876543210);
    copy::write_f64_not_null(&mut buf, 3.15);
    // Bool is stored as i8: 1 for true, 0 for false
    copy::write_i8_not_null(&mut buf, 1); // true

    // Write another row
    copy::write_i16_not_null(&mut buf, -100);
    copy::write_i32_not_null(&mut buf, -200);
    copy::write_i64_not_null(&mut buf, -300);
    copy::write_f64_not_null(&mut buf, -2.72);
    copy::write_i8_not_null(&mut buf, 0); // false

    let mut writer = client
        .copy_in("test_copy_types", &["small", "med", "big", "dbl", "flag"])
        .expect("Failed to start COPY");

    writer.send(&buf).expect("Failed to send data");
    let rows_inserted = writer.finish().expect("Failed to finish COPY");

    assert_eq!(rows_inserted, 2);

    // Verify data
    let rows = client
        .query("SELECT * FROM test_copy_types ORDER BY small DESC")
        .expect("Failed to query");

    assert_eq!(rows.len(), 2);

    // First row (small=123)
    assert_eq!(rows[0].get_i16(0), Some(123));
    assert_eq!(rows[0].get_i32(1), Some(456789));
    assert_eq!(rows[0].get_i64(2), Some(9876543210));
    let dbl = rows[0].get_f64(3).expect("Expected f64");
    assert!((dbl - 3.15).abs() < 1e-5);
    assert_eq!(rows[0].get_bool(4), Some(true));

    // Second row (small=-100)
    assert_eq!(rows[1].get_i16(0), Some(-100));
    assert_eq!(rows[1].get_i32(1), Some(-200));
    assert_eq!(rows[1].get_i64(2), Some(-300));
    let dbl2 = rows[1].get_f64(3).expect("Expected f64");
    assert!((dbl2 + 2.72).abs() < 1e-5);
    assert_eq!(rows[1].get_bool(4), Some(false));

    client.close().expect("Failed to close");
}

#[test]
fn test_copy_in_text_with_special_chars() {
    let server = TestServer::new().expect("Failed to create test server");
    let client = server.connect().expect("Failed to connect");

    client
        .exec("CREATE TABLE test_copy_text (id INT NOT NULL, text TEXT NOT NULL)")
        .expect("Failed to create table");

    let mut buf = BytesMut::new();
    copy::write_header(&mut buf);

    // Various text values with special characters (both columns NOT NULL)
    let texts = [
        "simple",
        "with spaces",
        "unicode: 你好世界",
        "emoji: 🚀🎉",
        "newline:\nhere",
        "tab:\there",
        "quote: \"hello\"",
        "backslash: \\path",
    ];

    for (i, text) in texts.iter().enumerate() {
        copy::write_i32_not_null(&mut buf, i as i32);
        copy::write_varbinary_not_null(&mut buf, text.as_bytes());
    }

    let mut writer = client
        .copy_in("test_copy_text", &["id", "text"])
        .expect("Failed to start COPY");

    writer.send(&buf).expect("Failed to send data");
    let rows_inserted = writer.finish().expect("Failed to finish COPY");

    assert_eq!(rows_inserted, texts.len() as u64);

    // Verify all text values
    let rows = client
        .query("SELECT id, text FROM test_copy_text ORDER BY id")
        .expect("Failed to query");

    assert_eq!(rows.len(), texts.len());

    for (i, expected) in texts.iter().enumerate() {
        let actual = rows[i].get_string(1).expect("Expected text");
        assert_eq!(
            &actual, *expected,
            "Text mismatch at index {i}: expected {expected:?}, got {actual:?}"
        );
    }

    client.close().expect("Failed to close");
}
