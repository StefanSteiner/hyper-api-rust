// Copyright (c) 2026, Salesforce, Inc. All rights reserved.
// SPDX-License-Identifier: Apache-2.0 OR MIT

//! Pre-flight smoke battery for cross-database SQL shapes that the
//! "remove v1 limitations" PR plumbs through ingest/merge/export.
//!
//! Plan: `HYPERDB_MCP_REMOVE_V1_LIMITATIONS_PLAN.md` §"Pre-flight smoke
//! battery". Each test verifies one SQL shape against a *user-attached
//! writable* database. If any shape fails, the iter-1 merge-in-target-DB
//! design is dead and the plan needs to redesign before plumbing.
//!
//! Marked `#[ignore]` so they don't run on every `cargo test` — invoke
//! explicitly via `cargo test -p hyperdb-mcp --test cross_db_dml_smoke
//! -- --ignored`.

use hyperdb_mcp::attach::{AttachRegistry, AttachRequest, AttachSource, OnMissing};
use hyperdb_mcp::engine::Engine;
use tempfile::TempDir;

/// Build a primary workspace plus an attached writable database under
/// alias `"smoke"` pointing at a freshly-created `.hyper` file. Returns
/// `(engine, registry, _dir)` — drop order matters: registry → engine →
/// dir, so the temp directory outlives the engine.
fn setup() -> (Engine, AttachRegistry, TempDir) {
    let dir = TempDir::new().unwrap();
    let primary_path = dir.path().join("primary.hyper");
    let attached_path = dir.path().join("attached.hyper");

    let engine = Engine::new_no_daemon(Some(primary_path.to_string_lossy().into())).unwrap();
    let registry = AttachRegistry::new();
    registry
        .attach(
            &engine,
            AttachRequest {
                alias: "smoke".into(),
                source: AttachSource::LocalFile {
                    path: attached_path,
                },
                writable: true,
                on_missing: OnMissing::Create,
            },
        )
        .unwrap();
    (engine, registry, dir)
}

/// Shape 1: `CREATE TABLE "alias"."public"."tmp" AS SELECT ...` — the
/// merge path's temp-table strategy depends on this.
#[test]
#[ignore = "pre-flight smoke; run with --ignored"]
fn cross_db_ctas_into_attached_writable() {
    let (engine, _reg, _dir) = setup();
    engine
        .execute_command(
            "CREATE TABLE \"smoke\".\"public\".\"tmp\" AS \
             SELECT * FROM (VALUES (1, 'a'), (2, 'b')) AS v(k, val)",
        )
        .expect("CTAS into attached writable DB must succeed");

    let rows = engine
        .execute_query_to_json("SELECT k, val FROM \"smoke\".\"public\".\"tmp\" ORDER BY k")
        .unwrap();
    assert_eq!(rows.len(), 2);
    assert_eq!(rows[0]["k"], 1);
    assert_eq!(rows[1]["val"], "b");
}

/// Shape 2: qualified `DELETE ... USING ...` where target and source are
/// both in the attached DB — the merge path's row-by-key delete step.
#[test]
#[ignore = "pre-flight smoke; run with --ignored"]
fn cross_db_qualified_delete_using() {
    let (engine, _reg, _dir) = setup();

    engine
        .execute_command("CREATE TABLE \"smoke\".\"public\".\"t\" (k INT, v TEXT)")
        .unwrap();
    engine
        .execute_command(
            "INSERT INTO \"smoke\".\"public\".\"t\" VALUES (1, 'old'), (2, 'keep'), (3, 'old')",
        )
        .unwrap();

    engine
        .execute_command(
            "CREATE TABLE \"smoke\".\"public\".\"tmp\" AS \
             SELECT * FROM (VALUES (1), (3)) AS v(k)",
        )
        .unwrap();

    engine
        .execute_command(
            "DELETE FROM \"smoke\".\"public\".\"t\" t \
             USING \"smoke\".\"public\".\"tmp\" s \
             WHERE t.k = s.k",
        )
        .expect("qualified DELETE-USING must succeed across same attached DB");

    let rows = engine
        .execute_query_to_json("SELECT k, v FROM \"smoke\".\"public\".\"t\" ORDER BY k")
        .unwrap();
    assert_eq!(rows.len(), 1, "rows 1 and 3 should be deleted");
    assert_eq!(rows[0]["v"], "keep");
}

