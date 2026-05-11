// Copyright (c) 2026, Salesforce, Inc. All rights reserved.
// SPDX-License-Identifier: Apache-2.0 OR MIT

//! Integration tests for the high-level `AsyncConnection::prepare` API.

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
async fn async_prepared_scalar() {
    let (_h, conn) = fresh_async_conn("async_prepared_scalar").await.unwrap();
    let stmt = conn.prepare("SELECT 7").await.unwrap();
    let v: i32 = stmt.fetch_scalar(&[]).await.unwrap();
    assert_eq!(v, 7);
    conn.close().await.unwrap();
}

#[tokio::test(flavor = "current_thread")]
async fn async_prepared_fetch_all() {
    let (_h, conn) = fresh_async_conn("async_prepared_fetch_all").await.unwrap();
    conn.execute_command("CREATE TABLE t (id INT NOT NULL)")
        .await
        .unwrap();
    conn.execute_command("INSERT INTO t VALUES (1), (2), (3)")
        .await
        .unwrap();

    let stmt = conn.prepare("SELECT id FROM t ORDER BY id").await.unwrap();
    let rows = stmt.fetch_all(&[]).await.unwrap();
    assert_eq!(rows.len(), 3);
    conn.close().await.unwrap();
}

#[tokio::test(flavor = "current_thread")]
async fn async_prepared_streaming() {
    let (_h, conn) = fresh_async_conn("async_prepared_streaming").await.unwrap();
    conn.execute_command("CREATE TABLE s (v INT NOT NULL)")
        .await
        .unwrap();
    for i in 1..=8i32 {
        conn.execute_command(&format!("INSERT INTO s VALUES ({i})"))
            .await
            .unwrap();
    }

    let stmt = conn.prepare("SELECT v FROM s ORDER BY v").await.unwrap();
    {
        let mut rs = stmt.query(&[]).await.unwrap();
        let mut total = 0;
        while let Some(chunk) = rs.next_chunk().await.unwrap() {
            total += chunk.len();
        }
        assert_eq!(total, 8);
    }
    conn.close().await.unwrap();
}

#[tokio::test(flavor = "current_thread")]
async fn async_prepared_reuse() {
    let (_h, conn) = fresh_async_conn("async_prepared_reuse").await.unwrap();
    conn.execute_command("CREATE TABLE r (v INT NOT NULL)")
        .await
        .unwrap();
    let insert = conn.prepare("INSERT INTO r VALUES (1)").await.unwrap();
    for _ in 0..4 {
        let n = insert.execute(&[]).await.unwrap();
        assert_eq!(n, 1);
    }
    let count: i64 = conn.fetch_scalar("SELECT COUNT(*) FROM r").await.unwrap();
    assert_eq!(count, 4);
    conn.close().await.unwrap();
}
