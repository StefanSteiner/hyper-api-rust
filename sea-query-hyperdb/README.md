# sea-query-hyper

HyperDB SQL dialect backend for [sea-query](https://crates.io/crates/sea-query).

This crate provides `HyperQueryBuilder`, a sea-query backend that generates SQL
compatible with HyperDB's SQL dialect. HyperDB is largely PostgreSQL-compatible,
so this builder currently delegates to `PostgresQueryBuilder` for all operations.
As Hyper's SQL dialect evolves, this builder will be updated to emit Hyper-specific
syntax where needed.

---

## Quick Start

```toml
[dependencies]
sea-query-hyperdb = "0.1"
sea-query = "0.30"
```

```rust
use sea_query::{Query, Expr, Iden};
use sea_query_hyper::HyperQueryBuilder;

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
use sea_query::{Query, Expr, Iden, Order};
use sea_query_hyper::HyperQueryBuilder;

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

Use `HyperQueryBuilder` instead of `PostgresQueryBuilder` so your code
automatically picks up Hyper-specific syntax as the dialect evolves. Both produce
identical SQL today.

`HyperQueryBuilder` implements all sea-query backend traits:

- **`QueryBuilder`** -- SELECT, INSERT, UPDATE, DELETE
- **`SchemaBuilder`** -- CREATE TABLE, ALTER TABLE, DROP TABLE
- **`GenericBuilder`** -- full builder interface combining query and schema

---

## Advanced Features

Sea-query supports advanced SQL features that work with `HyperQueryBuilder`.

**Window functions:**

```rust,ignore
use sea_query::{Query, Expr, Alias, WindowDef};

let sql = Query::select()
    .columns([Users::Name, Users::Salary])
    .expr_as(
        Expr::col(Users::Salary).sum().over(
            WindowDef::new().partition_by(Users::Department)
        ),
        Alias::new("dept_total"),
    )
    .from(Users::Table)
    .to_string(HyperQueryBuilder);
```

**Complex JOINs:**

```rust,ignore
use sea_query::{Query, Expr};

let sql = Query::select()
    .columns([(Users::Table, Users::Name), (Departments::Table, Departments::Name)])
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

- **No indexes** -- `CREATE INDEX` is accepted but has no performance effect
- **No primary/foreign/unique keys** -- constraint definitions are accepted but not enforced
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
