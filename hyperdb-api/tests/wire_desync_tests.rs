// Copyright (c) 2026, Salesforce, Inc. All rights reserved.
// SPDX-License-Identifier: Apache-2.0 OR MIT

//! Regression tests for connection resynchronization after server errors.
//!
//! When the server returns an `ErrorResponse`, the `PostgreSQL` wire protocol
//! still requires the client to consume the trailing `ReadyForQuery` before
//! the next query. If the client skips that read, the unread bytes remain
//! in the socket buffer and get misinterpreted as the start of the next
//! query's response — classic wire desync.
//!
//! These tests provoke a server-side error and then immediately run a
//! variety of follow-up operations (simple query, command, transaction,
//! prepared statement, COPY) to verify the connection stays in sync.
//!
//! Several tests exercise the deprecated raw `begin_transaction` /
//! `commit` / `rollback` methods on `Connection` to lock in their wire
//! behavior; the deprecation warning is suppressed file-wide.
#![allow(
    deprecated,
    reason = "regression tests intentionally exercise the deprecated raw transaction API"
)]

mod common;

use common::TestConnection;
use hyperdb_api::Result;

/// After a bad `execute_query`, a follow-up `execute_scalar_query` must
/// return the correct answer (not an empty result or the previous error).
#[test]
fn query_after_bad_query_returns_correct_result() -> Result<()> {
    let tc = TestConnection::new()?;
    tc.execute_command("CREATE TABLE t (v INT)")?;
    tc.execute_command("INSERT INTO t VALUES (1), (2), (3)")?;

    // Provoke a user error — unknown column. Errors from streaming queries
    // surface when the caller iterates the result; use execute_scalar_query
    // which materializes and bubbles any server error immediately.
    let err = tc
        .connection
        .execute_scalar_query::<i64>("SELECT no_such_column FROM t");
    assert!(err.is_err(), "SELECT of unknown column must error");

    // Immediately run a valid query. Before the fix, this returned an empty
    // result (the stale ReadyForQuery read as the end of a zero-row response).
    let count = tc.execute_scalar_i64("SELECT COUNT(*) FROM t")?;
    assert_eq!(count, 3, "wire is desynced — expected count=3 got {count}");
    Ok(())
}

/// After a bad `execute_command` (DDL), a subsequent `execute_command`
/// must still be processed correctly.
#[test]
fn command_after_bad_command_works() -> Result<()> {
    let tc = TestConnection::new()?;
    let err = tc
        .execute_command("CREATE TABLE (") // intentional syntax error
        .err();
    assert!(err.is_some(), "syntax error must be reported");

    // This must succeed cleanly, not inherit residual state.
    tc.execute_command("CREATE TABLE good (id INT)")?;
    tc.execute_command("INSERT INTO good VALUES (42)")?;
    let v = tc.execute_scalar_i64("SELECT v FROM (SELECT id as v FROM good)")?;
    assert_eq!(v, 42);
    Ok(())
}

/// After a transaction with a failing statement mid-flight, a ROLLBACK +
/// fresh transaction must still work. This mirrors the hyperdb-mcp
/// watcher pattern that originally exposed the bug.
#[test]
fn transaction_after_failed_statement_in_tx() -> Result<()> {
    let tc = TestConnection::new()?;
    tc.execute_command("CREATE TABLE acc (v INT NOT NULL)")?;

    // Start tx, insert once, then provoke an error.
    tc.connection.begin_transaction_unguarded()?;
    tc.execute_command("INSERT INTO acc VALUES (1)")?;
    let err = tc.execute_command("INSERT INTO acc VALUES (NULL)"); // NOT NULL violation
    assert!(err.is_err());
    tc.connection.rollback_unguarded()?;

    // The table should still exist (DDL auto-commits in Hyper) but be empty
    // (INSERT rolled back). Critically, COUNT(*) must return an actual
    // number, not a desync'd empty result.
    let count = tc.execute_scalar_i64("SELECT COUNT(*) FROM acc")?;
    assert_eq!(count, 0, "rollback should leave zero rows");
    Ok(())
}