/// Shape 3: qualified `INSERT ... SELECT` — the merge path's append step
/// after the delete.
#[test]
#[ignore = "pre-flight smoke; run with --ignored"]
fn cross_db_qualified_insert_select() {
    let (engine, _reg, _dir) = setup();

    engine
        .execute_command("CREATE TABLE \"smoke\".\"public\".\"t\" (k INT, v TEXT)")
        .unwrap();
    engine
        .execute_command(
            "CREATE TABLE \"smoke\".\"public\".\"tmp\" AS \
             SELECT * FROM (VALUES (1, 'a'), (2, 'b')) AS v(k, v)",
        )
        .unwrap();

    engine
        .execute_command(
            "INSERT INTO \"smoke\".\"public\".\"t\" \
             SELECT * FROM \"smoke\".\"public\".\"tmp\"",
        )
        .expect("qualified INSERT-SELECT must succeed");

    let rows = engine
        .execute_query_to_json("SELECT k, v FROM \"smoke\".\"public\".\"t\" ORDER BY k")
        .unwrap();
    assert_eq!(rows.len(), 2);
}

/// Shape 4: qualified `ALTER TABLE ... ADD COLUMN` — the merge path's
/// new-column promotion step when the incoming schema is wider.
#[test]
#[ignore = "pre-flight smoke; run with --ignored"]
fn cross_db_qualified_alter_add_column() {
    let (engine, _reg, _dir) = setup();

    engine
        .execute_command("CREATE TABLE \"smoke\".\"public\".\"t\" (k INT)")
        .unwrap();

    engine
        .execute_command("ALTER TABLE \"smoke\".\"public\".\"t\" ADD COLUMN extra TEXT")
        .expect("qualified ALTER ADD COLUMN must succeed");

    engine
        .execute_command("INSERT INTO \"smoke\".\"public\".\"t\" VALUES (1, 'x')")
        .unwrap();
    let rows = engine
        .execute_query_to_json("SELECT extra FROM \"smoke\".\"public\".\"t\"")
        .unwrap();
    assert_eq!(rows[0]["extra"], "x");
}

/// Shape 6 (Iter 4): `CREATE TABLE … (col TEXT PRIMARY KEY, …)` against
/// an attached writable DB.
///
/// **Result: rejected by Hyper with "Index support is disabled (0A000)".**
/// Hyper backs PKs with indexes, which aren't supported in the bundled
/// build. PK-based atomic upsert is therefore not viable; Iter 4
/// designs around it via a Rust-side per-(alias, table_name) Mutex
/// plus DELETE+INSERT in a transaction (the existing pattern, just
/// extended cross-DB).
#[test]
#[ignore = "pre-flight smoke; run with --ignored — DOCUMENTS that Hyper rejects PK"]
fn cross_db_create_table_with_primary_key_is_rejected() {
    let (engine, _reg, _dir) = setup();

    let err = engine
        .execute_command(
            "CREATE TABLE \"smoke\".\"public\".\"with_pk\" \
             (k TEXT PRIMARY KEY, v INT)",
        )
        .expect_err("Hyper must reject PK creation — design depends on this");
    assert!(
        err.message.contains("Index support is disabled")
            || err.message.contains("not implemented")
            || err.message.contains("not supported"),
        "expected an index/PK rejection, got: {}",
        err.message
    );
}

