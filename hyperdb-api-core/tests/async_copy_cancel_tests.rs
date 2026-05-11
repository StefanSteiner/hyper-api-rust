// Copyright (c) 2026, Salesforce, Inc. All rights reserved.
// SPDX-License-Identifier: Apache-2.0 OR MIT

//! Regression tests for the async `pending_copy_cancel` drain path.
//!
//! Background: when a caller drops an `AsyncCopyInWriter` without calling
//! `finish()` or `cancel()`, its `Drop` impl calls
//! `AsyncRawConnection::queue_copy_fail`, which appends a `CopyFail`
//! message to the connection's write buffer but *does not flush* (async
//! I/O is impossible in `Drop`). The server has not yet seen the cancel;
//! the next async operation on the connection must flush and drain the
//! resulting `ErrorResponse + ReadyForQuery` before writing any new
//! protocol bytes.
//!
//! Every `pub async fn` on `AsyncRawConnection` that initiates new
//! server work begins with `self.drain_pending_copy_cancel().await?;` to
//! honor that invariant. This test exercises one of the paths that was
//! previously missing the call (`execute_prepared_no_result`) — reachable
//! from the public `AsyncClient` API — to lock in that the invariant is
//! maintained end-to-end.
//!
//! Without the fix, the buggy method's `flush()` sends
//! `[CopyFail | Bind | Execute | Sync]` in a single write. The server
//! answers with the `CopyFail`'s `ErrorResponse + ReadyForQuery`
//! interleaved with the execute response. The client's read loop
//! misattributes the COPY error to the execute call, the execute's
//! actual response remains queued on the wire, and the *next* unrelated
//! call sees a wire desync.

mod common;

use common::TestServer;
use hyperdb_api_core::client::{AsyncClient, Config};
use hyperdb_api_core::types::oids;

/// Abandoning an `AsyncCopyInWriter` and then executing a prepared
/// statement must succeed — the execute path must drain the queued
/// `CopyFail` before writing its own bytes.
#[tokio::test(flavor = "current_thread")]
async fn execute_prepared_after_abandoned_copy_in() {
    let server = TestServer::new().expect("failed to create test server");
    let config: Config = server.config();

    let client = AsyncClient::connect(&config)
        .await
        .expect("failed to connect async client");

    // Clean setup — connection is healthy, pending_copy_cancel is false.
    // Create both the table the prepared statement will hit and a
    // separate table to COPY into.
    client
        .exec("CREATE TABLE pstmt_target (id INT NOT NULL)")
        .await
        .expect("create pstmt_target");
    client
        .exec("CREATE TABLE copy_target (id INT NOT NULL)")
        .await
        .expect("create copy_target");

    // Prepare the statement while the connection is clean. The
    // `prepare` path was always correct; the bug is in what happens
    // *after* we abandon a copy and then use the prepared handle.
    let stmt = client
        .prepare_typed("INSERT INTO pstmt_target VALUES ($1)", &[oids::INT])
        .await
        .expect("prepare");

    // Kick off a COPY IN, then drop the writer without `finish()` or
    // `cancel()`. This invokes `AsyncCopyInWriter::Drop`, which queues
    // a `CopyFail` into the connection's write buffer. The buffer is
    // NOT flushed at this point — flushing is async and `Drop` is sync.
    {
        let _writer = client
            .copy_in("copy_target", &["id"])
            .await
            .expect("start copy_in");
        // Deliberately do not call `_writer.finish()` or `.cancel()`.
        // Dropping here is the whole point of the test.
    }

    // Now exercise the previously-buggy path.
    // `AsyncClient::execute_prepared_no_result` -> `AsyncRawConnection::
    // execute_prepared_no_result`, which (pre-fix) did NOT drain the
    // pending CopyFail and would corrupt the wire.
    let affected = client
        .execute_prepared_no_result(&stmt, [Some(vec![0, 0, 0, 42])].as_slice())
        .await
        .expect("execute_prepared_no_result after abandoned copy");

    assert_eq!(
        affected, 1,
        "execute_prepared_no_result must report exactly one row inserted \
         (this is the test signal that the drain fired and the wire was \
         clean before Bind/Execute went out)",
    );

    // Second follow-up operation to verify the connection is not just
    // in a "barely working" state but is fully restored. If the drain
    // had failed to fully consume the COPY's trailing messages, this
    // second call would see residual bytes and either fail or return
    // a stale result.
    let count: u64 = client
        .exec("INSERT INTO pstmt_target VALUES (99)")
        .await
        .expect("plain exec after drained copy cancel");
    assert_eq!(count, 1, "follow-up exec should affect one row");

    // And one more: verify the actual table contents match our
    // operations. Anything wrong on the wire earlier would have
    // manifested as a silently-dropped or duplicated insert.
    let rows = client
        .query("SELECT id FROM pstmt_target ORDER BY id")
        .await
        .expect("final select");
    assert_eq!(rows.len(), 2, "expected two rows total");
}
