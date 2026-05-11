// Copyright (c) 2026, Salesforce, Inc. All rights reserved.
// SPDX-License-Identifier: Apache-2.0 OR MIT

//! Unified query result type that works across TCP and gRPC transports.
//!
//! This module provides [`QueryResult`] which abstracts over the differences
//! between TCP streaming results and gRPC Arrow-based results, giving users
//! a consistent API regardless of the underlying transport.

#![expect(
    dead_code,
    reason = "infrastructure for unified result iteration; exercised per transport feature combo"
)]

#[allow(
    unused_imports,
    reason = "imported for use in doc comments that reference the type path"
)]
use crate::error::Result;

use crate::arrow_result::{ArrowChunk, ArrowRow, ArrowRowset};

/// A unified row that provides typed value access regardless of transport.
///
/// This enum wraps either a TCP `StreamRow` or a gRPC `ArrowRow`,
/// providing the same `get<T>()` interface for both.
#[derive(Debug)]
pub enum UnifiedRow<'a> {
    /// Row from TCP transport (binary protocol).
    Tcp(&'a hyperdb_api_core::client::StreamRow),
    /// Row from gRPC transport (Arrow format).
    Arrow(ArrowRow<'a>),
}

impl UnifiedRow<'_> {
    /// Returns the number of columns in this row.
    pub fn column_count(&self) -> usize {
        match self {
            UnifiedRow::Tcp(row) => row.column_count(),
            UnifiedRow::Arrow(row) => row.column_count(),
        }
    }

    /// Gets a value at the given column index with type conversion.
    ///
    /// Returns `None` if the value is NULL or the column doesn't exist.
    pub fn get<T: UnifiedFromRow>(&self, col: usize) -> Option<T> {
        T::from_unified_row(self, col)
    }

    /// Gets an i16 value at the given column.
    pub fn get_i16(&self, col: usize) -> Option<i16> {
        self.get::<i16>(col)
    }

    /// Gets an i32 value at the given column.
    pub fn get_i32(&self, col: usize) -> Option<i32> {
        self.get::<i32>(col)
    }

    /// Gets an i64 value at the given column.
    pub fn get_i64(&self, col: usize) -> Option<i64> {
        self.get::<i64>(col)
    }

    /// Gets an f32 value at the given column.
    pub fn get_f32(&self, col: usize) -> Option<f32> {
        self.get::<f32>(col)
    }

    /// Gets an f64 value at the given column.
    pub fn get_f64(&self, col: usize) -> Option<f64> {
        self.get::<f64>(col)
    }

    /// Gets a bool value at the given column.
    pub fn get_bool(&self, col: usize) -> Option<bool> {
        self.get::<bool>(col)
    }

    /// Gets a String value at the given column.
    pub fn get_string(&self, col: usize) -> Option<String> {
        self.get::<String>(col)
    }

    /// Checks if the value at the given column is NULL.
    pub fn is_null(&self, col: usize) -> bool {
        match self {
            UnifiedRow::Tcp(row) => row.is_null(col),
            UnifiedRow::Arrow(row) => row.is_null(col),
        }
    }
}

/// Trait for types that can be extracted from a `UnifiedRow`.
pub trait UnifiedFromRow: Sized {
    /// Extract a value from a unified row at the given column.
    fn from_unified_row(row: &UnifiedRow<'_>, col: usize) -> Option<Self>;
}

impl UnifiedFromRow for i16 {
    fn from_unified_row(row: &UnifiedRow<'_>, col: usize) -> Option<Self> {
        match row {
            UnifiedRow::Tcp(r) => r.get::<i16>(col),
            UnifiedRow::Arrow(r) => r.get::<i16>(col),
        }
    }
}

impl UnifiedFromRow for i32 {
    fn from_unified_row(row: &UnifiedRow<'_>, col: usize) -> Option<Self> {
        match row {
            UnifiedRow::Tcp(r) => r.get::<i32>(col),
            UnifiedRow::Arrow(r) => r.get::<i32>(col),
        }
    }
}

impl UnifiedFromRow for i64 {
    fn from_unified_row(row: &UnifiedRow<'_>, col: usize) -> Option<Self> {
        match row {
            UnifiedRow::Tcp(r) => r.get::<i64>(col),
            UnifiedRow::Arrow(r) => r.get::<i64>(col),
        }
    }
}

impl UnifiedFromRow for f32 {
    fn from_unified_row(row: &UnifiedRow<'_>, col: usize) -> Option<Self> {
        match row {
            UnifiedRow::Tcp(r) => r.get::<f32>(col),
            UnifiedRow::Arrow(r) => r.get::<f32>(col),
        }
    }
}

impl UnifiedFromRow for f64 {
    fn from_unified_row(row: &UnifiedRow<'_>, col: usize) -> Option<Self> {
        match row {
            UnifiedRow::Tcp(r) => r.get::<f64>(col),
            UnifiedRow::Arrow(r) => r.get::<f64>(col),
        }
    }
}

impl UnifiedFromRow for bool {
    fn from_unified_row(row: &UnifiedRow<'_>, col: usize) -> Option<Self> {
        match row {
            UnifiedRow::Tcp(r) => r.get::<bool>(col),
            UnifiedRow::Arrow(r) => r.get::<bool>(col),
        }
    }
}

impl UnifiedFromRow for String {
    fn from_unified_row(row: &UnifiedRow<'_>, col: usize) -> Option<Self> {
        match row {
            UnifiedRow::Tcp(r) => r.get::<String>(col),
            UnifiedRow::Arrow(r) => r.get::<String>(col),
        }
    }
}

/// A unified chunk of rows from a query result.
///
/// Provides iteration over rows regardless of the underlying transport.
#[derive(Debug)]
pub enum UnifiedChunk<'a> {
    /// Chunk from TCP transport (holds reference to avoid ownership issues).
    Tcp(&'a [hyperdb_api_core::client::StreamRow]),
    /// Chunk from gRPC transport.
    Arrow(&'a ArrowChunk),
}

impl UnifiedChunk<'_> {
    /// Returns the number of rows in this chunk.
    pub fn len(&self) -> usize {
        match self {
            UnifiedChunk::Tcp(rows) => rows.len(),
            UnifiedChunk::Arrow(chunk) => chunk.len(),
        }
    }

    /// Returns true if this chunk has no rows.
    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }

