// Copyright (c) 2026, Salesforce, Inc. All rights reserved.
// SPDX-License-Identifier: Apache-2.0 OR MIT

//! Query parameter support for gRPC queries.
//!
//! This module provides types and utilities for parameterized queries over gRPC.
//! Parameters can be passed as JSON or Arrow IPC format.
//!
//! # Example
//!
//! ```no_run
//! use hyperdb_api_core::client::grpc::{GrpcClient, GrpcConfig, QueryParameters, ParameterStyle};
//!
//! # async fn example() -> Result<(), Box<dyn std::error::Error>> {
//! # let config = GrpcConfig::new("http://localhost:7484");
//! let mut client = GrpcClient::connect(config).await?;
//!
//! // Using JSON parameters with $1, $2 style (use from_json_value for mixed types)
//! let params = QueryParameters::from_json_value(&serde_json::json!([42, "hello"]))?;
//! let result = client.execute_query_with_params(
//!     "SELECT * FROM users WHERE id = $1 AND name = $2",
//!     params,
//!     ParameterStyle::DollarNumbered,
//! ).await?;
//!
//! // Using JSON parameters with named style
//! let params = QueryParameters::json_named()
//!     .add("id", &42i64)?
//!     .add("name", &"hello")?
//!     .build();
//! let result = client.execute_query_with_params(
//!     "SELECT * FROM users WHERE id = :id AND name = :name",
//!     params,
//!     ParameterStyle::Named,
//! ).await?;
//! # Ok(())
//! # }
//! ```

use bytes::Bytes;
use serde::Serialize;
use serde_json::Value as JsonValue;

use super::proto::hyper_service::query_param::{ParameterStyle as ProtoParameterStyle, Parameters};
use super::proto::hyper_service::{QueryParameterArrow, QueryParameterJson};

/// Parameter style for SQL queries.
///
/// This determines how parameters are referenced in the SQL query string.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum ParameterStyle {
    /// Use question marks: `SELECT * FROM users WHERE id = ?`
    #[default]
    QuestionMark,
    /// Use dollar-numbered placeholders: `SELECT * FROM users WHERE id = $1`
    DollarNumbered,
    /// Use named parameters with colon: `SELECT * FROM users WHERE id = :id`
    Named,
}

impl From<ParameterStyle> for i32 {
    fn from(style: ParameterStyle) -> Self {
        match style {
            ParameterStyle::QuestionMark => ProtoParameterStyle::QuestionMark as i32,
            ParameterStyle::DollarNumbered => ProtoParameterStyle::DollarNumbered as i32,
            ParameterStyle::Named => ProtoParameterStyle::Named as i32,
        }
    }
}

impl From<ParameterStyle> for ProtoParameterStyle {
    fn from(style: ParameterStyle) -> Self {
        match style {
            ParameterStyle::QuestionMark => ProtoParameterStyle::QuestionMark,
            ParameterStyle::DollarNumbered => ProtoParameterStyle::DollarNumbered,
            ParameterStyle::Named => ProtoParameterStyle::Named,
        }
    }
}

/// Query parameters for gRPC queries.
///
/// Parameters can be encoded as JSON or Arrow IPC format.
/// JSON is simpler but Arrow is more efficient for large parameter sets.
#[derive(Debug, Clone)]
pub enum QueryParameters {
    /// JSON-encoded parameters.
    Json(String),
    /// Arrow IPC-encoded parameters. Stored as `Bytes` so the parameter
    /// payload can be handed to prost without a copy.
    Arrow(Bytes),
}

impl QueryParameters {
    /// Creates JSON parameters from a JSON string.
    ///
    /// # Example
    ///
    /// ```
    /// use hyperdb_api_core::client::grpc::QueryParameters;
    ///
    /// // For positional parameters ($1, $2 or ?)
    /// let params = QueryParameters::from_json_string("[42, \"hello\"]");
    ///
    /// // For named parameters (:id, :name)
    /// let params = QueryParameters::from_json_string(r#"{"id": 42, "name": "hello"}"#);
    /// ```
    pub fn from_json_string(json: impl Into<String>) -> Self {
        QueryParameters::Json(json.into())
    }

    /// Creates JSON parameters from a serializable value.
    ///
    /// # Example
    ///
    /// ```
    /// use hyperdb_api_core::client::grpc::QueryParameters;
    ///
    /// let params = QueryParameters::from_json_value(&vec![42, 100, 200])?;
    /// # Ok::<(), serde_json::Error>(())
    /// ```
    ///
    /// # Errors
    ///
    /// Returns a [`serde_json::Error`] if `value` cannot be serialized
    /// to JSON (for example, a type with a failing `Serialize` impl).
    pub fn from_json_value<T: Serialize>(value: &T) -> Result<Self, serde_json::Error> {
        let json = serde_json::to_string(value)?;
        Ok(QueryParameters::Json(json))
    }

