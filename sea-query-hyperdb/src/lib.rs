// Copyright (c) 2026, Salesforce, Inc. All rights reserved.
// SPDX-License-Identifier: Apache-2.0 OR MIT

//! `HyperDB` SQL dialect backend for [sea-query](https://crates.io/crates/sea-query).
//!
//! This crate provides [`HyperQueryBuilder`], a sea-query backend that generates SQL
//! compatible with `HyperDB`'s SQL dialect. `HyperDB` speaks a large subset of the
//! `PostgreSQL` dialect, so this builder reuses [`PostgresQueryBuilder`]'s rendering for
//! most constructs, but it is **not** a pass-through: the builder owns statement
//! dispatch, value rendering and a few expression forms (see "Where Hyper differs" on
//! [`HyperQueryBuilder`]), and its output is checked against a real `hyperd` in the
//! integration tests. Hyper rejects some syntax Postgres accepts; those constructs are
//! rendered as written and the server reports the error.
//!
//! # Usage
//!
//! `HyperQueryBuilder` implements all sea-query backend traits ([`QueryBuilder`],
//! [`SchemaBuilder`], [`GenericBuilder`]), so it can be used anywhere a sea-query
//! backend is accepted:
//!
//! ```rust
//! use sea_query::{Query, Expr, ExprTrait, Iden, PostgresQueryBuilder};
//! use sea_query_hyperdb::HyperQueryBuilder;
//!
//! #[derive(Iden)]
//! enum Users {
//!     Table,
//!     Id,
//!     Name,
//! }
//!
//! let query = Query::select()
//!     .column(Users::Id)
//!     .column(Users::Name)
//!     .from(Users::Table)
//!     .and_where(Expr::col(Users::Id).gt(10))
//!     .to_owned();
//!
//! // Build with parameter placeholders ($1, $2, ...)
//! let (sql, values) = query.build(HyperQueryBuilder);
//! assert!(sql.contains("SELECT"));
//!
//! // Or get the SQL string with values inlined
//! let sql_string = query.to_string(HyperQueryBuilder);
//! assert!(sql_string.contains("WHERE"));
//! ```
//!
//! # With hyperdb-api
//!
//! ```rust,ignore
//! use hyperdb_api::Connection;
//! use sea_query::{Query, Expr, Iden};
//! use sea_query_hyperdb::HyperQueryBuilder;
//!
//! #[derive(Iden)]
//! enum Products {
//!     Table,
//!     Name,
//!     Price,
//! }
//!
//! let query = Query::select()
//!     .column(Products::Name)
//!     .column(Products::Price)
//!     .from(Products::Table)
//!     .and_where(Expr::col(Products::Price).gt(100))
//!     .to_owned();
//!
//! let sql = query.to_string(HyperQueryBuilder);
//! let result = conn.execute_query(&sql)?;
//! ```

use sea_query::backend::{
    EscapeBuilder, ForeignKeyBuilder, GenericBuilder, IndexBuilder, OperLeftAssocDecider,
    PrecedenceDecider, QueryBuilder, QuotedBuilder, SchemaBuilder, TableBuilder, TableRefBuilder,
};
use sea_query::extension::sqlite::SqliteBinOper;
use sea_query::{
    Alias, BinOper, ColumnDef, ColumnType, DynIden, ExplainStatement, Expr,
    ForeignKeyCreateStatement, ForeignKeyDropStatement, Func, IndexCreateStatement,
    IndexDropStatement, IntoIden, Oper, OptionEnum, OrderExpr, PartitionBy, PartitionValues,
    PostgresQueryBuilder, Quote, SelectDistinct, SelectInto, SelectStatement, SqlWriter,
    SubQueryStatement, TableAlterStatement, TableRef, TableRenameStatement, Value,
};

/// HyperDB-specific SQL dialect backend for sea-query.
///
/// Generates SQL compatible with `HyperDB`'s dialect, as documented at
/// <https://developer.salesforce.com/docs/data/data-cloud-query-guide/references/dc-sql-reference/select.html>.
///
/// # Delegation Strategy
///
/// `HyperDB`'s SQL dialect is largely PostgreSQL-compatible — identifier quoting,
/// placeholder syntax (`$1, $2, ...`), operator precedence, string escaping
/// (`E'...'`), and most DDL/DML syntax match `PostgreSQL`. Rather than
/// duplicating that logic, this builder delegates the individual rendering hooks
/// (escaping, operators, functions, `DISTINCT ON`, `NULLS FIRST/LAST`,
/// `TABLESAMPLE`, index and DDL hooks) to [`PostgresQueryBuilder`].
///
/// It is deliberately **not** a blanket pass-through. sea-query's `QueryBuilder`
/// calls back into `self` for every nested construct, so a hook left at the
/// generic default silently changes what is emitted (dropping `DISTINCT ON`,
/// panicking on `ILIKE`, MySQL-style backslash escaping). Every hook Postgres
/// overrides is therefore either delegated or replaced here.
///
/// # Where Hyper differs from `PostgresQueryBuilder`
///
/// - Statement dispatch is Hyper's own, so nested statements use Hyper's rules.
/// - `Value::Enum` renders as a bare string (Hyper has no enum types, so no
///   `::"type"` hint); `Expr::AsEnum` keeps its `CAST(.. AS "type")`.
/// - `Value::Char` is rendered as a full Unicode scalar (no byte truncation).
/// - SQLite-only operators are written out instead of panicking; Hyper then
///   rejects them (except `->` / `->>`).
/// - `PARTITION BY KEY` / `VALUES LESS THAN` are written out instead of
///   panicking. Hyper has no declarative partitioning and rejects all of it.
/// - Operators Hyper lacks (`@>`, `<@`, `&&`, `@@`, `%`, `<->`, ...) are
///   rendered verbatim and fail server-side with a syntax error.
///
/// Known limitations: `!~`, `!~*` and `SIMILAR TO` have no typed sea-query
/// operator; use `BinOper::Custom("!~")` or `Expr::cust`. The statement nested
/// inside `EXPLAIN`, `ORDER BY` expressions, `CREATE INDEX` and `ALTER TABLE` is
/// rendered by [`PostgresQueryBuilder`] (their fields are crate-private in
/// sea-query), so Hyper's overrides do not apply there. `JoinOn::Columns`
/// is not implemented by sea-query itself and panics for every backend.
///
/// Notable `HyperDB` SQL features:
/// - Optional `FROM` clause for simple expressions
/// - `DISTINCT ON` extension
/// - Standard `UNION`, `INTERSECT`, `EXCEPT` operations
/// - Zero-column result tables
///
/// # Examples
///
/// ```rust
/// use sea_query::{Query, Expr, ExprTrait, Iden};
/// use sea_query_hyperdb::HyperQueryBuilder;
///
/// #[derive(Iden)]
/// enum Users {
///     Table,
///     Name,
///     Age,
/// }
///
/// let sql = Query::select()
///     .column(Users::Name)
///     .from(Users::Table)
///     .and_where(Expr::col(Users::Age).gt(18))
///     .to_string(HyperQueryBuilder);
///
/// assert!(sql.contains(r#""users""#));
/// ```
#[derive(Debug, Clone, Copy, Default)]
pub struct HyperQueryBuilder;

