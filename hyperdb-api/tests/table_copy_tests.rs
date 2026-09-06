// Copyright (c) 2026, Salesforce, Inc. All rights reserved.
// SPDX-License-Identifier: Apache-2.0 OR MIT

//! Tests for constraint-preserving table copy.
//!
//! Each test pins one half of the contract: what `copy_table` reproduces, and
//! what it refuses to reproduce silently. The engine-capability tests double as
//! executable documentation of which constraint classes Hyper accepts at all —
//! if a future `hyperd` starts accepting `PRIMARY KEY` or `CHECK`, they fail and
//! point at the design note that needs revisiting.

use hyperdb_api::{Catalog, TableConstraint, UnpreservedReason, escape_sql_path};

mod common;
use common::{TestConnection, test_result_path};

/// Reads back `(column, not_null, has_default)` straight from `pg_catalog`,
/// deliberately bypassing the `Catalog` API so the assertions describe what is
/// actually in the file rather than what our own reflection reports.
fn raw_column_facts(
    test: &TestConnection,
    qualifier: &str,
    table: &str,
) -> Vec<(String, bool, bool)> {
    let sql = format!(
        "SELECT a.attname, CAST(a.attnotnull AS TEXT), CAST(a.atthasdef AS TEXT) \
         FROM {qualifier}pg_catalog.pg_attribute a \
         JOIN {qualifier}pg_catalog.pg_class c ON a.attrelid = c.oid \
         WHERE c.relname = '{table}' AND a.attnum > 0 ORDER BY a.attnum"
    );
    let mut rows = Vec::new();
    let mut result = test
        .connection
        .execute_query(&sql)
        .expect("column facts query failed");
    while let Some(chunk) = result.next_chunk().expect("chunk") {
        for row in &chunk {
            rows.push((
                row.get::<String>(0).unwrap_or_default(),
                row.get::<String>(1).unwrap_or_default() == "true",
                row.get::<String>(2).unwrap_or_default() == "true",
            ));
        }
    }
    rows
}

/// Reads back `(column, collation)` straight from `pg_catalog`, bypassing the
/// `Catalog` API for the same reason as [`raw_column_facts`].
///
/// An uncollated column reports the sentinel `default`, which is normalised to
/// `None` here — the engine refuses `COLLATE "default"` back, so it is a
/// read-only marker rather than a collation.
fn raw_collations(
    test: &TestConnection,
    qualifier: &str,
    table: &str,
) -> Vec<(String, Option<String>)> {
    let sql = format!(
        "SELECT a.attname, coll.collname \
         FROM {qualifier}pg_catalog.pg_attribute a \
         JOIN {qualifier}pg_catalog.pg_class c ON a.attrelid = c.oid \
         LEFT JOIN {qualifier}pg_catalog.pg_collation coll ON coll.oid = a.attcollation \
         WHERE c.relname = '{table}' AND a.attnum > 0 ORDER BY a.attnum"
    );
    let mut rows = Vec::new();
    let mut result = test
        .connection
        .execute_query(&sql)
        .expect("collation query failed");
    while let Some(chunk) = result.next_chunk().expect("chunk") {
        for row in &chunk {
            rows.push((
                row.get::<String>(0).unwrap_or_default(),
                row.get::<String>(1).filter(|name| name != "default"),
            ));
        }
    }
    rows
}

/// Reads back `(contype, column)` pairs from `pg_constraint`.
fn raw_constraint_facts(
    test: &TestConnection,
    qualifier: &str,
    table: &str,
) -> Vec<(String, String)> {
    let sql = format!(
        "SELECT CAST(con.contype AS TEXT), a.attname \
         FROM {qualifier}pg_catalog.pg_constraint con \
         JOIN {qualifier}pg_catalog.pg_class c ON con.conrelid = c.oid, \
         unnest(con.conkey) WITH ORDINALITY AS k(attnum, ord) \
         JOIN {qualifier}pg_catalog.pg_attribute a \
           ON a.attrelid = con.conrelid AND a.attnum = k.attnum \
         WHERE c.relname = '{table}' ORDER BY con.contype, con.conname, k.ord"
    );
    let mut rows = Vec::new();
    let mut result = test
        .connection
        .execute_query(&sql)
        .expect("constraint facts query failed");
    while let Some(chunk) = result.next_chunk().expect("chunk") {
        for row in &chunk {
            rows.push((
                row.get::<String>(0).unwrap_or_default(),
                row.get::<String>(1).unwrap_or_default(),
            ));
        }
    }
    rows
}

