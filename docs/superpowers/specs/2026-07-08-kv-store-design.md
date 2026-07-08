# Key-Value Store for the Hyper Rust API + MCP — Design

**Status:** Approved (design), M1 plan pending
**Date:** 2026-07-08
**Author:** Stefan Steiner (with Claude)

## Context

The `hyperdb-api` crate is a pure-Rust client for the Hyper database (PostgreSQL
wire protocol + Hyper extensions). Hyper is an OLAP columnar engine; it has **no
native key-value store**. This feature adds a small, ergonomic KV abstraction
*on top of* an ordinary Hyper SQL table, plus (in a second milestone) an MCP tool
surface so an LLM can use it as a frictionless persistent scratchpad.

The seed for this design was an exploratory conversation with Gemini. This spec
adjusts that sketch to fit two hard facts verified against the Hyper engine
source (`../hyper-db`) and the crate's own architecture:

1. **Hyper has no `ON CONFLICT` / `MERGE` / `UPSERT`.** Confirmed against the SQL
   grammar (`hyper/parser/sql/sql.ypp`, `SQLKeywords.hpp` — no `CONFLICT`
   keyword, `INSERT` has no upsert clause). Upsert must be emulated as
   `UPDATE`-then-conditional-`INSERT` inside a transaction — the exact idiom the
   repo already uses for its `_table_catalog` meta-table
   (`hyperdb-mcp/src/table_catalog.rs`).
2. **`query_as!` cannot be used *inside* `hyperdb-api`.** The macro lives in the
   sibling crate `hyperdb-api-derive`, which depends back on `hyperdb-api`; using
   it internally would create a dependency cycle (documented at
   `hyperdb-api/src/lib.rs:208-211`). The library implements its own queries with
   `command_params` / `query_params` / `fetch_optional_scalar`. **`query_as!`
   remains fully available to end users** querying the KV table.

### Why the `query_as!` constraint costs no runtime performance

`query_as!`'s benefit is **compile-time SQL verification**, not runtime speed.
The same SQL string with the same bound parameters travels the same wire path to
`hyperd` regardless of whether the macro or `command_params` produced it —
identical execution, identical speed. Using `command_params` internally forgoes
only compile-time validation of the library's ~8 hardcoded queries (written and
tested once), and end users lose nothing.

## Goals

- A typed, string-native KV store usable from both the sync (`Connection`) and
  async (`AsyncConnection`) APIs, following the crate's existing dual-API
  convention.
- **Multiple named stores** partitioned by a `store_name` namespace column.
- Core operations: get, set (upsert), delete, exists, size, keys, pop
  (destructive get-next), clear, and cross-store discovery (`list_stores`).
- Opt-in typed access via `serde_json` (`get_as` / `set_as`).
- A later MCP milestone exposing these as tools plus a documented SQL LEFT JOIN
  "enrichment" pattern (KV metadata ⋈ analytical tables).

## Non-Goals

- No FIFO queue / blocking semantics (`pop` is a destructive read, not a queue).
- No TTL/expiry, no watch/subscribe, no transactions spanning multiple KV calls
  exposed to the caller (each op is internally atomic; no caller-managed txn).
- No binary (`BYTES`) values in M1 — values are `TEXT` (strings, incl. JSON).
- No duplicate keys within a store (composite PK enforces uniqueness; a
  history/append variant is explicitly out of scope).
- No public table-name/location configuration in M1's surface (see Milestone 1
  §"Table targeting").

## Architecture Overview

### Backing table

A single, fixed backing table holds every named store, namespaced by
`store_name` (the "single table" approach — chosen over table-per-store):

```sql
CREATE TABLE IF NOT EXISTS _hyperdb_kv_store (
    store_name TEXT NOT NULL,
    key        TEXT NOT NULL,
    value      TEXT,                 -- NULL allowed: a key may hold a null value
    PRIMARY KEY (store_name, key)
);
```

**Table name: `_hyperdb_kv_store`.** The `_hyperdb_` prefix matches the crate's
live convention (`HYPERDB_INTERNAL_PREFIX` in `hyperdb-mcp/src/engine.rs:1623`,
alongside `_hyperdb_saved_queries`). In M2 this makes `is_internal_table()`
(`engine.rs:1634`) auto-hide the table from the MCP `describe`/`status` listings
with zero special-casing. Hidden ≠ inaccessible: the LLM still joins it freely
once it learns the name from the readme / MCP resource.

