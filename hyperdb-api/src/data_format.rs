// Copyright (c) 2026, Salesforce, Inc. All rights reserved.
// SPDX-License-Identifier: Apache-2.0 OR MIT

//! Data format specification for COPY operations.
//!
//! This module defines the supported data formats for bulk data insertion
//! via the COPY protocol.

/// Data format for COPY operations.
///
/// Hyper supports multiple binary formats for efficient data transfer.
/// The format determines how data is encoded when sent to the server.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub(crate) enum DataFormat {
    /// Hyper's native binary format (default).
    ///
    /// This is the most efficient format for row-by-row data building
    /// using the [`Inserter`](crate::Inserter) API.
    #[default]
    HyperBinary,

    /// Apache Arrow IPC stream format.
    ///
    /// Use this format with [`ArrowInserter`](crate::ArrowInserter) to insert
    /// pre-formatted Arrow IPC stream data. This is useful when integrating
    /// with Arrow-based data pipelines.
    ArrowStream,
}

impl DataFormat {
    #[expect(
        clippy::trivially_copy_pass_by_ref,
        reason = "signature kept for API consistency with the trait family that unifies Copy and non-Copy implementers"
    )]
    /// Returns the SQL format string for COPY commands.
    ///
    /// This is used internally to construct COPY statements.
    #[inline]
    pub(crate) fn as_sql_str(&self) -> &'static str {
        match self {
            DataFormat::HyperBinary => "HYPERBINARY",
            DataFormat::ArrowStream => "ARROWSTREAM",
        }
    }
}

impl std::fmt::Display for DataFormat {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            DataFormat::HyperBinary => write!(f, "HyperBinary"),
            DataFormat::ArrowStream => write!(f, "ArrowStream"),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_data_format_sql_str() {
        assert_eq!(DataFormat::HyperBinary.as_sql_str(), "HYPERBINARY");
        assert_eq!(DataFormat::ArrowStream.as_sql_str(), "ARROWSTREAM");
    }

    #[test]
    fn test_data_format_default() {
        assert_eq!(DataFormat::default(), DataFormat::HyperBinary);
    }

    #[test]
    fn test_data_format_display() {
        assert_eq!(format!("{}", DataFormat::HyperBinary), "HyperBinary");
        assert_eq!(format!("{}", DataFormat::ArrowStream), "ArrowStream");
    }
}