// ============================================================
// Engine capability baseline
// ============================================================

/// The constraint classes the issue asked us to preserve are rejected by the
/// engine outright, so no Hyper table can carry one to begin with.
#[test]
fn test_engine_rejects_unsupported_constraint_classes() {
    let test = TestConnection::new().expect("Failed to create test connection");

    for (label, ddl) in [
        ("PRIMARY KEY", "CREATE TABLE c_pk (a INTEGER PRIMARY KEY)"),
        ("UNIQUE", "CREATE TABLE c_uq (a INTEGER UNIQUE)"),
        ("CHECK", "CREATE TABLE c_ck (a INTEGER CHECK (a > 0))"),
        (
            "named constraint",
            "CREATE TABLE c_nm (a INTEGER NOT NULL, CONSTRAINT n ASSUMED UNIQUE (a))",
        ),
    ] {
        let err = test.execute_command(ddl).expect_err(&format!(
            "{label} unexpectedly accepted - revisit the design note"
        ));
        let msg = err.to_string();
        assert!(
            msg.contains("Index support is disabled")
                || msg.contains("check constraints not implemented yet")
                || msg.contains("named constraints not implemented yet"),
            "{label} rejected for an unexpected reason: {msg}"
        );
    }
}

/// `CTAS` is what the copy replaces, and this is the loss it causes.
#[test]
fn test_ctas_drops_constraints() {
    let test = TestConnection::new().expect("Failed to create test connection");

    test.execute_command(
        "CREATE TABLE src (id INTEGER NOT NULL, qty INTEGER DEFAULT 7, \
         ASSUMED PRIMARY KEY (id))",
    )
    .expect("create source");
    test.execute_command("CREATE TABLE ctas AS SELECT * FROM src")
        .expect("ctas");

    let facts = raw_column_facts(&test, "", "ctas");
    assert_eq!(facts.len(), 2);
    for (column, not_null, has_default) in &facts {
        assert!(!not_null, "CTAS unexpectedly kept NOT NULL on {column}");
        assert!(!has_default, "CTAS unexpectedly kept DEFAULT on {column}");
    }
    assert!(
        raw_constraint_facts(&test, "", "ctas").is_empty(),
        "CTAS unexpectedly kept constraints"
    );
}

// ============================================================
// Reflection
// ============================================================

#[test]
fn test_get_table_definition_reads_constraints_and_defaults() {
    let test = TestConnection::new().expect("Failed to create test connection");

    test.execute_command(
        "CREATE TABLE reflected (id INTEGER NOT NULL, code TEXT NOT NULL, \
         grp INTEGER NOT NULL, qty INTEGER DEFAULT 7, note TEXT DEFAULT 'n/a', \
         ASSUMED PRIMARY KEY (id), ASSUMED UNIQUE (code), ASSUMED UNIQUE (grp, code))",
    )
    .expect("create table");

    let def = Catalog::new(&test.connection)
        .get_table_definition("reflected")
        .expect("reflect");

    let column = |name: &str| {
        def.column_by_name(name)
            .unwrap_or_else(|| panic!("missing column {name}"))
            .clone()
    };
    assert!(!column("id").nullable);
    assert!(column("qty").nullable);
    assert_eq!(column("qty").default_expr(), Some("7"));
    assert_eq!(column("note").default_expr(), Some("'n/a'"));
    assert_eq!(column("id").default_expr(), None);

    let constraints = def.constraints();
    assert!(
        constraints.contains(&TableConstraint::AssumedPrimaryKey {
            columns: vec!["id".to_string()]
        }),
        "missing assumed primary key: {constraints:?}"
    );
    assert!(
        constraints.contains(&TableConstraint::AssumedUnique {
            columns: vec!["code".to_string()]
        }),
        "missing single-column assumed unique: {constraints:?}"
    );
    assert!(
        constraints.contains(&TableConstraint::AssumedUnique {
            columns: vec!["grp".to_string(), "code".to_string()]
        }),
        "composite assumed unique lost or reordered: {constraints:?}"
    );
}

// ============================================================
// Copy fidelity
// ============================================================

