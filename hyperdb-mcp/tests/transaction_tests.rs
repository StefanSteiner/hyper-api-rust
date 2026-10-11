// Copyright (c) 2026, Salesforce, Inc. All rights reserved.
// SPDX-License-Identifier: Apache-2.0 OR MIT

//! Tests that verify ingest operations are transactional: a failure
//! midway through a batch leaves zero rows behind.

mod common;

use common::TestEngine;
use hyperdb_mcp::engine::Engine;
use hyperdb_mcp::ingest::{IngestOptions, ingest_json};
use serde_json::Value;

/// Run a SELECT and return the rows. A known pre-existing wire-protocol
/// desync in hyperdb-api occasionally returns an empty first response right
/// after a Hyper-level error (e.g. a failed INSERT inside a transaction).
/// Retrying once is enough to flush the stale state.
fn query_resilient(engine: &Engine, sql: &str) -> Vec<Value> {
    let first = engine.execute_query_to_json(sql).unwrap();
    if first.is_empty() {
        engine.execute_query_to_json(sql).unwrap()
    } else {
        first
    }
}

/// Sanity check: the transaction helper itself rolls back correctly on
/// error. Inserts one row successfully, then triggers an error, then
/// verifies the row is gone.
#[test]
fn execute_in_transaction_rolls_back_on_error() {
    use hyperdb_mcp::error::{ErrorCode, McpError};
    let mut te = TestEngine::new_ephemeral();
    te.engine
        .execute_command("CREATE TABLE direct (v INT)")
        .unwrap();

    let result: Result<(), McpError> = te.engine.execute_in_transaction(|txn| {
        txn.execute_command("INSERT INTO direct VALUES (1)")?;
        txn.execute_command("INSERT INTO direct VALUES (2)")?;
        Err(McpError::new(ErrorCode::InternalError, "simulated failure"))
    });
    assert!(result.is_err());

    let rows = te
        .engine
        .execute_query_to_json("SELECT COUNT(*) as cnt FROM direct")
        .unwrap();
    let count = rows[0]["cnt"].as_i64().unwrap();
    assert_eq!(count, 0, "rollback should leave zero rows");
}

/// Sanity check: successful commits stick.
#[test]
fn execute_in_transaction_commits_on_success() {
    let mut te = TestEngine::new_ephemeral();
    te.engine
        .execute_command("CREATE TABLE direct (v INT)")
        .unwrap();

    te.engine
        .execute_in_transaction(|txn| {
            txn.execute_command("INSERT INTO direct VALUES (10)")?;
            txn.execute_command("INSERT INTO direct VALUES (20)")?;
            Ok(())
        })
        .unwrap();

    let rows = te
        .engine
        .execute_query_to_json("SELECT COUNT(*) as cnt FROM direct")
        .unwrap();
    let count = rows[0]["cnt"].as_i64().unwrap();
    assert_eq!(count, 2);
}

/// The regression test for the RAII migration: an `Ok` closure must
/// **commit**, not silently roll back when the guard drops.
///
/// `execute_in_transaction_commits_on_success` reads the rows back on the
/// same session that wrote them, which an uncommitted-but-still-open
/// transaction would also satisfy. This test closes that hole by forcing
/// the write to be durable past the end of its own transaction: a second,
/// independent transaction rolls back, and the first batch must survive.
/// Swap the `txn.commit()?` in `execute_in_transaction` for a bare drop and
/// this fails while the simpler test still passes.
#[test]
fn execute_in_transaction_commit_outlives_its_transaction() {
    use hyperdb_mcp::error::{ErrorCode, McpError};
    let mut te = TestEngine::new_ephemeral();
    te.engine
        .execute_command("CREATE TABLE direct (v INT)")
        .unwrap();

    te.engine
        .execute_in_transaction(|txn| txn.execute_command("INSERT INTO direct VALUES (10)"))
        .unwrap();

    // A second transaction that aborts. If the first one had never
    // committed, its row would be discarded here along with this one.
    let result: Result<(), McpError> = te.engine.execute_in_transaction(|txn| {
        txn.execute_command("INSERT INTO direct VALUES (20)")?;
        Err(McpError::new(ErrorCode::InternalError, "simulated failure"))
    });
    assert!(result.is_err());

    let rows = query_resilient(&te.engine, "SELECT v FROM direct ORDER BY v");
    assert_eq!(rows.len(), 1, "committed row must survive a later rollback");
    assert_eq!(rows[0]["v"].as_i64().unwrap(), 10);
}

