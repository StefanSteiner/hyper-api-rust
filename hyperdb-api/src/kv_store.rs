// Copyright (c) 2026, Salesforce, Inc. All rights reserved.
// SPDX-License-Identifier: Apache-2.0 OR MIT

//! Key-value store over a fixed Hyper table.
//!
//! [`KvStore`] is an ergonomic string-native KV abstraction backed by a
//! single table, [`KV_TABLE`], namespaced by a `store_name` column. Every
//! named store shares that table; a handle binds one store name, validated
//! once at [`Connection::kv_store`](crate::Connection::kv_store).
//!
//! Hyper has no native KV store and no `ON CONFLICT`/`MERGE`; `set` is an
//! `UPDATE`-then-conditional-`INSERT` upsert. See the crate `DEVELOPMENT.md`
//! for the design rationale.

use crate::error::{Error, Result};

/// Fixed backing table for every named KV store.
///
/// The `_hyperdb_` prefix matches the crate's internal-table convention so
/// downstream tooling can auto-hide it from schema listings.
pub(crate) const KV_TABLE: &str = "_hyperdb_kv_store";

/// Maximum length, in bytes, of a store name or key.
pub(crate) const KV_MAX_NAME_BYTES: usize = 512;

/// Human-readable description of the allowed store-name/key charset.
///
/// Used in validation error messages so the allowed set is stated in one
/// place (M-DOCUMENTED-MAGIC) rather than duplicated as a string literal.
pub(crate) const KV_CHARSET: &str = "A-Z a-z 0-9 _ . -";

/// Validates a store name or key: non-empty, ASCII `[A-Za-z0-9_.-]+`, `<= 512` bytes.
///
/// `kind` labels the value in the error message (`"store name"` / `"key"`).
///
/// # Errors
///
/// Returns [`Error::InvalidName`] if `name` is empty, exceeds
/// [`KV_MAX_NAME_BYTES`] bytes, or contains a byte outside the ASCII
/// [`KV_CHARSET`] (`A-Z a-z 0-9 _ . -`).
pub(crate) fn validate_kv_name(name: &str, kind: &str) -> Result<()> {
    if name.is_empty() {
        return Err(Error::invalid_name(format!("KV {kind} must not be empty")));
    }
    if name.len() > KV_MAX_NAME_BYTES {
        return Err(Error::invalid_name(format!(
            "KV {kind} exceeds {KV_MAX_NAME_BYTES}-byte limit ({} bytes)",
            name.len()
        )));
    }
    if let Some(bad) = name
        .bytes()
        .find(|&b| !(b.is_ascii_alphanumeric() || b == b'_' || b == b'.' || b == b'-'))
    {
        return Err(Error::invalid_name(format!(
            "KV {kind} contains an invalid byte {bad:#04x}; allowed: {KV_CHARSET}"
        )));
    }
    Ok(())
}

/// Builds the `CREATE TABLE IF NOT EXISTS` DDL for the KV backing table.
///
/// Single source of truth for the schema, shared by the sync and async
/// constructors and both `kv_list_stores` guards. The table has **no**
/// `PRIMARY KEY`: Hyper rejects one at create time (`0A000: Index support is
/// disabled`, see `hyperdb-mcp/src/table_catalog.rs`), so per-`(store_name,
/// key)` uniqueness is an application-side invariant enforced by the
/// UPDATE-then-conditional-INSERT upsert, not an engine constraint.
pub(crate) fn kv_create_table_sql(table_ref: &str) -> String {
    format!(
        "CREATE TABLE IF NOT EXISTS {table_ref} \
         (store_name TEXT NOT NULL, key TEXT NOT NULL, value TEXT)"
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn accepts_valid_names() {
        for ok in [
            "a",
            "store_1",
            "my.key-2",
            "A",
            &"z".repeat(KV_MAX_NAME_BYTES),
        ] {
            assert!(validate_kv_name(ok, "key").is_ok(), "should accept {ok:?}");
        }
    }

    #[test]
    fn rejects_empty() {
        let err = validate_kv_name("", "store name").unwrap_err();
        assert!(matches!(err, Error::InvalidName(_)));
        assert!(err.to_string().contains("must not be empty"));
    }

    #[test]
    fn rejects_too_long() {
        let long = "a".repeat(KV_MAX_NAME_BYTES + 1);
        let err = validate_kv_name(&long, "key").unwrap_err();
        assert!(matches!(err, Error::InvalidName(_)));
        assert!(err.to_string().contains("byte limit"));
    }

    #[test]
    fn rejects_bad_charset() {
        for bad in ["a b", "a/b", "a'b", "a\"b", "a;b", "naïve", "a\0b"] {
            let err = validate_kv_name(bad, "key").unwrap_err();
            assert!(
                matches!(err, Error::InvalidName(_)),
                "should reject {bad:?}"
            );
        }
    }
}