#[test]
fn test_copy_table_preserves_all_supported_classes() {
    let test = TestConnection::new().expect("Failed to create test connection");

    test.execute_command(
        "CREATE TABLE src (id INTEGER NOT NULL, code TEXT NOT NULL, \
         qty INTEGER DEFAULT 7, note TEXT DEFAULT 'n/a', \
         ASSUMED PRIMARY KEY (id), ASSUMED UNIQUE (code))",
    )
    .expect("create source");
    test.execute_command("INSERT INTO src (id, code) VALUES (1, 'a')")
        .expect("seed 1");
    test.execute_command("INSERT INTO src (id, code) VALUES (2, 'b')")
        .expect("seed 2");

    let report = Catalog::new(&test.connection)
        .copy_table("public.src", "public.dst")
        .expect("copy");

    assert_eq!(report.rows_copied, 2);
    assert_eq!(report.not_null_columns, 2);
    assert_eq!(report.default_columns, 2);
    assert_eq!(report.assumed_primary_keys, 1);
    assert_eq!(report.assumed_unique_constraints, 1);
    assert!(report.is_fully_preserved(), "{:?}", report.unpreserved);

    let facts = raw_column_facts(&test, "", "dst");
    assert_eq!(
        facts,
        vec![
            ("id".to_string(), true, false),
            ("code".to_string(), true, false),
            ("qty".to_string(), false, true),
            ("note".to_string(), false, true),
        ]
    );

    let constraints = raw_constraint_facts(&test, "", "dst");
    assert!(
        constraints.contains(&("p".to_string(), "id".to_string())),
        "assumed primary key not reproduced: {constraints:?}"
    );
    assert!(
        constraints.contains(&("u".to_string(), "code".to_string())),
        "assumed unique not reproduced: {constraints:?}"
    );
}

#[test]
fn test_copy_table_reports_non_portable_defaults() {
    let test = TestConnection::new().expect("Failed to create test connection");

    // NOW() reads back out of pg_attrdef database-qualified, so it cannot be
    // re-emitted into another database without creating a dangling reference.
    test.execute_command(
        "CREATE TABLE src (id INTEGER NOT NULL, made TIMESTAMP DEFAULT NOW(), \
         qty INTEGER DEFAULT 7)",
    )
    .expect("create source");
    test.execute_command("INSERT INTO src (id) VALUES (1)")
        .expect("seed");

    let report = Catalog::new(&test.connection)
        .copy_table("public.src", "public.dst")
        .expect("copy");

    assert_eq!(report.rows_copied, 1);
    // The portable default survives; the database-qualified one does not.
    assert_eq!(report.default_columns, 1);
    assert!(!report.is_fully_preserved());
    assert_eq!(report.unpreserved.len(), 1);
    let item = &report.unpreserved[0];
    assert_eq!(item.column, "made");
    assert_eq!(item.reason, UnpreservedReason::NonPortableDefault);
    assert!(
        item.detail.contains("now"),
        "detail should quote the offending expression, got {}",
        item.detail
    );

    // The destination must not carry a default pointing at the source database.
    let facts = raw_column_facts(&test, "", "dst");
    assert_eq!(
        facts,
        vec![
            ("id".to_string(), true, false),
            ("made".to_string(), false, false),
            ("qty".to_string(), false, true),
        ]
    );
}