    /// Returns the first row, if any.
    pub fn first(&self) -> Option<UnifiedRow<'_>> {
        match self {
            UnifiedChunk::Tcp(rows) => rows.first().map(UnifiedRow::Tcp),
            UnifiedChunk::Arrow(chunk) => chunk.first().map(UnifiedRow::Arrow),
        }
    }

    /// Returns an iterator over the rows.
    pub fn iter(&self) -> UnifiedChunkIter<'_> {
        UnifiedChunkIter {
            chunk: self,
            index: 0,
        }
    }
}

impl<'a, 'b> IntoIterator for &'b UnifiedChunk<'a>
where
    'a: 'b,
{
    type Item = UnifiedRow<'b>;
    type IntoIter = UnifiedChunkIter<'b>;

    fn into_iter(self) -> Self::IntoIter {
        UnifiedChunkIter {
            chunk: self,
            index: 0,
        }
    }
}

/// Iterator over rows in a `UnifiedChunk`.
#[derive(Debug)]
pub struct UnifiedChunkIter<'a> {
    chunk: &'a UnifiedChunk<'a>,
    index: usize,
}

impl<'a> Iterator for UnifiedChunkIter<'a> {
    type Item = UnifiedRow<'a>;

    fn next(&mut self) -> Option<Self::Item> {
        let result = match self.chunk {
            UnifiedChunk::Tcp(rows) => rows.get(self.index).map(UnifiedRow::Tcp),
            UnifiedChunk::Arrow(chunk) => chunk.row(self.index).map(UnifiedRow::Arrow),
        };
        if result.is_some() {
            self.index += 1;
        }
        result
    }

    fn size_hint(&self) -> (usize, Option<usize>) {
        let remaining = self.chunk.len().saturating_sub(self.index);
        (remaining, Some(remaining))
    }
}

impl ExactSizeIterator for UnifiedChunkIter<'_> {}

/// Unified query result that works across both TCP and gRPC transports.
///
/// This provides a consistent iteration API regardless of the underlying
/// transport mechanism.
pub(crate) enum QueryResult<'conn> {
    /// TCP streaming result.
    Tcp(crate::result::Rowset<'conn>),
    /// gRPC Arrow-based result.
    Arrow(ArrowQueryResult),
}

/// Wrapper for Arrow-based query results from gRPC.
pub(crate) struct ArrowQueryResult {
    rowset: ArrowRowset,
    current_chunk: Option<ArrowChunk>,
}

impl ArrowQueryResult {
    /// Creates a new `ArrowQueryResult` from shared Arrow IPC bytes (zero-copy).
    pub(crate) fn from_bytes(bytes: bytes::Bytes) -> Result<Self> {
        let rowset = ArrowRowset::from_bytes(bytes)?;
        Ok(ArrowQueryResult {
            rowset,
            current_chunk: None,
        })
    }

    /// Creates a new `ArrowQueryResult` from a borrowed Arrow IPC slice.
    ///
    /// This copies the slice once into an arrow `Buffer`. Prefer
    /// [`from_bytes`](Self::from_bytes) when you already own a `Bytes`.
    pub(crate) fn from_ipc_slice(data: &[u8]) -> Result<Self> {
        let rowset = ArrowRowset::from_ipc_slice(data)?;
        Ok(ArrowQueryResult {
            rowset,
            current_chunk: None,
        })
    }

    /// Gets the next chunk.
    pub(crate) fn next_chunk(&mut self) -> Result<Option<&ArrowChunk>> {
        self.current_chunk = self.rowset.next_chunk()?;
        Ok(self.current_chunk.as_ref())
    }

    /// Returns the underlying `ArrowRowset`.
    pub(crate) fn into_rowset(self) -> ArrowRowset {
        self.rowset
    }
}

impl QueryResult<'_> {}