/// After the `Drop` of an unexhausted `Rowset`, the next query on the same
/// connection must work. This exercises the stream-release drain path.
#[test]
fn query_after_abandoned_result_stream() -> Result<()> {
    let tc = TestConnection::new()?;
    tc.execute_command("CREATE TABLE many (i INT)")?;
    for i in 0..100 {
        tc.execute_command(&format!("INSERT INTO many VALUES ({i})"))?;
    }

    // Start a query, read one chunk, then drop without fully consuming.
    {
        let mut rs = tc
            .connection
            .execute_query("SELECT * FROM many ORDER BY i")?;
        let _first = rs.next_chunk()?; // partial read
        // `rs` is dropped here — drain must happen in Drop impl.
    }

    // Fresh query on the same connection must produce correct result.
    let count = tc.execute_scalar_i64("SELECT COUNT(*) FROM many")?;
    assert_eq!(count, 100);
    Ok(())
}

/// A failed COPY FROM STDIN must leave the connection usable for normal
/// queries (the original reproducer from the hyperdb-mcp watcher).
#[test]
fn query_after_failed_copy() -> Result<()> {
    let tc = TestConnection::new()?;
    tc.execute_command("CREATE TABLE stream (id INT, name TEXT)")?;

    // Try a COPY that will fail: wrong column count in the SQL query.
    // Using execute_command lets Hyper respond with ErrorResponse at the
    // query-execution layer rather than requiring the STDIN data path.
    let err =
        tc.execute_command("COPY stream FROM '/definitely/not/a/real/path.csv' WITH (FORMAT csv)");
    assert!(err.is_err(), "expected I/O error from COPY");

    // The connection must still be usable for regular queries.
    let count = tc.execute_scalar_i64("SELECT COUNT(*) FROM stream")?;
    assert_eq!(count, 0);
    Ok(())
}

/// Abandoning a large streaming result (many rows queued) and running a
/// follow-up query must still work. This is the worst case for the Drop
/// path: the server has a large backlog already flushed into the socket.
/// `Drop` cancels the query and then drains that backlog (`DataRow`s are
/// free against the drain's non-row budget), so the connection is handed
/// back clean.
///
/// This test is a plain correctness check; it does not prove the cancel
/// fired (a 150,000-row backlog drains fine even without one). See
/// `cancel_fires_when_streaming_result_is_abandoned` for the test that
/// does.
#[test]
fn query_after_abandoning_large_stream() -> Result<()> {
    let tc = TestConnection::new()?;
    tc.execute_command("CREATE TABLE big (v INT)")?;
    // More than one 65,536-row chunk, so rows are genuinely still in flight
    // after the first chunk is read.
    tc.execute_command("INSERT INTO big SELECT * FROM generate_series(0, 149999)")?;

    {
        let mut rs = tc
            .connection
            .execute_query("SELECT * FROM big ORDER BY v")?;
        let _ = rs.next_chunk()?;
    }

    let count = tc.execute_scalar_i64("SELECT COUNT(*) FROM big")?;
    assert_eq!(count, 150_000);
    Ok(())
}

/// Exercises the cancel-on-`Drop` path for streaming result sets.
///
/// Hyper computes a result and queues it for the socket quickly, so a
/// wall-clock bound cannot tell "cancelled" from "drained everything"
/// (measured: ~20 ms either way for a few hundred thousand rows). The
/// discriminating signal is the drain's hard ceiling on abandoned
/// `DataRow`s: a result far above that ceiling can only be handed back
/// clean if the server was told to stop producing. Without the
/// `CancelRequest` the drain exhausts the ceiling, marks the connection
/// desynchronized, and the follow-up query fails.
#[test]
fn cancel_fires_when_streaming_result_is_abandoned() -> Result<()> {
    let tc = TestConnection::new()?;

    // 10,000,000 rows is more than twice the drain's DataRow ceiling
    // (about 4.2 million); the cancelled backlog is typically well under a
    // million.
    {
        let mut rs = tc
            .connection
            .execute_query("SELECT v FROM generate_series(1, 10000000) v")?;
        let first = rs.next_chunk()?.expect("first chunk");
        assert_eq!(first.len(), 65_536);
    }

    // The connection must be healthy and in sync.
    let count = tc.execute_scalar_i64("SELECT COUNT(*) FROM generate_series(1, 10)")?;
    assert_eq!(count, 10);
    Ok(())
}

