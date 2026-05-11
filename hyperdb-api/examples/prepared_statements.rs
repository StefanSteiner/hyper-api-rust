// Copyright (c) 2026, Salesforce, Inc. All rights reserved.
// SPDX-License-Identifier: Apache-2.0 OR MIT

//! Demonstrates the `Connection::prepare()` API for reusing a single
//! parsed plan across many executions.
//!
//! Prepared statements are the right tool when the same SQL is
//! executed many times with different parameter values — the server
//! parses and plans the statement once and executes it repeatedly
//! against the cached plan.

use hyperdb_api::{Connection, CreateMode, HyperProcess, Result};
use hyperdb_api_core::types::oids;

fn main() -> Result<()> {
    let hyper = HyperProcess::new(None, None)?;
    let endpoint = hyper.require_endpoint()?;
    let db_path = std::env::temp_dir().join("prepared_statements_example.hyper");
    if db_path.exists() {
        std::fs::remove_file(&db_path).ok();
    }

    let conn = Connection::connect(
        endpoint,
        db_path.to_str().expect("utf-8 path"),
        CreateMode::CreateIfNotExists,
    )?;

    conn.execute_command("CREATE TABLE users (id INT NOT NULL, name TEXT)")?;

    // Prepare an INSERT statement once and execute it many times.
    // The explicit OID list tells the server the parameter types up
    // front — necessary for `INSERT ... VALUES ($1, $2)` where the
    // server can't otherwise infer the types.
    let insert = conn.prepare_typed(
        "INSERT INTO users VALUES ($1, $2)",
        &[oids::INT, oids::TEXT],
    )?;
    for (id, name) in &[(1i32, "Alice"), (2, "Bob"), (3, "Carol"), (4, "Dave")] {
        insert.execute(&[id, name])?;
    }

    // Prepare a SELECT statement with a parameter and reuse it across
    // multiple point-lookups. `prepare` (no `_typed`) works here
    // because the server can infer $1 = INT from the `WHERE id = $1`
    // context.
    let lookup = conn.prepare_typed("SELECT name FROM users WHERE id = $1", &[oids::INT])?;
    for id in [1i32, 3, 4] {
        let name: String = lookup.fetch_scalar(&[&id])?;
        println!("user {id}: {name}");
    }

    // fetch_all is available on the PreparedStatement itself.
    let list_all = conn.prepare("SELECT id, name FROM users ORDER BY id")?;
    let rows = list_all.fetch_all(&[])?;
    println!("total users: {}", rows.len());

    conn.close()?;
    Ok(())
}