/// Every exit path must leave the session with no transaction open.
///
/// The guard's whole job is discharging the `BEGIN`/`COMMIT`-or-`ROLLBACK`
/// pairing obligation. If any path leaked one, the *next* `BEGIN` would
/// fail with "transaction already in progress" on a connection that is
/// otherwise healthy — and the server's `ConnectionLost` auto-reconnect
/// would not rescue it, because the connection is live. This walks
/// commit → error → panic → commit on one engine and asserts each
/// subsequent transaction still opens.
#[test]
fn execute_in_transaction_never_leaks_an_open_transaction() {
    use hyperdb_mcp::error::{ErrorCode, McpError};
    let mut te = TestEngine::new_ephemeral();
    te.engine
        .execute_command("CREATE TABLE direct (v INT)")
        .unwrap();

    // 1. Commit path.
    te.engine
        .execute_in_transaction(|txn| txn.execute_command("INSERT INTO direct VALUES (1)"))
        .expect("commit path");

    // 2. Error path — must not wedge the session for the next BEGIN.
    let err: Result<(), McpError> = te.engine.execute_in_transaction(|txn| {
        txn.execute_command("INSERT INTO direct VALUES (2)")?;
        Err(McpError::new(ErrorCode::InternalError, "simulated failure"))
    });
    assert!(err.is_err());

    // 3. Panic path — rollback happens in `Drop` as the unwind passes.
    let outcome = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        te.engine.execute_in_transaction::<_, ()>(|txn| {
            txn.execute_command("INSERT INTO direct VALUES (3)")?;
            panic!("simulated closure bug");
        })
    }));
    assert!(outcome.is_err(), "panic should propagate out");

    // 4. If any of the three leaked a `BEGIN`, this one fails.
    te.engine
        .execute_in_transaction(|txn| txn.execute_command("INSERT INTO direct VALUES (4)"))
        .expect("session must still accept BEGIN after commit/error/panic");

    let rows = query_resilient(&te.engine, "SELECT v FROM direct ORDER BY v");
    let values: Vec<i64> = rows.iter().map(|r| r["v"].as_i64().unwrap()).collect();
    assert_eq!(
        values,
        vec![1, 4],
        "only the two committed rows survive; the error and panic paths rolled back"
    );
}

/// A panic inside the transaction closure (e.g. an unwrap on None, array
/// indexing OOB, arithmetic overflow) must not leave an open transaction
/// on the connection. The RAII guard rolls back from its `Drop` as the
/// unwind passes through `execute_in_transaction`; without that, the next
/// operation would hit "transaction already in progress" and the engine
/// would be wedged until restart.
#[test]
fn execute_in_transaction_rolls_back_on_panic() {
    let mut te = TestEngine::new_ephemeral();
    te.engine
        .execute_command("CREATE TABLE direct (v INT)")
        .unwrap();

    // Trigger a panic inside the closure. `catch_unwind` at the
    // std::panic boundary lets the test assert the engine is still
    // usable afterwards without aborting the whole test binary.
    let outcome = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        te.engine.execute_in_transaction::<_, ()>(|txn| {
            txn.execute_command("INSERT INTO direct VALUES (1)")?;
            // Simulate a programmer error mid-transaction — any panic
            // will do. Using `panic!` directly keeps clippy from
            // second-guessing a synthetic `.unwrap()` on a literal.
            panic!("simulated closure bug");
        })
    }));
    assert!(outcome.is_err(), "panic should propagate out");

    // Engine must not be wedged — a fresh INSERT should succeed, and
    // the row from *before* the panic must have been rolled back.
    te.engine
        .execute_command("INSERT INTO direct VALUES (42)")
        .expect("engine must still be usable after panic inside tx");

    let rows = query_resilient(&te.engine, "SELECT v FROM direct ORDER BY v");
    assert_eq!(rows.len(), 1, "only the post-panic row should survive");
    assert_eq!(rows[0]["v"].as_i64().unwrap(), 42);
}