**Why single-table (not table-per-store):**

| Concern | Single table | Table-per-store |
|---|---|---|
| Point lookup / COUNT | radix-prefix on `store_name`, then key | scan smaller table | ≈ equal |
| `list_stores` | `SELECT DISTINCT store_name` (one query) | query system catalog | single-table simpler |
| Create/drop a store | no DDL / `DELETE WHERE store_name=` | runtime `CREATE`/`DROP TABLE` | single-table simpler + safer |
| SQL shape | 100% static | dynamic `format!("… {store} …")` names | single-table safer |
| Disk reclaim on clear | `DELETE` leaves MVCC tombstones until compaction | `DROP TABLE` reclaims instantly | table-per-store wins (negligible at KV scale) |

The lookup speed is a wash at the expected scale (config / agent state /
scratchpad — thousands of rows). The single-table win is operational: fully
static SQL, no runtime DDL, no `format!`-built table names.

### PRIMARY KEY enforcement — verify empirically

The Hyper grammar supports enforced, index-backed `PRIMARY KEY` (default index
is an Adaptive Radix Tree; see `hyper/cts/infra/RelationOptions.hpp`). However, a
comment in `hyperdb-mcp/src/saved_queries.rs` asserts "Hyper has no indexes" and
that crate enforces uniqueness application-side. **M1's first implementation step
empirically probes PK enforcement** against the pinned `hyperd` (insert a
duplicate `(store_name, key)`, expect an error). Outcome:

- **If enforced:** keep the PK; it provides both uniqueness and fast lookups.
- **If not enforced:** keep the PK as an optimization/index hint, and rely on the
  upsert emulation (below), which already guarantees single-row-per-key
  application-side.

Either way **the public API is identical** — this only affects internal
guarantees and test expectations.

### Upsert emulation

Hyper has no `ON CONFLICT`. `set` is implemented inside a transaction as:

```sql
UPDATE _hyperdb_kv_store SET value = $3 WHERE store_name = $1 AND key = $2;
-- if 0 rows affected:
INSERT INTO _hyperdb_kv_store (store_name, key, value)
SELECT $1, $2, $3
WHERE NOT EXISTS (
    SELECT 1 FROM _hyperdb_kv_store WHERE store_name = $1 AND key = $2
);
```

This mirrors `table_catalog.rs`'s proven pattern. `hyperd` serializes statements
within a transaction, so the read-modify-write is atomic.

## Milestone 1 — Rust API (`hyperdb-api`)

**The real feature. PR title uses a `feat:` prefix.**

### Public surface

Following the `Catalog`/`Inserter` convention (companion struct borrowing
`&'conn Connection`, with an async twin borrowing `&'conn AsyncConnection`):

```rust
// sync — src/kv_store.rs
impl Connection {
    /// Open a handle to a named KV store, creating the backing table if needed.
    pub fn kv_store(&self, name: &str) -> Result<KvStore<'_>>;
    /// Discover which named stores currently exist (SELECT DISTINCT store_name).
    pub fn kv_list_stores(&self) -> Result<Vec<String>>;
}

pub struct KvStore<'conn> { /* &conn, validated store_name, internal target */ }

impl<'conn> KvStore<'conn> {
    pub fn get(&self, key: &str) -> Result<Option<String>>;
    pub fn set(&self, key: &str, value: &str) -> Result<()>;          // upsert
    pub fn get_as<T: DeserializeOwned>(&self, key: &str) -> Result<Option<T>>;
    pub fn set_as<T: Serialize>(&self, key: &str, value: &T) -> Result<()>;
    pub fn delete(&self, key: &str) -> Result<bool>;                 // true if a row was removed
    pub fn exists(&self, key: &str) -> Result<bool>;
    pub fn size(&self) -> Result<i64>;                               // COUNT(*) for this store
    pub fn keys(&self) -> Result<Vec<String>>;                       // ORDER BY key
    pub fn pop(&self) -> Result<Option<(String, String)>>;           // destructive get-next
    pub fn clear(&self) -> Result<u64>;                              // DELETE all in this store; returns count
    pub fn name(&self) -> &str;
}
```

