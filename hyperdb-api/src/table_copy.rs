// Copyright (c) 2026, Salesforce, Inc. All rights reserved.
// SPDX-License-Identifier: Apache-2.0 OR MIT

//! Reporting types for [`Catalog::copy_table`](crate::Catalog::copy_table).
//!
//! # Why a copy needs a fidelity report
//!
//! `CREATE TABLE … AS SELECT` infers the destination schema from the query's
//! result columns, which carry types but no constraints. Every column in the
//! copy comes out nullable, with no defaults and no keys. A backup taken that
//! way is byte-identical in its data and silently degraded in its schema.
//!
//! [`Catalog::copy_table`](crate::Catalog::copy_table) instead reflects the
//! source schema out of `pg_catalog` and issues an explicit `CREATE TABLE`
//! before moving any rows. Most of the source schema survives that round trip,
//! but not all of it can, so the call returns a [`CopyTableReport`] saying
//! exactly what it reproduced and what it dropped. Callers that need a
//! guarantee should check [`CopyTableReport::is_fully_preserved`] rather than
//! assuming success means fidelity.
//!
//! # What Hyper can and cannot carry
//!
//! Hyper rejects `PRIMARY KEY`, `UNIQUE`, and `FOREIGN KEY` on `CREATE TABLE`
//! with `Index support is disabled`, and `CHECK` with `check constraints not
//! implemented yet`. A Hyper table therefore never *has* one of those to lose.
//! What it can have is `NOT NULL`, `DEFAULT`, and the assumed key forms
//! ([`TableConstraint`](crate::TableConstraint)) — and all four survive a copy.

/// Why a piece of the source schema could not be reproduced.
#[derive(Debug, Clone, PartialEq, Eq)]
#[non_exhaustive]
pub enum UnpreservedReason {
    /// The column's `DEFAULT` expression is not portable to another database.
    ///
    /// Hyper stores non-literal defaults fully qualified by the database that
    /// contains the table: `NOW()` comes back out of `pg_attrdef.adsrc` as
    /// `"mydb"."pg_catalog"."now"()`, and `DATE '2020-01-01'` as
    /// `"mydb"."pg_catalog"."date" '2020-01-01'`. Re-emitting that text into a
    /// different database is accepted by the engine but stores a reference to
    /// `"mydb"` — a database that will not be attached when the copy is opened
    /// on its own later. Dropping the default is the lesser evil, so long as
    /// it is reported.
    NonPortableDefault,
}

impl std::fmt::Display for UnpreservedReason {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::NonPortableDefault => f.write_str(
                "DEFAULT expression is database-qualified and would not resolve \
                 in the destination database",
            ),
        }
    }
}

/// One piece of source schema that [`Catalog::copy_table`](crate::Catalog::copy_table)
/// could not reproduce.
#[derive(Debug, Clone, PartialEq, Eq)]
#[non_exhaustive]
pub struct UnpreservedItem {
    /// The column the dropped schema element belonged to.
    pub column: String,
    /// Why it could not be carried across.
    pub reason: UnpreservedReason,
    /// The offending source text, so the caller can reapply it by hand.
    pub detail: String,
}

impl std::fmt::Display for UnpreservedItem {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}: {} ({})", self.column, self.reason, self.detail)
    }
}

/// What a [`Catalog::copy_table`](crate::Catalog::copy_table) call reproduced,
/// and what it dropped.
///
/// A copy that returns `Ok` has moved every row. It has **not** necessarily
/// reproduced every constraint — check [`is_fully_preserved`](Self::is_fully_preserved).
#[derive(Debug, Clone, Default, PartialEq, Eq)]
#[non_exhaustive]
pub struct CopyTableReport {
    /// Rows written into the destination table.
    pub rows_copied: u64,
    /// `NOT NULL` columns carried across.
    pub not_null_columns: usize,
    /// `DEFAULT` expressions carried across.
    pub default_columns: usize,
    /// `ASSUMED PRIMARY KEY` constraints carried across.
    pub assumed_primary_keys: usize,
    /// `ASSUMED UNIQUE` constraints carried across.
    pub assumed_unique_constraints: usize,
    /// Schema elements that could not be reproduced.
    pub unpreserved: Vec<UnpreservedItem>,
}