    /// Creates JSON parameters for positional placeholders ($1, $2 or ?).
    ///
    /// Values are serialized as a JSON array. All values must be the same type.
    /// For mixed types, use [`from_json_value`](Self::from_json_value) with `serde_json::json!`.
    ///
    /// # Example
    ///
    /// ```
    /// use hyperdb_api_core::client::grpc::QueryParameters;
    ///
    /// // Same-type parameters
    /// let params = QueryParameters::json_positional(&[&42i64, &100i64])?;
    ///
    /// // Mixed types: use from_json_value with serde_json::json!
    /// let params = QueryParameters::from_json_value(&serde_json::json!([42, "hello", true]))?;
    /// # Ok::<(), serde_json::Error>(())
    /// ```
    ///
    /// # Errors
    ///
    /// Returns a [`serde_json::Error`] if any element of `values` fails
    /// to serialize to JSON.
    pub fn json_positional<T: Serialize + ?Sized>(
        values: &[&T],
    ) -> Result<Self, serde_json::Error> {
        let json_values: Vec<JsonValue> = values
            .iter()
            .map(serde_json::to_value)
            .collect::<Result<_, _>>()?;
        let json = serde_json::to_string(&json_values)?;
        Ok(QueryParameters::Json(json))
    }

    /// Creates a builder for named JSON parameters (:name style).
    ///
    /// # Example
    ///
    /// ```
    /// use hyperdb_api_core::client::grpc::QueryParameters;
    ///
    /// let params = QueryParameters::json_named()
    ///     .add("id", &42i64)?
    ///     .add("name", &"Alice")?
    ///     .add("active", &true)?
    ///     .build();
    /// # Ok::<(), serde_json::Error>(())
    /// ```
    #[must_use]
    pub fn json_named() -> JsonNamedParamsBuilder {
        JsonNamedParamsBuilder::new()
    }

    /// Creates Arrow IPC parameters from raw bytes.
    ///
    /// The bytes should contain an Arrow IPC stream with schema and a single
    /// record batch containing the parameter values.
    ///
    /// # Example
    ///
    /// ```ignore
    /// use hyperdb_api_core::client::grpc::QueryParameters;
    /// use arrow::array::{Int64Array, StringArray};
    /// use arrow::datatypes::{DataType, Field, Schema};
    /// use arrow::record_batch::RecordBatch;
    /// use arrow::ipc::writer::StreamWriter;
    /// use std::sync::Arc;
    ///
    /// # fn example() -> Result<(), Box<dyn std::error::Error>> {
    /// // Create Arrow arrays for parameters
    /// let id_array = Int64Array::from(vec![42]);
    /// let name_array = StringArray::from(vec!["Alice"]);
    ///
    /// // Create schema and record batch
    /// let schema = Arc::new(Schema::new(vec![
    ///     Field::new("id", DataType::Int64, false),
    ///     Field::new("name", DataType::Utf8, false),
    /// ]));
    /// let batch = RecordBatch::try_new(schema.clone(), vec![
    ///     Arc::new(id_array),
    ///     Arc::new(name_array),
    /// ])?;
    ///
    /// // Serialize to IPC
    /// let mut buf = Vec::new();
    /// let mut writer = StreamWriter::try_new(&mut buf, &batch.schema())?;
    /// writer.write(&batch)?;
    /// writer.finish()?;
    ///
    /// let params = QueryParameters::from_arrow(buf);
    /// # Ok(())
    /// # }
    /// ```
    pub fn from_arrow(data: impl Into<Bytes>) -> Self {
        QueryParameters::Arrow(data.into())
    }

    /// Converts to the proto Parameters type.
    pub(crate) fn into_proto(self) -> Parameters {
        match self {
            QueryParameters::Json(json) => {
                Parameters::JsonParameters(QueryParameterJson { data: json })
            }
            QueryParameters::Arrow(data) => {
                Parameters::ArrowParameters(QueryParameterArrow { data })
            }
        }
    }

    /// Returns true if this is JSON-encoded parameters.
    pub fn is_json(&self) -> bool {
        matches!(self, QueryParameters::Json(_))
    }

    /// Returns true if this is Arrow-encoded parameters.
    pub fn is_arrow(&self) -> bool {
        matches!(self, QueryParameters::Arrow(_))
    }
}

