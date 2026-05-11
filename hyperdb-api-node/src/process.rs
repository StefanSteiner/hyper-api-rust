// Copyright (c) 2026, Salesforce, Inc. All rights reserved.
// SPDX-License-Identifier: Apache-2.0 OR MIT

use napi::bindgen_prelude::*;
use napi_derive::napi;

use std::path::Path;

use crate::connection::Connection;
use crate::types::CreateMode;

// =============================================================================
// HyperProcess
// =============================================================================

/// Manages a local Hyper server process.
///
/// The server is automatically started when a `HyperProcess` is created and
/// stopped when `close()` is called. You **must** call `close()` when done
/// to ensure the server process is properly terminated.
///
/// @example
/// ```js
/// const hyper = new HyperProcess();
/// const conn = await hyper.connectToDatabase('test.hyper', CreateMode.CreateIfNotExists);
/// // ... use the connection ...
/// await conn.close();
/// hyper.close();
/// ```
#[napi]
#[derive(Debug)]
pub struct HyperProcess {
    inner: Option<hyperdb_api::HyperProcess>,
}

#[napi]
impl HyperProcess {
    #[allow(
        clippy::needless_pass_by_value,
        reason = "call-site ergonomics: function consumes logically-owned parameters, refactoring signatures is not worth per-site churn"
    )]
    /// Creates and starts a new Hyper server process.
    ///
    /// The server binary (`hyperd`) is automatically located. Pass a custom
    /// path if it's not in the standard location.
    ///
    /// @param hyperPath - Optional path to the `hyperd` binary.
    #[napi(constructor)]
    pub fn new(hyper_path: Option<String>) -> Result<Self> {
        let path = hyper_path.as_ref().map(|p| Path::new(p.as_str()));
        let process = hyperdb_api::HyperProcess::new(path, None)
            .map_err(|e| Error::from_reason(e.to_string()))?;
        Ok(HyperProcess {
            inner: Some(process),
        })
    }

    /// Returns the server endpoint (e.g., "localhost:7483").
    ///
    /// Use this to connect to the server via `Connection.connect()`.
    #[napi(getter)]
    pub fn endpoint(&self) -> Result<String> {
        let process = self
            .inner
            .as_ref()
            .ok_or_else(|| Error::from_reason("HyperProcess is closed"))?;
        process
            .endpoint()
            .map(std::string::ToString::to_string)
            .ok_or_else(|| Error::from_reason("No endpoint available"))
    }

    /// Convenience method: connects to this server with a database.
    ///
    /// @param databasePath - Path to the database file.
    /// @param createMode - How to handle database creation.
    #[napi]
    pub async fn connect_to_database(
        &self,
        database_path: String,
        create_mode: CreateMode,
    ) -> Result<Connection> {
        let process = self
            .inner
            .as_ref()
            .ok_or_else(|| Error::from_reason("HyperProcess is closed"))?;
        let endpoint = process
            .endpoint()
            .map(std::string::ToString::to_string)
            .ok_or_else(|| Error::from_reason("No endpoint available"))?;

        Connection::connect(endpoint, database_path, create_mode).await
    }

    #[allow(
        clippy::unnecessary_wraps,
        reason = "signature retained for API symmetry / future fallibility; returning Result/Option keeps callers from breaking when the function later grows failure cases"
    )]
    /// Stops the Hyper server process.
    ///
    /// This must be called when you're done to ensure proper cleanup.
    /// After calling this, no further connections can be made.
    #[napi]
    pub fn close(&mut self) -> Result<()> {
        // Drop the process, which triggers graceful shutdown
        self.inner.take();
        Ok(())
    }

    /// Returns true if the process is still running.
    #[napi(getter)]
    pub fn is_open(&self) -> bool {
        self.inner.is_some()
    }

    /// Returns the path to the Hyper log file (`hyperd.log`).
    ///
    /// Use this with `conn.enableQueryStats(hyper.logPath)` to enable
    /// detailed query performance statistics collection.
    ///
    /// Returns `null` if the log directory could not be determined.
    #[napi(getter)]
    pub fn log_path(&self) -> Result<Option<String>> {
        let process = self
            .inner
            .as_ref()
            .ok_or_else(|| Error::from_reason("HyperProcess is closed"))?;
        Ok(process
            .log_dir()
            .map(|dir| dir.join("hyperd.log").to_string_lossy().to_string()))
    }
}