impl CopyTableReport {
    /// Returns `true` if every constraint on the source was reproduced.
    #[must_use]
    pub fn is_fully_preserved(&self) -> bool {
        self.unpreserved.is_empty()
    }

    /// Total number of constraints carried across, across all classes.
    #[must_use]
    pub fn preserved_count(&self) -> usize {
        self.not_null_columns
            .saturating_add(self.default_columns)
            .saturating_add(self.assumed_primary_keys)
            .saturating_add(self.assumed_unique_constraints)
    }

    /// Folds another report into this one, for callers copying many tables.
    pub fn merge(&mut self, other: Self) {
        self.rows_copied = self.rows_copied.saturating_add(other.rows_copied);
        self.not_null_columns = self.not_null_columns.saturating_add(other.not_null_columns);
        self.default_columns = self.default_columns.saturating_add(other.default_columns);
        self.assumed_primary_keys = self
            .assumed_primary_keys
            .saturating_add(other.assumed_primary_keys);
        self.assumed_unique_constraints = self
            .assumed_unique_constraints
            .saturating_add(other.assumed_unique_constraints);
        self.unpreserved.extend(other.unpreserved);
    }
}

/// Returns `true` if a `DEFAULT` expression can be re-emitted into a different
/// database unchanged.
///
/// The test is deliberately blunt: an expression is portable only if it
/// contains no double-quote character. Hyper writes every non-literal default
/// back out of `pg_attrdef.adsrc` with quoted, database-qualified identifiers
/// (`"mydb"."pg_catalog"."now"()`), while plain literals — `-5`, `TRUE`,
/// `1.5`, `3.14`, `'it''s'` — contain none.
///
/// The rule can only err toward caution. A string literal that happens to
/// contain a double quote (`'say "hi"'`) is portable but gets rejected, which
/// costs a reported default rather than a silently broken one. It can never
/// admit a database-qualified expression, because those always carry quotes.
pub(crate) fn is_portable_default(expr: &str) -> bool {
    !expr.contains('"')
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn plain_literals_are_portable() {
        // Exactly the forms observed round-tripping through pg_attrdef.adsrc.
        for expr in [
            "-5", "TRUE", "1.5", "3.14", "0", "'it''s'", "'plain'", "NULL",
        ] {
            assert!(is_portable_default(expr), "{expr} should be portable");
        }
    }

    #[test]
    fn database_qualified_expressions_are_not_portable() {
        for expr in [
            r#""mydb"."pg_catalog"."now"()"#,
            r#""mydb"."pg_catalog"."date" '2020-01-01'"#,
            r#""mydb"."public"."my_fn"()"#,
        ] {
            assert!(!is_portable_default(expr), "{expr} should not be portable");
        }
    }

    #[test]
    fn quoted_text_literal_errs_toward_caution() {
        // Portable in truth, rejected by the rule. Costs a reported default,
        // never a broken one.
        assert!(!is_portable_default(r#"'say "hi"'"#));
    }

    #[test]
    fn merge_accumulates_counts_and_items() {
        let mut a = CopyTableReport {
            rows_copied: 2,
            not_null_columns: 1,
            assumed_primary_keys: 1,
            ..Default::default()
        };
        a.merge(CopyTableReport {
            rows_copied: 3,
            not_null_columns: 2,
            default_columns: 1,
            assumed_unique_constraints: 1,
            unpreserved: vec![UnpreservedItem {
                column: "t".into(),
                reason: UnpreservedReason::NonPortableDefault,
                detail: r#""db"."pg_catalog"."now"()"#.into(),
            }],
            ..Default::default()
        });

        assert_eq!(a.rows_copied, 5);
        assert_eq!(a.not_null_columns, 3);
        assert_eq!(a.default_columns, 1);
        assert_eq!(a.assumed_primary_keys, 1);
        assert_eq!(a.assumed_unique_constraints, 1);
        assert_eq!(a.preserved_count(), 6);
        assert!(!a.is_fully_preserved());
        assert_eq!(a.unpreserved.len(), 1);
    }

    #[test]
    fn empty_report_is_fully_preserved() {
        let report = CopyTableReport::default();
        assert!(report.is_fully_preserved());
        assert_eq!(report.preserved_count(), 0);
    }
}
