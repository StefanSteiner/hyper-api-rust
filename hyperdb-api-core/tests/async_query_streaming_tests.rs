// Copyright (c) 2026, Salesforce, Inc. All rights reserved.
// SPDX-License-Identifier: Apache-2.0 OR MIT

//! Integration tests for `AsyncClient::query_streaming` — the async
//! analog of `Client::query_streaming`.

mod common;

use common::TestServer;
use hyperdb_api_core::client::{AsyncClient, Config};

/// Happy path: stream all rows back in two chunks.
#[tokio::test(flavor = "current_thread")]
async fn streams_in_chunks() {
    let server = TestServer::new().expect("test server");
    let config: Config = server.config();
    let client = AsyncClient::connect(&config).await.expect("connect");

    client
        .exec("CREATE TABLE nums (v INT NOT NULL)")
        .await
        .expect("create");

    // Insert 10 rows 1..=10.
    for i in 1..=10 {
        client
            .exec(&format!("INSERT INTO nums VALUES ({i})"))
            .await
            .expect("insert");
    }

    let mut stream = client
        .query_streaming("SELECT v FROM nums ORDER BY v", 4)
        .await
        .expect("query_streaming");

    let mut all = Vec::new();
    while let Some(chunk) = stream.next_chunk().await.expect("next_chunk") {
        all.extend(chunk);
    }
    assert_eq!(all.len(), 10);
    drop(stream);

    client.close().await.expect("close");
}

/// Empty result set: `next_chunk` returns Ok(None) immediately.
#[tokio::test(flavor = "current_thread")]
async fn empty_result_set() {
    let server = TestServer::new().expect("test server");
    let config: Config = server.config();
    let client = AsyncClient::connect(&config).await.expect("connect");

    client
        .exec("CREATE TABLE empty_t (v INT NOT NULL)")
        .await
        .expect("create");

    let mut stream = client
        .query_streaming("SELECT v FROM empty_t", 100)
        .await
        .expect("query_streaming");

    let first = stream.next_chunk().await.expect("next_chunk");
    assert!(
        first.is_none(),
        "empty table should yield Ok(None) on first chunk"
    );
    drop(stream);

    client.close().await.expect("close");
}

/// Schema is populated after the first chunk (or first successful read)
/// because `RowDescription` is the first message the server emits.
#[tokio::test(flavor = "current_thread")]
async fn schema_populated_after_first_chunk() {
    let server = TestServer::new().expect("test server");
    let config: Config = server.config();
    let client = AsyncClient::connect(&config).await.expect("connect");

    client
        .exec("CREATE TABLE schema_t (id INT NOT NULL, name TEXT)")
        .await
        .expect("create");
    client
        .exec("INSERT INTO schema_t VALUES (1, 'a')")
        .await
        .expect("insert");

    let mut stream = client
        .query_streaming("SELECT id, name FROM schema_t", 10)
        .await
        .expect("query_streaming");
    let _ = stream.next_chunk().await.expect("next_chunk");

    let schema = stream.schema().expect("schema after first chunk");
    assert_eq!(schema.len(), 2);
    drop(stream);

    client.close().await.expect("close");
}
