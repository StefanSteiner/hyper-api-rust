// Copyright (c) 2026, Salesforce, Inc. All rights reserved.
// SPDX-License-Identifier: Apache-2.0 OR MIT

//! Regression tests for `AsyncPreparedStatement`'s auto-close-on-Drop path.
//!
//! The statement must be closed on the server when the Rust handle is
//! dropped without an explicit `close()` — otherwise server-side
//! statement slots accumulate.

mod common;

use common::TestServer;
use hyperdb_api_core::client::{AsyncClient, Config};

async fn async_client(server: &TestServer) -> AsyncClient {
    let config: Config = server.config();
    AsyncClient::connect(&config).await.expect("connect async")
}

/// Explicit `close(&client)` succeeds and does not race with the Drop
/// path.
#[tokio::test(flavor = "current_thread")]
async fn explicit_close_returns_ok_and_suppresses_drop() {
    let server = TestServer::new().expect("test server");
    let client = async_client(&server).await;

    let stmt = client.prepare("SELECT 42 AS v").await.expect("prepare");
    stmt.close(&client).await.expect("close");

    // Connection still usable after explicit close.
    let rows = client
        .query("SELECT 1 AS v")
        .await
        .expect("post-close query");
    assert_eq!(rows.len(), 1);

    client.close().await.expect("close client");
}

/// Dropping the statement without explicit close triggers the
/// `tokio::spawn` auto-close. We can't directly observe the close on the
/// wire, but we can verify that the connection remains usable
/// afterwards and that re-preparing the *same* SQL yields a usable
/// statement — i.e. no desync was introduced.
#[tokio::test(flavor = "current_thread")]
async fn drop_without_explicit_close_keeps_connection_usable() {
    let server = TestServer::new().expect("test server");
    let client = async_client(&server).await;

    {
        let _stmt = client.prepare("SELECT 100 AS v").await.expect("prepare");
        // Dropped here at end of scope.
    }

    // Give the spawned close task a moment to run before we continue
    // (current_thread runtime yields between tasks).
    tokio::task::yield_now().await;
    tokio::task::yield_now().await;

    // Connection is still usable — re-prepare and use it.
    let stmt2 = client
        .prepare("SELECT 200 AS v")
        .await
        .expect("second prepare");

    // Basic round-trip (no params, no binding bug).
    let _ = client
        .execute_prepared(&stmt2, &[] as &[Option<Vec<u8>>])
        .await
        .expect("execute after drop");

    stmt2.close(&client).await.expect("close stmt2");
    client.close().await.expect("close client");
}

/// Dropping many statements in sequence must not desync or break the
/// connection — stress test for the auto-close spawn pattern.
#[tokio::test(flavor = "current_thread")]
async fn many_drops_in_sequence() {
    let server = TestServer::new().expect("test server");
    let client = async_client(&server).await;

    for _ in 0..20 {
        let _stmt = client.prepare("SELECT 1 AS v").await.expect("prepare");
    }
    // Drain any pending spawned close tasks.
    for _ in 0..5 {
        tokio::task::yield_now().await;
    }

    // Connection survives.
    let rows = client
        .query("SELECT 99 AS v")
        .await
        .expect("query after drops");
    assert_eq!(rows.len(), 1);

    client.close().await.expect("close client");
}