impl QuotedBuilder for HyperQueryBuilder {
    fn quote(&self) -> Quote {
        PostgresQueryBuilder.quote()
    }
}

/// String escaping follows `PostgreSQL` rules, which is what Hyper implements:
/// `standard_conforming_strings` is on, so a plain `'...'` literal treats the
/// backslash literally and only `E'...'` literals interpret backslash escapes.
/// sea-query's default (MySQL) rules would turn `\` and `'` into sequences that
/// a plain Hyper literal does not understand, which both corrupts data and
/// opens an injection hole.
impl EscapeBuilder for HyperQueryBuilder {
    fn needs_escape(&self, s: &str) -> bool {
        PostgresQueryBuilder.needs_escape(s)
    }

    fn write_escaped(&self, buffer: &mut impl std::fmt::Write, string: &str) {
        // Same as Postgres except NUL, which is written as a full three-digit
        // octal escape: a bare `\0` followed by literal digits would be read
        // back as a longer octal escape (`\0` + `47` -> `\047`, a quote).
        let mut rest = string;
        while let Some(pos) = rest.find('\0') {
            PostgresQueryBuilder.write_escaped(buffer, &rest[..pos]);
            let _ = buffer.write_str("\\000");
            rest = &rest[pos + 1..];
        }
        PostgresQueryBuilder.write_escaped(buffer, rest);
    }

    fn unescape_string(&self, string: &str) -> String {
        PostgresQueryBuilder.unescape_string(string)
    }
}

impl TableRefBuilder for HyperQueryBuilder {}

impl OperLeftAssocDecider for HyperQueryBuilder {
    fn well_known_left_associative(&self, op: &BinOper) -> bool {
        PostgresQueryBuilder.well_known_left_associative(op)
    }
}

impl PrecedenceDecider for HyperQueryBuilder {
    fn inner_expr_well_known_greater_precedence(&self, inner: &Expr, outer_oper: &Oper) -> bool {
        PostgresQueryBuilder.inner_expr_well_known_greater_precedence(inner, outer_oper)
    }
}

impl QueryBuilder for HyperQueryBuilder {
    // Statement dispatch is Hyper's own. Delegating this to
    // `PostgresQueryBuilder` would render the whole statement with *its*
    // `self`, so none of the overrides below would ever be consulted for
    // anything nested inside a statement.
    fn prepare_query_statement(&self, query: &SubQueryStatement, sql: &mut impl SqlWriter) {
        match query {
            SubQueryStatement::SelectStatement(stmt) => self.prepare_select_statement(stmt, sql),
            SubQueryStatement::InsertStatement(stmt) => self.prepare_insert_statement(stmt, sql),
            SubQueryStatement::UpdateStatement(stmt) => self.prepare_update_statement(stmt, sql),
            SubQueryStatement::DeleteStatement(stmt) => self.prepare_delete_statement(stmt, sql),
            SubQueryStatement::WithStatement(stmt) => self.prepare_with_query(stmt, sql),
        }
    }

    fn prepare_select_into(&self, into_table: &SelectInto, sql: &mut impl SqlWriter) {
        PostgresQueryBuilder.prepare_select_into(into_table, sql);
    }

    // KNOWN LIMITATION: the statement nested in an `EXPLAIN`, and the options,
    // are rendered by `PostgresQueryBuilder` (`ExplainStatement`'s fields are
    // crate-private in sea-query, so they cannot be re-rendered here). Every
    // Hyper override is bypassed inside it: `Value::Char` outside ASCII panics,
    // `Value::Enum` keeps its `::"type"` hint, and SQLite operators panic.
    fn prepare_explain_statement(&self, explain: &ExplainStatement, sql: &mut impl SqlWriter) {
        PostgresQueryBuilder.prepare_explain_statement(explain, sql);
    }

    fn prepare_value(&self, value: Value, sql: &mut impl SqlWriter) {
        // Hyper has no enum types: bind the variant as a plain string, with
        // no `::"type"` hint (matches `write_value`'s inlined form).
        let value = match value {
            Value::Enum(OptionEnum::Some(variant)) => {
                Value::String(Some(variant.value.into_owned()))
            }
            Value::Enum(OptionEnum::None(_)) => Value::String(None),
            other => other,
        };
        sql.push_param(value, self);
    }

    fn placeholder(&self) -> (&'static str, bool) {
        PostgresQueryBuilder.placeholder()
    }