#[test]
fn test_copy_table_across_databases_persists_after_reattach() {
    let test = TestConnection::new().expect("Failed to create test connection");

    test.execute_command(
        "CREATE TABLE src (id INTEGER NOT NULL, code TEXT NOT NULL, \
         qty INTEGER DEFAULT 7, ASSUMED PRIMARY KEY (id), ASSUMED UNIQUE (code))",
    )
    .expect("create source");
    test.execute_command("INSERT INTO src (id, code) VALUES (1, 'a')")
        .expect("seed");

    let backup = test_result_path("table_copy_backup", "hyper").expect("backup path");
    let _ = std::fs::remove_file(&backup);
    let backup_sql = escape_sql_path(&backup.to_string_lossy());

    test.execute_command(&format!("CREATE DATABASE {backup_sql}"))
        .expect("create backup db");
    test.execute_command(&format!("ATTACH DATABASE {backup_sql} AS \"bak\""))
        .expect("attach backup");

    // The source database's own name, so both sides can be fully qualified —
    // unqualified DDL stops resolving once a second database is attached.
    let source_db = test
        .database_path
        .file_stem()
        .expect("stem")
        .to_string_lossy()
        .to_string();

    let report = Catalog::new(&test.connection)
        .copy_table(
            format!("\"{source_db}\".\"public\".\"src\""),
            "\"bak\".\"public\".\"src\"",
        )
        .expect("cross-database copy");

    assert_eq!(report.rows_copied, 1);
    assert!(report.is_fully_preserved(), "{:?}", report.unpreserved);

    test.execute_command("DETACH DATABASE \"bak\"")
        .expect("detach");
    test.execute_command(&format!("ATTACH DATABASE {backup_sql} AS \"bak2\""))
        .expect("reattach");

    // Everything must still be there after the file is closed and reopened —
    // this is the property a backup actually depends on.
    let facts = raw_column_facts(&test, "\"bak2\".", "src");
    assert_eq!(
        facts,
        vec![
            ("id".to_string(), true, false),
            ("code".to_string(), true, false),
            ("qty".to_string(), false, true),
        ]
    );
    let constraints = raw_constraint_facts(&test, "\"bak2\".", "src");
    assert!(
        constraints.contains(&("p".to_string(), "id".to_string())),
        "{constraints:?}"
    );
    assert!(
        constraints.contains(&("u".to_string(), "code".to_string())),
        "{constraints:?}"
    );

    let mut result = test
        .connection
        .execute_query("SELECT CAST(COUNT(*) AS TEXT) FROM \"bak2\".\"public\".\"src\"")
        .expect("count");
    let chunk = result.next_chunk().expect("chunk").expect("row");
    assert_eq!(chunk[0].get::<String>(0).as_deref(), Some("1"));

    test.execute_command("DETACH DATABASE \"bak2\"")
        .expect("final detach");
    let _ = std::fs::remove_file(&backup);
}

/// A copy must not write into a table that already exists.
///
/// (There is deliberately no test that a rebuilt `NOT NULL` rejects bad data:
/// a Hyper source cannot hold a `NULL` in a `NOT NULL` column, so the
/// violation cannot be constructed from a real source table.)
#[test]
fn test_copy_table_rejects_an_existing_destination() {
    let test = TestConnection::new().expect("Failed to create test connection");

    test.execute_command("CREATE TABLE nullable_src (a INTEGER)")
        .expect("create source");
    test.execute_command("INSERT INTO nullable_src VALUES (NULL)")
        .expect("seed null");

    let catalog = Catalog::new(&test.connection);

    // Copying a nullable table is fine.
    let report = catalog
        .copy_table("public.nullable_src", "public.nullable_dst")
        .expect("copy");
    assert_eq!(report.rows_copied, 1);
    assert_eq!(report.not_null_columns, 0);

    // Copying onto an existing name must fail rather than silently merge.
    let err = catalog
        .copy_table("public.nullable_src", "public.nullable_dst")
        .expect_err("copy onto existing table should fail");
    assert!(err.to_string().contains("exist"), "unexpected error: {err}");
}

#[test]
fn test_copy_table_handles_quoted_identifiers() {
    let test = TestConnection::new().expect("Failed to create test connection");

    test.execute_command(
        "CREATE TABLE \"odd names\" (\"col one\" INTEGER NOT NULL, \"Col Two\" TEXT NOT NULL, \
         ASSUMED PRIMARY KEY (\"col one\"), ASSUMED UNIQUE (\"Col Two\"))",
    )
    .expect("create source");
    test.execute_command("INSERT INTO \"odd names\" VALUES (1, 'x')")
        .expect("seed");

    let report = Catalog::new(&test.connection)
        .copy_table("public.\"odd names\"", "public.\"odd copy\"")
        .expect("copy");

    assert_eq!(report.rows_copied, 1);
    assert_eq!(report.not_null_columns, 2);
    assert_eq!(report.assumed_primary_keys, 1);
    assert_eq!(report.assumed_unique_constraints, 1);

    let facts = raw_column_facts(&test, "", "odd copy");
    assert_eq!(
        facts,
        vec![
            ("col one".to_string(), true, false),
            ("Col Two".to_string(), true, false),
        ]
    );
}

#[test]
fn test_copy_table_into_non_public_schema() {
    let test = TestConnection::new().expect("Failed to create test connection");

    test.execute_command("CREATE SCHEMA archive")
        .expect("create schema");
    test.execute_command("CREATE TABLE src (id INTEGER NOT NULL, ASSUMED PRIMARY KEY (id))")
        .expect("create source");
    test.execute_command("INSERT INTO src VALUES (1)")
        .expect("seed");

    let report = Catalog::new(&test.connection)
        .copy_table("public.src", "archive.src")
        .expect("copy");

    assert_eq!(report.rows_copied, 1);
    assert!(report.is_fully_preserved());

    let def = Catalog::new(&test.connection)
        .get_table_definition("archive.src")
        .expect("reflect copy");
    assert!(!def.column_by_name("id").expect("id column").nullable);
    assert_eq!(
        def.constraints(),
        [TableConstraint::AssumedPrimaryKey {
            columns: vec!["id".to_string()]
        }]
    );
}