The async twin (`src/async_kv_store.rs`) exposes `AsyncConnection::kv_store` /
`kv_list_stores` returning `AsyncKvStore<'conn>` with the same method names as
`async fn`. No `Owned` (`Arc<AsyncConnection>`) variant in M1 — deferred under
YAGNI until a caller needs a spawnable handle.

### Method semantics

- **Handle binds the store name once** — validated at `kv_store(name)`, not on
  every call.
- **`set`** — upsert via the emulation above.
- **`get`** — `SELECT value ... WHERE store_name=$1 AND key=$2`, returns
  `Ok(None)` when absent. Note: a present key with a SQL-NULL value also yields
  `Ok(None)`; M1 treats "absent" and "null value" identically at the `get` level.
- **`get_as` / `set_as`** — `serde_json` layer. `set_as` serializes `T` to a JSON
  string and stores it; `get_as` fetches the string and deserializes, mapping
  parse failures to `Error::Serialization`. `get_as` returns `Ok(None)` when the
  key is absent.
- **`delete`** — returns `true` iff a row was removed (via affected-row count).
- **`exists`** — cheap `SELECT 1 ... LIMIT 1` existence check.
- **`size`** — `SELECT COUNT(*) ... WHERE store_name=$1`, returns `i64`
  directly (no narrowing cast).
- **`keys`** — `SELECT key ... ORDER BY key ASC`, collected to `Vec<String>`.
- **`pop`** — in a transaction: `SELECT key, value ... ORDER BY key ASC LIMIT 1`,
  then `DELETE` that exact `(store_name, key)`; returns the pair or `None`.
  Atomic peek+delete.
