// Copyright (c) 2026, Salesforce, Inc. All rights reserved.
// SPDX-License-Identifier: Apache-2.0 OR MIT

//! Tests for the Inserter API.

use hyperdb_api::{Catalog, Date, Geography, Inserter, SqlType, TableDefinition};

mod common;
use common::TestConnection;

#[test]
fn test_inserter_null_in_non_null_column() {
    let test = TestConnection::new().expect("Failed to create test connection");

    let table_def = TableDefinition::new("foo").add_required_column("a", SqlType::int());
    Catalog::new(&test.connection)
        .create_table(&table_def)
        .expect("Failed to create table");

    let mut inserter =
        Inserter::new(&test.connection, &table_def).expect("Failed to create inserter");

    // Trying to insert NULL into a NOT NULL column should fail early during add_row
    let result = inserter.add_row(&[&None::<i32>]);
    assert!(
        result.is_err(),
        "add_row should fail for NULL in NOT NULL column"
    );
}

#[test]
fn test_inserter_type_match() {
    let test = TestConnection::new().expect("Failed to create test connection");

    let table_def = TableDefinition::new("foo").add_nullable_column("a", SqlType::int());
    Catalog::new(&test.connection)
        .create_table(&table_def)
        .expect("Failed to create table");

    let mut inserter =
        Inserter::new(&test.connection, &table_def).expect("Failed to create inserter");

    // Insert an integer value (matching the column type)
    inserter.add_row(&[&42i32]).expect("add_row should succeed");
    inserter.execute().expect("execute should succeed");

    // Verify the value was inserted correctly
    let mut result = test
        .connection
        .execute_query("SELECT a FROM foo")
        .expect("Failed to query");
    let chunk = result
        .next_chunk()
        .expect("Failed to get chunk")
        .expect("Expected chunk");
    let value = chunk
        .first()
        .expect("Expected row")
        .get_i32(0)
        .expect("NULL value");
    assert_eq!(value, 42);
}

/// Tests that inserter creation succeeds for non-existent tables, but execution fails.
///
/// # Deferred Validation Design
///
/// The Inserter uses **deferred validation** by design:
///
/// 1. **`Inserter::new()`** succeeds even if the target table doesn't exist. This is because
///    the inserter only buffers data locally; no database communication occurs until `execute()`.
///
/// 2. **`add_row()`** succeeds because it only validates row structure against the `TableDefinition`,
///    not against the actual database schema.
///
/// 3. **`execute()`** is where actual database communication happens (COPY protocol), so this
///    is when table existence and schema mismatches are detected.
///
/// This design allows for:
/// - Batch preparation before database round-trips
/// - Better performance by avoiding early validation queries
/// - Simpler error handling (all DB errors occur at execute time)
///
/// The tradeoff is that errors like missing tables are detected later, which may be
/// counterintuitive but follows the principle of lazy evaluation for I/O operations.
#[test]
fn test_inserter_missing_table() {
    let test = TestConnection::new().expect("Failed to create test connection");

    let table_def = TableDefinition::new("nonexistent").add_nullable_column("a", SqlType::int());

    // Inserter creation succeeds - no database communication yet (deferred validation)
    let mut inserter =
        Inserter::new(&test.connection, &table_def).expect("Inserter creation should succeed");

    // add_row succeeds - only local buffering, no DB validation
    inserter.add_row(&[&42i32]).expect("add_row should succeed");

    // execute() fails - this is when COPY protocol actually communicates with the database
    // and discovers the table doesn't exist
    assert!(
        inserter.execute().is_err(),
        "execute should fail for non-existent table"
    );
}