/// Column collation must survive the copy, read out of the destination.
///
/// This is a regression test with a specific history: the first version of
/// `copy_table` reflected everything *except* collation, so an explicitly
/// collated column silently landed in the destination uncollated — while the
/// report claimed `fully_preserved`. `CREATE TABLE … AS SELECT`, the thing
/// this method replaced, had preserved it. The assertion deliberately reads
/// `pg_collation` in the destination rather than trusting the report, because
/// it was precisely the report that was wrong.
#[test]
fn test_copy_table_preserves_column_collation() {
    let test = TestConnection::new().expect("Failed to create test connection");

    test.execute_command(
        "CREATE TABLE coll_src (a TEXT COLLATE \"en_US\" NOT NULL, b TEXT COLLATE \"binary\", \
         c TEXT)",
    )
    .expect("create source");
    test.execute_command("INSERT INTO coll_src VALUES ('x', 'y', 'z')")
        .expect("seed");

    // The source is what we claim it is.
    assert_eq!(
        raw_collations(&test, "", "coll_src"),
        [
            ("a".to_string(), Some("en_US".to_string())),
            ("b".to_string(), Some("binary".to_string())),
            ("c".to_string(), None),
        ]
    );

    let report = Catalog::new(&test.connection)
        .copy_table("public.coll_src", "public.coll_dst")
        .expect("copy");

    // Assert against the destination file first: the report was the thing
    // that lied last time, so it is not the primary evidence here.
    assert_eq!(
        raw_collations(&test, "", "coll_dst"),
        [
            ("a".to_string(), Some("en_US".to_string())),
            ("b".to_string(), Some("binary".to_string())),
            ("c".to_string(), None),
        ],
        "collation was dropped by the copy"
    );

    assert_eq!(report.rows_copied, 1);
    assert_eq!(report.collated_columns, 2);
    assert!(
        report.is_fully_preserved(),
        "unpreserved: {:?}",
        report.unpreserved
    );
}

/// Names that are reserved words must not break the copy.
///
/// The reflected identifiers are emitted into `CREATE TABLE` and into both
/// column lists of the `INSERT … SELECT`. An all-lowercase keyword passes the
/// "is this a legal bare identifier" check — which does not know the keyword
/// list — so anything less than unconditional quoting emits `select INTEGER`
/// and the statement is rejected. `CTAS` never spelled the names out, so this
/// is a case the copy has to handle that its predecessor did not.
#[test]
fn test_copy_table_handles_reserved_word_names() {
    let test = TestConnection::new().expect("Failed to create test connection");

    test.execute_command(
        "CREATE TABLE \"order\" (\"select\" INTEGER NOT NULL, \"from\" TEXT, \
         ASSUMED PRIMARY KEY (\"select\"))",
    )
    .expect("create source");
    test.execute_command("INSERT INTO \"order\" VALUES (1, 'x')")
        .expect("seed");

    let report = Catalog::new(&test.connection)
        .copy_table("public.order", "public.group")
        .expect("copy with reserved-word names");

    assert_eq!(report.rows_copied, 1);
    assert!(report.is_fully_preserved());

    assert_eq!(
        raw_column_facts(&test, "", "group"),
        [
            ("select".to_string(), true, false),
            ("from".to_string(), false, false),
        ]
    );
    assert_eq!(
        raw_constraint_facts(&test, "", "group"),
        [("p".to_string(), "select".to_string())]
    );
}