    fn prepare_expr(&self, simple_expr: &Expr, sql: &mut impl SqlWriter) {
        match simple_expr {
            Expr::AsEnum(type_name, inner) => {
                // `CAST(<expr> AS "<type>")`, with embedded quotes doubled.
                // A trailing `[]` stays outside the quotes (array type).
                let name = type_name.inner();
                let mut base: &str = name.as_ref();
                let mut dims = 0;
                while let Some(stripped) = base.strip_suffix("[]") {
                    base = stripped;
                    dims += 1;
                }
                let array_suffix = "[]".repeat(dims);
                write_or_ignore(sql, "CAST(");
                self.prepare_expr(inner, sql);
                write_or_ignore(sql, " AS ");
                write_or_ignore(
                    sql,
                    &format!("\"{}\"{array_suffix}", base.replace('"', "\"\"")),
                );
                write_or_ignore(sql, ")");
            }
            _ => self.prepare_expr_common(simple_expr, sql),
        }
    }

    fn write_value(&self, buf: &mut impl std::fmt::Write, value: &Value) -> std::fmt::Result {
        match value {
            // `Value::Char` holds a full Unicode scalar; sea-query's default
            // truncates it to one byte and panics on non-ASCII input.
            Value::Char(Some(c)) => {
                self.write_string_quoted(c.encode_utf8(&mut [0u8; 4]), buf);
                Ok(())
            }
            // Hyper has no enum types: emit the bare string, no `::"type"` hint.
            Value::Enum(OptionEnum::Some(variant)) => {
                self.write_string_quoted(variant.value.as_ref(), buf);
                Ok(())
            }
            _ => self.write_value_common(buf, value),
        }
    }

    fn write_string_quoted(&self, string: &str, buffer: &mut impl std::fmt::Write) {
        // Postgres' version escapes through *its* `write_escaped`, which would
        // bypass the NUL fix above, so spell it out against `self`.
        let prefix = if self.needs_escape(string) { "E'" } else { "'" };
        let _ = buffer.write_str(prefix);
        self.write_escaped(buffer, string);
        let _ = buffer.write_str("'");
    }

    fn write_bytes(&self, bytes: &[u8], buffer: &mut impl std::fmt::Write) {
        PostgresQueryBuilder.write_bytes(bytes, buffer);
    }

    fn if_null_function(&self) -> &str {
        PostgresQueryBuilder.if_null_function()
    }

    fn prepare_bin_oper(&self, bin_oper: &BinOper, sql: &mut impl SqlWriter) {
        match bin_oper {
            // Hyper rejects the operators it lacks (`@>`, `<@`, `&&`, `@@`,
            // `%`, `<->`, ...) with a clean `42601`, so emit them verbatim.
            BinOper::PgOperator(_) => PostgresQueryBuilder.prepare_bin_oper(bin_oper, sql),
            // Hyper has no SQLite operators; write them out so the server
            // answers with a syntax error instead of the builder panicking.
            BinOper::SqliteOperator(oper) => match oper {
                SqliteBinOper::GetJsonField => write_or_ignore(sql, "->"),
                SqliteBinOper::CastJsonField => write_or_ignore(sql, "->>"),
                SqliteBinOper::Glob => write_or_ignore(sql, "GLOB"),
                SqliteBinOper::Match => write_or_ignore(sql, "MATCH"),
                // `SqliteBinOper` is `#[non_exhaustive]`: a future variant gets
                // a placeholder the server rejects, never a panic and never
                // Rust `Debug` text in the SQL.
                _ => write_or_ignore(sql, "/* unsupported SQLite operator */"),
            },
            _ => self.prepare_bin_oper_common(bin_oper, sql),
        }
    }

    fn prepare_function_name(&self, function: &Func, sql: &mut impl SqlWriter) {
        PostgresQueryBuilder.prepare_function_name(function, sql);
    }

    fn prepare_table_sample(&self, select: &SelectStatement, sql: &mut impl SqlWriter) {
        PostgresQueryBuilder.prepare_table_sample(select, sql);
    }

    fn prepare_select_distinct(&self, select_distinct: &SelectDistinct, sql: &mut impl SqlWriter) {
        PostgresQueryBuilder.prepare_select_distinct(select_distinct, sql);
    }

    // KNOWN LIMITATION: `OrderExpr` has no public accessors, so the ORDER BY
    // expression cannot be re-rendered here; it (and `Order::Field`) is
    // rendered by `PostgresQueryBuilder`, bypassing Hyper overrides for
    // `Value::Char` outside ASCII, `Value::Enum` and SQLite operators. Plain
    // columns, strings (properly escaped), numbers and functions are fine.
    fn prepare_order_expr(&self, order_expr: &OrderExpr, sql: &mut impl SqlWriter) {
        PostgresQueryBuilder.prepare_order_expr(order_expr, sql);
    }
}

/// Write a fragment to a [`SqlWriter`]. Writing into sea-query's in-memory
/// `String` buffers cannot fail, so the `fmt::Result` is intentionally dropped
/// rather than turned into a panic.
fn write_or_ignore(sql: &mut impl SqlWriter, fragment: &str) {
    let _ = sql.write_str(fragment);
}

impl ForeignKeyBuilder for HyperQueryBuilder {
    fn prepare_table_ref_fk_stmt(&self, table_ref: &TableRef, sql: &mut impl SqlWriter) {
        PostgresQueryBuilder.prepare_table_ref_fk_stmt(table_ref, sql);
    }

    fn prepare_foreign_key_create_statement_internal(
        &self,
        create: &ForeignKeyCreateStatement,
        sql: &mut impl SqlWriter,
        mode: sea_query::backend::Mode,
    ) {
        PostgresQueryBuilder.prepare_foreign_key_create_statement_internal(create, sql, mode);
    }

