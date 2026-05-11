// Copyright (c) 2026, Salesforce, Inc. All rights reserved.
// SPDX-License-Identifier: Apache-2.0 OR MIT

//! Tests for the three-tier schema inference system: JSON structural inference
//! (Tier 2), CSV heuristic inference (Tier 3), and the type name mapping used
//! across all tiers.

#![expect(
    clippy::cast_sign_loss,
    reason = "test uses fixed-size signed literals as u64; sign-loss is unreachable"
)]

use hyperdb_mcp::error::ErrorCode;
use hyperdb_mcp::schema::{
    apply_schema_override, infer_csv_schema, infer_json_schema, map_hyper_type,
    normalize_schema_param, widen_csv_numeric_columns, ColumnSchema,
};
use serde_json::{json, Value};
use std::fmt::Write as _;

/// Verify that integer and string JSON values are inferred as INT and TEXT
/// respectively, and that columns present in every row are marked non-nullable.
#[test]
fn infer_json_ints() {
    let data = r#"[{"id": 1, "name": "Alice"}, {"id": 2, "name": "Bob"}]"#;
    let schema = infer_json_schema(data).unwrap();
    assert_eq!(schema.len(), 2);
    let id_col = schema.iter().find(|c| c.name == "id").unwrap();
    assert_eq!(id_col.hyper_type, "INT");
    assert!(!id_col.nullable);
    let name_col = schema.iter().find(|c| c.name == "name").unwrap();
    assert_eq!(name_col.hyper_type, "TEXT");
}

/// Verify that a column containing an explicit null in any row is marked
/// nullable, even when present in every object.
#[test]
fn infer_json_nullable() {
    let data = r#"[{"a": 1, "b": "x"}, {"a": null, "b": "y"}]"#;
    let schema = infer_json_schema(data).unwrap();
    let a_col = schema.iter().find(|c| c.name == "a").unwrap();
    assert!(a_col.nullable);
}

/// Verify that a column with incompatible types (integer + string) across
/// rows widens to TEXT as the safe fallback.
#[test]
fn infer_json_mixed_types_widen_to_text() {
    let data = r#"[{"v": 1}, {"v": "hello"}]"#;
    let schema = infer_json_schema(data).unwrap();
    let v_col = schema.iter().find(|c| c.name == "v").unwrap();
    assert_eq!(v_col.hyper_type, "TEXT");
}

/// Verify that JSON boolean values are inferred as BOOL.
#[test]
fn infer_json_booleans() {
    let data = r#"[{"flag": true}, {"flag": false}]"#;
    let schema = infer_json_schema(data).unwrap();
    let col = schema.iter().find(|c| c.name == "flag").unwrap();
    assert_eq!(col.hyper_type, "BOOL");
}

/// Verify that JSON floating-point numbers are inferred as DOUBLE PRECISION.
#[test]
fn infer_json_floats() {
    let data = r#"[{"val": 1.5}, {"val": 2.7}]"#;
    let schema = infer_json_schema(data).unwrap();
    let col = schema.iter().find(|c| c.name == "val").unwrap();
    assert_eq!(col.hyper_type, "DOUBLE PRECISION");
}

/// Verify that integers exceeding i32 range are promoted to BIGINT rather
/// than staying as INT. `3_000_000_000` overflows i32 max (`2_147_483_647`).
#[test]
fn infer_json_bigints() {
    let data = r#"[{"big": 3000000000}]"#;
    let schema = infer_json_schema(data).unwrap();
    let col = schema.iter().find(|c| c.name == "big").unwrap();
    assert_eq!(col.hyper_type, "BIGINT");
}

/// Verify that keys missing from some objects produce nullable columns.
/// Object 1 has only "a", object 2 has only "b" — both should be nullable.
#[test]
fn infer_json_missing_keys() {
    let data = r#"[{"a": 1}, {"b": 2}]"#;
    let schema = infer_json_schema(data).unwrap();
    assert_eq!(schema.len(), 2);
    assert!(schema.iter().all(|c| c.nullable));
}