/// Shape 7 (Iter 4): `ALTER TABLE … ADD CONSTRAINT … PRIMARY KEY` against
/// an attached writable DB.
///
/// **Result: rejected by Hyper with "named constraints not implemented
/// yet (0A000)".** Confirms that PK migration via ALTER is also a dead
/// end. Iter 4 falls back to non-PK design — see the plan.
#[test]
#[ignore = "pre-flight smoke; run with --ignored — DOCUMENTS that Hyper rejects ALTER ADD CONSTRAINT"]
fn cross_db_alter_add_primary_key_is_rejected() {
    let (engine, _reg, _dir) = setup();

    engine
        .execute_command("CREATE TABLE \"smoke\".\"public\".\"add_pk\" (k TEXT, v INT)")
        .unwrap();
    let err = engine
        .execute_command(
            "ALTER TABLE \"smoke\".\"public\".\"add_pk\" \
             ADD CONSTRAINT add_pk_pk PRIMARY KEY (k)",
        )
        .expect_err("Hyper must reject ADD CONSTRAINT");
    assert!(
        err.message.contains("not implemented")
            || err.message.contains("Index support is disabled"),
        "expected a not-implemented/index rejection, got: {}",
        err.message
    );
}

/// Shape 8 (Iter 4): `INSERT … ON CONFLICT … DO UPDATE` against an
/// attached writable DB.
///
/// **Result: rejected because ON CONFLICT requires a unique index on
/// the conflict columns, which Hyper doesn't support.** Atomic upsert
/// in a single statement is therefore not viable.
#[test]
#[ignore = "pre-flight smoke; run with --ignored — DOCUMENTS that ON CONFLICT is unavailable"]
fn cross_db_insert_on_conflict_is_rejected() {
    let (engine, _reg, _dir) = setup();

    engine
        .execute_command("CREATE TABLE \"smoke\".\"public\".\"upsert\" (k TEXT, v INT, prose TEXT)")
        .unwrap();
    engine
        .execute_command("INSERT INTO \"smoke\".\"public\".\"upsert\" VALUES ('a', 1, 'hello')")
        .unwrap();

    let err = engine
        .execute_command(
            "INSERT INTO \"smoke\".\"public\".\"upsert\" (k, v, prose) \
             VALUES ('a', 99, 'overwritten') \
             ON CONFLICT (k) DO UPDATE SET v = EXCLUDED.v",
        )
        .expect_err("Hyper must reject ON CONFLICT — syntax not supported");
    // Hyper rejects ON CONFLICT at the *parser* layer ("syntax error: got
    // ON, expected FETCH, FOR, LIMIT, OFFSET") — the dialect doesn't
    // include the upsert grammar at all, regardless of index support.
    assert!(
        err.message.contains("syntax error")
            || err.message.contains("Index support is disabled")
            || err.message.contains("not implemented"),
        "expected a syntax/index rejection, got: {}",
        err.message
    );
}

/// Shape 5: qualified `pg_catalog.pg_tables` and column-introspection
/// probes against the attached DB — the basis for `table_exists_in` and
/// `column_metadata_in`.
#[test]
#[ignore = "pre-flight smoke; run with --ignored"]
fn cross_db_qualified_pg_catalog_probes() {
    let (engine, _reg, _dir) = setup();

    engine
        .execute_command("CREATE TABLE \"smoke\".\"public\".\"probe_me\" (id INT, label TEXT)")
        .unwrap();

    let table_rows = engine
        .execute_query_to_json(
            "SELECT tablename FROM \"smoke\".pg_catalog.pg_tables \
             WHERE schemaname = 'public' AND tablename = 'probe_me'",
        )
        .expect("qualified pg_tables probe must succeed");
    assert_eq!(
        table_rows.len(),
        1,
        "table 'probe_me' must be visible via attached pg_catalog"
    );

    let column_rows = engine
        .execute_query_to_json(
            "SELECT a.attname AS name, t.typname AS type \
             FROM \"smoke\".pg_catalog.pg_attribute a \
             JOIN \"smoke\".pg_catalog.pg_class c ON a.attrelid = c.oid \
             JOIN \"smoke\".pg_catalog.pg_type t ON a.atttypid = t.oid \
             JOIN \"smoke\".pg_catalog.pg_namespace n ON c.relnamespace = n.oid \
             WHERE n.nspname = 'public' AND c.relname = 'probe_me' AND a.attnum > 0 \
             ORDER BY a.attnum",
        )
        .expect("qualified pg_attribute/pg_type join must succeed");
    assert_eq!(column_rows.len(), 2);
    assert_eq!(column_rows[0]["name"], "id");
    assert_eq!(column_rows[1]["name"], "label");
}