#[test]
#[expect(
    clippy::approx_constant,
    reason = "test literal 3.14 chosen for readability; not intended as an approximation of PI"
)]
fn test_inserter_all_types() {
    let test = TestConnection::new().expect("Failed to create test connection");

    // Create a simple table with a subset of types to test basic functionality
    let table_def = TableDefinition::new("all_types_test")
        .add_nullable_column("col_bool", SqlType::bool())
        .add_nullable_column("col_int", SqlType::int())
        .add_nullable_column("col_bigint", SqlType::big_int())
        .add_nullable_column("col_text", SqlType::text())
        .add_nullable_column("col_double", SqlType::double())
        .add_nullable_column("col_date", SqlType::date());

    Catalog::new(&test.connection)
        .create_table(&table_def)
        .expect("Failed to create table");

    let mut inserter =
        Inserter::new(&test.connection, &table_def).expect("Failed to create inserter");

    // Insert a row with all types
    let date = Date::new(2024, 1, 15);
    inserter
        .add_row(&[
            &true,
            &42i32,
            &1234567890123i64,
            &"hello",
            &3.14159f64,
            &date,
        ])
        .expect("Failed to add row");

    inserter.execute().expect("Failed to execute inserter");

    // Verify the data
    let mut result = test
        .connection
        .execute_query("SELECT * FROM all_types_test")
        .expect("Failed to query");

    let chunk = result
        .next_chunk()
        .expect("Failed to get chunk")
        .expect("Expected chunk");
    let row = chunk.first().expect("Expected row");

    let bool_val = row.get::<bool>(0).expect("NULL bool");
    let int_val = row.get_i32(1).expect("NULL int");
    let bigint_val = row.get_i64(2).expect("NULL bigint");
    let text_val = row.get::<String>(3).expect("NULL text");
    let double_val = row.get_f64(4).expect("NULL double");

    assert!(bool_val);
    assert_eq!(int_val, 42);
    assert_eq!(bigint_val, 1234567890123i64);
    assert_eq!(text_val, "hello");
    assert!((double_val - 3.14159).abs() < 0.0001);
}

#[test]
fn test_inserter_bulk_insert() {
    let test = TestConnection::new().expect("Failed to create test connection");

    let table_def = TableDefinition::new("numbers")
        .add_required_column("id", SqlType::int())
        .add_nullable_column("value", SqlType::double());
    Catalog::new(&test.connection)
        .create_table(&table_def)
        .expect("Failed to create table");

    let mut inserter =
        Inserter::new(&test.connection, &table_def).expect("Failed to create inserter");

    // Insert many rows
    for i in 0..1000 {
        inserter
            .add_row(&[&i, &(f64::from(i) * 1.5)])
            .expect("Failed to add row");
    }

    inserter.execute().expect("Failed to execute inserter");

    // Verify all rows were inserted
    let count = test
        .count_tuples("numbers")
        .expect("Failed to count tuples");
    assert_eq!(count, 1000);
}

#[test]
fn test_inserter_nullable_columns() {
    let test = TestConnection::new().expect("Failed to create test connection");

    let table_def = TableDefinition::new("nullable_test")
        .add_required_column("id", SqlType::int())
        .add_nullable_column("name", SqlType::text());
    Catalog::new(&test.connection)
        .create_table(&table_def)
        .expect("Failed to create table");

    let mut inserter =
        Inserter::new(&test.connection, &table_def).expect("Failed to create inserter");

    // Insert row with value
    inserter
        .add_row(&[&1i32, &Some("Alice")])
        .expect("Failed to add row with value");

    // Insert row with NULL
    inserter
        .add_row(&[&2i32, &None::<&str>])
        .expect("Failed to add row with NULL");

    inserter.execute().expect("Failed to execute inserter");

    // Verify rows
    let mut result = test
        .connection
        .execute_query("SELECT id, name FROM nullable_test ORDER BY id")
        .expect("Failed to query table");

    let mut rows: Vec<(i32, Option<String>)> = Vec::new();
    while let Some(chunk) = result.next_chunk().expect("Failed to get chunk") {
        for row in &chunk {
            let id = row.get_i32(0).expect("NULL id");
            let name = row.get::<String>(1);
            rows.push((id, name));
        }
    }

    assert_eq!(rows.len(), 2);
    assert_eq!(rows[0], (1, Some("Alice".to_string())));
    assert_eq!(rows[1], (2, None));
}

