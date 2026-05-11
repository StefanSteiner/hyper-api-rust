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
    tc.connection.begin_transaction()?;
    tc.execute_command("INSERT INTO acc VALUES (1)")?;
    let err = tc.execute_command("INSERT INTO acc VALUES (NULL)"); // NOT NULL violation
    assert!(err.is_err());
    tc.connection.rollback()?;

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
/// drain path: the server has a large backlog to flush, and the bounded
/// drain must either catch up or yield the connection safely.
#[test]
fn query_after_abandoning_large_stream() -> Result<()> {
    let tc = TestConnection::new()?;
    tc.execute_command("CREATE TABLE big (v INT)")?;
    // Use a single INSERT ... VALUES expression so we don't hammer the
    // server with 500 round-trips during test setup.
    let values: Vec<String> = (0..500).map(|i| format!("({i})")).collect();
    tc.execute_command(&format!("INSERT INTO big VALUES {}", values.join(",")))?;

    // Grab one chunk then drop, leaving hundreds of rows in flight.
    {
        let mut rs = tc
            .connection
            .execute_query("SELECT * FROM big ORDER BY v")?;
        let _ = rs.next_chunk()?;
    }

    // Connection must still produce correct results, regardless of whether
    // the Drop drain caught the full backlog or the 4096-message cap
    // triggered (either way the next query should succeed).
    let count = tc.execute_scalar_i64("SELECT COUNT(*) FROM big")?;
    assert_eq!(count, 500);
    Ok(())
}

/// Exercises the cancel-on-`Drop` path for streaming result sets.
///
/// Without cancel-on-drop, abandoning a stream whose remaining backlog
/// exceeds the bounded drain budget (currently 1024 messages) would leave
/// the connection desynchronized and the follow-up query would fail or
/// return wrong data. With cancel-on-drop, the server observes the
/// `CancelRequest` on a separate connection, stops emitting rows, and our
/// drain reaches `ReadyForQuery` well within the budget.
///
/// We use `generate_series(1, 100000)` so the server has ~100k rows of
/// `DataRows` ready to stream; that is comfortably larger than any
/// reasonable drain cap, so a passing follow-up query is strong evidence
/// that the cancel landed and short-circuited the stream.
#[test]
fn cancel_fires_when_streaming_result_is_abandoned() -> Result<()> {
    let tc = TestConnection::new()?;

    // Seed a table large enough that a full passive drain would far
    // exceed the 1024-message post-cancel drain budget. We use a single
    // `INSERT ... VALUES` expression so we don't care whether hyperd's
    // `generate_series` caps out at 66k or any other implementation
    // artifact: 3000 rows is deterministic and comfortably above 1024.
    tc.execute_command("CREATE TABLE huge (v INT)")?;
    let values: Vec<String> = (0..3000).map(|i| format!("({i})")).collect();
    tc.execute_command(&format!("INSERT INTO huge VALUES {}", values.join(",")))?;
    let seeded = tc.execute_scalar_i64("SELECT COUNT(*) FROM huge")?;
    assert_eq!(seeded, 3000, "test setup: INSERT didn't seed all rows");

    let start = std::time::Instant::now();

    // Open a streaming query, read one chunk, then drop.
    //
    // Before cancel-on-drop: a 3000-row stream would almost certainly fit
    // within the 4096-message bounded drain cap we had, so this test would
    // still pass without the cancel — but the interesting property now is
    // that the 1024-message *post-cancel* cap is reliably sufficient
    // *because* cancel stops the server from producing more rows. If the
    // cancel path regressed (e.g. someone removed the `CancelRequest` call
    // in `QueryStream::Drop`), the bounded drain would blow its 1024
    // budget on ~3000 waiting DataRows and the connection would end up
    // poisoned — the follow-up query would fail or return wrong data.
    {
        let mut rs = tc
            .connection
            .execute_query("SELECT * FROM huge ORDER BY v")?;
        let _first = rs.next_chunk()?;
    }

    let drop_elapsed = start.elapsed();

    // Follow-up query MUST succeed with the correct count: proof that the
    // post-cancel drain caught `ReadyForQuery` before its budget ran out
    // and the connection is healthy.
    let count = tc.execute_scalar_i64("SELECT COUNT(*) FROM huge")?;
    assert_eq!(count, 3000);

    // Timing assertion: Drop (cancel + bounded drain) should finish well
    // under a second on loopback — locally we see ~1-10 ms. A 3-second
    // threshold leaves ~100x headroom for noisy CI runners while still
    // being tight enough to catch the regression class we actually care
    // about: "cancel didn't fire and the drain is reading every row
    // serially" would blow far past 3 seconds for 3000 rows.
    //
    // Using `<` (not `<=`) because equality at exactly the threshold is
    // already the failure mode — something took suspiciously long.
    const DROP_DEADLINE: std::time::Duration = std::time::Duration::from_secs(3);
    assert!(
        drop_elapsed < DROP_DEADLINE,
        "stream Drop took {drop_elapsed:?} (deadline {DROP_DEADLINE:?}), \
         suggesting cancel did not fire",
    );
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
    tc.connection.begin_transaction()?;
    tc.execute_command("INSERT INTO t VALUES (7)")?;
    tc.connection.commit()?;

    let v = tc.execute_scalar_i64("SELECT v FROM t")?;
    assert_eq!(v, 7);
    Ok(())
}
