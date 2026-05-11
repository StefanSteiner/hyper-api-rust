// Copyright (c) 2026, Salesforce, Inc. All rights reserved.
// SPDX-License-Identifier: Apache-2.0 OR MIT

//! Tests for CSV/text export and import via COPY protocol.
//!
//! Features tested:
//! - #2: CSV export via COPY OUT
//! - #5: Streaming COPY OUT (`copy_out_to_writer`)
//! - #6: CSV import via COPY IN

mod common;
use common::TestConnection;

use hyperdb_api::copy::CopyOptions;

// =============================================================================
// #2: CSV Export
// =============================================================================

#[test]
fn test_export_csv_basic() {
    let test = TestConnection::new().expect("Failed to create test connection");

    test.execute_command(
        "CREATE TABLE csv_export (id INT NOT NULL, name TEXT, value DOUBLE PRECISION)",
    )
    .expect("create");
    test.execute_command("INSERT INTO csv_export VALUES (1, 'Alice', 1.5), (2, 'Bob', 2.5)")
        .expect("insert");

    let csv = test
        .connection
        .export_csv_string("SELECT * FROM csv_export ORDER BY id")
        .expect("export");

    // Should have a header row + 2 data rows
    let lines: Vec<&str> = csv.trim().lines().collect();
    assert_eq!(
        lines.len(),
        3,
        "Expected header + 2 data rows, got: {lines:?}"
    );

    // Header
    assert!(
        lines[0].contains("id"),
        "Header should contain 'id': {}",
        lines[0]
    );
    assert!(
        lines[0].contains("name"),
        "Header should contain 'name': {}",
        lines[0]
    );

    // Data rows
    assert!(
        lines[1].contains("Alice"),
        "Row 1 should contain 'Alice': {}",
        lines[1]
    );
    assert!(
        lines[2].contains("Bob"),
        "Row 2 should contain 'Bob': {}",
        lines[2]
    );
}

#[test]
fn test_export_csv_to_writer() {
    let test = TestConnection::new().expect("Failed to create test connection");

    test.execute_command("CREATE TABLE csv_writer (id INT NOT NULL, name TEXT)")
        .expect("create");
    test.execute_command("INSERT INTO csv_writer VALUES (1, 'test')")
        .expect("insert");

    let mut buf = Vec::new();
    let bytes = test
        .connection
        .export_csv("SELECT * FROM csv_writer", &mut buf)
        .expect("export");

    assert!(bytes > 0, "Should write some bytes");
    let csv = String::from_utf8(buf).expect("valid utf8");
    assert!(csv.contains("test"), "Should contain data: {csv}");
}

#[test]
fn test_export_csv_empty_table() {
    let test = TestConnection::new().expect("Failed to create test connection");

    test.execute_command("CREATE TABLE csv_empty (id INT)")
        .expect("create");

    let csv = test
        .connection
        .export_csv_string("SELECT * FROM csv_empty")
        .expect("export");

    // Should have just the header row
    let lines: Vec<&str> = csv.trim().lines().collect();
    assert_eq!(lines.len(), 1, "Should only have header: {lines:?}");
}

#[test]
fn test_export_text_custom_options() {
    let test = TestConnection::new().expect("Failed to create test connection");

    test.execute_command("CREATE TABLE csv_custom (id INT NOT NULL, name TEXT)")
        .expect("create");
    test.execute_command("INSERT INTO csv_custom VALUES (1, 'Alice'), (2, 'Bob')")
        .expect("insert");

    // CSV with custom delimiter (pipe-separated, with header)
    let opts = CopyOptions::csv().with_delimiter(b'|').with_header(true);
    let mut buf = Vec::new();
    test.connection
        .export_text("SELECT * FROM csv_custom ORDER BY id", &opts, &mut buf)
        .expect("export pipe-delimited");

    let csv = String::from_utf8(buf).expect("valid utf8");
    assert!(
        csv.contains('|'),
        "Pipe-delimited should contain pipes: {csv:?}"
    );
    assert!(csv.contains("Alice"), "Should contain data: {csv:?}");
}

#[test]
fn test_export_csv_large() {
    let test = TestConnection::new().expect("Failed to create test connection");

    test.execute_command("CREATE TABLE csv_large (id INT NOT NULL)")
        .expect("create");
    test.execute_command("INSERT INTO csv_large SELECT * FROM GENERATE_SERIES(1, 10000)")
        .expect("insert");

    let mut buf = Vec::new();
    let bytes = test
        .connection
        .export_csv("SELECT * FROM csv_large", &mut buf)
        .expect("export");

    assert!(bytes > 0);
    let csv = String::from_utf8(buf).expect("valid utf8");
    let lines: Vec<&str> = csv.trim().lines().collect();
    // Header + 10000 data rows
    assert_eq!(lines.len(), 10001);
}