#[test]
fn test_inserter_from_table() {
    let test = TestConnection::new().expect("Failed to create test connection");

    // Create table using SQL
    test.execute_command(
        "CREATE TABLE products (id INT NOT NULL, name TEXT, price DOUBLE PRECISION)",
    )
    .expect("Failed to create table");

    // Get the table definition from catalog
    let catalog = Catalog::new(&test.connection);
    let table_def = catalog
        .get_table_definition("products")
        .expect("Failed to get table definition");

    // Create inserter from the table definition
    let mut inserter =
        Inserter::new(&test.connection, &table_def).expect("Failed to create inserter");

    inserter
        .add_row(&[&1i32, &"Widget", &9.99f64])
        .expect("Failed to add row");
    inserter
        .add_row(&[&2i32, &"Gadget", &19.99f64])
        .expect("Failed to add row");

    inserter.execute().expect("Failed to execute inserter");

    // Verify
    let count = test.count_tuples("products").expect("Failed to count");
    assert_eq!(count, 2);
}

#[test]
fn test_inserter_geography_round_trip() {
    let test = TestConnection::new().expect("Failed to create test connection");

    let table_def = TableDefinition::new("geo_test")
        .add_required_column("id", SqlType::int())
        .add_nullable_column("location", SqlType::geography());

    Catalog::new(&test.connection)
        .create_table(&table_def)
        .expect("Failed to create table");

    let mut inserter =
        Inserter::new(&test.connection, &table_def).expect("Failed to create inserter");

    // Create a Geography from WKT (San Francisco coordinates)
    let geo = Geography::from_wkt("POINT(-122.4194 37.7749)")
        .expect("Failed to create geography from WKT");
    let original_bytes = geo.as_bytes().to_vec();

    // Insert using IntoValue trait
    inserter.add_row(&[&1i32, &geo]).expect("Failed to add row");

    inserter.execute().expect("Failed to execute inserter");

    // Read back and verify byte-identical
    let mut result = test
        .connection
        .execute_query("SELECT id, location FROM geo_test")
        .expect("Failed to query");

    let chunk = result
        .next_chunk()
        .expect("Failed to get chunk")
        .expect("Expected chunk");
    let row = chunk.first().expect("Expected row");

    let id = row.get_i32(0).expect("NULL id");
    assert_eq!(id, 1);

    // Geography does NOT implement RowValue yet, so read as Vec<u8>
    let geo_bytes = row.get::<Vec<u8>>(1).expect("NULL geography");
    assert_eq!(
        geo_bytes, original_bytes,
        "Geography bytes should be byte-identical after round-trip"
    );
}

#[test]
fn test_inserter_geography_nullable() {
    let test = TestConnection::new().expect("Failed to create test connection");

    let table_def = TableDefinition::new("geo_nullable_test")
        .add_required_column("id", SqlType::int())
        .add_nullable_column("location", SqlType::geography());

    Catalog::new(&test.connection)
        .create_table(&table_def)
        .expect("Failed to create table");

    let mut inserter =
        Inserter::new(&test.connection, &table_def).expect("Failed to create inserter");

    // Insert a row with a Geography value
    let geo1 = Geography::from_wkt("POINT(-122.4194 37.7749)")
        .expect("Failed to create geography from WKT");
    inserter
        .add_row(&[&1i32, &Some(geo1)])
        .expect("Failed to add row with geography");

    // Insert a row with NULL
    inserter
        .add_row(&[&2i32, &None::<Geography>])
        .expect("Failed to add row with NULL");

    inserter.execute().expect("Failed to execute inserter");

    // Verify rows
    let mut result = test
        .connection
        .execute_query("SELECT id, location FROM geo_nullable_test ORDER BY id")
        .expect("Failed to query table");

    let mut rows: Vec<(i32, Option<Vec<u8>>)> = Vec::new();
    while let Some(chunk) = result.next_chunk().expect("Failed to get chunk") {
        for row in &chunk {
            let id = row.get_i32(0).expect("NULL id");
            let location = row.get::<Vec<u8>>(1);
            rows.push((id, location));
        }
    }

    assert_eq!(rows.len(), 2);
    assert!(rows[0].1.is_some(), "First row should have a geography");
    assert!(rows[1].1.is_none(), "Second row should have NULL");
}

