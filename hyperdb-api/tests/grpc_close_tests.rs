// Copyright (c) 2026, Salesforce, Inc. All rights reserved.
// SPDX-License-Identifier: Apache-2.0 OR MIT

//! `close()` on a gRPC connection that has a database must not try to
//! `DETACH DATABASE` (gRPC is read-only, so the server would reject it and
//! `close()` would return an error).

mod common;

use common::test_result_path;
use hyperdb_api::{
    AsyncConnection, Connection, CreateMode, HyperProcess, ListenMode, Parameters, Result,
};

/// Creates a database file via TCP, then starts a gRPC-only `hyperd`.
fn grpc_hyperd_with_db(name: &str) -> Result<(HyperProcess, String, String)> {
    let db = test_result_path(name, "hyper")?
        .to_string_lossy()
        .to_string();
    {
        let tcp = HyperProcess::new(None, None)?;
        let conn = Connection::new(&tcp, &db, CreateMode::CreateAndReplace)?;
        conn.execute_command("CREATE TABLE t (id INT NOT NULL)")?;
        conn.close()?;
    }
    let mut params = Parameters::new();
    params.set("log_dir", "test_results");
    params.set_listen_mode(ListenMode::Grpc { port: 0 });
    let hyper = HyperProcess::new(None, Some(&params))?;
    let url = hyper.grpc_url().expect("gRPC URL");
    Ok((hyper, url, db))
}

#[test]
fn grpc_close_with_database_succeeds_sync() -> Result<()> {
    let (_hyper, url, db) = grpc_hyperd_with_db("grpc_close_sync")?;
    let conn = Connection::builder(&url)
        .database(&db)
        .create_mode(CreateMode::DoNotCreate)
        .build()?;
    assert_eq!(conn.transport_type(), "gRPC");
    conn.close()?;
    let _ = std::fs::remove_file(&db);
    Ok(())
}

#[tokio::test]
async fn grpc_close_with_database_succeeds_async() -> Result<()> {
    let (_hyper, url, db) = grpc_hyperd_with_db("grpc_close_async")?;
    let conn = AsyncConnection::builder(&url)
        .database(&db)
        .create_mode(CreateMode::DoNotCreate)
        .build()
        .await?;
    conn.close().await?;
    let _ = std::fs::remove_file(&db);
    Ok(())
}
