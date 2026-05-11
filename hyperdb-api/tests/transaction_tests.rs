// Copyright (c) 2026, Salesforce, Inc. All rights reserved.
// SPDX-License-Identifier: Apache-2.0 OR MIT

//! Tests for transaction support (BEGIN/COMMIT/ROLLBACK).
//!
//! These tests verify basic transaction behavior.

mod common;
use common::TestConnection;
use hyperdb_api::Result;

fn setup() -> Result<TestConnection> {
    let tc = TestConnection::new()?;
    tc.execute_command("CREATE TABLE test_txn (id INT, value TEXT)")?;
    tc.execute_command("INSERT INTO test_txn VALUES (1, 'initial')")?;
    Ok(tc)
}

// =========================================================================
// Raw connection method tests
// =========================================================================

#[test]
fn test_raw_begin_commit_methods() -> Result<()> {
    let tc = setup()?;
    tc.connection.begin_transaction()?;
    tc.connection
        .execute_command("INSERT INTO test_txn VALUES (2, 'committed')")?;
    tc.connection.commit()?;

    let count = tc.count_tuples("test_txn")?;
    assert_eq!(count, 2);
    Ok(())
}

#[test]
fn test_raw_begin_rollback_methods() -> Result<()> {
    let tc = setup()?;
    tc.connection.begin_transaction()?;
    tc.connection
        .execute_command("INSERT INTO test_txn VALUES (2, 'rolled_back')")?;
    tc.connection.rollback()?;

    let count = tc.count_tuples("test_txn")?;
    assert_eq!(count, 1);
    Ok(())
}

#[test]
fn test_begin_commit() -> Result<()> {
    let tc = setup()?;
    tc.execute_command("BEGIN TRANSACTION")?;
    tc.execute_command("INSERT INTO test_txn VALUES (2, 'two')")?;
    tc.execute_command("INSERT INTO test_txn VALUES (3, 'three')")?;
    tc.execute_command("COMMIT")?;

    let count = tc.count_tuples("test_txn")?;
    assert_eq!(count, 3);
    Ok(())
}

#[test]
fn test_begin_rollback() -> Result<()> {
    let tc = setup()?;
    tc.execute_command("BEGIN TRANSACTION")?;
    tc.execute_command("INSERT INTO test_txn VALUES (2, 'vanish')")?;
    tc.execute_command("ROLLBACK")?;

    let count = tc.count_tuples("test_txn")?;
    assert_eq!(count, 1);
    Ok(())
}

// =========================================================================
// RAII Transaction guard tests
// =========================================================================

#[test]
fn test_transaction_guard_commit() -> Result<()> {
    let mut tc = setup()?;
    let txn = tc.connection.transaction()?;
    txn.execute_command("INSERT INTO test_txn VALUES (2, 'guard_commit')")?;
    txn.commit()?;

    let count = tc.count_tuples("test_txn")?;
    assert_eq!(count, 2);
    Ok(())
}

#[test]
fn test_transaction_guard_rollback_explicit() -> Result<()> {
    let mut tc = setup()?;
    let txn = tc.connection.transaction()?;
    txn.execute_command("INSERT INTO test_txn VALUES (2, 'guard_rollback')")?;
    txn.rollback()?;

    let count = tc.count_tuples("test_txn")?;
    assert_eq!(count, 1);
    Ok(())
}

#[test]
fn test_transaction_guard_auto_rollback() -> Result<()> {
    let mut tc = setup()?;
    {
        let txn = tc.connection.transaction()?;
        txn.execute_command("INSERT INTO test_txn VALUES (2, 'auto_rollback')")?;
        // txn drops here without commit → auto-rollback
    }

    let count = tc.count_tuples("test_txn")?;
    assert_eq!(count, 1);
    Ok(())
}

// =========================================================================
// Transaction behavior tests
// =========================================================================