// =============================================================================
// COPY lifecycle: cancel / drop / reuse / staging-table cleanup
// =============================================================================

/// More rows than one auto-flush chunk (`CHUNK_ROW_LIMIT` = 64_000), so the
/// inserter has opened a COPY session before `cancel`/drop.
const ROWS_OVER_AUTOFLUSH: i32 = 64_001;

fn create_int_table(test: &TestConnection, name: &str) -> TableDefinition {
    let table_def = TableDefinition::new(name).add_nullable_column("a", SqlType::int());
    Catalog::new(&test.connection)
        .create_table(&table_def)
        .expect("Failed to create table");
    table_def
}

fn count_rows(test: &TestConnection, table: &str) -> i64 {
    test.count_tuples(table).expect("COUNT(*) must succeed")
}

/// Cancelling an inserter whose COPY session is open must abort the COPY
/// (nothing committed) and leave the connection usable.
#[test]
fn test_inserter_cancel_after_autoflush_leaves_connection_usable() {
    let test = TestConnection::new().expect("Failed to create test connection");
    let table_def = create_int_table(&test, "cancel_copy");

    let mut inserter =
        Inserter::new(&test.connection, &table_def).expect("Failed to create inserter");
    for i in 0..ROWS_OVER_AUTOFLUSH {
        inserter.add_row(&[&i]).expect("add_row");
    }
    inserter.cancel();

    assert_eq!(
        count_rows(&test, "cancel_copy"),
        0,
        "cancel must commit nothing"
    );
    assert_eq!(test.execute_scalar_i32("SELECT 41 + 1").unwrap(), 42);

    // A brand-new inserter on the same table sees none of the cancelled rows.
    let mut fresh = Inserter::new(&test.connection, &table_def).expect("inserter");
    fresh.add_row(&[&7i32]).unwrap();
    fresh.add_row(&[&8i32]).unwrap();
    assert_eq!(fresh.execute().unwrap(), 2);
    assert_eq!(count_rows(&test, "cancel_copy"), 2);
}

/// Dropping an inserter mid-COPY (no execute/cancel) must also leave the
/// connection usable.
#[test]
fn test_inserter_dropped_mid_copy_leaves_connection_usable() {
    let test = TestConnection::new().expect("Failed to create test connection");
    let table_def = create_int_table(&test, "drop_copy");

    {
        let mut inserter =
            Inserter::new(&test.connection, &table_def).expect("Failed to create inserter");
        for i in 0..ROWS_OVER_AUTOFLUSH {
            inserter.add_row(&[&i]).expect("add_row");
        }
    }

    assert_eq!(count_rows(&test, "drop_copy"), 0);
    assert_eq!(test.execute_scalar_i32("SELECT 41 + 1").unwrap(), 42);
}

/// A finished inserter must not leak state into the next one on the same
/// table: 64_001 rows via execute, then a fresh inserter adds exactly its own.
/// (Reusing the *same* inserter is a compile error, see the `compile_fail`
/// doctest on `Inserter::execute`.)
#[test]
fn test_inserter_execute_then_fresh_inserter() {
    let test = TestConnection::new().expect("Failed to create test connection");
    let table_def = create_int_table(&test, "exec_then_fresh");

    let mut inserter = Inserter::new(&test.connection, &table_def).expect("inserter");
    for i in 0..ROWS_OVER_AUTOFLUSH {
        inserter.add_row(&[&i]).unwrap();
    }
    assert_eq!(
        inserter.execute().unwrap(),
        u64::try_from(ROWS_OVER_AUTOFLUSH).unwrap()
    );

    let mut fresh = Inserter::new(&test.connection, &table_def).expect("inserter");
    fresh.add_row(&[&1i32]).unwrap();
    assert_eq!(fresh.execute().unwrap(), 1);
    assert_eq!(
        count_rows(&test, "exec_then_fresh"),
        i64::from(ROWS_OVER_AUTOFLUSH) + 1
    );
}

