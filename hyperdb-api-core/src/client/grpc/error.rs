// Copyright (c) 2026, Salesforce, Inc. All rights reserved.
// SPDX-License-Identifier: Apache-2.0 OR MIT

//! gRPC-specific error types.
//!
//! This module handles conversion from gRPC status codes and Hyper's structured
//! error details to the common [`crate::Error`] type.

use std::fmt;

use tonic::Status;

use crate::client::error::{Error, ErrorKind};

/// gRPC-specific error information.
///
/// This wraps additional error details from Hyper's gRPC error responses,
/// including SQLSTATE codes, hints, and detailed error messages.
#[derive(Debug, Clone)]
pub struct GrpcError {
    /// The SQLSTATE error code (e.g., "42703" for undefined column)
    pub sqlstate: Option<String>,
    /// The primary error message
    pub message: String,
    /// Additional detail about the error
    pub detail: Option<String>,
    /// A hint for how to resolve the error
    pub hint: Option<String>,
    /// The error source ("User" or "System")
    pub error_source: Option<String>,
}

impl fmt::Display for GrpcError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}", self.message)?;
        if let Some(ref detail) = self.detail {
            write!(f, ": {detail}")?;
        }
        Ok(())
    }
}

impl std::error::Error for GrpcError {}

#[expect(
    clippy::needless_pass_by_value,
    reason = "call-site ergonomics: function consumes logically-owned parameters, refactoring signatures is not worth per-site churn"
)]
/// Converts a tonic gRPC Status to our Error type.
///
/// This function attempts to parse Hyper's structured error details from the
/// gRPC status. If that fails, it falls back to parsing XML error format,
/// and finally to using the raw gRPC error message.
pub(super) fn from_grpc_status(status: Status) -> Error {
    // First, try to parse structured error details (ErrorInfo proto)
    if let Some(error_info) = parse_error_info(&status) {
        return Error::new_with_details(
            grpc_code_to_error_kind(status.code()),
            error_info.message,
            error_info.detail,
            error_info.hint,
            error_info.sqlstate,
        );
    }

    // Fall back to parsing XML error format from the message
    if let Some(error) = parse_xml_error(status.message()) {
        return error;
    }

    // Last resort: use the raw gRPC error message
    Error::new(grpc_code_to_error_kind(status.code()), status.message())
}

/// Attempts to parse `ErrorInfo` from the gRPC status details.
fn parse_error_info(status: &Status) -> Option<GrpcError> {
    // The error details are in the status metadata as a serialized google.rpc.Status
    // containing salesforce.hyperdb.grpc.v1.ErrorInfo
    //
    // For now, we'll implement a simplified version that extracts from the status
    // details bytes. A full implementation would use prost to decode the Any types.

    // Try to decode the details
    let details = status.details();
    if details.is_empty() {
        return None;
    }

    // Try to parse as google.rpc.Status containing ErrorInfo
    // This is a simplified implementation - we look for known field patterns
    parse_error_info_from_bytes(details)
}

/// Parses `ErrorInfo` from raw bytes.
///
/// This is a simplified parser that looks for the `ErrorInfo` fields in the
/// serialized protobuf data.
fn parse_error_info_from_bytes(data: &[u8]) -> Option<GrpcError> {
    // Try to decode using prost
    use prost::Message;

    // The details are wrapped in google.rpc.Status
    // which contains a repeated Any field with ErrorInfo
    #[derive(Clone, PartialEq, Message)]
    struct GoogleRpcStatus {
        #[prost(int32, tag = "1")]
        code: i32,
        #[prost(string, tag = "2")]
        message: String,
        #[prost(message, repeated, tag = "3")]
        details: Vec<prost_types::Any>,
    }

    if let Ok(rpc_status) = GoogleRpcStatus::decode(data) {
        for detail in rpc_status.details {
            // Check if this is an ErrorInfo
            if detail
                .type_url
                .ends_with("salesforce.hyperdb.grpc.v1.ErrorInfo")
            {
                // Try to decode the ErrorInfo
                if let Some(error_info) = decode_error_info(&detail.value) {
                    return Some(error_info);
                }
            }
        }
    }

    None
}

/// Decodes `ErrorInfo` from its serialized form.
fn decode_error_info(data: &[u8]) -> Option<GrpcError> {
    use prost::Message;

    // ErrorInfo proto fields:
    // 1: primary_message (string)
    // 2: sqlstate (string)
    // 3: customer_hint (string)
    // 4: customer_detail (string)
    // 5: system_detail (string)
    // 6: position (message)
    // 7: error_source (string)
    #[derive(Clone, PartialEq, Message)]
    struct ErrorInfo {
        #[prost(string, tag = "1")]
        primary_message: String,
        #[prost(string, tag = "2")]
        sqlstate: String,
        #[prost(string, tag = "3")]
        customer_hint: String,
        #[prost(string, tag = "4")]
        customer_detail: String,
        #[prost(string, tag = "5")]
        system_detail: String,
        // Skipping position (tag 6) for now
        #[prost(string, tag = "7")]
        error_source: String,
    }

    if let Ok(info) = ErrorInfo::decode(data) {
        // Build error message combining primary_message and customer_detail
        let message = if info.customer_detail.is_empty() {
            info.primary_message.clone()
        } else {
            format!("{}: {}", info.primary_message, info.customer_detail)
        };

        return Some(GrpcError {
            sqlstate: if info.sqlstate.is_empty() {
                None
            } else {
                Some(info.sqlstate)
            },
            message,
            detail: if info.customer_detail.is_empty() {
                None
            } else {
                Some(info.customer_detail)
            },
            hint: if info.customer_hint.is_empty() {
                None
            } else {
                Some(info.customer_hint)
            },
            error_source: if info.error_source.is_empty() {
                None
            } else {
                Some(info.error_source)
            },
        });
    }

    None
}