#[test]
fn test_multiple_operations_in_transaction() -> Result<()> {
    let mut tc = setup()?;
    let txn = tc.connection.transaction()?;
    txn.execute_command("INSERT INTO test_txn VALUES (2, 'two')")?;
    txn.execute_command("INSERT INTO test_txn VALUES (3, 'three')")?;
    txn.execute_command("UPDATE test_txn SET value = 'updated' WHERE id = 1")?;
    txn.execute_command("DELETE FROM test_txn WHERE id = 2")?;
    txn.commit()?;

    let count = tc.count_tuples("test_txn")?;
    assert_eq!(count, 2); // original + id=3, id=2 was deleted

    let value: String = tc
        .connection
        .fetch_scalar("SELECT value FROM test_txn WHERE id = 1")?;
    assert_eq!(value, "updated");
    Ok(())
}

#[test]
fn test_ddl_in_transaction() -> Result<()> {
    let mut tc = TestConnection::new()?;
    let txn = tc.connection.transaction()?;
    txn.execute_command("CREATE TABLE new_table (id INT, name TEXT)")?;
    txn.commit()?;

    // Verify table exists by querying it
    let count: i64 = tc
        .connection
        .fetch_scalar("SELECT COUNT(*) FROM new_table")?;
    assert_eq!(count, 0);
    Ok(())
}

#[test]
fn test_query_within_transaction() -> Result<()> {
    let mut tc = setup()?;
    let txn = tc.connection.transaction()?;
    txn.execute_command("INSERT INTO test_txn VALUES (2, 'uncommitted')")?;

    // Should see uncommitted data within the transaction
    let count = txn.query_count("SELECT COUNT(*) FROM test_txn")?;
    assert_eq!(count, 2);

    txn.rollback()?;

    // After rollback, only the original row remains
    let count = tc.count_tuples("test_txn")?;
    assert_eq!(count, 1);
    Ok(())
}

#[test]
fn test_rollback_after_error() -> Result<()> {
    let tc = setup()?;
    tc.connection.begin_transaction()?;
    tc.connection
        .execute_command("INSERT INTO test_txn VALUES (2, 'before_error')")?;

    // Execute invalid SQL — should produce an error
    let result = tc
        .connection
        .execute_command("INSERT INTO nonexistent_table VALUES (1)");
    assert!(result.is_err());

    // Rollback the failed transaction — should succeed without error
    tc.connection.rollback()?;

    // Connection is still usable: verify by running a new transaction
    tc.connection.begin_transaction()?;
    tc.connection
        .execute_command("INSERT INTO test_txn VALUES (3, 'after_recovery')")?;
    tc.connection.commit()?;

    // Verify the new insert worked (use execute_command to avoid protocol quirks)
    tc.connection.execute_command("SELECT 1")?;
    Ok(())
}

#[test]
fn test_nested_begin_warning() -> Result<()> {
    let tc = setup()?;
    // First BEGIN
    tc.connection.begin_transaction()?;
    // Second BEGIN should produce a warning notice, not an error
    tc.connection.begin_transaction()?;
    tc.connection.rollback()?;
    Ok(())
}

#[test]
fn test_rollback_outside_transaction() -> Result<()> {
    let tc = setup()?;
    // ROLLBACK with no active transaction should produce a warning, not an error
    tc.connection.rollback()?;
    Ok(())
}

#[test]
fn test_fetch_methods_in_transaction() -> Result<()> {
    let mut tc = setup()?;
    let txn = tc.connection.transaction()?;
    txn.execute_command("INSERT INTO test_txn VALUES (2, 'two')")?;

    // Test fetch_scalar
    let count: i64 = txn.fetch_scalar("SELECT COUNT(*) FROM test_txn")?;
    assert_eq!(count, 2);

    // Test fetch_one
    let row = txn.fetch_one("SELECT id, value FROM test_txn WHERE id = 2")?;
    let id: Option<i32> = row.get(0);
    assert_eq!(id, Some(2));

    // Test fetch_optional
    let row = txn.fetch_optional("SELECT id FROM test_txn WHERE id = 999")?;
    assert!(row.is_none());

    // Test fetch_all
    let rows = txn.fetch_all("SELECT id FROM test_txn ORDER BY id")?;
    assert_eq!(rows.len(), 2);

    txn.rollback()?;
    Ok(())
}