/// A failing JSON ingest in append mode must roll back all prior rows from
/// the same call. The target table is pre-created with a NOT NULL constraint;
/// our ingest JSON has three rows where the third violates the constraint
/// (missing the non-null key, which our INSERT emits as NULL).
#[test]
fn failed_json_ingest_rolls_back_partial_inserts() {
    let mut te = TestEngine::new_ephemeral();

    te.engine
        .execute_command("CREATE TABLE t (id INT NOT NULL, name TEXT)")
        .unwrap();

    let data = r#"[
        {"id": 1, "name": "Alice"},
        {"id": 2, "name": "Bob"},
        {"name": "NoId"}
    ]"#;

    let opts = IngestOptions {
        table: "t".into(),
        mode: hyperdb_mcp::ingest::IngestMode::Append,
        schema_override: None,
        target_db: None,
    };

    let result = ingest_json(&mut te.engine, data, &opts);
    assert!(result.is_err(), "ingest should fail on NOT NULL violation");

    // Crucially: the first two rows must have been rolled back. If
    // transactions were missing, we'd see count == 2 here.
    let rows = query_resilient(&te.engine, "SELECT COUNT(*) as cnt FROM t");
    let count = rows[0]["cnt"].as_i64().expect("count should be int");
    assert_eq!(
        count, 0,
        "expected rollback to leave zero rows, but found {count}"
    );
}

/// A successful ingest in replace mode commits both the DROP TABLE and all
/// INSERTs atomically — no observable intermediate state.
#[test]
fn successful_replace_commits_atomically() {
    let mut te = TestEngine::new_ephemeral();

    // Pre-populate the table with data that a replace-mode ingest should overwrite.
    te.engine
        .execute_command("CREATE TABLE t (id INT, name TEXT)")
        .unwrap();
    te.engine
        .execute_command("INSERT INTO t VALUES (99, 'Old')")
        .unwrap();

    let data = r#"[
        {"id": 1, "name": "Alice"},
        {"id": 2, "name": "Bob"}
    ]"#;
    let opts = IngestOptions {
        table: "t".into(),
        mode: hyperdb_mcp::ingest::IngestMode::Replace,
        schema_override: None,
        target_db: None,
    };
    let result = ingest_json(&mut te.engine, data, &opts).unwrap();
    assert_eq!(result.rows, 2);

    let rows = query_resilient(&te.engine, "SELECT COUNT(*) as cnt FROM t");
    assert_eq!(rows[0]["cnt"].as_i64().unwrap(), 2);

    // The old row is gone (replace mode drops and recreates).
    let rows = query_resilient(&te.engine, "SELECT COUNT(*) as cnt FROM t WHERE id = 99");
    assert_eq!(rows[0]["cnt"].as_i64().unwrap(), 0);
}

/// The canonical upsert pattern (UPDATE + INSERT WHERE NOT EXISTS) wrapped
/// in a transaction. When the row exists, the UPDATE applies; when it
/// doesn't, the INSERT fires. Both statements commit atomically.
///
/// This is the use case that motivated the `execute` tool's array shape:
/// Hyper has no `ON CONFLICT`, so users build upserts as two SQL
/// statements that must run atomically. Here we exercise the same shape
/// at the engine level — the MCP handler is a thin wrapper around this.
#[test]
fn batched_upsert_inserts_when_row_missing() {
    let mut te = TestEngine::new_ephemeral();
    te.engine
        .execute_command("CREATE TABLE settings (key TEXT NOT NULL, value TEXT NOT NULL)")
        .unwrap();

    te.engine
        .execute_in_transaction(|txn| {
            txn.execute_command("UPDATE settings SET value = 'dark' WHERE key = 'theme'")?;
            txn.execute_command(
                "INSERT INTO settings (key, value) SELECT 'theme', 'dark' \
                 WHERE NOT EXISTS (SELECT 1 FROM settings WHERE key = 'theme')",
            )?;
            Ok(())
        })
        .unwrap();

    let rows = query_resilient(&te.engine, "SELECT value FROM settings WHERE key = 'theme'");
    assert_eq!(rows.len(), 1);
    assert_eq!(rows[0]["value"].as_str().unwrap(), "dark");
}