/// Parses XML-format error message (legacy Hyper error format).
///
/// Example: `<sqlstate>42703</sqlstate><primary>column not found</primary><detail>...</detail>`
fn parse_xml_error(message: &str) -> Option<Error> {
    // Quick check if this looks like XML
    if !message.contains("<sqlstate>") && !message.contains("<primary>") {
        return None;
    }

    let sqlstate = extract_xml_tag(message, "sqlstate");
    let primary = extract_xml_tag(message, "primary");
    let detail = extract_xml_tag(message, "detail");
    let hint = extract_xml_tag(message, "hint");

    // Build the error message
    let error_message = match (&primary, &detail) {
        (Some(p), Some(d)) => format!("{p}: {d}"),
        (Some(p), None) => p.clone(),
        (None, Some(d)) => d.clone(),
        (None, None) => message.to_string(),
    };

    // Determine error kind from SQLSTATE
    let kind = sqlstate
        .as_ref()
        .map_or(ErrorKind::Query, |s| sqlstate_to_error_kind(s));

    Some(Error::new_with_details(
        kind,
        error_message,
        detail,
        hint,
        sqlstate,
    ))
}

/// Extracts content from an XML tag like `<tag>content</tag>`.
fn extract_xml_tag(text: &str, tag: &str) -> Option<String> {
    let start_tag = format!("<{tag}>");
    let end_tag = format!("</{tag}>");

    let start = text.find(&start_tag)? + start_tag.len();
    let end = text[start..].find(&end_tag)? + start;

    Some(text[start..end].to_string())
}

/// Converts gRPC status code to `ErrorKind`.
fn grpc_code_to_error_kind(code: tonic::Code) -> ErrorKind {
    match code {
        tonic::Code::Ok => ErrorKind::Other, // Shouldn't happen for errors
        tonic::Code::Cancelled => ErrorKind::Cancelled,
        tonic::Code::Unknown => ErrorKind::Query,
        tonic::Code::InvalidArgument => ErrorKind::Query,
        tonic::Code::DeadlineExceeded => ErrorKind::Timeout,
        tonic::Code::NotFound => ErrorKind::Query,
        tonic::Code::AlreadyExists => ErrorKind::Query,
        tonic::Code::PermissionDenied => ErrorKind::Authentication,
        tonic::Code::ResourceExhausted => ErrorKind::Query,
        tonic::Code::FailedPrecondition => ErrorKind::Query,
        tonic::Code::Aborted => ErrorKind::Query,
        tonic::Code::OutOfRange => ErrorKind::Query,
        tonic::Code::Unimplemented => ErrorKind::FeatureNotSupported,
        tonic::Code::Internal => ErrorKind::Query,
        tonic::Code::Unavailable => ErrorKind::Connection,
        tonic::Code::DataLoss => ErrorKind::Query,
        tonic::Code::Unauthenticated => ErrorKind::Authentication,
    }
}

/// Converts SQLSTATE code to `ErrorKind`.
fn sqlstate_to_error_kind(sqlstate: &str) -> ErrorKind {
    match sqlstate {
        // Query canceled
        "57014" => ErrorKind::Cancelled,
        // Authentication errors (28xxx)
        s if s.starts_with("28") => ErrorKind::Authentication,
        // Connection errors (08xxx)
        s if s.starts_with("08") => ErrorKind::Connection,
        // Feature not supported (0A000)
        "0A000" => ErrorKind::FeatureNotSupported,
        // Everything else is a query error
        _ => ErrorKind::Query,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_parse_xml_error() {
        let msg = "<sqlstate>42703</sqlstate><primary>column not found</primary><detail>column \"foo\" does not exist</detail>";
        let error = parse_xml_error(msg).unwrap();
        assert!(error.to_string().contains("column not found"));
    }

    #[test]
    fn test_extract_xml_tag() {
        assert_eq!(
            extract_xml_tag("<foo>bar</foo>", "foo"),
            Some("bar".to_string())
        );
        assert_eq!(
            extract_xml_tag("<a>1</a><b>2</b>", "b"),
            Some("2".to_string())
        );
        assert_eq!(extract_xml_tag("<a>1</a>", "c"), None);
    }

    #[test]
    fn test_grpc_code_mapping() {
        assert!(matches!(
            grpc_code_to_error_kind(tonic::Code::Cancelled),
            ErrorKind::Cancelled
        ));
        assert!(matches!(
            grpc_code_to_error_kind(tonic::Code::Unauthenticated),
            ErrorKind::Authentication
        ));
        assert!(matches!(
            grpc_code_to_error_kind(tonic::Code::Unavailable),
            ErrorKind::Connection
        ));
    }
}