- **`clear`** — `DELETE ... WHERE store_name=$1`; returns count removed
  (Gemini's `drop_store`, renamed — the shared table always survives).
- **`kv_list_stores`** — `SELECT DISTINCT store_name ORDER BY store_name`.

### Name validation

`store_name` and `key` must be non-empty, match `[A-Za-z0-9_.-]+`, and be at most
512 bytes. Violations return `Error::invalid_name`. SQL injection is already
impossible via parameterized queries; this rule keeps names clean and
predictable (per the LLM-ergonomics rationale). Applied to `store_name` at
`kv_store(name)`, and to `key` on every keyed call.

### Table targeting (internal in M1, used by M2)

`KvStore` internally holds an optional schema/database qualifier. M1's **public**
surface is only `kv_store(name)` (default location). A crate-internal constructor
(e.g. `KvStore::with_target(conn, name, target)`) accepts a schema/database
target; M2 uses it to route into the MCP's `persistent` attached database. This
keeps M1's public surface minimal while satisfying M2 without a later API break.

### Errors

Add one variant: `Error::Serialization(String)` (in `hyperdb-api/src/error.rs`)
with a constructor helper `Error::serialization(...)`, for `get_as`/`set_as`
failures. Reuse existing variants otherwise: `invalid_name`,
`feature_not_supported`, `Server`, etc. Do **not** introduce a separate error
enum.

### Transport gating

Write and parameterized paths are TCP-only, matching `Inserter::new` and
`query_params` (which return `Error::feature_not_supported` on gRPC). All KV
methods use parameterized queries, so the whole surface is TCP-gated.

### Dependencies

Add `serde` + `serde_json` to `hyperdb-api` (both already ubiquitous in the
workspace). `ToSqlParam` already has a `serde_json::Value` impl, confirming
`serde_json` is an acceptable dependency here.

### Testing (M1)

Integration + unit tests in `hyperdb-api/tests/`, using `HyperProcess::new()` to
start a real `hyperd` (per repo rules: no fabricated flags; capture and report
real output; a silent hang is a failure, not a pass). Coverage:

- PK-enforcement probe (documents actual engine behavior).
- Upsert round-trip: set → get, set again (overwrite) → get.
- Null value handling.
- `get_as` / `set_as` round-trip for a struct; malformed-JSON → `Serialization`.
- `pop` ordering (alphabetical) + atomicity + `None` on empty.
- Multi-store isolation: same key in two stores stays distinct.
- Cross-store self-join with `store_name` filters (documents the M2 pattern;
  verifies no row multiplication when filters present).
- Charset/empty/length validation rejects.
- `delete` / `exists` / `size` / `keys` / `clear` / `kv_list_stores`.
- **Both** sync and async twins.

`cargo clippy` + `cargo fmt` before every commit. No narrowing `as` casts (repo
rule #7) — use `TryFrom` where any width conversion arises.

### CHANGELOG

Add an `### Added` bullet under `## [Unreleased]` in `hyperdb-api/CHANGELOG.md`
(public API surface change).

## Milestone 2 — MCP (`hyperdb-mcp`)

**Designed here for coherence; planned & implemented separately. Minor change —
PR title uses a `fix:` prefix.**

Mirrors the existing `SavedQueryStore` pattern (`hyperdb-mcp/src/saved_queries.rs`):
a store abstraction with a `SessionStore` (in-memory, for `--ephemeral-only`) and
a `WorkspaceStore` (backs onto the `persistent` attached DB) split.

### Tools

`kv_get`, `kv_set`, `kv_delete`, `kv_list` (keys in a store), `kv_list_stores`,
`kv_size`, `kv_pop`, `kv_clear`. Each follows the repo tool template: a
`#[derive(Deserialize, JsonSchema)]` param struct with doc-commented fields, a
`#[tool(description = "...")]` handler with signature
`fn(&self, Parameters(p): Parameters<P>) -> Result<CallToolResult, rmcp::ErrorData>`,
`self.check_writable(...)` guard on mutators, a `self.with_engine(|engine| {...})`
body routed into the **`persistent`** DB by default (survives reconnects),
returning via `ok_content` / `err_content` with structured `McpError`.

Every new tool name must be added to the hardcoded array in
`hyperdb-mcp/tests/readme_tests.rs` **and** documented in
`hyperdb-mcp/src/readme.rs`, or the structural coverage test fails.

Tool descriptions frame the store as a persistent scratchpad, e.g. `kv_set`:
"Persistent scratchpad. Save variables, state, summaries, or JSON configs to
remember later without creating a database table."

### MCP Resource

Register `hyper://schema/kv` (text/plain) describing the `_hyperdb_kv_store`
shape (columns, composite PK, intent) so hosts can inject it as ambient schema
context.

### LEFT JOIN enrichment pattern

Document — in `readme.rs` and the `execute`/`query` tool descriptions — that any
analytical table can be enriched with KV metadata without `ALTER TABLE`:

```sql
SELECT t.*, kv.value AS metadata
FROM my_custom_table t
LEFT JOIN _hyperdb_kv_store kv
       ON t.id = kv.key AND kv.store_name = 'your_namespace'
WHERE t.status = 'active';
```

**Documentation must always include the `kv.store_name = '…'` filter.** Omitting
it fans out any key that exists in multiple stores (row multiplication) — a
query-authoring footgun independent of the single-table design. No new API is
needed for joins.

### Optional (stretch)

An `enrich-analytics` MCP Prompt that pre-bakes the join template. Marked a
stretch goal for M2, not required.

### CHANGELOG

Add a bullet under `## [Unreleased]` in `hyperdb-mcp/CHANGELOG.md`.

## Milestones, branches, PR titles

| Milestone | Crate | Branch | PR title prefix |
|---|---|---|---|
| M1 — API | `hyperdb-api` | current branch family | **`feat:`** (the real feature) |
| M2 — MCP | `hyperdb-mcp` | separate branch | **`fix:`** (minor surfacing) |

One design doc (this) covers both. The implementation plan written next covers
**M1 only**; M2 gets its own plan later. M1 must land and publish before M2 can
consume the new API.

## Risks

- **PK enforcement unknown until probed.** Mitigated: first implementation step
  verifies it; the upsert emulation guarantees correctness regardless. Public
  API is unaffected by the outcome.
- **`DELETE`-based `clear` leaves MVCC tombstones** until compaction. Negligible
  at KV scale; acceptable given the single-table simplicity win. Documented.
- **`serde_json` dependency added to `hyperdb-api`.** Low risk — already used
  transitively via `ToSqlParam`'s `serde_json::Value` impl.
- **Join footgun (missing `store_name` filter).** Mitigated by always including
  the filter in documented examples.

## Follow-ups (post-merge)

- Write a feature memory doc in the `~/dev/ssteiner-ai` repo once M1/M2 land, as
  done for prior features.