/// A reader that fails mid-stream: `import_text` must abort the COPY and
/// leave the connection usable.
#[test]
fn test_import_text_read_error_leaves_connection_usable() {
    use hyperdb_api::copy::CopyOptions;

    struct FailingReader {
        sent: bool,
    }
    impl std::io::Read for FailingReader {
        fn read(&mut self, buf: &mut [u8]) -> std::io::Result<usize> {
            if self.sent {
                return Err(std::io::Error::other("boom"));
            }
            self.sent = true;
            let line = b"1\n2\n";
            buf[..line.len()].copy_from_slice(line);
            Ok(line.len())
        }
    }

    let test = TestConnection::new().expect("Failed to create test connection");
    let _ = create_int_table(&test, "import_fail");

    let err = test
        .connection
        .import_text(
            "import_fail",
            &CopyOptions::tsv(),
            FailingReader { sent: false },
        )
        .expect_err("import must report the read error");
    assert!(err.to_string().contains("read import data"), "got: {err}");

    assert_eq!(count_rows(&test, "import_fail"), 0);
    assert_eq!(test.execute_scalar_i32("SELECT 41 + 1").unwrap(), 42);
}

/// Runs `body` on a thread and fails the test (instead of hanging forever) if
/// it does not finish within `secs`. The connection is moved into the thread;
/// the `HyperProcess` stays on the caller so a timeout kills `hyperd`.
fn run_with_timeout(
    secs: u64,
    body: impl FnOnce(hyperdb_api::Connection) + Send + 'static,
) -> hyperdb_api::HyperProcess {
    let TestConnection {
        hyper, connection, ..
    } = TestConnection::new().expect("Failed to create test connection");
    let (tx, rx) = std::sync::mpsc::channel();
    std::thread::spawn(move || {
        body(connection);
        let _ = tx.send(());
    });
    rx.recv_timeout(std::time::Duration::from_secs(secs))
        .expect("body did not finish in time (deadlock?)");
    hyper
}

/// `MappedInserter::cancel` used to run `DROP TABLE` while the inner
/// inserter's `CopyInWriter` still held the connection lock (a same-thread
/// self-deadlock). It must abort the COPY first, then drop staging.
#[test]
fn test_mapped_inserter_cancel_after_autoflush_does_not_deadlock() {
    use hyperdb_api::ColumnMapping;

    let _hyper = run_with_timeout(30, |conn| {
        conn.execute_command("CREATE TABLE mapped_target (a INT)")
            .unwrap();
        let def = TableDefinition::new("ignored").add_nullable_column("a", SqlType::int());
        let mut inserter = Inserter::with_column_mappings(
            &conn,
            &def,
            "mapped_target",
            &[ColumnMapping::new("a")],
        )
        .unwrap();
        for i in 0..ROWS_OVER_AUTOFLUSH {
            inserter.add_row(&[&i]).unwrap();
        }
        inserter.cancel();

        assert_eq!(
            conn.execute_scalar_query::<i64>("SELECT 1").unwrap(),
            Some(1)
        );
        let leftover = conn
            .execute_scalar_query::<i64>(
                "SELECT COUNT(*) FROM pg_catalog.pg_class WHERE relname LIKE '\\_hyper\\_staging\\_%'",
            )
            .unwrap();
        assert_eq!(leftover, Some(0), "staging table must be dropped");
        assert_eq!(
            conn.execute_scalar_query::<i64>("SELECT COUNT(*) FROM mapped_target")
                .unwrap(),
            Some(0)
        );
    });
}

/// Two mapped inserters on one connection used a pid-only staging name and
/// collided on `CREATE TEMPORARY TABLE`.
#[test]
fn test_two_mapped_inserters_on_one_connection_do_not_collide() {
    use hyperdb_api::ColumnMapping;

    let test = TestConnection::new().expect("Failed to create test connection");
    test.execute_command("CREATE TABLE mapped_two (a INT)")
        .unwrap();
    let def = TableDefinition::new("ignored").add_nullable_column("a", SqlType::int());
    let maps = [ColumnMapping::new("a")];

    let mut one = Inserter::with_column_mappings(&test.connection, &def, "mapped_two", &maps)
        .expect("first mapped inserter");
    let mut two = Inserter::with_column_mappings(&test.connection, &def, "mapped_two", &maps)
        .expect("second mapped inserter must not collide on the staging name");
    one.add_row(&[&1i32]).unwrap();
    two.add_row(&[&2i32]).unwrap();
    assert_eq!(one.execute().unwrap(), 1);
    assert_eq!(two.execute().unwrap(), 1);
    assert_eq!(count_rows(&test, "mapped_two"), 2);
}

