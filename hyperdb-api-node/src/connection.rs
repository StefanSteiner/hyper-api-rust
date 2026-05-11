// Copyright (c) 2026, Salesforce, Inc. All rights reserved.
// SPDX-License-Identifier: Apache-2.0 OR MIT

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::time::Duration;

use napi::bindgen_prelude::*;
use napi_derive::napi;

use crate::columnar::{self, ColumnarStream};
use crate::query_stream::{self, QueryStream};
use crate::result::{extract_row, ResultColumnInfo, RowData};
use crate::types::CreateMode;

// =============================================================================
// Connection
// =============================================================================

/// An async connection to a Hyper database.
///
/// All I/O methods return Promises — use `await` when calling them. The
/// binding is backed by [`hyperdb_api::AsyncConnection`], so concurrent
/// calls share a single tokio task pool rather than each burning a
/// thread on the napi blocking pool.
#[napi]
#[derive(Debug)]
pub struct Connection {
    pub(crate) inner: Arc<hyperdb_api::AsyncConnection>,
    closed: AtomicBool,
}

impl Connection {
    pub(crate) fn inner_arc(&self) -> Arc<hyperdb_api::AsyncConnection> {
        Arc::clone(&self.inner)
    }

    /// Borrow the Arc directly — used by `prepare_arc` entry points that
    /// need `&Arc<Self>` to call the `self: &Arc<Self>` hyperdb-api method.
    pub(crate) fn inner_arc_ref(&self) -> &Arc<hyperdb_api::AsyncConnection> {
        &self.inner
    }
}

#[napi]
impl Connection {
    /// Connects to a Hyper server with a database.
    #[napi(factory)]
    pub async fn connect(
        endpoint: String,
        database_path: String,
        create_mode: CreateMode,
    ) -> Result<Connection> {
        let mode: hyperdb_api::CreateMode = create_mode.into();
        let conn = hyperdb_api::AsyncConnection::connect(&endpoint, &database_path, mode)
            .await
            .map_err(|e| Error::from_reason(e.to_string()))?;
        Ok(Connection {
            inner: Arc::new(conn),
            closed: AtomicBool::new(false),
        })
    }

    /// Connects with username and password authentication.
    #[napi(factory)]
    pub async fn connect_with_auth(
        endpoint: String,
        database_path: String,
        create_mode: CreateMode,
        user: String,
        password: String,
    ) -> Result<Connection> {
        let mode: hyperdb_api::CreateMode = create_mode.into();
        let conn = hyperdb_api::AsyncConnection::connect_with_auth(
            &endpoint,
            &database_path,
            mode,
            &user,
            &password,
        )
        .await
        .map_err(|e| Error::from_reason(e.to_string()))?;
        Ok(Connection {
            inner: Arc::new(conn),
            closed: AtomicBool::new(false),
        })
    }

    /// Connects to a Hyper server without attaching a database.
    #[napi(factory)]
    pub async fn without_database(endpoint: String) -> Result<Connection> {
        let conn = hyperdb_api::AsyncConnection::without_database(&endpoint)
            .await
            .map_err(|e| Error::from_reason(e.to_string()))?;
        Ok(Connection {
            inner: Arc::new(conn),
            closed: AtomicBool::new(false),
        })
    }

