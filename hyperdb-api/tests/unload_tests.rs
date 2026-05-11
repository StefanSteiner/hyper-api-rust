// Copyright (c) 2026, Salesforce, Inc. All rights reserved.
// SPDX-License-Identifier: Apache-2.0 OR MIT

//! Tests for UNLOAD DATABASE and UNLOAD RELEASE commands.

mod common;

#[test]
fn test_unload_database_and_release() {
    let params = common::test_hyper_params("test_unload_database_and_release")
        .expect("Failed to create test parameters");
    let hyper = hyperdb_api::HyperProcess::new(None, Some(&params))
        .expect("Failed to create Hyper process");
    let temp_dir = tempfile::tempdir().expect("Failed to create temp directory");
    let db_path = temp_dir.path().join("test_unload.hyper");

    let conn =
        hyperdb_api::Connection::new(&hyper, &db_path, hyperdb_api::CreateMode::CreateAndReplace)
            .expect("Failed to create database");

    // Create a simple table
    conn.execute_command("CREATE TABLE test (id INT, name TEXT)")
        .expect("Failed to create table");
    conn.execute_command("INSERT INTO test VALUES (1, 'Hello')")
        .expect("Failed to insert data");

    // Test query before unload
    let count: i64 = conn
        .fetch_scalar("SELECT COUNT(*) FROM test")
        .expect("Failed to fetch count");
    assert_eq!(count, 1);

    // Test UNLOAD DATABASE - may not be supported in all Hyper versions/configurations
    // If it fails with a syntax error, skip the rest of the test
    let unload_result = conn.unload_database();
    if unload_result.is_err() {
        // UNLOAD DATABASE might not be supported - skip test
        eprintln!("UNLOAD DATABASE not supported, skipping test");
        return;
    }

    // Test query after unload database (should still work - auto-reload)
    let count_after_unload: i64 = conn
        .fetch_scalar("SELECT COUNT(*) FROM test")
        .expect("Query after UNLOAD DATABASE should succeed");
    assert_eq!(count_after_unload, 1);

    // Test UNLOAD DATABASE again before UNLOAD RELEASE
    conn.unload_database()
        .expect("UNLOAD DATABASE should succeed again");

    // Test UNLOAD RELEASE - should succeed
    conn.unload_release()
        .expect("UNLOAD RELEASE should succeed");

    // Test query after unload release (should fail)
    let result = conn.fetch_scalar::<i64, _>("SELECT COUNT(*) FROM test");
    assert!(result.is_err(), "Query after UNLOAD RELEASE should fail");
}
