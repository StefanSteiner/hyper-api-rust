// Copyright (c) 2026, Salesforce, Inc. All rights reserved.
// SPDX-License-Identifier: Apache-2.0 OR MIT

//! `copy_database` (sync + async) emits `CREATE DATABASE <dst> FROM <src>`.
//! Hyper has no `COPY DATABASE` statement.

mod common;

use common::test_result_path;
use hyperdb_api::{AsyncConnection, Connection, CreateMode, HyperProcess, Result};

/// Creates a populated source database (closed afterwards) and returns
/// `(source, destination)` paths with the destination absent.
fn make_source(hyper: &HyperProcess, name: &str) -> Result<(String, String)> {
    let src = test_result_path(&format!("{name}_src"), "hyper")?
        .to_string_lossy()
        .to_string();
    let dst = test_result_path(&format!("{name}_dst"), "hyper")?
        .to_string_lossy()
        .to_string();
    let _ = std::fs::remove_file(&dst);
    let conn = Connection::new(hyper, &src, CreateMode::CreateAndReplace)?;
    conn.execute_command("CREATE TABLE t (id INT NOT NULL)")?;
    conn.execute_command("INSERT INTO t VALUES (1), (2), (3)")?;
    conn.close()?;
    Ok((src, dst))
}

fn count_in_copy(hyper: &HyperProcess, dst: &str) -> Result<i64> {
    let conn = Connection::new(hyper, dst, CreateMode::DoNotCreate)?;
    let n: i64 = conn.fetch_scalar("SELECT count(*) FROM t")?;
    conn.close()?;
    Ok(n)
}

#[test]
fn copy_database_sync_copies_data() -> Result<()> {
    let hyper = HyperProcess::new(None, None)?;
    let (src, dst) = make_source(&hyper, "copy_db_sync")?;
    let scratch = test_result_path("copy_db_sync_scratch", "hyper")?
        .to_string_lossy()
        .to_string();
    let conn = Connection::new(&hyper, &scratch, CreateMode::CreateAndReplace)?;
    conn.copy_database(&src, &dst)?;
    assert!(std::path::Path::new(&dst).exists());
    assert_eq!(count_in_copy(&hyper, &dst)?, 3);
    conn.close()?;
    for f in [&src, &dst, &scratch] {
        let _ = std::fs::remove_file(f);
    }
    Ok(())
}

#[tokio::test]
async fn copy_database_async_copies_data() -> Result<()> {
    let hyper = HyperProcess::new(None, None)?;
    let (src, dst) = make_source(&hyper, "copy_db_async")?;
    let scratch = test_result_path("copy_db_async_scratch", "hyper")?
        .to_string_lossy()
        .to_string();
    let endpoint = hyper.require_endpoint()?.to_string();
    let conn = AsyncConnection::connect(&endpoint, &scratch, CreateMode::CreateAndReplace).await?;
    conn.copy_database(&src, &dst).await?;
    assert!(std::path::Path::new(&dst).exists());
    assert_eq!(count_in_copy(&hyper, &dst)?, 3);
    conn.close().await?;
    for f in [&src, &dst, &scratch] {
        let _ = std::fs::remove_file(f);
    }
    Ok(())
}