    fn prepare_foreign_key_drop_statement_internal(
        &self,
        drop: &ForeignKeyDropStatement,
        sql: &mut impl SqlWriter,
        mode: sea_query::backend::Mode,
    ) {
        PostgresQueryBuilder.prepare_foreign_key_drop_statement_internal(drop, sql, mode);
    }
}

impl IndexBuilder for HyperQueryBuilder {
    // KNOWN LIMITATION: `IndexCreateStatement` (and `TableAlterStatement`
    // below) expose no public accessors, so they are rendered whole by
    // `PostgresQueryBuilder`; a partial-index `WHERE` / `ALTER ... DEFAULT`
    // expression bypasses Hyper overrides (non-ASCII `Value::Char`, `Value::Enum`,
    // SQLite operators).
    fn prepare_index_create_statement(
        &self,
        create: &IndexCreateStatement,
        sql: &mut impl SqlWriter,
    ) {
        PostgresQueryBuilder.prepare_index_create_statement(create, sql);
    }

    fn prepare_table_ref_index_stmt(&self, table_ref: &TableRef, sql: &mut impl SqlWriter) {
        PostgresQueryBuilder.prepare_table_ref_index_stmt(table_ref, sql);
    }

    fn prepare_index_drop_statement(&self, drop: &IndexDropStatement, sql: &mut impl SqlWriter) {
        PostgresQueryBuilder.prepare_index_drop_statement(drop, sql);
    }

    fn prepare_index_prefix(&self, create: &IndexCreateStatement, sql: &mut impl SqlWriter) {
        PostgresQueryBuilder.prepare_index_prefix(create, sql);
    }

    // Consulted by the default `CREATE TABLE ... INDEX / PRIMARY KEY`
    // constraint path; Postgres adds `CONSTRAINT`, `NULLS NOT DISTINCT` and
    // `INCLUDE`. Delegated whole, so the pieces it calls (index type, columns,
    // filter) are Postgres' own and need no separate override.
    fn prepare_table_index_expression(
        &self,
        create: &IndexCreateStatement,
        sql: &mut impl SqlWriter,
    ) {
        PostgresQueryBuilder.prepare_table_index_expression(create, sql);
    }
}

impl TableBuilder for HyperQueryBuilder {
    // Hyper's own copy of Postgres' body (via the public accessors) so the
    // `DEFAULT` / `CHECK` expressions are rendered by `self`.
    fn prepare_column_def(&self, column_def: &ColumnDef, sql: &mut impl SqlWriter) {
        let name = Alias::new(column_def.get_column_name()).into_iden();
        self.prepare_iden(&name, sql);
        let spec = column_def.get_column_spec();
        if let Some(column_type) = column_def.get_column_type() {
            write_or_ignore(sql, " ");
            if spec.auto_increment {
                match column_type {
                    ColumnType::SmallInteger => write_or_ignore(sql, "smallint"),
                    ColumnType::Integer => write_or_ignore(sql, "integer"),
                    ColumnType::BigInteger => write_or_ignore(sql, "bigint"),
                    // Postgres panics here; keep the type and let Hyper reject it.
                    other => self.prepare_column_type(other, sql),
                }
                write_or_ignore(sql, " GENERATED BY DEFAULT AS IDENTITY");
            } else {
                self.prepare_column_type(column_type, sql);
            }
        }
        self.prepare_column_spec(spec, sql);
    }

    fn prepare_column_type(&self, column_type: &ColumnType, sql: &mut impl SqlWriter) {
        PostgresQueryBuilder.prepare_column_type(column_type, sql);
    }

    fn column_spec_auto_increment_keyword(&self) -> &str {
        PostgresQueryBuilder.column_spec_auto_increment_keyword()
    }

    fn prepare_table_alter_statement(&self, alter: &TableAlterStatement, sql: &mut impl SqlWriter) {
        PostgresQueryBuilder.prepare_table_alter_statement(alter, sql);
    }

    fn prepare_table_rename_statement(
        &self,
        rename: &TableRenameStatement,
        sql: &mut impl SqlWriter,
    ) {
        PostgresQueryBuilder.prepare_table_rename_statement(rename, sql);
    }

    // Hyper has no declarative partitioning. sea-query's default hooks would
    // silently drop `PARTITION BY` / `FOR VALUES`, leaving an unpartitioned
    // table the caller thinks is partitioned; write the Postgres syntax so
    // the server rejects it. Postgres' own hooks panic on the MySQL-only
    // `KEY` / `LESS THAN` forms, so those are spelled out here instead.
    fn prepare_partition_by(&self, partition_by: &PartitionBy, sql: &mut impl SqlWriter) {
        let (keyword, cols) = match partition_by {
            PartitionBy::Range(cols) => ("RANGE", cols),
            PartitionBy::List(cols) => ("LIST", cols),
            PartitionBy::Hash(cols) => ("HASH", cols),
            PartitionBy::Key(cols) => ("KEY", cols),
        };
        write_or_ignore(sql, keyword);
        write_or_ignore(sql, " (");
        write_partition_cols(*self, cols, sql);
        write_or_ignore(sql, ")");
    }

    fn prepare_partition_values(
        &self,
        partition_values: &PartitionValues,
        sql: &mut impl SqlWriter,
    ) {
        write_or_ignore(sql, "FOR VALUES ");
        match partition_values {
            PartitionValues::In(values) => {
                write_or_ignore(sql, "IN (");
                write_partition_exprs(*self, values, sql);
                write_or_ignore(sql, ")");
            }
            PartitionValues::FromTo(from, to) => {
                write_or_ignore(sql, "FROM (");
                write_partition_exprs(*self, from, sql);
                write_or_ignore(sql, ") TO (");
                write_partition_exprs(*self, to, sql);
                write_or_ignore(sql, ")");
            }
            PartitionValues::LessThan(values) => {
                write_or_ignore(sql, "LESS THAN (");
                write_partition_exprs(*self, values, sql);
                write_or_ignore(sql, ")");
            }
            PartitionValues::With(modulus, remainder) => {
                write_or_ignore(
                    sql,
                    &format!("WITH (MODULUS {modulus}, REMAINDER {remainder})"),
                );
            }
        }
    }
}

