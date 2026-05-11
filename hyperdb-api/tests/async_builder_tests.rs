// Copyright (c) 2026, Salesforce, Inc. All rights reserved.
// SPDX-License-Identifier: Apache-2.0 OR MIT

//! Integration tests for [`AsyncConnectionBuilder`].

mod common;

use common::{test_hyper_params, test_result_path};
use hyperdb_api::{AsyncConnection, AsyncConnectionBuilder, CreateMode, HyperProcess};

#[tokio::test(flavor = "current_thread")]
async fn builder_happy_path() {
    let db_path = test_result_path("async_builder_happy", "hyper").unwrap();
    let params = test_hyper_params("async_builder_happy").unwrap();
    let hyper = HyperProcess::new(None, Some(&params)).unwrap();
    let endpoint = hyper.require_endpoint().unwrap().to_string();

    let conn = AsyncConnectionBuilder::new(&endpoint)
        .database(&db_path)
        .create_mode(CreateMode::CreateAndReplace)
        .build()
        .await
        .unwrap();

    conn.execute_command("CREATE TABLE t (id INT NOT NULL)")
        .await
        .unwrap();
    let count: i64 = conn.fetch_scalar("SELECT COUNT(*) FROM t").await.unwrap();
    assert_eq!(count, 0);

    conn.close().await.unwrap();
}

#[tokio::test(flavor = "current_thread")]
async fn connection_builder_helper() {
    // AsyncConnection::builder() returns an AsyncConnectionBuilder.
    let db_path = test_result_path("async_builder_helper", "hyper").unwrap();
    let params = test_hyper_params("async_builder_helper").unwrap();
    let hyper = HyperProcess::new(None, Some(&params)).unwrap();
    let endpoint = hyper.require_endpoint().unwrap().to_string();

    let conn = AsyncConnection::builder(&endpoint)
        .create_or_open_database(&db_path)
        .build()
        .await
        .unwrap();

    conn.ping().await.unwrap();
    conn.close().await.unwrap();
}
