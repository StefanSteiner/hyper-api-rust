// Copyright (c) 2026, Salesforce, Inc. All rights reserved.
// SPDX-License-Identifier: Apache-2.0 OR MIT

//! Integration tests for [`AsyncInserter`], mirroring the sync `Inserter` tests.

mod common;

use common::{test_hyper_params, test_result_path};
use hyperdb_api::{
    AsyncConnection, AsyncInserter, CreateMode, HyperProcess, Result, SqlType, TableDefinition,
};

/// More rows than one chunk (64,000) so at least one auto-flush happens.
const ROWS_OVER_AUTOFLUSH: i32 = 64_001;

async fn fresh_conn(name: &str) -> Result<(HyperProcess, AsyncConnection)> {
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

async fn create(conn: &AsyncConnection, def: &TableDefinition) {
    conn.execute_command(&def.to_create_sql(false).unwrap())
        .await
        .unwrap();
}

async fn count(conn: &AsyncConnection, table: &str) -> i64 {
    conn.fetch_scalar::<i64, _>(format!("SELECT COUNT(*) FROM {table}"))
        .await
        .unwrap()
}

fn int_table(name: &str) -> TableDefinition {
    TableDefinition::new(name).add_nullable_column("a", SqlType::int())
}

#[tokio::test(flavor = "current_thread")]
async fn async_inserter_basic_insert_round_trips_values() {
    let (_h, conn) = fresh_conn("async_ins_basic").await.unwrap();
    let def = TableDefinition::new("t")
        .add_required_column("id", SqlType::int())
        .add_required_column("name", SqlType::text())
        .add_required_column("v", SqlType::double());
    create(&conn, &def).await;

    let mut ins = AsyncInserter::new(&conn, &def).unwrap();
    for i in 0..3_i32 {
        ins.add_i32(i).unwrap();
        ins.add_str(&format!("row{i}")).unwrap();
        ins.add_f64(f64::from(i) * 1.5).unwrap();
        ins.end_row().await.unwrap();
    }
    assert_eq!(ins.execute().await.unwrap(), 3);

    let name: String = conn
        .fetch_scalar("SELECT name FROM t WHERE id = 2")
        .await
        .unwrap();
    assert_eq!(name, "row2");
    let v: f64 = conn
        .fetch_scalar("SELECT v FROM t WHERE id = 2")
        .await
        .unwrap();
    assert!((v - 3.0).abs() < f64::EPSILON);
    conn.close().await.unwrap();
}

#[tokio::test(flavor = "current_thread")]
async fn async_inserter_zero_rows_returns_zero() {
    let (_h, conn) = fresh_conn("async_ins_zero").await.unwrap();
    let def = int_table("t");
    create(&conn, &def).await;
    let ins = AsyncInserter::new(&conn, &def).unwrap();
    assert_eq!(ins.execute().await.unwrap(), 0);
    assert_eq!(count(&conn, "t").await, 0);
}

#[tokio::test(flavor = "current_thread")]
async fn async_inserter_multiple_batches_use_fresh_inserters() {
    let (_h, conn) = fresh_conn("async_ins_batches").await.unwrap();
    let def = int_table("t");
    create(&conn, &def).await;

    for batch in 0..3_i32 {
        let mut ins = AsyncInserter::new(&conn, &def).unwrap();
        for i in 0..10_i32 {
            ins.add_i32(batch * 10 + i).unwrap();
            ins.end_row().await.unwrap();
        }
        assert_eq!(ins.execute().await.unwrap(), 10);
    }
    assert_eq!(count(&conn, "t").await, 30);
    let sum: i64 = conn.fetch_scalar("SELECT SUM(a) FROM t").await.unwrap();
    assert_eq!(sum, (0..30).sum::<i64>());
}

#[tokio::test(flavor = "current_thread")]
async fn async_inserter_large_volume_crosses_autoflush() {
    let (_h, conn) = fresh_conn("async_ins_large").await.unwrap();
    let def = int_table("t");
    create(&conn, &def).await;

    let total = ROWS_OVER_AUTOFLUSH * 2 + 17;
    let mut ins = AsyncInserter::new(&conn, &def).unwrap();
    for i in 0..total {
        ins.add_i32(i).unwrap();
        ins.end_row().await.unwrap();
    }
    assert_eq!(ins.execute().await.unwrap(), u64::try_from(total).unwrap());
    assert_eq!(count(&conn, "t").await, i64::from(total));
    let max: i32 = conn.fetch_scalar("SELECT MAX(a) FROM t").await.unwrap();
    assert_eq!(max, total - 1);
}

/// A payload past the 16 MiB size limit (but under the row limit) must also
/// auto-flush and arrive intact.
#[tokio::test(flavor = "current_thread")]
async fn async_inserter_large_payload_crosses_size_flush() {
    let (_h, conn) = fresh_conn("async_ins_bytes").await.unwrap();
    let def = TableDefinition::new("t").add_nullable_column("s", SqlType::text());
    create(&conn, &def).await;

    let blob = "x".repeat(100_000);
    let rows = 400_u64; // ~40 MB
    let mut ins = AsyncInserter::new(&conn, &def).unwrap();
    for _ in 0..rows {
        ins.add_str(&blob).unwrap();
        ins.end_row().await.unwrap();
    }
    assert_eq!(ins.execute().await.unwrap(), rows);
    let bytes: i64 = conn
        .fetch_scalar("SELECT SUM(LENGTH(s)) FROM t")
        .await
        .unwrap();
    assert_eq!(bytes, i64::try_from(rows).unwrap() * 100_000);
}

#[tokio::test(flavor = "current_thread")]
async fn async_inserter_nulls_round_trip() {
    let (_h, conn) = fresh_conn("async_ins_nulls").await.unwrap();
    let def = TableDefinition::new("t")
        .add_nullable_column("a", SqlType::int())
        .add_nullable_column("b", SqlType::text());
    create(&conn, &def).await;

    let mut ins = AsyncInserter::new(&conn, &def).unwrap();
    ins.add_null().unwrap();
    ins.add_str("x").unwrap();
    ins.end_row().await.unwrap();
    ins.add_i32(5).unwrap();
    ins.add_null().unwrap();
    ins.end_row().await.unwrap();
    assert_eq!(ins.execute().await.unwrap(), 2);

    assert_eq!(count(&conn, "t").await, 2);
    let a_nulls: i64 = conn
        .fetch_scalar("SELECT COUNT(*) FROM t WHERE a IS NULL")
        .await
        .unwrap();
    let b_nulls: i64 = conn
        .fetch_scalar("SELECT COUNT(*) FROM t WHERE b IS NULL")
        .await
        .unwrap();
    assert_eq!((a_nulls, b_nulls), (1, 1));
}

#[tokio::test(flavor = "current_thread")]
async fn async_inserter_null_in_required_column_is_rejected() {
    let (_h, conn) = fresh_conn("async_ins_notnull").await.unwrap();
    let def = TableDefinition::new("t").add_required_column("a", SqlType::int());
    create(&conn, &def).await;
    let mut ins = AsyncInserter::new(&conn, &def).unwrap();
    assert!(ins.add_null().is_err());
}

#[tokio::test(flavor = "current_thread")]
async fn async_inserter_incomplete_row_errors_at_execute() {
    let (_h, conn) = fresh_conn("async_ins_incomplete").await.unwrap();
    let def = TableDefinition::new("t")
        .add_nullable_column("a", SqlType::int())
        .add_nullable_column("b", SqlType::int());
    create(&conn, &def).await;
    let mut ins = AsyncInserter::new(&conn, &def).unwrap();
    ins.add_i32(1).unwrap();
    assert!(ins.execute().await.is_err());
    assert_eq!(count(&conn, "t").await, 0);
}

#[tokio::test(flavor = "current_thread")]
async fn async_inserter_cancel_after_autoflush_leaves_connection_usable() {
    let (_h, conn) = fresh_conn("async_ins_cancel").await.unwrap();
    let def = int_table("t");
    create(&conn, &def).await;

    let mut ins = AsyncInserter::new(&conn, &def).unwrap();
    for i in 0..ROWS_OVER_AUTOFLUSH {
        ins.add_i32(i).unwrap();
        ins.end_row().await.unwrap();
    }
    ins.cancel();

    assert_eq!(count(&conn, "t").await, 0, "cancel must commit nothing");
    let v: i32 = conn.fetch_scalar("SELECT 41 + 1").await.unwrap();
    assert_eq!(v, 42);

    let mut fresh = AsyncInserter::new(&conn, &def).unwrap();
    for i in 7..9_i32 {
        fresh.add_i32(i).unwrap();
        fresh.end_row().await.unwrap();
    }
    assert_eq!(fresh.execute().await.unwrap(), 2);
    assert_eq!(count(&conn, "t").await, 2);
}

#[tokio::test(flavor = "current_thread")]
async fn async_inserter_dropped_mid_copy_leaves_connection_usable() {
    let (_h, conn) = fresh_conn("async_ins_drop").await.unwrap();
    let def = int_table("t");
    create(&conn, &def).await;
    {
        let mut ins = AsyncInserter::new(&conn, &def).unwrap();
        for i in 0..ROWS_OVER_AUTOFLUSH {
            ins.add_i32(i).unwrap();
            ins.end_row().await.unwrap();
        }
    }
    assert_eq!(count(&conn, "t").await, 0);
    let v: i32 = conn.fetch_scalar("SELECT 41 + 1").await.unwrap();
    assert_eq!(v, 42);
}

#[tokio::test(flavor = "current_thread")]
async fn async_inserter_server_rejected_value_errors_and_stays_in_sync() {
    let (_h, conn) = fresh_conn("async_ins_reject").await.unwrap();
    conn.execute_command("CREATE TABLE t (a VARCHAR(3))")
        .await
        .unwrap();
    let def = TableDefinition::new("t").add_nullable_column("a", SqlType::text());

    let mut ins = AsyncInserter::new(&conn, &def).unwrap();
    let mut failed = false;
    for i in 0..ROWS_OVER_AUTOFLUSH {
        ins.add_str(if i == 10 { "toolong" } else { "ok" }).unwrap();
        if ins.end_row().await.is_err() {
            failed = true;
            break;
        }
    }
    if !failed {
        assert!(ins.execute().await.is_err(), "server must reject the value");
    }
    for expected in [42, 43] {
        let v: i32 = conn
            .fetch_scalar(format!("SELECT {expected}"))
            .await
            .unwrap();
        assert_eq!(v, expected);
    }
    assert_eq!(count(&conn, "t").await, 0);
}