fn write_partition_cols(builder: HyperQueryBuilder, cols: &[DynIden], sql: &mut impl SqlWriter) {
    for (index, col) in cols.iter().enumerate() {
        if index > 0 {
            write_or_ignore(sql, ", ");
        }
        builder.prepare_iden(col, sql);
    }
}

fn write_partition_exprs(builder: HyperQueryBuilder, exprs: &[Expr], sql: &mut impl SqlWriter) {
    for (index, expr) in exprs.iter().enumerate() {
        if index > 0 {
            write_or_ignore(sql, ", ");
        }
        builder.prepare_expr(expr, sql);
    }
}

impl SchemaBuilder for HyperQueryBuilder {}

impl GenericBuilder for HyperQueryBuilder {}

#[cfg(test)]
mod tests {
    use super::*;
    use sea_query::{ExprTrait, Iden, Query, Table};

    #[derive(Iden)]
    enum Users {
        Table,
        Id,
        Name,
        Age,
    }

    #[test]
    fn select_generates_valid_sql() {
        let query = Query::select()
            .column(Users::Name)
            .from(Users::Table)
            .and_where(Expr::col(Users::Age).gt(18))
            .to_owned();

        let sql = query.to_string(HyperQueryBuilder);
        assert_eq!(
            sql,
            query.to_string(PostgresQueryBuilder),
            "for plain SELECTs HyperQueryBuilder must match PostgresQueryBuilder's wording"
        );
    }

    #[test]
    fn select_build_produces_placeholders() {
        let query = Query::select()
            .column(Users::Name)
            .from(Users::Table)
            .and_where(Expr::col(Users::Age).gt(18))
            .to_owned();

        let (sql, values) = query.build(HyperQueryBuilder);
        let (pg_sql, pg_values) = query.build(PostgresQueryBuilder);
        assert_eq!(sql, pg_sql);
        assert_eq!(format!("{values:?}"), format!("{pg_values:?}"));
    }

    #[test]
    fn insert_generates_valid_sql() {
        let query = Query::insert()
            .into_table(Users::Table)
            .columns([Users::Name, Users::Age])
            .values_panic(["Alice".into(), 30i32.into()])
            .to_owned();

        let sql = query.to_string(HyperQueryBuilder);
        assert_eq!(sql, query.to_string(PostgresQueryBuilder));
    }

    #[test]
    fn update_generates_valid_sql() {
        let query = Query::update()
            .table(Users::Table)
            .value(Users::Name, "Bob")
            .and_where(Expr::col(Users::Id).eq(1))
            .to_owned();

        let sql = query.to_string(HyperQueryBuilder);
        assert_eq!(sql, query.to_string(PostgresQueryBuilder));
    }

    #[test]
    fn delete_generates_valid_sql() {
        let query = Query::delete()
            .from_table(Users::Table)
            .and_where(Expr::col(Users::Id).eq(1))
            .to_owned();

        let sql = query.to_string(HyperQueryBuilder);
        assert_eq!(sql, query.to_string(PostgresQueryBuilder));
    }

    #[test]
    fn create_table_generates_valid_sql() {
        use sea_query::{ColumnDef, Table};

        let stmt = Table::create()
            .table(Users::Table)
            .if_not_exists()
            .col(ColumnDef::new(Users::Id).integer().not_null().primary_key())
            .col(ColumnDef::new(Users::Name).string().not_null())
            .col(ColumnDef::new(Users::Age).integer())
            .to_owned();

        let sql = stmt.to_string(HyperQueryBuilder);
        assert_eq!(sql, stmt.to_string(PostgresQueryBuilder));
    }

    // ---- Q1: string / bytes / char escaping (security) ----------------------

    fn constant_eq(text: &str) -> String {
        Query::select()
            .column(Users::Id)
            .from(Users::Table)
            .and_where(
                Expr::col(Users::Name).eq(Expr::Constant(Value::String(Some(text.to_owned())))),
            )
            .to_string(HyperQueryBuilder)
    }

    #[test]
    fn constant_string_injection_is_escaped() {
        assert_eq!(
            constant_eq("x\\' OR 1=1 --"),
            r#"SELECT "id" FROM "users" WHERE "name" = E'x\\\' OR 1=1 --'"#
        );
    }

    #[test]
    fn constant_plain_string_is_plain_literal() {
        assert_eq!(
            constant_eq("abc"),
            r#"SELECT "id" FROM "users" WHERE "name" = 'abc'"#
        );
    }

    #[test]
    fn constant_string_with_nul_never_emits_raw_nul() {
        let sql = constant_eq("a\0b");
        assert!(!sql.contains('\0'), "raw NUL in SQL text: {sql:?}");
        assert!(sql.ends_with(r"= E'a\000b'"), "{sql}");
    }

    #[test]
    fn nul_followed_by_digits_cannot_extend_the_octal_escape() {
        // `\0` + `47` would read back as `\047`, a quote, and break out of the literal.
        let sql = constant_eq("\u{0}47' OR 1=1 --");
        assert!(sql.contains(r"E'\00047\' OR 1=1 --'"), "{sql}");
    }

    #[test]
    fn column_default_char_non_ascii_is_rendered_by_hyper() {
        let sql = Table::create()
            .table(Users::Table)
            .col(ColumnDef::new(Users::Name).char().default('\u{e9}'))
            .to_string(HyperQueryBuilder);
        assert!(sql.contains("DEFAULT '\u{e9}'"), "{sql}");
    }

