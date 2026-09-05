# Changelog

All notable changes to the `hyperdb-api-node` package will be documented in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/),
and this project adheres to [Semantic Versioning](https://semver.org/).

## [0.1.3](https://github.com/tableau/hyper-api-rust/compare/v0.1.2...v0.1.3) (2026-05-18)


### Bug Fixes

* v0.1.2 release — bump versions and add safety net ([#17](https://github.com/tableau/hyper-api-rust/issues/17)) ([bae4536](https://github.com/tableau/hyper-api-rust/commit/bae453600ce94ddc318ccb1cfe89be8fa32eef85))

## [Unreleased]

### Changed

- **`getInt32()` and `getInt32Column()` now throw instead of silently
  truncating an out-of-range `BIGINT`.** Previously an `Int64` value that did
  not fit an `Int32` was wrapped with a narrowing cast, returning a
  plausible-looking but wrong number. Returning `null` was considered and
  rejected because it is indistinguishable from a SQL NULL. Use `getBigInt()`
  for lossless row access, or `getInt64Column()` to widen instead of narrow
  (noting that it returns `number`, which itself loses precision above 2^53).
- **Inserting an out-of-range integer into a `SMALLINT` or `INT` column is now
  rejected** rather than silently truncated. The previous code claimed Hyper
  would reject an oversized value at `execute()` time, which was not true for
  the narrowing paths: a wrapped value is a valid encoding of the wrong number,
  so it was accepted and the `.hyper` file held corrupt data.

  Float-to-integer coercions are unchanged. Those *saturate* in Rust rather
  than wrapping — an out-of-range float clamps to the type's bounds and `NaN`
  becomes 0 — so they are lossy but bounded, and remain the documented
  coercion for these paths.

### Fixed

- `NUMERIC` columns are now decoded correctly. Previously the bindings read
  numerics with `getF64`, which reinterpreted the raw unscaled-integer bytes as
  an IEEE-754 double and returned garbage values or `NaN`. Numerics are now
  decoded schema-aware (honoring the column scale): `getString` returns the
  exact decimal text (preserving scale and sign, including sub-unit negatives
  such as `-0.5000`), `getFloat64` returns the correct (possibly lossy) value,
  and `getInt32`/`getInt64` return the truncated integer. `getBigInt` on a
  `NUMERIC(p, 0)` column now preserves the full unscaled value (use it instead
  of `getInt64` for integer NUMERIC values above `Number.MAX_SAFE_INTEGER`); on
  a `NUMERIC(p, scale>0)` column it returns `null` (use `getString` for exact
  text or `getFloat64` for a lossy value). The columnar fast path
  (`executeQueryColumnar`) surfaces numerics as correct `f64` values instead of
  garbage. Relates to [#84](https://github.com/tableau/hyper-api-rust/issues/84).

## [0.1.1] - 2026-05-13

### Added

- `HyperProcess` for managing local `hyperd` server instances
- `Connection` with async `executeQuery`, `executeCommand`, `querySchema`
- `Catalog` for schema and table introspection
- `Inserter` for bulk data insertion via HyperBinary COPY
- `ConnectionPool` (in `pool.mjs`) for async connection pooling
- Arrow IPC support via `executeQueryToArrow` and Apache Arrow integration
- Query statistics collection via `enableQueryStats` and `lastQueryStats`
- Row-level data access with typed getters and JSON serialization
- Cross-platform native binaries (macOS ARM64/x64, Linux x64/ARM64/musl, Windows x64)