/// Verify that ISO 8601 date strings (YYYY-MM-DD) are recognized and
/// inferred as DATE rather than TEXT.
#[test]
fn infer_json_iso_dates() {
    let data = r#"[{"d": "2024-01-15"}, {"d": "2024-06-30"}]"#;
    let schema = infer_json_schema(data).unwrap();
    let col = schema.iter().find(|c| c.name == "d").unwrap();
    assert_eq!(col.hyper_type, "DATE");
}

/// Verify that ISO 8601 timestamp strings (YYYY-MM-DDThh:mm:ss) are
/// recognized and inferred as TIMESTAMP rather than TEXT.
#[test]
fn infer_json_timestamps() {
    let data = r#"[{"ts": "2024-01-15T10:30:00"}]"#;
    let schema = infer_json_schema(data).unwrap();
    let col = schema.iter().find(|c| c.name == "ts").unwrap();
    assert_eq!(col.hyper_type, "TIMESTAMP");
}

/// Verify CSV Tier 3 inference: header row provides column names, and types
/// are inferred from sampled values (integer, string, float).
#[test]
fn infer_csv_basic() {
    let csv_text = "id,name,score\n1,Alice,95.5\n2,Bob,88.0\n";
    let schema = infer_csv_schema(csv_text, true).unwrap();
    assert_eq!(schema.len(), 3);
    let id_col = schema.iter().find(|c| c.name == "id").unwrap();
    assert_eq!(id_col.hyper_type, "INT");
    let score_col = schema.iter().find(|c| c.name == "score").unwrap();
    assert_eq!(score_col.hyper_type, "DOUBLE PRECISION");
}

/// Verify that CSV without a header row generates synthetic column names
/// (`col_0`, `col_1`, ...) based on the column count in the first data row.
#[test]
fn infer_csv_no_header() {
    let csv_text = "1,Alice\n2,Bob\n";
    let schema = infer_csv_schema(csv_text, false).unwrap();
    assert_eq!(schema[0].name, "col_0");
    assert_eq!(schema[1].name, "col_1");
}

/// Verify that `map_hyper_type` recognizes all commonly used type names
/// including parameterized types like NUMERIC(12,2).
#[test]
fn map_hyper_type_names() {
    assert!(map_hyper_type("INT").is_some());
    assert!(map_hyper_type("BIGINT").is_some());
    assert!(map_hyper_type("TEXT").is_some());
    assert!(map_hyper_type("DOUBLE PRECISION").is_some());
    assert!(map_hyper_type("BOOL").is_some());
    assert!(map_hyper_type("DATE").is_some());
    assert!(map_hyper_type("TIMESTAMP").is_some());
    assert!(map_hyper_type("NUMERIC(12,2)").is_some());
}

/// Verify that a column where every row is null collapses to TEXT (the safe
/// catch-all) and is marked nullable.
#[test]
fn infer_json_all_null_becomes_text() {
    let data = r#"[{"x": null}, {"x": null}]"#;
    let schema = infer_json_schema(data).unwrap();
    let col = schema.iter().find(|c| c.name == "x").unwrap();
    assert_eq!(col.hyper_type, "TEXT");
    assert!(col.nullable);
}

// --- Partial schema override (A) ----------------------------------------

fn cols(entries: &[(&str, &str)]) -> Vec<ColumnSchema> {
    entries
        .iter()
        .map(|(n, t)| ColumnSchema {
            name: (*n).into(),
            hyper_type: (*t).into(),
            nullable: true,
        })
        .collect()
}

/// A partial override should overlay the listed columns by name and leave the
/// rest untouched, preserving the original column order regardless of how the
/// JSON object was serialized on the wire.
#[test]
fn apply_override_partial_preserves_order() {
    let inferred = cols(&[("year", "INT"), ("entity", "TEXT"), ("population", "INT")]);
    let override_map = json!({ "population": "BIGINT" })
        .as_object()
        .cloned()
        .unwrap();
    let result = apply_schema_override(inferred, &override_map).unwrap();
    assert_eq!(result[0].name, "year");
    assert_eq!(result[0].hyper_type, "INT");
    assert_eq!(result[1].name, "entity");
    assert_eq!(result[1].hyper_type, "TEXT");
    assert_eq!(result[2].name, "population");
    assert_eq!(result[2].hyper_type, "BIGINT");
}