    #[test]
    fn column_default_enum_is_a_bare_string() {
        let v = Value::Enum(OptionEnum::Some(Box::new(sea_query::Enum {
            type_name: "mood".into(),
            value: "happy".into(),
        })));
        let sql = Table::create()
            .table(Users::Table)
            .col(ColumnDef::new(Users::Name).string().default(Expr::val(v)))
            .to_string(HyperQueryBuilder);
        assert!(sql.contains("DEFAULT 'happy'"), "{sql}");
        assert!(!sql.contains("mood"), "{sql}");
    }

    #[test]
    fn column_auto_increment_matches_postgres_identity() {
        let build = |b: &dyn Fn(&mut ColumnDef) -> &mut ColumnDef| {
            let mut c = ColumnDef::new(Users::Id);
            b(&mut c);
            Table::create().table(Users::Table).col(c).to_owned()
        };
        let t = build(&|c| c.integer().auto_increment().primary_key());
        assert_eq!(
            t.to_string(HyperQueryBuilder),
            t.to_string(PostgresQueryBuilder)
        );
    }

    #[test]
    fn column_auto_increment_on_non_integer_does_not_panic() {
        let t = Table::create()
            .table(Users::Table)
            .col(ColumnDef::new(Users::Name).string().auto_increment())
            .to_owned();
        let sql = t.to_string(HyperQueryBuilder);
        assert!(sql.contains("varchar"), "{sql}");
    }

    #[test]
    fn build_turns_enum_values_into_plain_strings() {
        let v = Value::Enum(OptionEnum::Some(Box::new(sea_query::Enum {
            type_name: "mood".into(),
            value: "happy".into(),
        })));
        let (sql, values) = Query::select()
            .column(Users::Id)
            .from(Users::Table)
            .and_where(Expr::col(Users::Name).eq(Expr::val(v)))
            .build(HyperQueryBuilder);
        assert!(!sql.contains("::"), "{sql}");
        assert_eq!(values.0, vec![Value::String(Some("happy".into()))]);
    }