#[test]
fn batched_upsert_updates_when_row_exists() {
    let mut te = TestEngine::new_ephemeral();
    te.engine
        .execute_command("CREATE TABLE settings (key TEXT NOT NULL, value TEXT NOT NULL)")
        .unwrap();
    te.engine
        .execute_command("INSERT INTO settings (key, value) VALUES ('theme', 'light')")
        .unwrap();

    te.engine
        .execute_in_transaction(|txn| {
            txn.execute_command("UPDATE settings SET value = 'dark' WHERE key = 'theme'")?;
            txn.execute_command(
                "INSERT INTO settings (key, value) SELECT 'theme', 'dark' \
                 WHERE NOT EXISTS (SELECT 1 FROM settings WHERE key = 'theme')",
            )?;
            Ok(())
        })
        .unwrap();

    let rows = query_resilient(
        &te.engine,
        "SELECT COUNT(*) as cnt FROM settings WHERE key = 'theme'",
    );
    // Exactly one row, not two — UPDATE applied, INSERT's WHERE NOT EXISTS
    // suppressed it.
    assert_eq!(rows[0]["cnt"].as_i64().unwrap(), 1);

    let rows = query_resilient(&te.engine, "SELECT value FROM settings WHERE key = 'theme'");
    assert_eq!(rows[0]["value"].as_str().unwrap(), "dark");
}

/// Mid-batch failure rolls back ALL prior statements in the transaction.
/// Without atomicity, the first INSERT would be visible after the second
/// one fails — leaving the table in a state the user never asked for.
#[test]
fn batched_multi_table_mutation_rolls_back_on_second_failure() {
    let mut te = TestEngine::new_ephemeral();
    te.engine
        .execute_command("CREATE TABLE orders (id INT NOT NULL, customer_id INT)")
        .unwrap();
    te.engine
        .execute_command("CREATE TABLE customers (id INT NOT NULL, total_orders INT NOT NULL)")
        .unwrap();
    te.engine
        .execute_command("INSERT INTO customers VALUES (42, 0)")
        .unwrap();

    use hyperdb_mcp::error::McpError;
    let result: Result<(), McpError> = te.engine.execute_in_transaction(|txn| {
        txn.execute_command("INSERT INTO orders (id, customer_id) VALUES (1001, 42)")?;
        // This second statement violates NOT NULL on `id` because we
        // omit it — the entire batch must roll back.
        txn.execute_command("INSERT INTO orders (customer_id) VALUES (42)")?;
        txn.execute_command("UPDATE customers SET total_orders = total_orders + 2 WHERE id = 42")?;
        Ok(())
    });
    assert!(result.is_err(), "batch should fail on NOT NULL violation");

    // The first INSERT must have been rolled back.
    let rows = query_resilient(&te.engine, "SELECT COUNT(*) as cnt FROM orders");
    assert_eq!(
        rows[0]["cnt"].as_i64().unwrap(),
        0,
        "first INSERT must have rolled back — atomicity violated"
    );
    // The customers table must not have been touched either.
    let rows = query_resilient(
        &te.engine,
        "SELECT total_orders FROM customers WHERE id = 42",
    );
    assert_eq!(rows[0]["total_orders"].as_i64().unwrap(), 0);
}