/// Unknown override keys should produce a `SchemaMismatch` error that names the
/// real columns so an LLM can fix it without another round-trip.
#[test]
fn apply_override_unknown_column_errors() {
    let inferred = cols(&[("a", "INT"), ("b", "TEXT")]);
    let override_map = json!({ "nope": "BIGINT" }).as_object().cloned().unwrap();
    let err = apply_schema_override(inferred, &override_map).unwrap_err();
    assert_eq!(err.code, ErrorCode::SchemaMismatch);
    assert!(err.message.contains("nope"));
    assert!(err.message.contains("\"a\""));
    assert!(err.message.contains("\"b\""));
}

/// Unknown Hyper type names should also be rejected up front rather than
/// failing later when the table is created.
#[test]
fn apply_override_unknown_type_errors() {
    let inferred = cols(&[("x", "INT")]);
    let override_map = json!({ "x": "FLUBBER" }).as_object().cloned().unwrap();
    let err = apply_schema_override(inferred, &override_map).unwrap_err();
    assert_eq!(err.code, ErrorCode::SchemaMismatch);
    assert!(err.message.contains("FLUBBER"));
}

/// An empty override should be a no-op and leave every inferred type in place.
#[test]
fn apply_override_empty_is_noop() {
    let inferred = cols(&[("a", "INT"), ("b", "TEXT")]);
    let override_map = serde_json::Map::new();
    let result = apply_schema_override(inferred.clone(), &override_map).unwrap();
    assert_eq!(result.len(), 2);
    assert_eq!(result[0].hyper_type, "INT");
    assert_eq!(result[1].hyper_type, "TEXT");
}

// --- Full-file numeric widening (B) -------------------------------------

/// Regression test for the bug that triggered this whole workstream: a CSV
/// where the first 1 000 rows look like INT but a value near the end (think
/// OWID's 8-billion "World" population) exceeds i32. The first-pass inference
/// would mark the column INT; the widening pass must promote it to BIGINT.
#[test]
fn widen_promotes_int_to_bigint_on_late_big_value() {
    let mut csv = String::from("id,population\n");
    for i in 0..1200 {
        let _ = writeln!(csv, "{i},{}\n", 100 + i);
    }
    csv.push_str("9999,8000000000\n");

    let mut columns = infer_csv_schema(&csv, true).unwrap();
    assert_eq!(columns[1].hyper_type, "INT");
    widen_csv_numeric_columns(csv.as_bytes(), true, &mut columns).unwrap();
    assert_eq!(columns[1].hyper_type, "BIGINT");
}

/// An integer column whose late-file rows exceed `i64::MAX` must widen to
/// `NUMERIC(38,0)` rather than silently overflowing on COPY. The first-pass
/// inference only samples the first 1 000 rows, so the huge value is hidden
/// from it — exactly the scenario that motivated this full-file pass.
#[test]
fn widen_promotes_bigint_to_numeric_on_huge_late_value() {
    let mut csv = String::from("id,amount\n");
    for i in 0..1200 {
        let _ = writeln!(csv, "{i},{}\n", 5_000_000_000_u64 + i as u64);
    }
    csv.push_str("9999,99999999999999999999\n");

    let mut columns = infer_csv_schema(&csv, true).unwrap();
    assert_eq!(columns[1].hyper_type, "BIGINT");
    widen_csv_numeric_columns(csv.as_bytes(), true, &mut columns).unwrap();
    assert_eq!(columns[1].hyper_type, "NUMERIC(38,0)");
}

/// A column that starts integer-looking but later contains a decimal value
/// must be promoted to DOUBLE PRECISION, not kept as INT.
#[test]
fn widen_promotes_int_to_double_on_later_float() {
    let csv = "id,value\n1,100\n2,3.14\n";
    let mut columns = infer_csv_schema(csv, true).unwrap();
    widen_csv_numeric_columns(csv.as_bytes(), true, &mut columns).unwrap();
    assert_eq!(columns[1].hyper_type, "DOUBLE PRECISION");
}

