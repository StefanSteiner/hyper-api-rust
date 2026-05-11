// Copyright (c) 2026, Salesforce, Inc. All rights reserved.
// SPDX-License-Identifier: Apache-2.0 OR MIT

//! Error types for the Hyper client.

use std::error::Error as StdError;
use std::fmt;
use std::io;

/// The error type for Hyper client operations.
#[derive(Debug)]
pub struct Error {
    kind: ErrorKind,
    message: String,
    cause: Option<Box<dyn StdError + Send + Sync>>,
    /// SQLSTATE error code (for query errors)
    sqlstate_code: Option<String>,
    /// Additional detail about the error
    detail: Option<String>,
    /// Hint for resolving the error
    hint: Option<String>,
}

/// The kind of error that occurred.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ErrorKind {
    /// Connection failed.
    Connection,
    /// Authentication failed.
    Authentication,
    /// Query execution failed.
    Query,
    /// Invalid response from server.
    Protocol,
    /// I/O error.
    Io,
    /// Configuration error.
    Config,
    /// Operation timed out.
    Timeout,
    /// Operation was cancelled.
    Cancelled,
    /// The connection was closed.
    Closed,
    /// Type conversion error.
    Conversion,
    /// Feature not supported by this connection type.
    FeatureNotSupported,
    /// Other error.
    Other,
}

impl Error {
    /// Creates a new error with the given kind and message.
    pub fn new(kind: ErrorKind, message: impl Into<String>) -> Self {
        Error {
            kind,
            message: message.into(),
            cause: None,
            sqlstate_code: None,
            detail: None,
            hint: None,
        }
    }

    /// Creates a new error with a cause.
    pub fn with_cause<E>(kind: ErrorKind, message: impl Into<String>, cause: E) -> Self
    where
        E: Into<Box<dyn StdError + Send + Sync>>,
    {
        Error {
            kind,
            message: message.into(),
            cause: Some(cause.into()),
            sqlstate_code: None,
            detail: None,
            hint: None,
        }
    }

    /// Creates a new error with additional details (SQLSTATE, detail, hint).
    ///
    /// This is primarily used for gRPC errors that carry structured error information.
    pub fn new_with_details(
        kind: ErrorKind,
        message: impl Into<String>,
        detail: Option<String>,
        hint: Option<String>,
        sqlstate: Option<String>,
    ) -> Self {
        Error {
            kind,
            message: message.into(),
            cause: None,
            sqlstate_code: sqlstate,
            detail,
            hint,
        }
    }

    /// Returns the error kind.
    #[must_use]
    pub fn kind(&self) -> ErrorKind {
        self.kind
    }

    /// Returns the error message.
    #[must_use]
    pub fn message(&self) -> &str {
        &self.message
    }

    /// Returns the error detail, if available.
    #[must_use]
    pub fn detail(&self) -> Option<&str> {
        self.detail.as_deref()
    }

    /// Returns the error hint, if available.
    #[must_use]
    pub fn hint(&self) -> Option<&str> {
        self.hint.as_deref()
    }

    // Convenience constructors

    /// Creates a connection error.
    pub fn connection(message: impl Into<String>) -> Self {
        Self::new(ErrorKind::Connection, message)
    }

    /// Creates an authentication error.
    pub fn authentication(message: impl Into<String>) -> Self {
        Self::new(ErrorKind::Authentication, message)
    }

    /// Creates a query error.
    pub fn query(message: impl Into<String>) -> Self {
        Self::new(ErrorKind::Query, message)
    }

    /// Creates a protocol error.
    pub fn protocol(message: impl Into<String>) -> Self {
        Self::new(ErrorKind::Protocol, message)
    }

    /// Creates a closed connection error.
    #[must_use]
    pub fn closed() -> Self {
        Self::new(ErrorKind::Closed, "connection closed")
    }

    /// Creates a timeout error.
    #[must_use]
    pub fn timeout() -> Self {
        Self::new(ErrorKind::Timeout, "operation timed out")
    }

    /// Creates an error from an I/O error.
    #[must_use]
    pub fn io(err: io::Error) -> Self {
        Self::with_cause(ErrorKind::Io, err.to_string(), err)
    }

    /// Creates an error from a database error response.
    #[must_use]
    pub fn db(severity: &str, code: &str, message: &str) -> Self {
        Error {
            kind: ErrorKind::Query,
            message: format!("{severity}: {message} ({code})"),
            cause: None,
            sqlstate_code: Some(code.to_string()),
            detail: None,
            hint: None,
        }
    }

    /// Creates a "feature not supported" error.
    ///
    /// Used when an operation is not available on a particular connection type
    /// (e.g., write operations on gRPC connections).
    pub fn feature_not_supported(message: impl Into<String>) -> Self {
        Self::new(ErrorKind::FeatureNotSupported, message)
    }