    #[test]
    fn as_enum_strips_every_array_suffix() {
        let sql = Query::select()
            .expr(Expr::col(Users::Name).as_enum("my_type[][]"))
            .from(Users::Table)
            .to_string(HyperQueryBuilder);
        assert_eq!(sql, r#"SELECT CAST("name" AS "my_type"[][]) FROM "users""#);
    }

    #[test]
    fn like_escape_character_uses_standard_escaping() {
        use sea_query::LikeExpr;
        let sql = Query::select()
            .column(Users::Id)
            .from(Users::Table)
            .and_where(Expr::col(Users::Name).like(LikeExpr::new(r"a\_%").escape('\\')))
            .to_string(HyperQueryBuilder);
        assert!(sql.ends_with(r"ESCAPE E'\\'"), "{sql}");
    }

    #[test]
    fn order_field_values_are_escaped() {
        use sea_query::{Order, Values};
        let sql = Query::select()
            .column(Users::Id)
            .from(Users::Table)
            .order_by_expr(
                Expr::col(Users::Name),
                Order::Field(Values(vec!["a'b".into(), r"c\d".into()])),
            )
            .to_string(HyperQueryBuilder);
        assert!(
            sql.ends_with(
                r#"ORDER BY CASE WHEN "name"=E'a\'b' THEN 0 WHEN "name"=E'c\\d' THEN 1 ELSE 2 END"#
            ),
            "{sql}"
        );
    }

    #[test]
    fn inject_parameters_matches_to_string() {
        let query = Query::select()
            .column(Users::Id)
            .from(Users::Table)
            .and_where(Expr::col(Users::Name).eq("x\\' OR 1=1 --"))
            .and_where(Expr::col(Users::Age).eq("tab\there"))
            .to_owned();
        let (sql, values) = query.build(HyperQueryBuilder);
        let injected = sea_query::prepare::inject_parameters(&sql, &values.0, &HyperQueryBuilder);
        assert_eq!(injected, query.to_string(HyperQueryBuilder));
        assert!(injected.contains(r"E'x\\\' OR 1=1 --'"), "{injected}");
    }

    #[test]
    fn value_to_string_uses_postgres_escaping() {
        let v = Value::String(Some(r"a\b'c".to_owned()));
        assert_eq!(HyperQueryBuilder.value_to_string(&v), r"E'a\\b\'c'");
    }

    #[test]
    fn escape_string_round_trips_with_postgres_rules() {
        let raw = "a\"b\\c'";
        let escaped = HyperQueryBuilder.escape_string(raw);
        assert_eq!(escaped, r#"a"b\\c\'"#);
        assert_eq!(HyperQueryBuilder.unescape_string(&escaped), raw);
    }

    #[test]
    fn non_ascii_char_does_not_panic() {
        let sql = Query::select()
            .column(Users::Id)
            .from(Users::Table)
            .and_where(Expr::col(Users::Name).eq(Expr::Constant(Value::Char(Some('é')))))
            .to_string(HyperQueryBuilder);
        assert!(sql.ends_with("= 'é'"), "{sql}");
        let emoji = HyperQueryBuilder.value_to_string(&Value::Char(Some('😀')));
        assert_eq!(emoji, "'😀'");
    }

    #[test]
    fn bytes_use_hex_escape_format() {
        let v = Value::Bytes(Some(vec![0x00, 0x01, 0xff]));
        let text = HyperQueryBuilder.value_to_string(&v);
        assert_eq!(text, PostgresQueryBuilder.value_to_string(&v));
        assert!(text.starts_with(r"'\x"), "{text}");
    }

    // ---- Q3: operators and functions must not panic ---------------------------

    fn pg_ops() -> Vec<(sea_query::extension::postgres::PgBinOper, &'static str)> {
        use sea_query::extension::postgres::PgBinOper as P;
        vec![
            (P::ILike, "ILIKE"),
            (P::NotILike, "NOT ILIKE"),
            (P::Matches, "@@"),
            (P::Contains, "@>"),
            (P::Contained, "<@"),
            (P::Concatenate, "||"),
            (P::Overlap, "&&"),
            (P::Similarity, "%"),
            (P::WordSimilarity, "<%"),
            (P::StrictWordSimilarity, "<<%"),
            (P::SimilarityDistance, "<->"),
            (P::WordSimilarityDistance, "<<->"),
            (P::StrictWordSimilarityDistance, "<<<->"),
            (P::GetJsonField, "->"),
            (P::CastJsonField, "->>"),
            (P::Regex, "~"),
            (P::RegexCaseInsensitive, "~*"),
        ]
    }

    #[test]
    fn every_pg_bin_oper_renders() {
        for (op, text) in pg_ops() {
            let query = Query::select()
                .column(Users::Id)
                .from(Users::Table)
                .and_where(Expr::col(Users::Name).binary(BinOper::PgOperator(op), Expr::val("x")))
                .to_owned();
            let expected = format!(r#"SELECT "id" FROM "users" WHERE "name" {text} 'x'"#);
            assert_eq!(query.to_string(HyperQueryBuilder), expected);
            assert_eq!(
                query.to_string(HyperQueryBuilder),
                query.to_string(PostgresQueryBuilder)
            );
        }
    }

    #[test]
    fn pg_expr_helpers_render() {
        use sea_query::extension::postgres::PgExpr;
        let sql = Query::select()
            .column(Users::Id)
            .from(Users::Table)
            .and_where(Expr::col(Users::Name).ilike("a%"))
            .and_where(Expr::col(Users::Name).not_ilike("b%"))
            .and_where(Expr::col(Users::Name).concat("z").eq("yz"))
            .to_string(HyperQueryBuilder);
        // `PgExpr` is in scope here and blankets `contains` onto anything
        // `Into<Expr>`, so name the `str` method explicitly.
        assert!(str::contains(&sql, r#""name" ILIKE 'a%'"#), "{sql}");
        assert!(str::contains(&sql, r#""name" NOT ILIKE 'b%'"#), "{sql}");
        assert!(str::contains(&sql, r#""name" || 'z'"#), "{sql}");
    }

    #[test]
    fn sqlite_operators_never_panic() {
        use sea_query::extension::sqlite::SqliteBinOper as S;
        for (op, text) in [
            (S::GetJsonField, "->"),
            (S::CastJsonField, "->>"),
            (S::Glob, "GLOB"),
            (S::Match, "MATCH"),
        ] {
            let sql = Query::select()
                .column(Users::Id)
                .from(Users::Table)
                .and_where(
                    Expr::col(Users::Name).binary(BinOper::SqliteOperator(op), Expr::val("x")),
                )
                .to_string(HyperQueryBuilder);
            assert!(sql.contains(&format!(r#""name" {text} 'x'"#)), "{sql}");
        }
    }

    #[test]
    fn pg_functions_render() {
        use sea_query::PgDateTruncUnit;
        use sea_query::extension::postgres::PgFunc;
        let sql = Query::select()
            .expr(PgFunc::starts_with(Expr::col(Users::Name), "ab"))
            .expr(PgFunc::date_trunc(
                PgDateTruncUnit::Day,
                Expr::col(Users::Age),
            ))
            .expr(PgFunc::gen_random_uuid())
            .from(Users::Table)
            .to_string(HyperQueryBuilder);
        assert_eq!(
            sql,
            r#"SELECT STARTS_WITH("name", 'ab'), DATE_TRUNC('day', "age"), GEN_RANDOM_UUID() FROM "users""#
        );
    }

    #[test]
    fn as_enum_keeps_its_cast() {
        let sql = Query::select()
            .expr(Expr::col(Users::Name).as_enum("my_type"))
            .from(Users::Table)
            .to_string(HyperQueryBuilder);
        assert_eq!(sql, r#"SELECT CAST("name" AS "my_type") FROM "users""#);
    }

    #[test]
    fn table_sample_is_rendered() {
        use sea_query::extension::postgres::{PostgresSelectStatementExt, SampleMethod};
        let sql = Query::select()
            .column(Users::Id)
            .from(Users::Table)
            .table_sample(SampleMethod::BERNOULLI, 50.0, Some(7.0))
            .to_string(HyperQueryBuilder);
        assert_eq!(
            sql,
            r#"SELECT "id" FROM "users" TABLESAMPLE BERNOULLI (50) REPEATABLE (7)"#
        );
    }

    #[test]
    fn if_null_is_coalesce() {
        use sea_query::Func;
        let sql = Query::select()
            .expr(Func::if_null(Expr::col(Users::Name), "n/a"))
            .from(Users::Table)
            .to_string(HyperQueryBuilder);
        assert_eq!(sql, r#"SELECT COALESCE("name", 'n/a') FROM "users""#);
    }

    // ---- Q2: DISTINCT ON and NULLS ordering ------------------------------------

    #[test]
    fn distinct_on_is_rendered() {
        let sql = Query::select()
            .distinct_on([Users::Id])
            .columns([Users::Id, Users::Name])
            .from(Users::Table)
            .to_string(HyperQueryBuilder);
        assert_eq!(
            sql,
            r#"SELECT DISTINCT ON ("id") "id", "name" FROM "users""#
        );
    }

    #[test]
    fn distinct_on_multi_column_and_subquery() {
        let inner = Query::select()
            .distinct_on([Users::Id, Users::Name])
            .columns([Users::Id, Users::Name])
            .from(Users::Table)
            .to_owned();
        let sql = Query::select()
            .column(Users::Id)
            .from_subquery(inner, "s")
            .to_string(HyperQueryBuilder);
        assert_eq!(
            sql,
            r#"SELECT "id" FROM (SELECT DISTINCT ON ("id", "name") "id", "name" FROM "users") AS "s""#
        );
    }

    #[test]
    fn nulls_ordering_matrix() {
        use sea_query::{NullOrdering, Order};
        for (order, o_txt) in [(Order::Asc, "ASC"), (Order::Desc, "DESC")] {
            for (nulls, n_txt) in [(NullOrdering::First, "FIRST"), (NullOrdering::Last, "LAST")] {
                let sql = Query::select()
                    .column(Users::Id)
                    .from(Users::Table)
                    .order_by_with_nulls(Users::Age, order.clone(), nulls)
                    .to_string(HyperQueryBuilder);
                assert_eq!(
                    sql,
                    format!(r#"SELECT "id" FROM "users" ORDER BY "age" {o_txt} NULLS {n_txt}"#)
                );
            }
        }
    }

    #[test]
    fn nulls_ordering_inside_subquery() {
        use sea_query::{NullOrdering, Order};
        let inner = Query::select()
            .column(Users::Id)
            .from(Users::Table)
            .order_by_with_nulls(Users::Age, Order::Desc, NullOrdering::Last)
            .to_owned();
        let sql = Query::select()
            .column(Users::Id)
            .from_subquery(inner, "s")
            .to_string(HyperQueryBuilder);
        assert!(sql.contains(r#"ORDER BY "age" DESC NULLS LAST"#), "{sql}");
    }

    // ---- DDL hooks and Hyper-vs-Postgres parity --------------------------------

    #[test]
    fn inline_table_index_matches_postgres() {
        use sea_query::{ColumnDef, Index, IndexOrder, IndexType, Table};
        let stmt = Table::create()
            .table(Users::Table)
            .col(ColumnDef::new(Users::Id).integer().not_null())
            .col(ColumnDef::new(Users::Name).string())
            .index(
                Index::create()
                    .name("users_name_uq")
                    .col((Users::Name, IndexOrder::Desc))
                    .unique()
                    .nulls_not_distinct()
                    .index_type(IndexType::Hash),
            )
            .to_owned();
        let hyper = stmt.to_string(HyperQueryBuilder);
        assert_eq!(hyper, stmt.to_string(PostgresQueryBuilder));
        assert!(hyper.contains("NULLS NOT DISTINCT"), "{hyper}");
    }

    #[test]
    fn partition_by_is_never_silently_dropped_and_never_panics() {
        use sea_query::{ColumnDef, Table};
        for (stmt, keyword) in [
            (
                Table::create()
                    .table(Users::Table)
                    .col(ColumnDef::new(Users::Id).integer())
                    .partition_by_range([Users::Id])
                    .to_owned(),
                r#"PARTITION BY RANGE ("id")"#,
            ),
            (
                Table::create()
                    .table(Users::Table)
                    .col(ColumnDef::new(Users::Id).integer())
                    .partition_by_list([Users::Id])
                    .to_owned(),
                r#"PARTITION BY LIST ("id")"#,
            ),
            (
                Table::create()
                    .table(Users::Table)
                    .col(ColumnDef::new(Users::Id).integer())
                    .partition_by_hash([Users::Id])
                    .to_owned(),
                r#"PARTITION BY HASH ("id")"#,
            ),
            (
                // MySQL-only form: Postgres' builder panics, ours must not.
                Table::create()
                    .table(Users::Table)
                    .col(ColumnDef::new(Users::Id).integer())
                    .partition_by_key([Users::Id])
                    .to_owned(),
                r#"PARTITION BY KEY ("id")"#,
            ),
        ] {
            let sql = stmt.to_string(HyperQueryBuilder);
            assert!(sql.contains(keyword), "{sql}");
        }
    }

    #[test]
    fn dml_and_cte_match_postgres() {
        use sea_query::{
            CommonTableExpression, JoinType, OnConflict, Order, Query, UnionType, WithClause,
        };
        let insert = Query::insert()
            .into_table(Users::Table)
            .columns([Users::Name, Users::Age])
            .values_panic(["it's".into(), 3.into()])
            .on_conflict(
                OnConflict::column(Users::Id)
                    .update_column(Users::Name)
                    .to_owned(),
            )
            .returning_col(Users::Id)
            .to_owned();
        assert_eq!(
            insert.to_string(HyperQueryBuilder),
            insert.to_string(PostgresQueryBuilder)
        );

        let union = Query::select()
            .column(Users::Id)
            .from(Users::Table)
            .union(
                UnionType::All,
                Query::select()
                    .column(Users::Id)
                    .from(Users::Table)
                    .to_owned(),
            )
            .join(
                JoinType::LeftJoin,
                Users::Table,
                Expr::col((Users::Table, Users::Id)).equals((Users::Table, Users::Age)),
            )
            .order_by(Users::Id, Order::Desc)
            .limit(5)
            .offset(2)
            .to_owned();
        assert_eq!(
            union.to_string(HyperQueryBuilder),
            union.to_string(PostgresQueryBuilder)
        );

        let cte = CommonTableExpression::new()
            .query(
                Query::select()
                    .column(Users::Id)
                    .from(Users::Table)
                    .to_owned(),
            )
            .table_name("c")
            .to_owned();
        let with = Query::select()
            .column(Users::Id)
            .from("c")
            .to_owned()
            .with(WithClause::new().cte(cte).to_owned());
        assert_eq!(
            with.to_string(HyperQueryBuilder),
            with.to_string(PostgresQueryBuilder)
        );
    }

    #[test]
    fn enum_values_are_bare_strings() {
        // Hyper has no enum types, so unlike Postgres no `::"type"` hint.
        let v = Value::Enum(OptionEnum::Some(Box::new(sea_query::Enum {
            type_name: "mood".into(),
            value: "ha'ppy".into(),
        })));
        assert_eq!(HyperQueryBuilder.value_to_string(&v), r"E'ha\'ppy'");
    }
}
