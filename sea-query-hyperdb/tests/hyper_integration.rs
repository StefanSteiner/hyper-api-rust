// Copyright (c) 2026, Salesforce, Inc. All rights reserved.
// SPDX-License-Identifier: Apache-2.0 OR MIT

//! End-to-end tests: SQL produced by [`HyperQueryBuilder`] is executed on a
//! real `hyperd`, so the dialect claims are checked against the engine rather
//! than against sea-query's idea of Postgres.
//!
//! Needs `hyperd` (see `AGENTS.md`: `make download-hyperd`, then an absolute
//! `HYPERD_PATH`).

use hyperdb_api::{Connection, CreateMode, HyperProcess, Parameters};
use sea_query::extension::postgres::{
    PgBinOper, PgExpr, PgFunc, PostgresSelectStatementExt, SampleMethod,
};
use sea_query::{
    Alias, BinOper, Expr, ExprTrait, Iden, LikeExpr, NullOrdering, Order, PgDateTruncUnit, Query,
    Value, Values,
};
use sea_query_hyperdb::HyperQueryBuilder;

#[derive(Iden)]
enum T {
    Table,
    Id,
    S,
}

/// A running `hyperd` plus a fresh database; both are cleaned up on drop.
struct Fixture {
    // Field order matters: the connection must drop before the process.
    conn: Connection,
    _hyper: HyperProcess,
    dir: std::path::PathBuf,
}

impl Fixture {
    fn new(name: &str) -> Self {
        let dir = std::env::temp_dir().join(format!("sq_hyperdb_{name}_{}", std::process::id()));
        std::fs::create_dir_all(&dir).expect("create temp dir");
        let mut params = Parameters::new();
        params.set("log_dir", dir.to_string_lossy().to_string());
        let hyper = HyperProcess::new(None, Some(&params)).expect("start hyperd");
        let conn = Connection::new(&hyper, dir.join("t.hyper"), CreateMode::CreateAndReplace)
            .expect("connect");
        Self {
            conn,
            _hyper: hyper,
            dir,
        }
    }

    fn exec(&self, sql: &str) {
        self.conn
            .execute_command(sql)
            .unwrap_or_else(|e| panic!("`{sql}` failed: {e}"));
    }

    fn strings(&self, sql: &str) -> Vec<Option<String>> {
        self.conn
            .fetch_all(sql)
            .unwrap_or_else(|e| panic!("`{sql}` failed: {e}"))
            .iter()
            .map(|row| row.get::<String>(0))
            .collect()
    }

    fn one_string(&self, sql: &str) -> String {
        self.strings(sql)
            .pop()
            .flatten()
            .unwrap_or_else(|| panic!("`{sql}` returned no value"))
    }

    fn count(&self, sql: &str) -> usize {
        self.conn
            .fetch_all(sql)
            .unwrap_or_else(|e| panic!("`{sql}` failed: {e}"))
            .len()
    }
}

impl Drop for Fixture {
    fn drop(&mut self) {
        // Best effort: the temp dir only holds test artifacts.
        let _ = std::fs::remove_dir_all(&self.dir);
    }
}

fn hostile_strings() -> Vec<String> {
    // Every ASCII scalar except NUL (Hyper text cannot hold NUL), one at a time.
    let mut all: Vec<String> = (1u8..=127).map(|b| char::from(b).to_string()).collect();
    all.extend(
        [
            "é",
            "😀",
            r"\",
            "'",
            "\"",
            r"\'",
            r"\\'",
            "x\\' OR 1=1 --",
            "'; DROP TABLE t; --",
            "a\nb\tc\rd\u{8}e\u{c}f",
            r"\x41A\101\n",
            "%_\\%",
        ]
        .map(String::from),
    );
    all
}

