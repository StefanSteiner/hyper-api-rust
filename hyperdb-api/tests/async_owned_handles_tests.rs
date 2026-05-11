// Copyright (c) 2026, Salesforce, Inc. All rights reserved.
// SPDX-License-Identifier: Apache-2.0 OR MIT

//! Regression tests for the owned-Arc async handle variants —
//! [`hyperdb_api::AsyncPreparedStatementOwned`] and
//! [`hyperdb_api::AsyncArrowInserterOwned`].
//!
//! These ensure the handles can outlive the stack frame where the
//! connection was created — the core reason they exist. Target
//! consumer is N-API bindings that need `'static` types.

mod common;

use std::sync::Arc;

use common::{test_hyper_params, test_result_path};
use hyperdb_api::{
    AsyncArrowInserterOwned, AsyncConnection, CreateMode, HyperProcess, Result, SqlType,
    TableDefinition,
};

async fn fresh_async_conn(name: &str) -> Result<(HyperProcess, Arc<AsyncConnection>)> {
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
    Ok((hyper, Arc::new(conn)))
}

/// `AsyncPreparedStatementOwned` is `Send + 'static`, which is the
/// whole reason for its existence (N-API bindings need `'static`
/// types that can sit inside a napi class). The compile-time check
/// below is the strongest assertion possible.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn prepared_owned_is_send_and_static() {
    fn assert_send_static<T: Send + 'static>() {}
    assert_send_static::<hyperdb_api::AsyncPreparedStatementOwned>();

    // Runtime sanity: round-trip a scalar through the owned statement
    // without involving tokio::spawn (which is excluded by params
    // not being Send — intentional for encode_param references).
    let (_h, conn) = fresh_async_conn("prepared_owned_scalar").await.unwrap();
    let stmt = conn.prepare_arc("SELECT 42").await.unwrap();
    let v: i32 = stmt.fetch_scalar(&[]).await.unwrap();
    assert_eq!(v, 42);
}

/// `AsyncArrowInserterOwned` is `Send + 'static`, and round-trips a
/// minimal Arrow IPC stream to verify the COPY path is wired up.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn arrow_owned_roundtrip() {
    fn assert_send_static<T: Send + 'static>() {}
    assert_send_static::<AsyncArrowInserterOwned>();

    let (_h, conn) = fresh_async_conn("arrow_owned_roundtrip").await.unwrap();
    conn.execute_command("CREATE TABLE t (id INT NOT NULL, v DOUBLE PRECISION)")
        .await
        .unwrap();

    let table_def = TableDefinition::new("t")
        .add_required_column("id", SqlType::int())
        .add_nullable_column("v", SqlType::double());

    let mut inserter = AsyncArrowInserterOwned::new(Arc::clone(&conn), &table_def).unwrap();

    // Build a minimal Arrow IPC stream with two rows.
    use arrow::array::{Float64Array, Int32Array};
    use arrow::datatypes::{DataType, Field, Schema};
    use arrow::ipc::writer::StreamWriter;
    use arrow::record_batch::RecordBatch;
    use std::sync::Arc as StdArc;

    let schema = StdArc::new(Schema::new(vec![
        Field::new("id", DataType::Int32, false),
        Field::new("v", DataType::Float64, true),
    ]));
    let batch = RecordBatch::try_new(
        StdArc::clone(&schema),
        vec![
            StdArc::new(Int32Array::from(vec![1, 2])),
            StdArc::new(Float64Array::from(vec![Some(1.5), Some(2.5)])),
        ],
    )
    .unwrap();
    let mut buf: Vec<u8> = Vec::new();
    {
        let mut w = StreamWriter::try_new(&mut buf, &schema).unwrap();
        w.write(&batch).unwrap();
        w.finish().unwrap();
    }
    inserter.insert_raw(&buf).await.unwrap();
    let rows = inserter.execute().await.unwrap();
    assert_eq!(rows, 2);

    let count: i64 = conn.fetch_scalar("SELECT COUNT(*) FROM t").await.unwrap();
    assert_eq!(count, 2);
}
