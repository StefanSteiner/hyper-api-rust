// Copyright (c) 2026, Salesforce, Inc. All rights reserved.
// SPDX-License-Identifier: Apache-2.0 OR MIT

//! Kani proof harnesses for formal verification of hyperdb-api.
//!
//! These harnesses verify:
//! - PG_IDENTIFIER_LIMIT constant correctness
//! - Name type invariants (try_new rejects empty/long names)
//!
//! NOTE: Harnesses that use `escape_name`, `escape_string_literal`, or `Name::try_new`
//! are excluded because they involve heap allocation (`String::replace`, `format!`)
//! which Kani's solver cannot efficiently handle.

#[cfg(kani)]
mod name_proofs {
    use crate::names::PG_IDENTIFIER_LIMIT;

    /// Verifies PG identifier limit matches PostgreSQL standard.
    #[kani::proof]
    fn pg_identifier_limit_is_63() {
        assert_eq!(PG_IDENTIFIER_LIMIT, 63);
    }
}
