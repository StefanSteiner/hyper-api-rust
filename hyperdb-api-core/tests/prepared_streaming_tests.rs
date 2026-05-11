// Copyright (c) 2026, Salesforce, Inc. All rights reserved.
// SPDX-License-Identifier: Apache-2.0 OR MIT

//! Integration tests for `Client::execute_streaming` and
//! `AsyncClient::execute_prepared_streaming`.
//!
//! These cover the **chunked iteration** path over a prepared-statement
//! result — separate from the parameter-binding path, which has its own
//! existing test coverage.

mod common;

use common::TestServer;
use hyperdb_api_core::client::{AsyncClient, Client, Config};

/// Sync: streams rows in chunks, schema is available before the first
/// chunk call.
#[test]
fn sync_execute_streaming_chunks() {
    let server = TestServer::new().expect("test server");
    let client: Client = server.connect().expect("connect");

    client
        .exec("CREATE TABLE nums (v INT NOT NULL)")
        .expect("create");
    for i in 1..=10i32 {
        client
            .exec(&format!("INSERT INTO nums VALUES ({i})"))
            .expect("insert");
    }

    let stmt = client
        .prepare("SELECT v FROM nums ORDER BY v")
        .expect("prepare");

    {
        let mut stream = client
            .execute_streaming(&stmt, hyperdb_api_core::params![], 4)
            .expect("execute_streaming");

        assert_eq!(stream.schema().len(), 1, "schema known before first chunk");

        let mut all = Vec::new();
        while let Some(chunk) = stream.next_chunk().expect("next_chunk") {
            all.extend(chunk);
        }
        assert_eq!(all.len(), 10);
    }

    client.close().expect("close");
}

/// Sync: empty result set yields Ok(None) immediately.
#[test]
fn sync_execute_streaming_empty() {
    let server = TestServer::new().expect("test server");
    let client: Client = server.connect().expect("connect");

    client
        .exec("CREATE TABLE empty_t (v INT NOT NULL)")
        .expect("create");

    let stmt = client.prepare("SELECT v FROM empty_t").expect("prepare");

    {
        let mut stream = client
            .execute_streaming(&stmt, hyperdb_api_core::params![], 100)
            .expect("execute_streaming");
        let first = stream.next_chunk().expect("next_chunk");
        assert!(first.is_none());
    }

    client.close().expect("close");
}

/// Sync: the connection remains usable after streaming completes.
#[test]
fn sync_connection_usable_after_streaming() {
    let server = TestServer::new().expect("test server");
    let client: Client = server.connect().expect("connect");

    client
        .exec("CREATE TABLE after_t (v INT NOT NULL)")
        .expect("create");
    for i in 1..=3i32 {
        client
            .exec(&format!("INSERT INTO after_t VALUES ({i})"))
            .expect("insert");
    }

    let stmt = client
        .prepare("SELECT v FROM after_t ORDER BY v")
        .expect("prepare");

    {
        let mut stream = client
            .execute_streaming(&stmt, hyperdb_api_core::params![], 2)
            .expect("execute_streaming");
        while stream.next_chunk().expect("next_chunk").is_some() {}
    }

    // Following query on the same connection must succeed.
    let rows = client
        .query("SELECT COUNT(*) FROM after_t")
        .expect("post-stream query");
    assert_eq!(rows.len(), 1);

    client.close().expect("close");
}

async fn async_client(server: &TestServer) -> AsyncClient {
    let config: Config = server.config();
    AsyncClient::connect(&config).await.expect("connect async")
}

/// Async: streams rows in chunks.
#[tokio::test(flavor = "current_thread")]
async fn async_execute_streaming_chunks() {
    let server = TestServer::new().expect("test server");
    let client = async_client(&server).await;

    client
        .exec("CREATE TABLE nums_async (v INT NOT NULL)")
        .await
        .expect("create");
    for i in 1..=10i32 {
        client
            .exec(&format!("INSERT INTO nums_async VALUES ({i})"))
            .await
            .expect("insert");
    }

    let stmt = client
        .prepare("SELECT v FROM nums_async ORDER BY v")
        .await
        .expect("prepare");

    {
        let mut stream = client
            .execute_prepared_streaming(&stmt, hyperdb_api_core::params![], 3)
            .await
            .expect("execute_prepared_streaming");

        assert_eq!(stream.schema().len(), 1);

        let mut all = Vec::new();
        while let Some(chunk) = stream.next_chunk().await.expect("next_chunk") {
            all.extend(chunk);
        }
        assert_eq!(all.len(), 10);
    }

    client.close().await.expect("close");
}
