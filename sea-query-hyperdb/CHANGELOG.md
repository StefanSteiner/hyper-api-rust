# Changelog

All notable changes to the `sea-query-hyperdb` crate will be documented in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/),
and this project adheres to [Semantic Versioning](https://semver.org/).

## [Unreleased]

### Security

- **Fixed SQL injection in inlined string literals.** `HyperQueryBuilder` used
  sea-query's default MySQL backslash escaping, but Hyper (like PostgreSQL with
  `standard_conforming_strings`) treats a backslash in a plain `'...'` literal
  literally. A value such as `x\' OR 1=1 --` rendered through
  `Expr::Constant(Value::String(..))`, `Order::Field`, a `LIKE ... ESCAPE`
  constant, `inject_parameters`, or `HyperQueryBuilder::value_to_string` escaped
  its own quote and matched every row. Escaping now follows PostgreSQL rules
  (`E'...'` whenever a backslash, quote or control character is present).
  `Expr::val(..)` / `.eq(..)` / `INSERT` values were already safe by accident of
  delegation. Anyone who inlined untrusted strings through the affected paths
  should upgrade.

### Fixed

- `escape_string` / `unescape_string` now use PostgreSQL rules instead of MySQL
  rules, which corrupted data containing `\` or `"`.
- `Value::Bytes` renders as `'\x...'` instead of `x'...'` (Hyper: `0A000 bit
  strings not implemented`).
- Non-ASCII `Value::Char` no longer panics (it was truncated to one byte).
- Every `PgBinOper` (`ILIKE`, `NOT ILIKE`, `~`, `~*`, `||`, `->`, `->>`, ...), the
  `PgExpr` helpers, every `PgFunc` (`STARTS_WITH`, `DATE_TRUNC`, `ANY`, ...) and
  `SqliteBinOper` no longer hit `unimplemented!()`. Operators Hyper lacks (`@>`,
  `<@`, `&&`, `@@`, `%`, `<->`) are rendered verbatim and rejected by the server
  instead of panicking in the builder.
- `Select::distinct_on(..)` rendered `SELECT  "a"` with `DISTINCT ON` dropped;
  `order_by_with_nulls(.., NullOrdering::..)` lost `NULLS FIRST/LAST` (also
  inside `FROM` subqueries). Both are now emitted.
- `Expr::as_enum(..)` keeps its `CAST(.. AS "type")`; `TABLESAMPLE` (with
  `REPEATABLE`) is no longer silently dropped; `Func::if_null` emits `COALESCE`.
- `PARTITION BY` / `FOR VALUES` clauses are written out instead of being
  silently dropped or panicking (Hyper has no declarative partitioning and
  rejects them).
- A NUL character is escaped as `\000` rather than `\0`, so literal digits that
  follow it can no longer extend the octal escape and close the string literal.
- `Value::Enum` bound through `.build(..)` is sent as a plain string parameter,
  and `ColumnDef` `DEFAULT` expressions are rendered by Hyper (non-ASCII
  `Value::Char`, bare enum strings) instead of by the Postgres builder;
  auto-increment on a non-integer column no longer panics. `Expr::as_enum` with
  a multi-dimensional array type (`"t[][]"`) keeps every `[]` outside the quotes.
- SQLite-only operators unknown to this crate are written as a placeholder the
  server rejects, never as Rust `Debug` text.

### Changed

- **Breaking:** the SQL text for inlined strings containing `\`, `'`, `"` or
  control characters, for `Value::Bytes`, and for `Value::Char` changes as
  described above. Code that compared generated SQL byte-for-byte must update.
- **Breaking:** `Value::Enum` is rendered as a bare string literal (no
  `::"type"` hint), because Hyper has no enum types, and `Expr::as_enum(..)`
  renders `CAST(<expr> AS "type")`, so enum-typed casts still need a real type
  on the server.
- Minimum `sea-query` is now 1.0.2 (the version the builder was written and
  tested against).
- Known limitation: `ORDER BY` expressions, `EXPLAIN`, `CREATE INDEX` and
  `ALTER TABLE` are rendered by `PostgresQueryBuilder` because sea-query keeps
  their fields crate-private; Hyper-specific rendering of non-ASCII
  `Value::Char`, `Value::Enum` and SQLite operators does not apply inside them.
- `HyperQueryBuilder` is no longer documented as identical to
  `PostgresQueryBuilder`; it owns statement dispatch, value rendering and
  `AsEnum`. `!~`, `!~*` and `SIMILAR TO` need `BinOper::Custom` / `Expr::cust`.
- New dev-dependency on `hyperdb-api` (path-only) for `tests/hyper_integration.rs`.

## [1.0.0-rc.3] - 2026-09-07

These changes first shipped in 1.0.0-rc.1.

### Changed

- The minimum supported Rust version is now **1.88**, up from
  1.81, and the crate is compiled with **edition 2024**. 1.88 is the version
  Red Hat Enterprise Linux 9.7 ships as `rust-toolset`. The previous 1.81 was
  not achievable in practice — the lockfile already required 1.88 for several
  direct dependencies.

## [0.1.1] - 2026-05-13

### Added

- `HyperQueryBuilder` implementing `sea_query::QueryBuilder` for HyperDB SQL dialect
- PostgreSQL-compatible SQL generation with Hyper-specific type handling
- Support for all standard sea-query operations (SELECT, INSERT, UPDATE, DELETE, CREATE TABLE)
