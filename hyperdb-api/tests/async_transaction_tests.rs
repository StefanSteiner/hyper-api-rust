// Copyright (c) 2026, Salesforce, Inc. All rights reserved.
// SPDX-License-Identifier: Apache-2.0 OR MIT

//! Integration tests for the extended [`AsyncTransaction`] API.
//!
//! Sync `Transaction` exposes `execute_query`, `fetch_*`, `query_count`
//! and parameterized queries. This test file locks in that every one of
//! those has a working async delegation.

mod common;

use common::{test_hyper_params, test_result_path};
use hyperdb_api::{AsyncConnection, CreateMode, HyperProcess, Result};

async fn fresh_async_conn(name: &str) -> Result<(HyperProcess, AsyncConnection)> {
    let db_path = test_result_path(name, "hyper")?;
    let params = test_hyper_params(name)?;
    let hyper = HyperProcess::new(None, Some(&params))?;
    let endpoint = hyper.require_endpoint()?.to_string();
    let conn = AsyncConnection::connect(
        &endpoint,
        db_path.to_str().expect("path"),
        CreateMode::CreateAndReplace,
    )
    .await?;
    Ok((hyper, conn))
}

#[tokio::test(flavor = "current_thread")]
async fn fetch_inside_txn() {
    let (_hyper, mut conn) = fresh_async_conn("async_txn_fetch").await.unwrap();

    conn.execute_command("CREATE TABLE t (id INT NOT NULL, val INT)")
        .await
        .unwrap();
    conn.execute_command("INSERT INTO t VALUES (1, 10), (2, 20), (3, 30)")
        .await
        .unwrap();

    {
        let txn = conn.transaction().await.unwrap();
        // Read inside transaction — previously impossible through high-level API.
        let sum: i64 = txn.fetch_scalar("SELECT SUM(val) FROM t").await.unwrap();
        assert_eq!(sum, 60);

        // Parameterized query inside transaction.
        let rs = txn
            .query_params("SELECT id FROM t WHERE val > $1 ORDER BY id", &[&15i32])
            .await
            .unwrap();
        let rows = rs.collect_rows().await.unwrap();
        assert_eq!(rows.len(), 2);

        // Count helper.
        let n = txn.query_count("SELECT COUNT(*) FROM t").await.unwrap();
        assert_eq!(n, 3);

        txn.commit().await.unwrap();
    }

    conn.close().await.unwrap();
}

#[tokio::test(flavor = "current_thread")]
async fn read_modify_write_in_txn() {
    let (_hyper, mut conn) = fresh_async_conn("async_txn_rmw").await.unwrap();

    conn.execute_command("CREATE TABLE counters (k TEXT, v INT NOT NULL)")
        .await
        .unwrap();
    conn.execute_command("INSERT INTO counters VALUES ('x', 10)")
        .await
        .unwrap();

    {
        let txn = conn.transaction().await.unwrap();
        let cur: i32 = txn
            .fetch_scalar("SELECT v FROM counters WHERE k = 'x'")
            .await
            .unwrap();
        txn.command_params("UPDATE counters SET v = $1 WHERE k = 'x'", &[&(cur + 5)])
            .await
            .unwrap();
        txn.commit().await.unwrap();
    }

    let final_v: i32 = conn
        .fetch_scalar("SELECT v FROM counters WHERE k = 'x'")
        .await
        .unwrap();
    assert_eq!(final_v, 15);

    conn.close().await.unwrap();
}

#[tokio::test(flavor = "current_thread")]
async fn rollback_discards_writes() {
    let (_hyper, mut conn) = fresh_async_conn("async_txn_rollback").await.unwrap();

    conn.execute_command("CREATE TABLE t (v INT NOT NULL)")
        .await
        .unwrap();

    {
        let txn = conn.transaction().await.unwrap();
        txn.execute_command("INSERT INTO t VALUES (1)")
            .await
            .unwrap();
        txn.execute_command("INSERT INTO t VALUES (2)")
            .await
            .unwrap();
        txn.rollback().await.unwrap();
    }

    let count: i64 = conn.fetch_scalar("SELECT COUNT(*) FROM t").await.unwrap();
    assert_eq!(count, 0);

    conn.close().await.unwrap();
}