/// Replace-mode ingest with a mid-flight INSERT failure leaves the new
/// (empty) table behind, not partial rows.
///
/// Note: Hyper treats DDL (DROP TABLE, CREATE TABLE) as auto-committed
/// even inside a transaction, so replace-mode ingest cannot restore the
/// original table if it fails. The DDL is already durable by the time
/// the INSERTs run. What we can and do guarantee is that partial INSERTs
/// are rolled back, so the post-failure table is empty rather than
/// partially populated.
#[test]
fn failed_replace_leaves_empty_table_not_partial() {
    let mut te = TestEngine::new_ephemeral();

    te.engine
        .execute_command("CREATE TABLE t (id INT, name TEXT)")
        .unwrap();
    te.engine
        .execute_command("INSERT INTO t VALUES (100, 'Original')")
        .unwrap();

    // Override forces id to INT. Second row's string value will fail to
    // INSERT because it's sent as a quoted string and the implicit cast
    // 'bogus' -> INT fails at the server.
    let data = r#"[
        {"id": 1},
        {"id": "bogus"}
    ]"#;
    let mut override_map = serde_json::Map::new();
    override_map.insert("id".into(), serde_json::Value::String("INT".into()));
    let opts = IngestOptions {
        table: "t".into(),
        mode: hyperdb_mcp::ingest::IngestMode::Replace,
        schema_override: Some(override_map),
        target_db: None,
    };

    let result = ingest_json(&mut te.engine, data, &opts);
    assert!(result.is_err(), "ingest should fail on type cast error");

    // The pre-failure INSERT was rolled back — the table exists but has
    // zero rows, not one. (The original 'Original' row is gone due to
    // auto-committed DDL, so we can't get it back either way.)
    let rows = query_resilient(&te.engine, "SELECT COUNT(*) as cnt FROM t");
    let count = rows[0]["cnt"].as_i64().expect("count should be int");
    assert_eq!(
        count, 0,
        "expected partial INSERTs to be rolled back, leaving 0 rows"
    );
}

// ---------------------------------------------------------------------
// load_file merge atomicity (D1): the DELETE of matched rows and the
// INSERT of the incoming rows run in one transaction, and a failed merge
// leaves the target's rows *and columns* untouched.
// ---------------------------------------------------------------------

mod merge_atomicity {
    use super::{TestEngine, query_resilient};
    use hyperdb_mcp::error::ErrorCode;
    use hyperdb_mcp::ingest::{IngestMode, IngestOptions, ingest_csv, ingest_json};

    fn opts(table: &str, mode: IngestMode, target_db: Option<&str>) -> IngestOptions {
        IngestOptions {
            table: table.into(),
            mode,
            schema_override: None,
            target_db: target_db.map(str::to_string),
        }
    }

    fn merge_on_id() -> IngestMode {
        IngestMode::Merge {
            keys: vec!["id".into()],
        }
    }

    /// Seed `t` with (1,a),(2,b); `name` is NOT NULL by JSON inference.
    fn seed(te: &mut TestEngine, target_db: Option<&str>) {
        ingest_json(
            &mut te.engine,
            r#"[{"id":1,"name":"a"},{"id":2,"name":"b"}]"#,
            &opts("t", IngestMode::Replace, target_db),
        )
        .unwrap();
    }

    fn qualified(target_db: Option<&str>) -> String {
        match target_db {
            Some(db) => format!("\"{db}\".\"public\".\"t\""),
            None => "t".into(),
        }
    }

    fn rows_of(te: &TestEngine, target_db: Option<&str>) -> Vec<(i64, String)> {
        query_resilient(
            &te.engine,
            &format!("SELECT id, name FROM {} ORDER BY id", qualified(target_db)),
        )
        .iter()
        .map(|r| {
            (
                r["id"].as_i64().unwrap(),
                r["name"].as_str().unwrap().to_string(),
            )
        })
        .collect()
    }

    fn original() -> Vec<(i64, String)> {
        vec![(1, "a".into()), (2, "b".into())]
    }

    fn column_names(te: &TestEngine, target_db: Option<&str>) -> Vec<String> {
        te.engine
            .column_metadata_in(target_db, "t")
            .unwrap()
            .into_iter()
            .map(|c| c.name)
            .collect()
    }

    fn scratch_tables(te: &TestEngine) -> Vec<String> {
        te.engine
            .execute_query_to_json(
                "SELECT tablename FROM pg_catalog.pg_tables WHERE schemaname = 'public'",
            )
            .unwrap()
            .iter()
            .filter_map(|v| v["tablename"].as_str().map(str::to_string))
            .filter(|n| n.starts_with("__hyperdb_"))
            .collect()
    }

    #[test]
    fn merge_null_into_not_null_keeps_matched_rows() {
        let mut te = TestEngine::new_ephemeral();
        seed(&mut te, None);
        let res = ingest_json(
            &mut te.engine,
            r#"[{"id":1,"name":null},{"id":3,"name":"c"}]"#,
            &opts("t", merge_on_id(), None),
        );
        assert!(res.is_err(), "NULL into NOT NULL must fail");
        assert_eq!(rows_of(&te, None), original(), "matched row 1 was lost");
    }