// =============================================================================
// Server rejects a value mid-COPY (row 10 of an auto-flushed 64,001-row batch)
// =============================================================================

/// Creates `name (a VARCHAR(3))` in the database but describes it to the
/// inserter as `TEXT`, so a longer string is rejected by the *server* while
/// the COPY is streaming.
fn inserter_into_narrow_varchar<'a>(test: &'a TestConnection, name: &str) -> Inserter<'a> {
    test.execute_command(&format!("CREATE TABLE {name} (a VARCHAR(3))"))
        .expect("create table");
    let def = TableDefinition::new(name).add_nullable_column("a", SqlType::text());
    let mut inserter = Inserter::new(&test.connection, &def).expect("inserter");
    for i in 0..ROWS_OVER_AUTOFLUSH {
        let value = if i == 10 { "toolong" } else { "ok" };
        inserter.add_row(&[&value]).expect("add_row");
    }
    inserter
}

fn assert_in_sync_after_rejected_copy(test: &TestConnection, table: &str) {
    for expected in [42, 43, 44] {
        assert_eq!(
            test.execute_scalar_i32(&format!("SELECT {expected}"))
                .unwrap(),
            expected
        );
    }
    assert_eq!(
        count_rows(test, table),
        0,
        "rejected COPY must commit nothing"
    );
}

#[test]
fn test_inserter_server_rejected_mid_copy_then_drop_leaves_connection_in_sync() {
    let test = TestConnection::new().expect("Failed to create test connection");
    drop(inserter_into_narrow_varchar(&test, "reject_drop"));
    assert_in_sync_after_rejected_copy(&test, "reject_drop");
}

#[test]
fn test_inserter_server_rejected_mid_copy_then_cancel_leaves_connection_in_sync() {
    let test = TestConnection::new().expect("Failed to create test connection");
    inserter_into_narrow_varchar(&test, "reject_cancel").cancel();
    assert_in_sync_after_rejected_copy(&test, "reject_cancel");
}

#[test]
fn test_inserter_server_rejected_mid_copy_execute_errors_and_stays_in_sync() {
    let test = TestConnection::new().expect("Failed to create test connection");
    let result = inserter_into_narrow_varchar(&test, "reject_exec").execute();
    assert!(result.is_err(), "server must reject the over-long value");
    assert_in_sync_after_rejected_copy(&test, "reject_exec");
}

/// `StagingTable`'s destructor used to run `DROP TABLE` through the
/// connection even while the same thread still held a live result stream
/// (which owns the non-reentrant connection lock), hanging forever. It must
/// now notice the busy connection, leak the session-scoped temp table and
/// return.
#[test]
fn test_mapped_inserter_dropped_while_result_stream_is_live_does_not_deadlock() {
    use hyperdb_api::ColumnMapping;

    let _hyper = run_with_timeout(15, |conn| {
        conn.execute_command("CREATE TABLE mapped_busy (a INT)")
            .unwrap();
        let def = TableDefinition::new("ignored").add_nullable_column("a", SqlType::int());
        let inserter =
            Inserter::with_column_mappings(&conn, &def, "mapped_busy", &[ColumnMapping::new("a")])
                .unwrap();

        // A live stream holds the connection lock until it is dropped.
        let mut rows = conn
            .execute_query("SELECT v FROM generate_series(1, 200000) v")
            .unwrap();
        let _ = rows.next_chunk().unwrap();

        // Must return instead of deadlocking on the held lock.
        drop(inserter);
        drop(rows);

        // The connection is still usable afterwards.
        assert_eq!(
            conn.execute_scalar_query::<i64>("SELECT 1").unwrap(),
            Some(1)
        );
    });
}