    /// Executes a SQL command that doesn't return rows (CREATE, INSERT, DROP, etc.).
    #[napi]
    pub async fn execute_command(&self, sql: String) -> Result<i64> {
        self.inner
            .execute_command(&sql)
            .await
            .map(|n| {
                #[expect(
                    clippy::cast_possible_wrap,
                    reason = "NAPI BigInt ↔ Hyper u64 bit-pattern reinterpret; JS consumers read the BigInt as an unsigned affected-row count"
                )]
                let signed = n as i64;
                signed
            })
            .map_err(|e| Error::from_reason(e.to_string()))
    }

    /// Executes a SQL query and returns all result rows.
    ///
    /// All rows are collected into memory. Use `executeQueryStream` for very
    /// large result sets.
    #[napi]
    pub async fn execute_query(&self, sql: String) -> Result<Vec<RowData>> {
        let mut rowset = self
            .inner
            .execute_query(&sql)
            .await
            .map_err(|e| Error::from_reason(e.to_string()))?;

        let mut rows: Vec<RowData> = Vec::new();
        let mut schema: Option<hyperdb_api::ResultSchema> = None;

        while let Some(chunk) = rowset
            .next_chunk()
            .await
            .map_err(|e| Error::from_reason(e.to_string()))?
        {
            if schema.is_none() {
                schema = rowset.schema();
            }
            if let Some(s) = schema.as_ref() {
                for row in &chunk {
                    rows.push(RowData {
                        values: extract_row(row, s),
                    });
                }
            }
        }

        Ok(rows)
    }

    /// Executes a query and returns column metadata for the result set.
    #[napi]
    pub async fn query_schema(&self, sql: String) -> Result<Vec<ResultColumnInfo>> {
        let mut rowset = self
            .inner
            .execute_query(&sql)
            .await
            .map_err(|e| Error::from_reason(e.to_string()))?;

        // First chunk materializes the schema. Drain the rest so the
        // connection is clean for the next query.
        let _ = rowset
            .next_chunk()
            .await
            .map_err(|e| Error::from_reason(e.to_string()))?;

        let schema = rowset
            .schema()
            .ok_or_else(|| Error::from_reason("No schema available"))?;

        let result: Vec<ResultColumnInfo> = schema
            .columns()
            .iter()
            .map(|col| ResultColumnInfo {
                name: col.name().to_string(),
                type_name: col.sql_type().to_string(),
                // Column count in a result schema is structurally bounded
                // by Hyper (far below u32::MAX).
                index: u32::try_from(col.index()).unwrap_or(u32::MAX),
            })
            .collect();

        while rowset
            .next_chunk()
            .await
            .map_err(|e| Error::from_reason(e.to_string()))?
            .is_some()
        {}

        Ok(result)
    }

    /// Executes a SQL query and returns a streaming result set.
    #[napi]
    pub fn execute_query_stream(&self, sql: String) -> QueryStream {
        query_stream::start_query_stream(Arc::clone(&self.inner), sql)
    }

    /// Executes a SQL query and returns a columnar streaming result set.
    #[napi]
    pub fn execute_query_columnar(&self, sql: String) -> ColumnarStream {
        columnar::start_columnar_stream(Arc::clone(&self.inner), sql)
    }

    /// Executes a SQL query and returns the result as raw Arrow IPC stream bytes.
    #[napi]
    pub async fn execute_query_to_arrow(&self, sql: String) -> Result<Buffer> {
        let bytes = self
            .inner
            .execute_query_to_arrow(&sql)
            .await
            .map_err(|e| Error::from_reason(e.to_string()))?;
        Ok(Buffer::from(bytes.as_ref()))
    }

    /// Exports an entire table as raw Arrow IPC stream bytes.
    #[napi]
    pub async fn export_table_to_arrow(&self, table_name: String) -> Result<Buffer> {
        let quoted = table_name.replace('"', "\"\"");
        self.execute_query_to_arrow(format!("SELECT * FROM \"{quoted}\""))
            .await
    }

    /// Returns the database path, if a database is attached.
    #[napi(getter)]
    pub fn database(&self) -> Option<String> {
        self.inner.database().map(std::string::ToString::to_string)
    }

    /// Returns true if the connection is alive and has not been closed.
    #[napi(getter)]
    pub fn is_alive(&self) -> bool {
        !self.closed.load(Ordering::Acquire) && self.inner.is_alive()
    }

    /// Closes the connection.
    ///
    /// After calling this, `isAlive` returns `false` and no further
    /// operations should be performed. The underlying `AsyncConnection` is
    /// released when the last Arc-clone is dropped; calling `close` eagerly
    /// runs the detach-database handshake.
    #[napi]
    pub async fn close(&self) -> Result<()> {
        self.closed.store(true, Ordering::Release);
        // We can't move out of the Arc (JS still holds it), so mimic the
        // detach step from `AsyncConnection::close` without consuming.
        if let Some(db_path) = self.inner.database() {
            let db_alias = std::path::Path::new(db_path)
                .file_stem()
                .and_then(|s| s.to_str())
                .unwrap_or("db");
            let escaped = hyperdb_api::escape_sql_path(db_alias);
            let _ = self
                .inner
                .execute_command(&format!("DETACH DATABASE {escaped}"))
                .await;
        }
        Ok(())
    }

    #[allow(
        clippy::unnecessary_wraps,
        reason = "signature retained for API symmetry / future fallibility; returning Result/Option keeps callers from breaking when the function later grows failure cases"
    )]
    /// Enables query statistics collection by parsing Hyper's log file.
    #[napi]
    pub fn enable_query_stats(&self, log_path: String) -> Result<()> {
        let provider = hyperdb_api::LogFileStatsProvider::new(log_path);
        self.inner.enable_query_stats(provider);
        Ok(())
    }

    #[allow(
        clippy::unnecessary_wraps,
        reason = "signature retained for API symmetry / future fallibility; returning Result/Option keeps callers from breaking when the function later grows failure cases"
    )]
    /// Disables query statistics collection.
    #[napi]
    pub fn disable_query_stats(&self) -> Result<()> {
        self.inner.disable_query_stats();
        Ok(())
    }

    #[allow(
        clippy::unnecessary_wraps,
        reason = "signature retained for API symmetry / future fallibility; returning Result/Option keeps callers from breaking when the function later grows failure cases"
    )]
    /// Returns the query statistics from the most recent query execution.
    #[napi]
    pub fn last_query_stats(&self) -> Result<Option<crate::query_stats::JsQueryStats>> {
        Ok(self.inner.last_query_stats().map(std::convert::Into::into))
    }
}