    /// Creates a generic "other" error.
    pub fn other(message: impl Into<String>) -> Self {
        Self::new(ErrorKind::Other, message)
    }

    /// Extracts the `PostgreSQL` SQLSTATE code from the error, if present.
    ///
    /// SQLSTATE codes are 5-character codes that identify error conditions.
    /// See: <https://www.postgresql.org/docs/current/errcodes-appendix.html>
    ///
    /// # Example
    ///
    /// ```
    /// use hyperdb_api_core::client::{Error, ErrorKind};
    ///
    /// let err = Error::db("ERROR", "42P04", "database already exists");
    /// assert_eq!(err.sqlstate(), Some("42P04"));
    /// ```
    #[must_use]
    pub fn sqlstate(&self) -> Option<&str> {
        // First check if we have a stored SQLSTATE code
        if let Some(ref code) = self.sqlstate_code {
            return Some(code);
        }
        // Fall back to extracting from message for backwards compatibility
        if self.kind == ErrorKind::Query {
            extract_sqlstate(&self.message)
        } else {
            None
        }
    }
}

/// Extracts the SQLSTATE code from a Hyper error message.
///
/// Hyper error messages have the format: "SEVERITY: message (CODE)"
/// where CODE is the 5-character SQLSTATE code.
fn extract_sqlstate(message: &str) -> Option<&str> {
    // Find the last occurrence of '(' which should contain the SQLSTATE code
    let start = message.rfind('(')?;
    let end = message[start..].find(')')?;

    let code = message[start + 1..start + end].trim();

    // Validate that it looks like a SQLSTATE code (5 alphanumeric characters)
    if code.len() == 5 && code.chars().all(|c| c.is_ascii_alphanumeric()) {
        Some(code)
    } else {
        None
    }
}

impl fmt::Display for Error {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}", self.message)?;
        if let Some(ref detail) = self.detail {
            if !self.message.contains(detail) {
                write!(f, ": {detail}")?;
            }
        }
        if let Some(ref cause) = self.cause {
            write!(f, ": {cause}")?;
        }
        Ok(())
    }
}

impl StdError for Error {
    fn source(&self) -> Option<&(dyn StdError + 'static)> {
        self.cause.as_ref().map(|e| &**e as &dyn std::error::Error)
    }
}

impl From<io::Error> for Error {
    fn from(err: io::Error) -> Self {
        Error::io(err)
    }
}

/// Result type for Hyper client operations.
pub type Result<T> = std::result::Result<T, Error>;

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_sqlstate_extraction() {
        // Standard format: "SEVERITY: message (CODE)"
        let err = Error::db("ERROR", "42P04", "database \"test\" already exists");
        assert_eq!(err.sqlstate(), Some("42P04"));

        // Duplicate object
        let err = Error::db("ERROR", "42710", "duplicate object");
        assert_eq!(err.sqlstate(), Some("42710"));

        // Duplicate schema
        let err = Error::db("ERROR", "42P06", "schema \"public\" already exists");
        assert_eq!(err.sqlstate(), Some("42P06"));

        // Duplicate table
        let err = Error::db("ERROR", "42P07", "table \"users\" already exists");
        assert_eq!(err.sqlstate(), Some("42P07"));
    }

    #[test]
    fn test_sqlstate_non_query_error() {
        // Non-query errors should not have SQLSTATE
        let err = Error::connection("connection failed");
        assert_eq!(err.sqlstate(), None);

        let err = Error::timeout();
        assert_eq!(err.sqlstate(), None);
    }

    #[test]
    fn test_extract_sqlstate_edge_cases() {
        // Valid SQLSTATE
        assert_eq!(extract_sqlstate("ERROR: message (42P04)"), Some("42P04"));

        // With spaces
        assert_eq!(extract_sqlstate("ERROR: message ( 42P04 )"), Some("42P04"));

        // Multiple parentheses (should extract last one)
        assert_eq!(
            extract_sqlstate("ERROR: (extra info) message (42P04)"),
            Some("42P04")
        );

        // Invalid: too short
        assert_eq!(extract_sqlstate("ERROR: message (42P)"), None);

        // Invalid: too long
        assert_eq!(extract_sqlstate("ERROR: message (42P044)"), None);

        // Invalid: non-alphanumeric
        assert_eq!(extract_sqlstate("ERROR: message (42-04)"), None);

        // No parentheses
        assert_eq!(extract_sqlstate("ERROR: message"), None);

        // Empty parentheses
        assert_eq!(extract_sqlstate("ERROR: message ()"), None);
    }
}
