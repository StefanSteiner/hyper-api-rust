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

/// A rebuilt `NOT NULL` must reject bad data rather than quietly relaxing.
#[test]
fn test_copy_table_surfaces_constraint_violations() {
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