    #[test]
    fn merge_csv_null_into_not_null_keeps_matched_rows() {
        let mut te = TestEngine::new_ephemeral();
        seed(&mut te, None);
        let res = ingest_csv(
            &mut te.engine,
            "id,name\n1,\n3,c\n",
            &opts("t", merge_on_id(), None),
        );
        assert!(res.is_err(), "blank CSV name into NOT NULL must fail");
        assert_eq!(rows_of(&te, None), original(), "matched row 1 was lost");
    }

    #[test]
    fn failed_merge_rolls_back_added_column() {
        let mut te = TestEngine::new_ephemeral();
        seed(&mut te, None);
        let res = ingest_json(
            &mut te.engine,
            r#"[{"id":1,"name":null,"extra":5}]"#,
            &opts("t", merge_on_id(), None),
        );
        assert!(res.is_err());
        assert_eq!(rows_of(&te, None), original());
        assert_eq!(column_names(&te, None), vec!["id", "name"]);
    }

    #[test]
    fn merge_missing_not_null_column_is_rejected_up_front() {
        let mut te = TestEngine::new_ephemeral();
        seed(&mut te, None);
        let err = ingest_json(
            &mut te.engine,
            r#"[{"id":1,"extra":5}]"#,
            &opts("t", merge_on_id(), None),
        )
        .unwrap_err();
        assert_eq!(err.code, ErrorCode::InvalidArgument, "{}", err.message);
        assert!(err.message.contains("name"), "{}", err.message);
        assert_eq!(rows_of(&te, None), original());
        assert_eq!(column_names(&te, None), vec!["id", "name"]);
    }

    #[test]
    fn merge_failure_in_persistent_target_keeps_rows() {
        let mut te = TestEngine::new_ephemeral();
        seed(&mut te, Some("persistent"));
        let res = ingest_json(
            &mut te.engine,
            r#"[{"id":1,"name":null,"extra":5},{"id":3,"name":"c"}]"#,
            &opts("t", merge_on_id(), Some("persistent")),
        );
        assert!(res.is_err());
        assert_eq!(rows_of(&te, Some("persistent")), original());
        assert_eq!(column_names(&te, Some("persistent")), vec!["id", "name"]);
    }

    #[test]
    fn failed_merge_leaves_no_staging_table() {
        let mut te = TestEngine::new_ephemeral();
        seed(&mut te, None);
        let _ = ingest_json(
            &mut te.engine,
            r#"[{"id":1,"name":null}]"#,
            &opts("t", merge_on_id(), None),
        );
        assert_eq!(scratch_tables(&te), Vec::<String>::new());
    }

    /// The staging load itself fails (a non-numeric value in an INT column):
    /// Hyper has already committed the staging DDL, so the guard must be armed
    /// before the load or the `__hyperdb_merge_*` table leaks.
    #[test]
    fn failed_staging_load_leaves_no_staging_table() {
        let mut te = TestEngine::new_ephemeral();
        seed(&mut te, None);
        let mut schema = serde_json::Map::new();
        schema.insert("id".into(), serde_json::json!("INT"));
        let res = ingest_csv(
            &mut te.engine,
            "id,name\nbogus,x\n",
            &IngestOptions {
                table: "t".into(),
                mode: merge_on_id(),
                schema_override: Some(schema),
                target_db: None,
            },
        );
        assert!(res.is_err(), "non-numeric id in an INT column must fail");
        assert_eq!(rows_of(&te, None), original());
        assert_eq!(scratch_tables(&te), Vec::<String>::new());
    }

    #[test]
    fn successful_merge_still_upserts() {
        let mut te = TestEngine::new_ephemeral();
        seed(&mut te, None);
        let r = ingest_json(
            &mut te.engine,
            r#"[{"id":2,"name":"B"},{"id":3,"name":"c"}]"#,
            &opts("t", merge_on_id(), None),
        )
        .unwrap();
        assert_eq!(r.rows, 2);
        assert_eq!(
            rows_of(&te, None),
            vec![(1, "a".into()), (2, "B".into()), (3, "c".into())]
        );
    }
}
