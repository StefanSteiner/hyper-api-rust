# sea-query-hyperdb

HyperDB SQL dialect backend for [sea-query](https://crates.io/crates/sea-query).

This crate provides `HyperQueryBuilder`, a sea-query backend that generates SQL
compatible with HyperDB's SQL dialect. HyperDB speaks a large subset of
PostgreSQL, so the builder reuses `PostgresQueryBuilder`'s rendering for most
constructs (quoting, `$n` placeholders, `E'...'` string escaping, `ILIKE`,
`DISTINCT ON`, `NULLS FIRST/LAST`, `TABLESAMPLE`, ...). It is not a pass-through:
statement dispatch, value rendering and some expression forms are Hyper's own,
and the output is tested against a real `hyperd`.

---

## Quick Start

```toml
[dependencies]
sea-query-hyperdb = "1.0"
sea-query = "1.0"
```

```rust
use sea_query::{Expr, ExprTrait, Iden, Query};
use sea_query_hyperdb::HyperQueryBuilder;

#[derive(Iden)]
enum Users {
    Table,
    Id,
    Name,
}

let query = Query::select()
    .column(Users::Id)
    .column(Users::Name)
    .from(Users::Table)
    .and_where(Expr::col(Users::Id).gt(10))
    .to_owned();

// Build with parameter placeholders ($1, $2, ...)
let (sql, values) = query.build(HyperQueryBuilder);

// Or get the SQL string with values inlined
let sql_string = query.to_string(HyperQueryBuilder);
```

---

## Usage with hyperdb-api

Build queries with sea-query, convert to SQL strings, and pass them to
`hyperdb-api` connection methods:

```rust,ignore
use hyperdb_api::Connection;
use sea_query::{Expr, ExprTrait, Iden, Order, Query};
use sea_query_hyperdb::HyperQueryBuilder;

#[derive(Iden)]
enum Products {
    Table,
    Name,
    Price,
    Category,
}

let query = Query::select()
    .columns([Products::Name, Products::Price])
    .from(Products::Table)
    .and_where(Expr::col(Products::Price).gt(100))
    .and_where(Expr::col(Products::Category).eq("electronics"))
    .order_by(Products::Price, Order::Desc)
    .limit(10)
    .to_owned();

let sql = query.to_string(HyperQueryBuilder);
let result = conn.execute_query(&sql)?;
```

---

## HyperDB-Specific Dialect

Use `HyperQueryBuilder` instead of `PostgresQueryBuilder`. For ordinary
queries the SQL is the same, but the two differ where Hyper does:

- `Value::Enum` is a bare string (Hyper has no enum types, so no `::"type"` hint)
- `Value::Char` supports any Unicode character
- SQLite-only operators and the MySQL-only `PARTITION BY KEY` are written out
  rather than panicking, and Hyper then rejects them
- Syntax Hyper lacks (`@>`, `<@`, `&&`, `@@`, `%`, `<->`, partitioning) is
  rendered as written and fails server-side with an error

Known limitations (sea-query keeps these structs' fields crate-private, so they
are rendered by `PostgresQueryBuilder` and Hyper's overrides for non-ASCII
`Value::Char`, `Value::Enum` and SQLite operators do not apply inside them):
`ORDER BY` expressions (including `Order::Field`), `EXPLAIN`, `CREATE INDEX`
(partial-index `WHERE`) and `ALTER TABLE`. sea-query itself still panics on
`ColumnType::Year`, `ALTER TABLE` with no options, `JoinOn::Columns`, a
`WITH` query without a statement and some table-reference forms; those
panics are upstream and not masked here.

`!~`, `!~*` and `SIMILAR TO` have no typed sea-query operator; use
`BinOper::Custom("!~")` or `Expr::cust(..)`. `JoinOn::Columns` is not implemented
by sea-query for any backend.

Always build values through `Expr::val(..)` / `.eq(..)` where you can. Inlined
literals (`Expr::Constant`, `to_string`, `inject_parameters`) are escaped with
PostgreSQL rules (`E'...'` when a backslash or control character is present),
which is what Hyper expects; earlier releases used MySQL rules here (see the
changelog).

`HyperQueryBuilder` implements all sea-query backend traits:

- **`QueryBuilder`** -- SELECT, INSERT, UPDATE, DELETE
- **`SchemaBuilder`** -- CREATE TABLE, ALTER TABLE, DROP TABLE
- **`GenericBuilder`** -- full builder interface combining query and schema

---

## Advanced Features

Sea-query supports advanced SQL features that work with `HyperQueryBuilder`.

**Window functions:**

```rust,ignore
use sea_query::{Alias, Expr, ExprTrait, Query, WindowStatement};

let sql = Query::select()
    .columns([Users::Name, Users::Salary])
    .expr_window_as(
        Expr::col(Users::Salary).sum(),
        WindowStatement::partition_by(Users::Department),
        Alias::new("dept_total"),
    )
    .from(Users::Table)
    .to_string(HyperQueryBuilder);
```

**Complex JOINs:**

```rust,ignore
use sea_query::{Expr, ExprTrait, Query};

let sql = Query::select()
    .column((Users::Table, Users::Name))
    .column((Departments::Table, Departments::Name))
    .from(Users::Table)
    .inner_join(
        Departments::Table,
        Expr::col((Users::Table, Users::Department))
            .equals((Departments::Table, Departments::Id)),
    )
    .to_string(HyperQueryBuilder);
```

**Aggregates with GROUP BY / HAVING:**

```rust,ignore
use sea_query::{Alias, Expr, ExprTrait, Query};

let sql = Query::select()
    .column(Users::Department)
    .expr_as(Expr::cust("AVG(salary)"), Alias::new("avg_salary"))
    .from(Users::Table)
    .group_by_col(Users::Department)
    .and_having(Expr::cust("AVG(salary) > 60000"))
    .to_string(HyperQueryBuilder);
```

---

## HyperDB Limitations

When using sea-query with HyperDB, be aware of these architectural constraints:

- **No key or check constraints** -- Hyper rejects `PRIMARY KEY`, `UNIQUE`,
  `FOREIGN KEY`/`REFERENCES`, and `CHECK` in `CREATE TABLE`, so don't call
  `.primary_key()`, `.unique_key()`, or `.check()` on a `ColumnDef` or add
  foreign keys
- **No indexes** -- `CREATE INDEX` fails with `0A000: Index support is
  disabled`, so don't emit `Index::create()` statements
- **No auto-increment** -- `.auto_increment()` emits a
  `GENERATED BY DEFAULT AS IDENTITY` column, which Hyper does not accept
- **No `JSON`, `JSONB`, or `UUID` column types** -- use `TEXT`
- **Limited ALTER TABLE** -- some column modifications may not be supported

---

## When to Use sea-query

| Use Case | Recommended Approach |
|---|---|
| Simple CRUD operations | Raw SQL strings |
| Basic analytics (GROUP BY, aggregates) | Raw SQL strings |
| Complex reporting with window functions | `sea-query` + `sea-query-hyperdb` |
| CTEs or nested subqueries | `sea-query` + `sea-query-hyperdb` |
| Type-safe, composable query building | `sea-query` + `sea-query-hyperdb` |

---

## Examples

```bash
cargo run -p sea-query-hyperdb --example basic_usage
```

## Related Documentation

- [sea-query docs](https://docs.rs/sea-query) -- sea-query crate documentation
- [hyperdb-api](https://crates.io/crates/hyperdb-api) -- high-level Hyper database API

## License

Licensed under either of [Apache License, Version 2.0](http://www.apache.org/licenses/LICENSE-2.0)
or [MIT license](http://opensource.org/licenses/MIT), at your option.
