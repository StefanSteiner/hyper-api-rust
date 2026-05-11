// Copyright (c) 2026, Salesforce, Inc. All rights reserved.
// SPDX-License-Identifier: Apache-2.0 OR MIT

//! Notice handling for PostgreSQL/Hyper server messages.

use crate::protocol::message::backend::NoticeResponseBody;
use tracing::trace;

/// A notice or warning message from the server.
///
/// Notices are informational messages that don't indicate failure but may
/// contain useful information about query execution, deprecation warnings,
/// or performance hints.
#[derive(Debug, Clone)]
pub struct Notice {
    severity: Option<String>,
    code: Option<String>,
    message: String,
    detail: Option<String>,
    hint: Option<String>,
    position: Option<i32>,
}

impl Notice {
    /// Returns the severity level (e.g., "WARNING", "NOTICE").
    #[inline]
    #[must_use]
    pub fn severity(&self) -> Option<&str> {
        self.severity.as_deref()
    }

    /// Returns the SQLSTATE error code.
    #[inline]
    #[must_use]
    pub fn code(&self) -> Option<&str> {
        self.code.as_deref()
    }

    /// Returns the primary message text.
    #[inline]
    #[must_use]
    pub fn message(&self) -> &str {
        &self.message
    }

    /// Returns additional detail about the notice.
    #[inline]
    #[must_use]
    pub fn detail(&self) -> Option<&str> {
        self.detail.as_deref()
    }

    /// Returns a hint for resolving the issue.
    #[inline]
    #[must_use]
    pub fn hint(&self) -> Option<&str> {
        self.hint.as_deref()
    }

    /// Returns the position in the query where the notice was raised.
    #[inline]
    #[must_use]
    pub fn position(&self) -> Option<i32> {
        self.position
    }
}

impl Notice {
    /// Parses a Notice from a `NoticeResponseBody`.
    pub(crate) fn from_response_body(body: &NoticeResponseBody) -> Self {
        let mut severity = None;
        let mut code = None;
        let mut message = String::new();
        let mut detail = None;
        let mut hint = None;
        let mut position = None;

        for field in body.fields().filter_map(|r| {
            r.map_err(|e| trace!(target: "hyperdb_api_core::client", error = %e, "dropped error parsing notice field")).ok()
        }) {
            match (field.type_(), field.value()) {
                    (b'S', Ok(v)) => severity = Some(v.to_string()),
                    (b'V', Ok(v)) if severity.is_none() => severity = Some(v.to_string()),
                    (b'C', Ok(v)) => code = Some(v.to_string()),
                    (b'M', Ok(v)) => message = v.to_string(),
                    (b'D', Ok(v)) => detail = Some(v.to_string()),
                    (b'H', Ok(v)) => hint = Some(v.to_string()),
                    (b'P', Ok(v)) => position = v.parse().ok(),
                    _ => {}
                }
        }

        Notice {
            severity,
            code,
            message,
            detail,
            hint,
            position,
        }
    }

    /// Returns true if this is a warning-level notice.
    #[inline]
    #[must_use]
    pub fn is_warning(&self) -> bool {
        matches!(self.severity(), Some("WARNING" | "WARN"))
    }

    /// Returns true if this is an informational notice.
    #[inline]
    #[must_use]
    pub fn is_info(&self) -> bool {
        matches!(self.severity(), Some("NOTICE" | "INFO" | "LOG" | "DEBUG"))
    }
}

impl std::fmt::Display for Notice {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        if let Some(sev) = self.severity() {
            write!(f, "{sev}: ")?;
        }
        write!(f, "{}", self.message())?;
        if let Some(code) = self.code() {
            write!(f, " ({code})")?;
        }
        Ok(())
    }
}

/// Type alias for a notice receiver callback.
///
/// The callback receives a `Notice` and is called whenever the server sends
/// a notice or warning message during query execution.
///
/// # Thread Safety
///
/// The callback must be `Send + Sync` because it may be called from
/// different threads during concurrent query execution.
pub type NoticeReceiver = Box<dyn Fn(Notice) + Send + Sync>;

#[cfg(test)]
mod tests {
    use super::*;

    fn make_notice(severity: Option<&str>, code: Option<&str>, message: &str) -> Notice {
        Notice {
            severity: severity.map(String::from),
            code: code.map(String::from),
            message: message.to_string(),
            detail: None,
            hint: None,
            position: None,
        }
    }

    #[test]
    fn test_notice_display() {
        let notice = make_notice(Some("WARNING"), Some("01000"), "test warning");
        assert_eq!(format!("{notice}"), "WARNING: test warning (01000)");
    }

    #[test]
    fn test_notice_accessors() {
        let notice = make_notice(Some("WARNING"), Some("01000"), "test");
        assert_eq!(notice.severity(), Some("WARNING"));
        assert_eq!(notice.code(), Some("01000"));
        assert_eq!(notice.message(), "test");
    }

    #[test]
    fn test_notice_is_warning() {
        let warning = make_notice(Some("WARNING"), None, "");
        assert!(warning.is_warning());

        let info = make_notice(Some("INFO"), None, "");
        assert!(!info.is_warning());
        assert!(info.is_info());
    }
}