#[test]
fn hostile_strings_round_trip_through_every_path() {
    let fx = Fixture::new("roundtrip");
    for text in hostile_strings() {
        // Expr::val -> placeholder + Values, then re-inlined with inject_parameters.
        let select = Query::select().expr(Expr::val(text.as_str())).to_owned();
        let to_string = fx.one_string(&select.to_string(HyperQueryBuilder));
        assert_eq!(to_string, text, "Expr::val via to_string");

        let (sql, values) = select.build(HyperQueryBuilder);
        let injected = sea_query::prepare::inject_parameters(&sql, &values.0, &HyperQueryBuilder);
        assert_eq!(fx.one_string(&injected), text, "inject_parameters");

        // Expr::Constant goes through Hyper's own `write_value` / `write_string_quoted`.
        let constant = Query::select()
            .expr(Expr::Constant(Value::String(Some(text.clone()))))
            .to_string(HyperQueryBuilder);
        assert_eq!(fx.one_string(&constant), text, "Expr::Constant");
    }
}

#[test]
fn order_field_round_trips_hostile_strings() {
    let fx = Fixture::new("orderfield");
    fx.exec(r#"CREATE TABLE "t" ("id" INT, "s" TEXT)"#);
    let hostile = hostile_strings();
    for (id, text) in hostile.iter().enumerate() {
        let insert = Query::insert()
            .into_table(T::Table)
            .columns([T::Id, T::S])
            .values_panic([
                i32::try_from(id).expect("small index").into(),
                text.as_str().into(),
            ])
            .to_string(HyperQueryBuilder);
        fx.exec(&insert);
    }
    for text in &hostile {
        // The row equal to `text` must sort first iff `text` survived escaping.
        let sql = Query::select()
            .column(T::S)
            .from(T::Table)
            .order_by_expr(
                Expr::col(T::S),
                Order::Field(Values(vec![text.as_str().into()])),
            )
            .limit(1)
            .to_string(HyperQueryBuilder);
        assert_eq!(fx.one_string(&sql), *text, "Order::Field");
    }
}

#[test]
fn constant_injection_attempt_matches_nothing() {
    let fx = Fixture::new("injection");
    fx.exec(r#"CREATE TABLE "t" ("id" INT, "s" TEXT)"#);
    fx.exec(r#"INSERT INTO "t" VALUES (1, 'a'), (2, 'b'), (3, 'c')"#);
    let sql = Query::select()
        .column(T::Id)
        .from(T::Table)
        .and_where(Expr::col(T::S).eq(Expr::Constant(Value::String(Some(
            "x\\' OR 1=1 --".to_owned(),
        )))))
        .to_string(HyperQueryBuilder);
    assert_eq!(fx.count(&sql), 0, "{sql}");
}

#[test]
fn like_escape_clause_is_accepted() {
    let fx = Fixture::new("like");
    fx.exec(r#"CREATE TABLE "t" ("id" INT, "s" TEXT)"#);
    fx.exec(r#"INSERT INTO "t" VALUES (1, 'a_b'), (2, 'axb')"#);
    let sql = Query::select()
        .column(T::Id)
        .from(T::Table)
        .and_where(Expr::col(T::S).like(LikeExpr::new(r"a\_b").escape('\\')))
        .to_string(HyperQueryBuilder);
    assert_eq!(fx.count(&sql), 1, "{sql}");
}

#[test]
fn bytes_round_trip() {
    let fx = Fixture::new("bytes");
    let payload: Vec<u8> = vec![0x00, 0x01, 0x27, 0x5c, 0x7f, 0x80, 0xff];
    // The literal is `'\x...'` (untyped, like Postgres), so it is read back
    // through a BYTEA column rather than a bare SELECT.
    fx.exec(r#"CREATE TABLE "b" ("v" BYTEA)"#);
    let insert = Query::insert()
        .into_table(Alias::new("b"))
        .columns([Alias::new("v")])
        .values_panic([Expr::Constant(Value::Bytes(Some(payload.clone())))])
        .to_string(HyperQueryBuilder);
    fx.exec(&insert);
    let sql = Query::select()
        .column(Alias::new("v"))
        .from(Alias::new("b"))
        .to_string(HyperQueryBuilder);
    let row = fx
        .conn
        .fetch_one(&sql)
        .unwrap_or_else(|e| panic!("`{sql}` failed: {e}"));
    assert_eq!(row.get::<Vec<u8>>(0), Some(payload));
}

#[test]
fn nul_round_trips_exactly_and_cannot_end_the_literal() {
    let fx = Fixture::new("nul");
    for text in ["a\0b", "\u{0}47' OR 1=1 --", "\u{0}\u{0}7"] {
        let sql = Query::select()
            .expr(Expr::Constant(Value::String(Some(text.to_owned()))))
            .to_string(HyperQueryBuilder);
        assert!(!str::contains(&sql, '\0'), "raw NUL in SQL text");
        assert_eq!(fx.one_string(&sql), text, "sql: {sql}");
    }
    assert_eq!(fx.one_string("SELECT 'still alive'"), "still alive");
}

#[test]
fn non_ascii_char_constant_executes() {
    let fx = Fixture::new("char");
    let sql = Query::select()
        .expr(Expr::Constant(Value::Char(Some('é'))))
        .to_string(HyperQueryBuilder);
    assert_eq!(fx.one_string(&sql), "é");
}

#[test]
fn postgres_operators_and_functions_execute() {
    let fx = Fixture::new("operators");
    fx.exec(r#"CREATE TABLE "t" ("id" INT, "s" TEXT)"#);
    fx.exec(r#"INSERT INTO "t" VALUES (1, 'Alpha'), (2, 'beta'), (3, 'Gamma')"#);

    let count_where = |cond: Expr| {
        let sql = Query::select()
            .column(T::Id)
            .from(T::Table)
            .and_where(cond)
            .to_string(HyperQueryBuilder);
        fx.count(&sql)
    };

    assert_eq!(count_where(Expr::col(T::S).ilike("alp%")), 1);
    assert_eq!(count_where(Expr::col(T::S).not_ilike("alp%")), 2);
    assert_eq!(
        count_where(Expr::col(T::S).binary(PgBinOper::Regex, Expr::val("^[A-Z]"))),
        2
    );
    assert_eq!(
        count_where(Expr::col(T::S).binary(PgBinOper::RegexCaseInsensitive, Expr::val("^b"))),
        1
    );
    assert_eq!(
        count_where(Expr::col(T::S).binary(BinOper::Custom("!~"), Expr::val("^[A-Z]"))),
        1
    );
    assert_eq!(count_where(Expr::col(T::S).concat("!").eq("beta!")), 1);
    assert_eq!(
        count_where(Expr::expr(PgFunc::starts_with(Expr::col(T::S), "Ga"))),
        1
    );
    assert_eq!(
        count_where(
            Expr::col(T::Id)
                .eq(Expr::expr(PgFunc::any(Expr::cust("ARRAY[2,3]"))))
                .not()
        ),
        1
    );
}

#[test]
fn date_trunc_and_json_operators_execute() {
    let fx = Fixture::new("json");
    let sql = Query::select()
        .expr(
            Expr::expr(PgFunc::date_trunc(
                PgDateTruncUnit::Day,
                Expr::cust("TIMESTAMP '2024-05-06 07:08:09'"),
            ))
            .cast_as(Alias::new("text")),
        )
        .to_string(HyperQueryBuilder);
    assert!(fx.one_string(&sql).starts_with("2024-05-06"), "{sql}");

    let sql = Query::select()
        .expr(Expr::cust(r#"'{"a": "v"}'::json"#).cast_json_field("a"))
        .to_string(HyperQueryBuilder);
    assert_eq!(fx.one_string(&sql), "v", "{sql}");
}

#[test]
fn unsupported_operator_is_a_server_error_not_a_panic() {
    let fx = Fixture::new("unsupported");
    let sql = Query::select()
        .expr(Expr::cust("ARRAY[1,2]").contains(Expr::cust("ARRAY[1]")))
        .to_string(HyperQueryBuilder);
    assert!(str::contains(&sql, "@>"), "{sql}");
    let outcome = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        fx.conn.fetch_all(&sql).map(|rows| rows.len())
    }));
    let result = outcome.expect("builder or client panicked");
    let err = result.expect_err("Hyper has no @> operator");
    assert_eq!(err.sqlstate(), Some("42601"), "unexpected error: {err:?}");
}

#[test]
fn cast_tablesample_and_coalesce_execute() {
    let fx = Fixture::new("misc");
    fx.exec(r#"CREATE TABLE "t" ("id" INT, "s" TEXT)"#);
    fx.exec(r#"INSERT INTO "t" VALUES (1, NULL), (2, 'b'), (3, 'c')"#);

    let cast = Query::select()
        .expr(Expr::val("12").as_enum(Alias::new("text")))
        .to_string(HyperQueryBuilder);
    assert!(str::contains(&cast, r#"CAST('12' AS "text")"#), "{cast}");
    assert_eq!(fx.one_string(&cast), "12", "{cast}");

    let sample = Query::select()
        .column(T::Id)
        .from(T::Table)
        .table_sample(SampleMethod::BERNOULLI, 100.0, Some(1.0))
        .to_string(HyperQueryBuilder);
    assert!(str::contains(&sample, "REPEATABLE"), "{sample}");
    assert_eq!(fx.count(&sample), 3, "{sample}");

    let coalesce = Query::select()
        .expr(sea_query::Func::if_null(Expr::col(T::S), "none"))
        .from(T::Table)
        .order_by(T::Id, Order::Asc)
        .to_string(HyperQueryBuilder);
    assert!(str::contains(&coalesce, "COALESCE"), "{coalesce}");
    assert_eq!(
        fx.strings(&coalesce),
        vec![
            Some("none".to_owned()),
            Some("b".to_owned()),
            Some("c".to_owned())
        ]
    );
}

#[test]
fn distinct_on_and_nulls_ordering_execute() {
    let fx = Fixture::new("distinct");
    fx.exec(r#"CREATE TABLE "t" ("id" INT, "s" TEXT)"#);
    fx.exec(r#"INSERT INTO "t" VALUES (1, 'a'), (1, 'a'), (2, NULL), (2, 'z'), (3, NULL)"#);

    let sql = Query::select()
        .distinct_on([T::Id])
        .columns([T::Id, T::S])
        .from(T::Table)
        .order_by(T::Id, Order::Asc)
        .to_string(HyperQueryBuilder);
    assert_eq!(fx.count(&sql), 3, "{sql}");

    let first = |nulls: NullOrdering, order: Order| {
        let sql = Query::select()
            .column(T::S)
            .from(T::Table)
            .order_by_with_nulls(T::S, order, nulls)
            .to_string(HyperQueryBuilder);
        fx.strings(&sql)
    };
    let nulls_first = first(NullOrdering::First, Order::Asc);
    assert_eq!(nulls_first.first(), Some(&None));
    let nulls_last = first(NullOrdering::Last, Order::Desc);
    assert_eq!(nulls_last.last(), Some(&None));
    assert_eq!(nulls_last.first(), Some(&Some("z".to_owned())));
}

#[test]
fn column_defaults_with_char_and_enum_execute() {
    let fx = Fixture::new("defaults");
    let enum_value = Value::Enum(sea_query::OptionEnum::Some(Box::new(sea_query::Enum {
        type_name: "mood".into(),
        value: "happy".into(),
    })));
    let ddl = sea_query::Table::create()
        .table(Alias::new("d"))
        .col(
            sea_query::ColumnDef::new(Alias::new("c"))
                .text()
                .default('é'),
        )
        .col(
            sea_query::ColumnDef::new(Alias::new("m"))
                .text()
                .default(Expr::val(enum_value)),
        )
        .to_string(HyperQueryBuilder);
    fx.exec(&ddl);
    fx.exec(r#"INSERT INTO "d" ("c") VALUES (DEFAULT)"#);
    assert_eq!(fx.one_string(r#"SELECT "c" FROM "d""#), "é");
    assert_eq!(fx.one_string(r#"SELECT "m" FROM "d""#), "happy");
}
