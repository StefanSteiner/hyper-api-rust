// Copyright (c) 2026, Salesforce, Inc. All rights reserved.
// SPDX-License-Identifier: Apache-2.0 OR MIT

use std::fmt;

use napi::bindgen_prelude::*;
use napi_derive::napi;

// =============================================================================
// CreateMode
// =============================================================================

/// How to handle database creation when connecting.
#[napi(string_enum)]
#[derive(Debug, Clone, Copy)]
pub enum CreateMode {
    /// Do not create the database (it must already exist).
    DoNotCreate,
    /// Create the database (fails if it already exists).
    Create,
    /// Create the database if it doesn't exist.
    CreateIfNotExists,
    /// Drop and recreate the database.
    CreateAndReplace,
}

impl From<CreateMode> for hyperdb_api::CreateMode {
    fn from(mode: CreateMode) -> Self {
        match mode {
            CreateMode::DoNotCreate => hyperdb_api::CreateMode::DoNotCreate,
            CreateMode::Create => hyperdb_api::CreateMode::Create,
            CreateMode::CreateIfNotExists => hyperdb_api::CreateMode::CreateIfNotExists,
            CreateMode::CreateAndReplace => hyperdb_api::CreateMode::CreateAndReplace,
        }
    }
}

// =============================================================================
// SqlType
// =============================================================================

/// Represents a SQL column type.
///
/// Use the static factory methods to create instances:
/// `SqlType.int()`, `SqlType.text()`, `SqlType.bool()`, etc.
#[napi]
#[derive(Debug)]
pub struct SqlType {
    pub(crate) inner: hyperdb_api::SqlType,
}

#[napi]
impl SqlType {
    /// Boolean type.
    #[napi(factory)]
    pub fn bool() -> Self {
        SqlType {
            inner: hyperdb_api::SqlType::bool(),
        }
    }

    /// 16-bit integer (SMALLINT).
    #[napi(factory)]
    pub fn small_int() -> Self {
        SqlType {
            inner: hyperdb_api::SqlType::small_int(),
        }
    }

    /// 32-bit integer (INT).
    #[napi(factory)]
    pub fn int() -> Self {
        SqlType {
            inner: hyperdb_api::SqlType::int(),
        }
    }

    /// 64-bit integer (BIGINT).
    #[napi(factory)]
    pub fn big_int() -> Self {
        SqlType {
            inner: hyperdb_api::SqlType::big_int(),
        }
    }

    /// Single-precision floating point (REAL).
    #[napi(factory)]
    pub fn float() -> Self {
        SqlType {
            inner: hyperdb_api::SqlType::float(),
        }
    }

    /// Double-precision floating point (DOUBLE PRECISION).
    #[napi(factory)]
    pub fn double() -> Self {
        SqlType {
            inner: hyperdb_api::SqlType::double(),
        }
    }

    /// Numeric/decimal type with precision and scale.
    #[napi(factory)]
    pub fn numeric(precision: u32, scale: u32) -> Self {
        SqlType {
            inner: hyperdb_api::SqlType::numeric(precision, scale),
        }
    }

    /// Variable-length text (TEXT).
    #[napi(factory)]
    pub fn text() -> Self {
        SqlType {
            inner: hyperdb_api::SqlType::text(),
        }
    }

    /// Variable-length character string with optional max length (VARCHAR).
    #[napi(factory)]
    pub fn varchar(max_length: Option<u32>) -> Self {
        SqlType {
            inner: hyperdb_api::SqlType::varchar(max_length),
        }
    }

    /// Fixed-length character string (CHAR).
    #[napi(factory)]
    pub fn char(length: u32) -> Self {
        SqlType {
            inner: hyperdb_api::SqlType::char(length),
        }
    }

    /// Binary data (BYTEA).
    #[napi(factory)]
    pub fn bytes() -> Self {
        SqlType {
            inner: hyperdb_api::SqlType::bytes(),
        }
    }

    /// Date type.
    #[napi(factory)]
    pub fn date() -> Self {
        SqlType {
            inner: hyperdb_api::SqlType::date(),
        }
    }

    /// Time type.
    #[napi(factory)]
    pub fn time() -> Self {
        SqlType {
            inner: hyperdb_api::SqlType::time(),
        }
    }