/// Builder for named JSON parameters.
///
/// # Example
///
/// ```
/// use hyperdb_api_core::client::grpc::QueryParameters;
///
/// let params = QueryParameters::json_named()
///     .add("user_id", &123)?
///     .add("email", &"user@example.com")?
///     .build();
/// # Ok::<(), serde_json::Error>(())
/// ```
#[derive(Debug, Default)]
pub struct JsonNamedParamsBuilder {
    params: serde_json::Map<String, JsonValue>,
}

impl JsonNamedParamsBuilder {
    /// Creates a new builder.
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// Adds a named parameter.
    ///
    /// # Arguments
    ///
    /// * `name` - Parameter name (without the colon prefix)
    /// * `value` - Parameter value (must be JSON-serializable)
    ///
    /// # Errors
    ///
    /// Returns a [`serde_json::Error`] if `value` cannot be serialized
    /// to JSON.
    pub fn add<T: Serialize>(
        mut self,
        name: impl Into<String>,
        value: &T,
    ) -> Result<Self, serde_json::Error> {
        let json_value = serde_json::to_value(value)?;
        self.params.insert(name.into(), json_value);
        Ok(self)
    }

    #[must_use]
    /// Adds a null parameter.
    pub fn add_null(mut self, name: impl Into<String>) -> Self {
        self.params.insert(name.into(), JsonValue::Null);
        self
    }

    /// Builds the `QueryParameters`.
    ///
    /// # Panics
    ///
    /// Does not panic in practice. The `serde_json::to_string` call on a
    /// `Map<String, Value>` is infallible for valid JSON trees —
    /// serde_json only fails when a user-defined `Serialize` impl
    /// returns an error, which cannot happen for the already-validated
    /// `Value` payloads inserted via [`Self::add`] and [`Self::add_null`].
    #[must_use]
    pub fn build(self) -> QueryParameters {
        let json = serde_json::to_string(&JsonValue::Object(self.params))
            .expect("serializing Map<String, Value> never fails");
        QueryParameters::Json(json)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_json_positional_integers() {
        let params = QueryParameters::json_positional(&[&42i64, &100i64, &200i64]).unwrap();
        match params {
            QueryParameters::Json(json) => {
                assert_eq!(json, r"[42,100,200]");
            }
            QueryParameters::Arrow(_) => panic!("Expected JSON parameters"),
        }
    }

    #[test]
    fn test_json_positional_strings() {
        let params = QueryParameters::json_positional(&[&"hello", &"world"]).unwrap();
        match params {
            QueryParameters::Json(json) => {
                assert_eq!(json, r#"["hello","world"]"#);
            }
            QueryParameters::Arrow(_) => panic!("Expected JSON parameters"),
        }
    }

    #[test]
    fn test_json_positional_mixed_via_value() {
        // For mixed types, use from_json_value with serde_json::json!
        let values = serde_json::json!([42, "hello", true]);
        let params = QueryParameters::from_json_value(&values).unwrap();
        match params {
            QueryParameters::Json(json) => {
                assert_eq!(json, r#"[42,"hello",true]"#);
            }
            QueryParameters::Arrow(_) => panic!("Expected JSON parameters"),
        }
    }

    #[test]
    fn test_json_named() {
        let params = QueryParameters::json_named()
            .add("id", &42i64)
            .unwrap()
            .add("name", &"Alice")
            .unwrap()
            .build();
        match params {
            QueryParameters::Json(json) => {
                let parsed: serde_json::Value = serde_json::from_str(&json).unwrap();
                assert_eq!(parsed["id"], 42);
                assert_eq!(parsed["name"], "Alice");
            }
            QueryParameters::Arrow(_) => panic!("Expected JSON parameters"),
        }
    }

    #[test]
    fn test_json_named_with_null() {
        let params = QueryParameters::json_named()
            .add("id", &42i64)
            .unwrap()
            .add_null("optional")
            .build();
        match params {
            QueryParameters::Json(json) => {
                let parsed: serde_json::Value = serde_json::from_str(&json).unwrap();
                assert_eq!(parsed["id"], 42);
                assert!(parsed["optional"].is_null());
            }
            QueryParameters::Arrow(_) => panic!("Expected JSON parameters"),
        }
    }

    #[test]
    fn test_from_json_string() {
        let params = QueryParameters::from_json_string(r#"{"foo": "bar"}"#);
        assert!(params.is_json());
        assert!(!params.is_arrow());
    }

    #[test]
    fn test_arrow_params() {
        let params = QueryParameters::from_arrow(vec![1, 2, 3, 4]);
        assert!(params.is_arrow());
        assert!(!params.is_json());
    }

    #[test]
    fn test_parameter_style_conversion() {
        assert_eq!(i32::from(ParameterStyle::QuestionMark), 3);
        assert_eq!(i32::from(ParameterStyle::DollarNumbered), 1);
        assert_eq!(i32::from(ParameterStyle::Named), 2);
    }
}
