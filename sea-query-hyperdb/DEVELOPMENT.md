# sea-query-hyperdb Development

Contributor guide for the `sea-query-hyperdb` crate.

## Dialect Implementation

`HyperQueryBuilder` implements all sea-query backend traits (`QueryBuilder`,
`SchemaBuilder`, `GenericBuilder`, and their sub-traits). HyperDB's SQL dialect
is PostgreSQL-compatible for identifier quoting, placeholder syntax (`$1, $2,
...`), operator precedence, string escaping and most DDL/DML syntax, so most
hooks are explicit one-line delegations to `PostgresQueryBuilder`.

It is **not** a blanket pass-through, and that distinction matters. sea-query's
`QueryBuilder` calls back into `self` for every nested construct. If a hook
that `PostgresQueryBuilder` overrides is left at sea-query's generic default
here, the output changes silently: `EscapeBuilder` used MySQL backslash rules
(an injection hole against Hyper), `ILIKE` and `PgFunc` panicked,
`DISTINCT ON` and `NULLS FIRST/LAST` were dropped, `TABLESAMPLE` vanished and
`IFNULL` replaced `COALESCE`. When adding a hook, diff against the
Postgres builder's overrides first (`sea-query/src/backend/postgres/`).

Pieces that are Hyper's own (not delegated):

- `prepare_query_statement` dispatches on `SubQueryStatement` itself so nested
  statements are rendered with Hyper's overrides
- `prepare_value` (`push_param(.., self)`), `write_value` (`Char` as a full
  scalar, `Enum` without a type hint), `prepare_expr` (`AsEnum` as `CAST`)
- `prepare_bin_oper` (SQLite operators, never panics) and the partition hooks
  (no panic on `KEY` / `LESS THAN`)

- `write_escaped` / `write_string_quoted` (NUL as `\000`) and
  `prepare_column_def` (via `ColumnDef`'s public accessors, so `DEFAULT` /
  `CHECK` expressions use Hyper's `prepare_expr`)

`prepare_explain_statement`, `prepare_order_expr`, `prepare_index_create_statement`
and `prepare_table_alter_statement` stay delegated: `ExplainStatement`,
`OrderExpr`, `IndexCreateStatement` and `TableAlterStatement` have crate-private
fields and no accessors, so nothing nested inside them (including the `ORDER BY`
expression and `Order::Field`) can be re-rendered with Hyper's overrides. If
sea-query ever exposes them, replace the delegations with local copies.
Upstream `unimplemented!`/`panic!` sites remain for `ColumnType::Year`,
`ALTER TABLE` with no options, `JoinOn::Columns`, a `WITH` query without a
statement and `prepare_table_ref_table_stmt` forms; they are not masked.

## When Hyper Diverges from PostgreSQL

Override individual trait methods to emit Hyper-specific SQL while leaving the
rest delegated.

When adding an override:

1. Override the relevant method on the appropriate trait impl
2. Add a unit test asserting the exact SQL, and an integration test in
   `tests/hyper_integration.rs` if the claim is about what Hyper accepts
3. Document the divergence in the `HyperQueryBuilder` struct doc comment

## Testing

Unit tests live in the `#[cfg(test)] mod tests` block in `src/lib.rs` and assert
exact SQL: escaping and injection cases, every `PgBinOper`, `PgFunc` samples,
`DISTINCT ON`, `NULLS FIRST/LAST`, `TABLESAMPLE`, DDL parity with Postgres.

`tests/hyper_integration.rs` runs generated SQL on a real `hyperd` (round trips
of hostile strings through every inlining path, bytes, operators, `DISTINCT ON`,
and a `@>` query that must fail server-side, not panic). It needs the pinned
engine; set an absolute `HYPERD_PATH` (see the workspace `AGENTS.md`):

```bash
make download-hyperd
HYPERD_PATH="$PWD/.hyperd/current" cargo test -p sea-query-hyperdb
```

Run the example:

```bash
cargo run -p sea-query-hyperdb --example basic_usage
```
