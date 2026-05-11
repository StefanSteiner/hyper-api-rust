// Copyright (c) 2026, Salesforce, Inc. All rights reserved.
// SPDX-License-Identifier: Apache-2.0 OR MIT

//! Tests for type handling and conversions.

mod common;
use common::TestConnection;

#[test]
fn test_bool_type_handling() {
    let test = TestConnection::new().expect("Failed to create test connection");

    let catalog = test.catalog();
    let table_def = hyperdb_api::TableDefinition::new("test_table")
        .add_required_column("id", hyperdb_api::SqlType::int())
        .add_nullable_column("name", hyperdb_api::SqlType::text());
    catalog
        .create_table(&table_def)
        .expect("Failed to create table");

    // Query pg_catalog to check column metadata
    let query = r"SELECT a.attname, t.typname, NOT a.attnotnull as is_nullable
                   FROM pg_catalog.pg_attribute a
                   JOIN pg_catalog.pg_type t ON a.atttypid = t.oid
                   JOIN pg_catalog.pg_class c ON a.attrelid = c.oid
                   JOIN pg_catalog.pg_namespace n ON c.relnamespace = n.oid
                   WHERE n.nspname = 'public' AND c.relname = 'test_table'
                     AND a.attnum > 0 AND NOT a.attisdropped
                   ORDER BY a.attnum";

    let mut result = test.execute_query(query).expect("Failed to execute query");

    let mut rows = Vec::new();
    while let Some(chunk) = result.next_chunk().expect("Failed to get chunk") {
        for row in &chunk {
            let name = row.get::<String>(0).unwrap_or_default();
            let typename = row.get::<String>(1).unwrap_or_default();
            let is_nullable_bool = row.get::<bool>(2);
            rows.push((name, typename, is_nullable_bool));
        }
    }

    // Verify we got the expected columns
    assert_eq!(rows.len(), 2);

    // First column: id (int, not nullable)
    assert_eq!(rows[0].0, "id");
    // Type name can be "int4" or "integer" depending on Hyper version
    assert!(
        rows[0].1 == "int4" || rows[0].1 == "integer",
        "Unexpected type name: {}",
        rows[0].1
    );
    // is_nullable should be false (not nullable) - boolean conversion should work
    assert_eq!(rows[0].2, Some(false));

    // Second column: name (text, nullable)
    assert_eq!(rows[1].0, "name");
    assert_eq!(rows[1].1, "text");
    // is_nullable should be true (nullable) - boolean conversion should work
    assert_eq!(rows[1].2, Some(true));
}

#[test]
fn test_i64_type_handling() {
    let test = TestConnection::new().expect("Failed to create test connection");

    // Test SELECT 42 - should be readable as both i32 and i64
    let mut result = test
        .execute_query("SELECT 42")
        .expect("Failed to execute query");

    if let Some(chunk) = result.next_chunk().expect("Failed to get chunk") {
        assert_eq!(chunk.len(), 1);
        if let Some(row) = chunk.first() {
            // Should be readable as i32
            let i32_val = row.get_i32(0);
            assert_eq!(i32_val, Some(42));

            // Should be readable as i64 (via fallback from i32)
            let i64_val = row.get::<i64>(0);
            assert_eq!(i64_val, Some(42), "get::<i64> should work via fallback");

            // get_i64 might return None for integer literals (if type is int4 not int8)
            // but get::<i64> should work via fallback
            let i64_direct = row.get_i64(0);
            // Either get_i64 works directly, or get::<i64> works via fallback
            assert!(
                i64_direct == Some(42) || i64_val == Some(42),
                "i64 should be readable either directly or via fallback"
            );

            // Should be readable as String (though format may vary)
            let str_val = row.get::<String>(0);
            assert!(str_val.is_some(), "Should be readable as String");
            // The string might be "42" or binary representation, but should not be empty
            assert!(!str_val.unwrap().is_empty());
        }
    }

    // Test COUNT(*) - should return i64
    let catalog = test.catalog();
    let table_def = hyperdb_api::TableDefinition::new("test")
        .add_nullable_column("id", hyperdb_api::SqlType::int());
    catalog
        .create_table(&table_def)
        .expect("Failed to create table");

    test.execute_command("INSERT INTO test VALUES (1), (2), (3)")
        .expect("Failed to insert data");

    let mut result = test
        .execute_query("SELECT COUNT(*) FROM test")
        .expect("Failed to execute query");

    if let Some(chunk) = result.next_chunk().expect("Failed to get chunk") {
        assert_eq!(chunk.len(), 1);
        if let Some(row) = chunk.first() {
            // COUNT(*) should return i64 (bigint)
            let i64_val = row.get_i64(0);
            assert_eq!(i64_val, Some(3), "COUNT(*) should return i64");

            // Should be readable as i64 via generic get (which handles fallback)
            let i64_via_get = row.get::<i64>(0);
            assert_eq!(i64_via_get, Some(3), "get::<i64> should work");

            // COUNT(*) returns bigint, so get_i32 might not work directly
            // But get::<i64> should work, which is what matters for i64 type handling

            // Should be readable as String (format may vary - binary or text)
            let str_val = row.get::<String>(0);
            assert!(str_val.is_some(), "Should be readable as String");
        }
    }
}