/// Non-numeric columns and numeric columns already within range must be left
/// alone; widening is strictly a promotion pass.
#[test]
fn widen_does_not_touch_text_or_in_range_ints() {
    let csv = "name,count\nalice,10\nbob,20\n";
    let mut columns = infer_csv_schema(csv, true).unwrap();
    widen_csv_numeric_columns(csv.as_bytes(), true, &mut columns).unwrap();
    assert_eq!(columns[0].hyper_type, "TEXT");
    assert_eq!(columns[1].hyper_type, "INT");
}

// --- Schema parameter normalization (C) ---------------------------------

/// Happy path: a JSON object (the documented shape) passes through unchanged.
#[test]
fn normalize_schema_accepts_object() {
    let v = json!({ "postal_code": "TEXT", "accuracy": "TEXT" });
    let result = normalize_schema_param(Some(&v)).unwrap().unwrap();
    assert_eq!(
        result.get("postal_code").and_then(|v| v.as_str()),
        Some("TEXT")
    );
    assert_eq!(
        result.get("accuracy").and_then(|v| v.as_str()),
        Some("TEXT")
    );
}

/// `None` and `Value::Null` both round-trip to `Ok(None)` so callers can opt
/// out of an override cleanly.
#[test]
fn normalize_schema_accepts_none_and_null() {
    assert!(normalize_schema_param(None).unwrap().is_none());
    let null = Value::Null;
    assert!(normalize_schema_param(Some(&null)).unwrap().is_none());
}

/// Regression guard for the `load_file` / `load_data` schema-ignored bug: when a
/// client serializes `schema` as a JSON-encoded string (Windsurf/Cascade
/// behavior for `Option<Value>` tool params with an unconstrained JSON Schema),
/// we must parse it rather than silently dropping the override — otherwise
/// overrides like `{"postal_code": "TEXT"}` get lost and hyperd fails COPY
/// with `22P02 invalid input syntax for integer` on a leading-zero ZIP code.
#[test]
fn normalize_schema_accepts_stringified_object() {
    let v = Value::String(r#"{"postal_code": "TEXT", "accuracy": "TEXT"}"#.into());
    let result = normalize_schema_param(Some(&v)).unwrap().unwrap();
    assert_eq!(
        result.get("postal_code").and_then(|v| v.as_str()),
        Some("TEXT")
    );
    assert_eq!(
        result.get("accuracy").and_then(|v| v.as_str()),
        Some("TEXT")
    );
}

/// Empty / whitespace-only strings mean "no override", matching the `None`
/// contract. This avoids a spurious error from clients that explicitly blank
/// the field rather than omitting it.
#[test]
fn normalize_schema_accepts_empty_string_as_none() {
    let v = Value::String(String::new());
    assert!(normalize_schema_param(Some(&v)).unwrap().is_none());
    let v = Value::String("   ".into());
    assert!(normalize_schema_param(Some(&v)).unwrap().is_none());
}

/// A malformed JSON string produces a `SchemaMismatch` error naming the
/// expected shape, not a silent no-op.
#[test]
fn normalize_schema_rejects_bad_json_string() {
    let v = Value::String("not-json".into());
    let err = normalize_schema_param(Some(&v)).unwrap_err();
    assert_eq!(err.code, ErrorCode::SchemaMismatch);
    assert!(
        err.message.contains("not valid JSON"),
        "message should explain the parse failure: {}",
        err.message
    );
}

/// A string that parses to something other than a JSON object (e.g. array,
/// number) is rejected with an error pointing at the actual JSON kind so the
/// LLM can self-correct.
#[test]
fn normalize_schema_rejects_non_object_json_string() {
    let v = Value::String("[\"TEXT\"]".into());
    let err = normalize_schema_param(Some(&v)).unwrap_err();
    assert_eq!(err.code, ErrorCode::SchemaMismatch);
    assert!(
        err.message.contains("array"),
        "message should name the kind: {}",
        err.message
    );
}

/// Non-object, non-string JSON values (e.g. a bare number) are also rejected
/// — again, with a clear `SchemaMismatch` rather than a silent drop.
#[test]
fn normalize_schema_rejects_non_object_non_string_value() {
    let v = json!(42);
    let err = normalize_schema_param(Some(&v)).unwrap_err();
    assert_eq!(err.code, ErrorCode::SchemaMismatch);
    assert!(
        err.message.contains("number"),
        "message should name the kind: {}",
        err.message
    );
}
