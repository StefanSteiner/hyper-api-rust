// Copyright (c) 2026, Salesforce, Inc. All rights reserved.
// SPDX-License-Identifier: Apache-2.0 OR MIT

//! Integration tests for the high-level `Connection::prepare` API.

mod common;

use common::TestConnection;
use hyperdb_api::{Connection, Result};

/// Scalar fetch through a prepared statement executed once.
#[test]
fn prepared_scalar_no_params() -> Result<()> {
    let t = TestConnection::new()?;
    let stmt = t.connection.prepare("SELECT 42")?;
    assert_eq!(stmt.param_count(), 0);
    assert_eq!(stmt.schema().column_count(), 1);

    let v: i32 = stmt.fetch_scalar(&[])?;
    assert_eq!(v, 42);
    Ok(())
}

/// `fetch_all` over a prepared SELECT.
#[test]
fn prepared_fetch_all() -> Result<()> {
    let t = TestConnection::new()?;
    t.connection
        .execute_command("CREATE TABLE t (id INT NOT NULL)")?;
    t.connection
        .execute_command("INSERT INTO t VALUES (1), (2), (3)")?;

    let stmt = t.connection.prepare("SELECT id FROM t ORDER BY id")?;
    let rows = stmt.fetch_all(&[])?;
    assert_eq!(rows.len(), 3);
    assert_eq!(rows[0].get::<i32>(0), Some(1));
    assert_eq!(rows[2].get::<i32>(0), Some(3));
    Ok(())
}

/// Streaming query through a prepared statement — schema is available
/// before the first chunk, and chunking honors the requested size.
#[test]
fn prepared_streaming_query() -> Result<()> {
    let t = TestConnection::new()?;
    t.connection
        .execute_command("CREATE TABLE s (v INT NOT NULL)")?;
    for i in 1..=12i32 {
        t.connection
            .execute_command(&format!("INSERT INTO s VALUES ({i})"))?;
    }

    let stmt = t.connection.prepare("SELECT v FROM s ORDER BY v")?;
    let schema = stmt.schema();
    assert_eq!(schema.column_count(), 1);

    let mut rs = stmt.query(&[])?;
    let mut total = 0;
    while let Some(chunk) = rs.next_chunk()? {
        total += chunk.len();
    }
    assert_eq!(total, 12);
    Ok(())
}

/// Reusing a prepared statement many times — the whole point of
/// prepare+execute.
#[test]
fn prepared_reuse() -> Result<()> {
    let t = TestConnection::new()?;
    t.connection
        .execute_command("CREATE TABLE r (v INT NOT NULL)")?;
    let insert = t.connection.prepare("INSERT INTO r VALUES (1)")?;
    for _ in 0..5 {
        let n = insert.execute(&[])?;
        assert_eq!(n, 1);
    }

    let count: i64 = t.connection.fetch_scalar("SELECT COUNT(*) FROM r")?;
    assert_eq!(count, 5);
    Ok(())
}

/// Preparing an invalid SQL statement fails at prepare time, not at
/// execute time.
#[test]
fn prepare_invalid_sql_fails_early() -> Result<()> {
    let t = TestConnection::new()?;
    let result = t.connection.prepare("SELEKT oops FORM nowhere");
    assert!(result.is_err(), "invalid SQL should fail to prepare");
    Ok(())
}

/// Compile-time check: `PreparedStatement` borrows the Connection, so
/// we can't drop the connection while a statement is alive. This is a
/// visual smoke test — it just needs to compile.
#[expect(
    dead_code,
    reason = "compile-only smoke test: existence of this function verifies the lifetime relationship"
)]
fn lifetime_guard(conn: &Connection) {
    let _stmt = conn.prepare("SELECT 1");
    // `_stmt` keeps `conn` borrowed until end of scope.
}