// =============================================================================
// ConnectionBuilder
// =============================================================================

/// A fluent builder for creating connections with advanced options.
#[napi]
#[derive(Debug)]
pub struct ConnectionBuilder {
    endpoint: String,
    database: Option<String>,
    create_mode: Option<CreateMode>,
    user: Option<String>,
    password: Option<String>,
    login_timeout_ms: Option<u32>,
}

#[napi]
impl ConnectionBuilder {
    /// Creates a new `ConnectionBuilder` for the given endpoint.
    #[napi(constructor)]
    pub fn new(endpoint: String) -> Self {
        ConnectionBuilder {
            endpoint,
            database: None,
            create_mode: None,
            user: None,
            password: None,
            login_timeout_ms: None,
        }
    }

    /// Sets the database path.
    #[napi]
    pub fn database(&mut self, path: String) -> &Self {
        self.database = Some(path);
        self
    }

    /// Sets the database creation mode.
    #[napi]
    pub fn create_mode(&mut self, mode: CreateMode) -> &Self {
        self.create_mode = Some(mode);
        self
    }

    /// Sets the username for authentication.
    #[napi]
    pub fn user(&mut self, user: String) -> &Self {
        self.user = Some(user);
        self
    }

    /// Sets the password for authentication.
    #[napi]
    pub fn password(&mut self, password: String) -> &Self {
        self.password = Some(password);
        self
    }

    /// Sets the login timeout in milliseconds.
    #[napi]
    pub fn login_timeout(&mut self, ms: u32) -> &Self {
        self.login_timeout_ms = Some(ms);
        self
    }

    /// Builds and establishes the connection.
    #[napi]
    pub async fn build(&self) -> Result<Connection> {
        let mut builder = hyperdb_api::AsyncConnectionBuilder::new(self.endpoint.clone());

        if let Some(ref db) = self.database {
            builder = builder.database(db);
        }
        if let Some(mode) = self.create_mode {
            let m: hyperdb_api::CreateMode = mode.into();
            builder = builder.create_mode(m);
        }
        if let Some(ref u) = self.user {
            builder = builder.user(u);
        }
        if let Some(ref p) = self.password {
            builder = builder.password(p);
        }
        if let Some(ms) = self.login_timeout_ms {
            builder = builder.login_timeout(Duration::from_millis(u64::from(ms)));
        }

        let conn = builder
            .build()
            .await
            .map_err(|e| Error::from_reason(e.to_string()))?;

        Ok(Connection {
            inner: Arc::new(conn),
            closed: AtomicBool::new(false),
        })
    }
}
