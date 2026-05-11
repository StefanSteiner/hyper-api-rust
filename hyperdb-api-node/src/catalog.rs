// Copyright (c) 2026, Salesforce, Inc. All rights reserved.
// SPDX-License-Identifier: Apache-2.0 OR MIT

use std::sync::Arc;

use napi::bindgen_prelude::*;
use napi_derive::napi;

use crate::connection::Connection;
use crate::types::TableDefinition;

// =============================================================================
// Catalog
// =============================================================================

/// Provides database catalog operations (DDL): create/drop tables, schemas, etc.
///
/// A Catalog operates on the connection it was created from.
///
/// @example
/// ```js
/// const catalog = new Catalog(conn);
/// await catalog.createSchema('my_schema');
/// await catalog.createTable(tableDef);
/// const tables = await catalog.getTableNames('public');
/// ```
#[napi]
#[derive(Debug)]
pub struct Catalog {
    conn: Arc<hyperdb_api::AsyncConnection>,
}

fn escape_sql_literal(s: &str) -> String {
    s.replace('\'', "''")
}

fn identifier(name: &str) -> String {
    format!("\"{}\"", name.replace('"', "\"\""))
}

#[napi]
impl Catalog {
    /// Creates a new Catalog for the given connection.
    #[napi(constructor)]
    pub fn new(connection: &Connection) -> Self {
        Catalog {
            conn: connection.inner_arc(),
        }
    }

    /// Creates a schema if it does not already exist.
    #[napi]
    pub async fn create_schema(&self, schema_name: String) -> Result<()> {
        let sql = format!("CREATE SCHEMA IF NOT EXISTS {}", identifier(&schema_name));
        self.conn
            .execute_command(&sql)
            .await
            .map(|_| ())
            .map_err(|e| Error::from_reason(e.to_string()))
    }

    /// Creates a table from a `TableDefinition`.
    #[napi]
    pub async fn create_table(&self, table_def: &TableDefinition) -> Result<()> {
        let sql = table_def
            .inner
            .to_create_sql(true)
            .map_err(|e| Error::from_reason(e.to_string()))?;
        self.conn
            .execute_command(&sql)
            .await
            .map(|_| ())
            .map_err(|e| Error::from_reason(e.to_string()))
    }

    /// Creates a table if it doesn't already exist.
    #[napi]
    pub async fn create_table_if_not_exists(&self, table_def: &TableDefinition) -> Result<()> {
        let sql = table_def
            .inner
            .to_create_sql(false)
            .map_err(|e| Error::from_reason(e.to_string()))?;
        self.conn
            .execute_command(&sql)
            .await
            .map(|_| ())
            .map_err(|e| Error::from_reason(e.to_string()))
    }

    /// Drops a table.
    #[napi]
    pub async fn drop_table(&self, table_name: String) -> Result<()> {
        let sql = format!("DROP TABLE {table_name}");
        self.conn
            .execute_command(&sql)
            .await
            .map(|_| ())
            .map_err(|e| Error::from_reason(e.to_string()))
    }

    /// Drops a table if it exists (no error if missing).
    #[napi]
    pub async fn drop_table_if_exists(&self, table_name: String) -> Result<()> {
        let sql = format!("DROP TABLE IF EXISTS {table_name}");
        self.conn
            .execute_command(&sql)
            .await
            .map(|_| ())
            .map_err(|e| Error::from_reason(e.to_string()))
    }

    /// Drops a schema. Pass `cascade = true` to drop all objects in the schema.
    #[napi]
    pub async fn drop_schema(&self, schema_name: String, cascade: bool) -> Result<()> {
        let sql = if cascade {
            format!("DROP SCHEMA {} CASCADE", identifier(&schema_name))
        } else {
            format!("DROP SCHEMA {}", identifier(&schema_name))
        };
        self.conn
            .execute_command(&sql)
            .await
            .map(|_| ())
            .map_err(|e| Error::from_reason(e.to_string()))
    }

    /// Checks whether a schema exists.
    #[napi]
    pub async fn has_schema(&self, schema_name: String) -> Result<bool> {
        let sql = format!(
            "SELECT COUNT(*) FROM pg_catalog.pg_namespace WHERE nspname = '{}'",
            escape_sql_literal(&schema_name)
        );
        let count = self
            .conn
            .query_count(&sql)
            .await
            .map_err(|e| Error::from_reason(e.to_string()))?;
        Ok(count > 0)
    }

    /// Checks whether a table exists.
    ///
    /// @param tableName - "table" or "schema.table".
    #[napi]
    pub async fn has_table(&self, table_name: String) -> Result<bool> {
        let (schema, table) = match table_name.rsplit_once('.') {
            Some((s, t)) => (s.to_string(), t.to_string()),
            None => ("public".to_string(), table_name.clone()),
        };
        let sql = format!(
            "SELECT COUNT(*) FROM pg_catalog.pg_tables WHERE schemaname = '{}' AND tablename = '{}'",
            escape_sql_literal(&schema),
            escape_sql_literal(&table)
        );
        let count = self
            .conn
            .query_count(&sql)
            .await
            .map_err(|e| Error::from_reason(e.to_string()))?;
        Ok(count > 0)
    }

    /// Returns a list of table names in the given schema.
    #[napi]
    pub async fn get_table_names(&self, schema_name: String) -> Result<Vec<String>> {
        let sql = format!(
            "SELECT tablename FROM pg_catalog.pg_tables WHERE schemaname = '{}'",
            escape_sql_literal(&schema_name)
        );
        let rows = self
            .conn
            .fetch_all(sql)
            .await
            .map_err(|e| Error::from_reason(e.to_string()))?;
        Ok(rows
            .into_iter()
            .filter_map(|r| r.get::<String>(0))
            .collect())
    }

    /// Returns a list of schema names in the database.
    #[napi]
    pub async fn get_schema_names(&self) -> Result<Vec<String>> {
        let sql = "SELECT nspname FROM pg_catalog.pg_namespace \
                   WHERE nspname NOT IN ('pg_catalog', 'pg_temp', 'information_schema')";
        let rows = self
            .conn
            .fetch_all(sql)
            .await
            .map_err(|e| Error::from_reason(e.to_string()))?;
        Ok(rows
            .into_iter()
            .filter_map(|r| r.get::<String>(0))
            .collect())
    }
}
