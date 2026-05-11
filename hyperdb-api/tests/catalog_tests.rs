// Copyright (c) 2026, Salesforce, Inc. All rights reserved.
// SPDX-License-Identifier: Apache-2.0 OR MIT

//! Tests for Catalog operations.

use hyperdb_api::{Catalog, SqlType, TableDefinition};
use hyperdb_api_core::types::Nullability;

mod common;
use common::TestConnection;

#[test]
fn test_catalog_get_schema_names() {
    let test = TestConnection::new().expect("Failed to create test connection");

    // Create some schemas
    test.execute_command("CREATE SCHEMA foo")
        .expect("Failed to create schema");
    test.execute_command("CREATE SCHEMA \"with space\"")
        .expect("Failed to create schema with space");

    let catalog = Catalog::new(&test.connection);
    let schemas = catalog
        .get_schema_names::<&str>(None)
        .expect("Failed to get schema names");

    // Should include 'public' and the created schemas
    assert!(schemas.contains(&"public".to_string()));
    assert!(schemas.contains(&"foo".to_string()));
    assert!(schemas.contains(&"with space".to_string()));
}

#[expect(
    clippy::similar_names,
    reason = "paired bindings (request/response, reader/writer, etc.) are more readable with symmetric names than artificially distinct ones"
)]
#[test]
fn test_catalog_get_table_names() {
    let test = TestConnection::new().expect("Failed to create test connection");

    // Create tables
    let table1 = TableDefinition::new("table1").add_nullable_column("id", SqlType::int());
    Catalog::new(&test.connection)
        .create_table(&table1)
        .expect("Failed to create table1");

    let table2 = TableDefinition::new("table2").add_nullable_column("id", SqlType::int());
    Catalog::new(&test.connection)
        .create_table(&table2)
        .expect("Failed to create table2");

    let catalog = Catalog::new(&test.connection);
    let tables = catalog
        .get_table_names("public")
        .expect("Failed to get table names");

    assert!(tables.contains(&"table1".to_string()));
    assert!(tables.contains(&"table2".to_string()));
}

#[test]
fn test_catalog_create_table() {
    let test = TestConnection::new().expect("Failed to create test connection");

    let table_def = TableDefinition::new("products")
        .add_required_column("id", SqlType::int())
        .add_nullable_column("name", SqlType::text())
        .add_nullable_column("price", SqlType::double());

    Catalog::new(&test.connection)
        .create_table(&table_def)
        .expect("Failed to create table");

    // Verify table exists
    let catalog = Catalog::new(&test.connection);
    let tables = catalog
        .get_table_names("public")
        .expect("Failed to get table names");
    assert!(tables.contains(&"products".to_string()));
}

#[test]
fn test_catalog_create_table_with_schema() {
    let test = TestConnection::new().expect("Failed to create test connection");

    // Create a schema
    test.execute_command("CREATE SCHEMA myschema")
        .expect("Failed to create schema");

    // Use TableDefinition with schema
    let table_def = TableDefinition::new("mytable")
        .with_schema("myschema")
        .add_nullable_column("id", SqlType::int());

    Catalog::new(&test.connection)
        .create_table(&table_def)
        .expect("Failed to create table in schema");

    // Verify table exists in the schema
    let catalog = Catalog::new(&test.connection);
    let tables = catalog
        .get_table_names("myschema")
        .expect("Failed to get table names");
    assert!(tables.contains(&"mytable".to_string()));
}

#[test]
fn test_catalog_drop_table() {
    let test = TestConnection::new().expect("Failed to create test connection");

    let table_def = TableDefinition::new("temp_table").add_nullable_column("id", SqlType::int());
    Catalog::new(&test.connection)
        .create_table(&table_def)
        .expect("Failed to create table");

    // Verify table exists
    let catalog = Catalog::new(&test.connection);
    let tables = catalog
        .get_table_names("public")
        .expect("Failed to get table names");
    assert!(tables.contains(&"temp_table".to_string()));

    // Drop the table using SQL
    test.execute_command("DROP TABLE temp_table")
        .expect("Failed to drop table");

    // Verify table no longer exists
    let tables_after = catalog
        .get_table_names("public")
        .expect("Failed to get table names");
    assert!(!tables_after.contains(&"temp_table".to_string()));
}

#[test]
fn test_catalog_get_table_definition() {
    let test = TestConnection::new().expect("Failed to create test connection");

    let table_def = TableDefinition::new("test_table")
        .add_required_column("id", SqlType::int())
        .add_nullable_column("name", SqlType::text())
        .add_nullable_column("price", SqlType::double());

    Catalog::new(&test.connection)
        .create_table(&table_def)
        .expect("Failed to create table");

    // Get the table definition back
    let catalog = Catalog::new(&test.connection);
    let retrieved_def = catalog
        .get_table_definition("test_table")
        .expect("Failed to get table definition");

    assert_eq!(retrieved_def.name.as_str(), "test_table");
    assert_eq!(retrieved_def.columns().len(), 3);

    let cols = retrieved_def.columns();
    assert_eq!(cols[0].name.as_str(), "id");
    assert_eq!(cols[0].sql_type(), Some(SqlType::int()));
    assert_eq!(cols[0].nullability(), Nullability::NotNullable);

    assert_eq!(cols[1].name.as_str(), "name");
    assert_eq!(cols[1].sql_type(), Some(SqlType::text()));
    assert_eq!(cols[1].nullability(), Nullability::Nullable);

    assert_eq!(cols[2].name.as_str(), "price");
    assert_eq!(cols[2].sql_type(), Some(SqlType::double()));
    assert_eq!(cols[2].nullability(), Nullability::Nullable);
}

#[test]
fn test_catalog_has_schema() {
    let test = TestConnection::new().expect("Failed to create test connection");

    let catalog = Catalog::new(&test.connection);

    // 'public' schema should exist by default
    assert!(catalog
        .has_schema("public")
        .expect("Failed to check schema"));

    // Non-existent schema
    assert!(!catalog
        .has_schema("nonexistent")
        .expect("Failed to check schema"));

    // Create a new schema
    test.execute_command("CREATE SCHEMA newschema")
        .expect("Failed to create schema");

    assert!(catalog
        .has_schema("newschema")
        .expect("Failed to check schema"));
}

#[test]
fn test_catalog_has_table() {
    let test = TestConnection::new().expect("Failed to create test connection");

    let catalog = Catalog::new(&test.connection);

    // Table doesn't exist yet
    assert!(!catalog
        .has_table("my_table")
        .expect("Failed to check table"));

    // Create the table
    test.execute_command("CREATE TABLE my_table (id INT)")
        .expect("Failed to create table");

    // Now it should exist
    assert!(catalog
        .has_table("my_table")
        .expect("Failed to check table"));
}
