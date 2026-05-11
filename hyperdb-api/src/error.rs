// Copyright (c) 2026, Salesforce, Inc. All rights reserved.
// SPDX-License-Identifier: Apache-2.0 OR MIT

//! Error types for the pure Rust Hyper API.

use hyperdb_api_core::client::ErrorKind;
use std::error::Error as StdError;
use thiserror::Error as ThisError;

/// The error type for Hyper API operations.
///
/// This enum is `#[non_exhaustive]`: new variants and new fields on existing
/// struct variants may be added in minor releases. Match arms must include a
/// wildcard `_ =>` pattern.
#[derive(Debug, ThisError)]
#[non_exhaustive]
pub enum Error {
    /// Error from the underlying Hyper client.
    #[error("{0}")]
    Client(#[from] hyperdb_api_core::client::Error),

    /// I/O error.
    #[error("I/O error: {0}")]
    Io(#[from] std::io::Error),

    /// Invalid name error (empty or too long).
    #[error("Invalid name: {0}")]
    InvalidName(String),

    /// Invalid table definition.
    #[error("Invalid table definition: {0}")]
    InvalidTableDefinition(String),

    /// Database object not found (table, schema, etc.).
    #[error("Not found: {0}")]
    NotFound(String),

    /// Database object already exists.
    #[error("Already exists: {0}")]
    AlreadyExists(String),

    /// Generic error with a custom message.
    #[error("{message}")]
    #[non_exhaustive]
    Other {
        /// The error message.
        message: String,
        /// The underlying cause of the error, if any.
        #[source]
        source: Option<Box<dyn StdError + Send + Sync>>,
    },
}

impl Error {
    /// Creates a new error with the given message.
    ///
    /// This is a convenience constructor for creating generic errors.
    pub fn new(message: impl Into<String>) -> Self {
        Error::Other {
            message: message.into(),
            source: None,
        }
    }

    /// Creates a new error with a cause.
    ///
    /// This is a convenience constructor for creating generic errors with a source.
    pub fn with_cause<E>(message: impl Into<String>, cause: E) -> Self
    where
        E: Into<Box<dyn StdError + Send + Sync>>,
    {
        Error::Other {
            message: message.into(),
            source: Some(cause.into()),
        }
    }

    /// Returns the error kind, if this is a client error.
    ///
    /// This is available when the error originates from `hyperdb_api_core::client::Error`.
    /// Use this for matching on error categories (e.g., `ErrorKind::Connection`).
    #[must_use]
    pub fn kind(&self) -> Option<ErrorKind> {
        match self {
            Error::Client(err) => Some(err.kind()),
            _ => None,
        }
    }

    /// Returns the error message.
    #[must_use]
    pub fn message(&self) -> String {
        self.to_string()
    }

    /// Extracts the `PostgreSQL` SQLSTATE code from the error, if available.
    ///
    /// This is only available for database query errors from the Hyper client.
    ///
    /// # Example
    ///
    /// ```
    /// use hyperdb_api::Error;
    ///
    /// // Assuming we have a client error with SQLSTATE
    /// // let err: Error = ...;
    /// // if let Some("42P04") = err.sqlstate() {
    /// //     println!("Database already exists");
    /// // }
    /// ```
    #[must_use]
    pub fn sqlstate(&self) -> Option<&str> {
        match self {
            Error::Client(err) => err.sqlstate(),
            _ => None,
        }
    }
}

impl From<std::convert::Infallible> for Error {
    fn from(_: std::convert::Infallible) -> Self {
        unreachable!()
    }
}

/// Result type for Hyper API operations.
pub type Result<T> = std::result::Result<T, Error>;