    /// Timestamp without timezone.
    #[napi(factory)]
    pub fn timestamp() -> Self {
        SqlType {
            inner: hyperdb_api::SqlType::timestamp(),
        }
    }

    /// Timestamp with timezone.
    #[napi(factory)]
    pub fn timestamp_tz() -> Self {
        SqlType {
            inner: hyperdb_api::SqlType::timestamp_tz(),
        }
    }

    /// Time interval.
    #[napi(factory)]
    pub fn interval() -> Self {
        SqlType {
            inner: hyperdb_api::SqlType::interval(),
        }
    }

    /// JSON type.
    #[napi(factory)]
    pub fn json() -> Self {
        SqlType {
            inner: hyperdb_api::SqlType::json(),
        }
    }

    /// Geography type.
    #[napi(factory)]
    pub fn geography() -> Self {
        SqlType {
            inner: hyperdb_api::SqlType::tabgeography(),
        }
    }

    /// Returns the SQL type name (e.g., "INTEGER", "TEXT", "NUMERIC(18, 2)").
    #[napi]
    #[allow(
        clippy::inherent_to_string_shadow_display,
        reason = "napi-rs binding must expose an inherent `toString` method for the JS-facing class"
    )]
    pub fn to_string(&self) -> String {
        self.inner.to_string()
    }
}

impl fmt::Display for SqlType {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}", self.inner)
    }
}

// =============================================================================
// TableDefinition
// =============================================================================

/// Defines the schema of a database table.
///
/// Use this to create tables via `Catalog.createTable()` or for bulk inserts
/// via `Inserter`.
#[napi]
#[derive(Debug)]
pub struct TableDefinition {
    pub(crate) inner: hyperdb_api::TableDefinition,
}

#[napi]
impl TableDefinition {
    /// Creates a new table definition with the given name.
    #[napi(constructor)]
    pub fn new(name: String) -> Self {
        TableDefinition {
            inner: hyperdb_api::TableDefinition::new(name),
        }
    }

    /// Sets the schema name and returns the definition for chaining.
    #[napi]
    pub fn with_schema(&mut self, schema: String) -> &Self {
        // TableDefinition uses a consuming builder, so we need to take and replace
        let inner = std::mem::take(&mut self.inner);
        self.inner = inner.with_schema(schema);
        self
    }

    /// Adds a column to the table definition.
    ///
    /// @param name - Column name.
    /// @param sqlType - The SQL type for this column.
    /// @param nullable - Whether the column allows NULL values.
    #[napi]
    pub fn add_column(&mut self, name: String, sql_type: &SqlType, nullable: bool) -> &Self {
        let inner = std::mem::take(&mut self.inner);
        self.inner = if nullable {
            inner.add_nullable_column(name, sql_type.inner)
        } else {
            inner.add_required_column(name, sql_type.inner)
        };
        self
    }

    /// Returns the number of columns.
    #[napi(getter)]
    pub fn column_count(&self) -> u32 {
        // A table definition's column count is structurally bounded by
        // Hyper (far below u32::MAX); saturating is a safe diagnostic.
        u32::try_from(self.inner.column_count()).unwrap_or(u32::MAX)
    }

    /// Returns the table name.
    #[napi(getter)]
    pub fn name(&self) -> String {
        self.inner.name.clone()
    }

    /// Returns the schema name, if set.
    #[napi(getter)]
    pub fn schema(&self) -> Option<String> {
        self.inner.schema.clone()
    }

    /// Returns column information as an array of objects.
    #[napi]
    pub fn get_columns(&self) -> Vec<ColumnInfo> {
        self.inner
            .columns()
            .iter()
            .map(|col| ColumnInfo {
                name: col.name.clone(),
                type_name: col.type_name().to_string(),
                nullable: col.nullable,
            })
            .collect()
    }

    /// Generates the CREATE TABLE SQL statement.
    #[napi]
    pub fn to_create_sql(&self) -> Result<String> {
        self.inner
            .to_create_sql(true)
            .map_err(|e| Error::from_reason(e.to_string()))
    }
}

/// Information about a single column in a table definition.
#[napi(object)]
#[derive(Debug)]
pub struct ColumnInfo {
    /// Column name.
    pub name: String,
    /// SQL type name (e.g., "INTEGER", "TEXT").
    pub type_name: String,
    /// Whether the column allows NULL values.
    pub nullable: bool,
}