// =============================================================================
// #5: Streaming COPY OUT
// =============================================================================

#[test]
fn test_streaming_copy_out() {
    let test = TestConnection::new().expect("Failed to create test connection");

    test.execute_command("CREATE TABLE stream_out (id INT NOT NULL, data TEXT)")
        .expect("create");
    test.execute_command(
        "INSERT INTO stream_out SELECT i, 'row_' || i::TEXT FROM GENERATE_SERIES(1, 1000) AS s(i)",
    )
    .expect("insert");

    // Use export_csv which internally uses copy_out_to_writer (streaming)
    let mut buf = Vec::new();
    let bytes = test
        .connection
        .export_csv("SELECT * FROM stream_out ORDER BY id", &mut buf)
        .expect("streaming export");

    assert!(bytes > 0);
    let csv = String::from_utf8(buf).expect("valid utf8");
    let lines: Vec<&str> = csv.trim().lines().collect();
    assert_eq!(lines.len(), 1001, "Header + 1000 rows");
}

// =============================================================================
// #6: CSV Import
// =============================================================================

#[test]
fn test_import_csv_basic() {
    let test = TestConnection::new().expect("Failed to create test connection");

    test.execute_command("CREATE TABLE csv_import (id INT NOT NULL, name TEXT)")
        .expect("create");

    let csv_data = "1,Alice\n2,Bob\n3,Carol\n";
    let rows = test
        .connection
        .import_csv("csv_import", csv_data.as_bytes())
        .expect("import");

    assert_eq!(rows, 3);

    let count = test.count_tuples("csv_import").expect("count");
    assert_eq!(count, 3);

    // Verify data
    let row = test
        .connection
        .fetch_one("SELECT name FROM csv_import WHERE id = 2")
        .expect("fetch");
    assert_eq!(row.get::<String>(0), Some("Bob".to_string()));
}

#[test]
fn test_import_csv_with_header() {
    let test = TestConnection::new().expect("Failed to create test connection");

    test.execute_command("CREATE TABLE csv_header (id INT NOT NULL, name TEXT)")
        .expect("create");

    let csv_data = "id,name\n1,Alice\n2,Bob\n";
    let rows = test
        .connection
        .import_csv_with_header("csv_header", csv_data.as_bytes())
        .expect("import");

    assert_eq!(rows, 2);

    let count = test.count_tuples("csv_header").expect("count");
    assert_eq!(count, 2);
}

#[test]
fn test_import_text_tsv() {
    let test = TestConnection::new().expect("Failed to create test connection");

    test.execute_command("CREATE TABLE tsv_import (id INT NOT NULL, name TEXT)")
        .expect("create");

    let opts = CopyOptions::csv().with_delimiter(b'\t');
    let tsv_data = "1\tAlice\n2\tBob\n";
    let rows = test
        .connection
        .import_text("tsv_import", &opts, tsv_data.as_bytes())
        .expect("import");

    assert_eq!(rows, 2);
    let count = test.count_tuples("tsv_import").expect("count");
    assert_eq!(count, 2);
}

#[test]
fn test_csv_roundtrip() {
    let test = TestConnection::new().expect("Failed to create test connection");

    // Create and populate source table
    test.execute_command(
        "CREATE TABLE csv_src (id INT NOT NULL, name TEXT, value DOUBLE PRECISION)",
    )
    .expect("create src");
    test.execute_command(
        "INSERT INTO csv_src VALUES (1, 'Alice', 1.5), (2, 'Bob', 2.5), (3, 'Carol', 3.5)",
    )
    .expect("insert");

    // Export to CSV
    let csv = test
        .connection
        .export_csv_string("SELECT * FROM csv_src ORDER BY id")
        .expect("export");

    // Create destination table with same schema
    test.execute_command(
        "CREATE TABLE csv_dst (id INT NOT NULL, name TEXT, value DOUBLE PRECISION)",
    )
    .expect("create dst");

    // Import the CSV (with header)
    let rows = test
        .connection
        .import_csv_with_header("csv_dst", csv.as_bytes())
        .expect("import");

    assert_eq!(rows, 3);

    // Verify roundtrip preserved data
    let count = test.count_tuples("csv_dst").expect("count");
    assert_eq!(count, 3);

    let row = test
        .connection
        .fetch_one("SELECT name, value FROM csv_dst WHERE id = 1")
        .expect("fetch");
    assert_eq!(row.get::<String>(0), Some("Alice".to_string()));
}

#[test]
fn test_import_csv_empty() {
    let test = TestConnection::new().expect("Failed to create test connection");

    test.execute_command("CREATE TABLE csv_empty_import (id INT, name TEXT)")
        .expect("create");

    let csv_data = "";
    let rows = test
        .connection
        .import_csv("csv_empty_import", csv_data.as_bytes())
        .expect("import");

    assert_eq!(rows, 0);
}