/// A `CancelRequest` travels on a separate connection and has no ordering
/// against the main socket, so it could in principle land on the *next*
/// query after the drain and fail it with `57014`. Hammer the
/// drop-then-query pattern, including a result of exactly 65,537 rows (one
/// full chunk plus a one-row tail, so the stream is unfinished when
/// dropped), and require every follow-up to succeed.
#[test]
fn late_cancel_never_fails_the_next_query() -> Result<()> {
    let tc = TestConnection::new()?;
    for i in 0..50_i64 {
        let rows: i64 = match i % 3 {
            0 => 65_537,
            1 => 200_000,
            _ => 65_536,
        };
        {
            let mut rs = tc
                .connection
                .execute_query(&format!("SELECT v FROM generate_series(1, {rows}) v"))?;
            let _ = rs.next_chunk()?;
        }
        let n = tc
            .execute_scalar_i64("SELECT COUNT(*) FROM generate_series(1, 1000)")
            .unwrap_or_else(|e| panic!("iteration {i} ({rows} rows): follow-up failed: {e}"));
        assert_eq!(n, 1000, "iteration {i} ({rows} rows)");
    }
    Ok(())
}

/// Hammer the specific pattern that caused the hyperdb-mcp bug: a failing
/// SELECT followed by a BEGIN that's supposed to succeed. Before the fix,
/// `begin_transaction` would read the stale error and return it spuriously.
#[test]
fn begin_transaction_after_error_is_not_poisoned() -> Result<()> {
    let tc = TestConnection::new()?;
    tc.execute_command("CREATE TABLE t (v INT)")?;

    // Step 1: cause an error. Use a materializing call so the error surfaces.
    let err = tc
        .connection
        .execute_scalar_query::<i64>("SELECT made_up_column FROM t");
    assert!(err.is_err());

    // Step 2: BEGIN must succeed cleanly and not inherit the stale 42703.
    tc.connection.begin_transaction_unguarded()?;
    tc.execute_command("INSERT INTO t VALUES (7)")?;
    tc.connection.commit_unguarded()?;

    let v = tc.execute_scalar_i64("SELECT v FROM t")?;
    assert_eq!(v, 7);
    Ok(())
}

/// Seeds `t(v INT)` with `rows` rows via a single `generate_series` insert.
fn seed_series(tc: &TestConnection, rows: i64) -> Result<()> {
    tc.execute_command("CREATE TABLE t (v INT)")?;
    tc.execute_command(&format!(
        "INSERT INTO t SELECT * FROM generate_series(1, {rows})"
    ))?;
    assert_eq!(tc.execute_scalar_i64("SELECT COUNT(*) FROM t")?, rows);
    Ok(())
}

/// `fetch_one` / `fetch_optional` / `fetch_scalar` take the first row of a
/// result larger than one 65,536-row chunk; the unread backlog must be
/// drained so the next query is not answered with stale `DataRow`s.
#[test]
fn first_row_fetches_on_multi_chunk_results_keep_connection_in_sync() -> Result<()> {
    for rows in [70_000_i64, 65_536] {
        let tc = TestConnection::new()?;
        seed_series(&tc, rows)?;

        let row = tc.connection.fetch_one("SELECT v FROM t ORDER BY v")?;
        assert_eq!(row.get::<i32>(0), Some(1));
        assert_eq!(tc.execute_scalar_i64("SELECT COUNT(*) FROM t")?, rows);

        let row = tc.connection.fetch_optional("SELECT v FROM t ORDER BY v")?;
        assert!(row.is_some());
        assert_eq!(tc.execute_scalar_i64("SELECT COUNT(*) FROM t")?, rows);

        let v: i32 = tc.connection.fetch_scalar("SELECT v FROM t ORDER BY v")?;
        assert_eq!(v, 1);
        assert_eq!(tc.execute_scalar_i64("SELECT COUNT(*) FROM t")?, rows);
    }
    Ok(())
}