/// A name longer than PostgreSQL's 63-character `NAMEDATALEN` must copy.
///
/// Hyper stores such names without complaint, so refusing them in the typed
/// name API made a whole-database export fail on a table the engine had
/// already written — and, because the database name is part of every
/// qualified name, a long *file* name would have failed every table at once.
#[test]
fn test_copy_table_handles_names_longer_than_postgresql_limit() {
    let test = TestConnection::new().expect("Failed to create test connection");

    let long_name = "t".repeat(93);
    let long_column = "c".repeat(80);
    test.execute_command(&format!(
        "CREATE TABLE \"{long_name}\" (\"{long_column}\" INTEGER NOT NULL)"
    ))
    .expect("create source");
    test.execute_command(&format!("INSERT INTO \"{long_name}\" VALUES (1)"))
        .expect("seed");

    let report = Catalog::new(&test.connection)
        .copy_table(
            format!("public.\"{long_name}\""),
            "public.long_name_destination",
        )
        .expect("copy with over-length name");

    assert_eq!(report.rows_copied, 1);
    assert_eq!(
        raw_column_facts(&test, "", "long_name_destination"),
        [(long_column, true, false)]
    );
}

/// Proves the rationale for dropping database-qualified defaults, end to end.
///
/// The design's central claim is that re-emitting a default like
/// `"mydb"."pg_catalog"."now"()` into a *different* database produces a table
/// that breaks once `"mydb"` is no longer attached. That claim justifies
/// dropping such defaults, so it is worth demonstrating rather than asserting.
///
/// This test does both halves in one go: it copies a table carrying a
/// non-portable default into a second database, detaches the source, and then
/// shows (a) the copy is usable — inserting works, because the default was
/// dropped rather than carried — and (b) a table built the *other* way, with
/// the qualified default re-emitted verbatim, is not.
#[test]
fn test_non_portable_default_would_break_a_detached_copy() {
    let test = TestConnection::new().expect("Failed to create test connection");

    test.execute_command("CREATE TABLE npd_src (id INTEGER NOT NULL, ts TIMESTAMP DEFAULT now())")
        .expect("create source");
    test.execute_command("INSERT INTO npd_src (id) VALUES (1)")
        .expect("seed");

    let backup = test_result_path("table_copy_npd", "hyper").expect("backup path");
    let _ = std::fs::remove_file(&backup);
    let backup_sql = escape_sql_path(&backup.to_string_lossy());
    test.execute_command(&format!("CREATE DATABASE {backup_sql}"))
        .expect("create backup db");
    test.execute_command(&format!("ATTACH DATABASE {backup_sql} AS \"npd_bak\""))
        .expect("attach backup");

    let source_db = test
        .database_path
        .file_stem()
        .expect("stem")
        .to_string_lossy()
        .to_string();

    let report = Catalog::new(&test.connection)
        .copy_table(
            format!("\"{source_db}\".\"public\".\"npd_src\""),
            "\"npd_bak\".\"public\".\"npd_src\"",
        )
        .expect("cross-database copy");

    // The default was dropped, and said so, naming the table it came from.
    assert_eq!(report.rows_copied, 1);
    assert!(!report.is_fully_preserved());
    let dropped = report
        .unpreserved
        .iter()
        .find(|item| item.column == "ts")
        .expect("ts default should be reported");
    assert_eq!(dropped.reason, UnpreservedReason::NonPortableDefault);
    assert_eq!(dropped.table, "public.npd_src");
    assert!(
        dropped.detail.contains(&source_db),
        "the reported expression should name the source database: {}",
        dropped.detail
    );

    // Build the same table the naive way: re-emit the qualified default as-is.
    test.execute_command(&format!(
        "CREATE TABLE \"npd_bak\".\"public\".\"npd_naive\" \
         (id INTEGER NOT NULL, ts TIMESTAMP DEFAULT \"{source_db}\".\"pg_catalog\".\"now\"())"
    ))
    .expect("create naive copy");

    // Reopen the backup with the source database no longer attached. This is
    // the situation a backup is actually restored in.
    test.execute_command("DETACH DATABASE \"npd_bak\"")
        .expect("detach");
    let standalone =
        hyperdb_api::Connection::new(&test.hyper, &backup, hyperdb_api::CreateMode::DoNotCreate)
            .expect("reopen backup standalone");

    // The copied table works, because the unusable default is not there.
    standalone
        .execute_command("INSERT INTO npd_src (id) VALUES (2)")
        .expect("insert into the copied table should work");

    // The naive one does not — this is the failure the drop exists to avoid.
    let err = standalone
        .execute_command("INSERT INTO npd_naive (id) VALUES (2)")
        .expect_err("re-emitted qualified default should be unusable once detached");
    let message = err.to_string();
    assert!(
        message.contains("does not exist") && message.contains(&source_db),
        "expected an unresolved-schema error naming the source database, got: {message}"
    );

    drop(standalone);
    let _ = std::fs::remove_file(&backup);
}
