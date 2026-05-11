// Copyright (c) 2026, Salesforce, Inc. All rights reserved.
// SPDX-License-Identifier: Apache-2.0 OR MIT

#![allow(
    clippy::cast_precision_loss,
    reason = "diagnostic query-stats rate calculations; >2^53 rows or bytes is unreachable"
)]

use napi_derive::napi;

// =============================================================================
// QueryStats napi objects — flat JS-friendly structs
// =============================================================================

/// Pre-execution phase statistics (parsing and compilation).
#[napi(object)]
#[derive(Debug, Clone)]
pub struct JsPreExecutionStats {
    /// Time spent parsing the SQL (seconds).
    pub parsing_time_s: Option<f64>,
    /// Time spent compiling the query plan (seconds).
    pub compilation_time_s: Option<f64>,
    /// Total pre-execution elapsed time (seconds).
    pub elapsed_s: Option<f64>,
    /// Peak transaction memory during pre-execution (MB).
    pub peak_memory_mb: Option<f64>,
}

/// Execution phase statistics (runtime performance).
#[napi(object)]
#[derive(Debug, Clone)]
pub struct JsExecutionStats {
    /// Total execution elapsed time (seconds).
    pub elapsed_s: Option<f64>,
    /// CPU time consumed (seconds).
    pub cpu_time_s: Option<f64>,
    /// Total thread time (seconds).
    pub thread_time_s: Option<f64>,
    /// Time spent waiting (seconds).
    pub wait_time_s: Option<f64>,
    /// Total rows processed.
    pub processed_rows_total: Option<f64>,
    /// Rows processed from native storage.
    pub processed_rows_native: Option<f64>,
    /// Time spent on storage access (seconds).
    pub storage_access_time_s: Option<f64>,
    /// Number of storage access operations.
    pub storage_access_count: Option<f64>,
    /// Bytes read from storage.
    pub storage_access_bytes: Option<f64>,
    /// Peak transaction memory during execution (MB).
    pub peak_memory_mb: Option<f64>,
}

/// Detailed statistics for a single query execution.
#[napi(object)]
#[derive(Debug, Clone)]
pub struct JsQueryStats {
    /// Total elapsed wall-clock time (seconds).
    pub elapsed_s: f64,
    /// Time spent committing (seconds).
    pub commit_time_s: Option<f64>,
    /// Time waiting to be scheduled (seconds).
    pub time_to_schedule_s: Option<f64>,
    /// Pre-execution phase stats.
    pub pre_execution: Option<JsPreExecutionStats>,
    /// Execution phase stats.
    pub execution: Option<JsExecutionStats>,
    /// Result size (MB).
    pub result_size_mb: Option<f64>,
    /// Peak result buffer memory (MB).
    pub peak_result_buffer_memory_mb: Option<f64>,
    /// Plan cache status.
    pub plan_cache_status: Option<String>,
    /// Plan cache hit count.
    pub plan_cache_hit_count: Option<f64>,
    /// Statement type.
    pub statement_type: Option<String>,
    /// Result row count.
    pub rows: Option<f64>,
    /// Result column count.
    pub cols: Option<f64>,
    /// Truncated query text from Hyper log.
    pub query_truncated: Option<String>,
}

// =============================================================================
// Conversion from hyperdb-api types
// =============================================================================

impl From<hyperdb_api::QueryStats> for JsQueryStats {
    fn from(s: hyperdb_api::QueryStats) -> Self {
        JsQueryStats {
            elapsed_s: s.elapsed_s,
            commit_time_s: s.commit_time_s,
            time_to_schedule_s: s.time_to_schedule_s,
            pre_execution: s.pre_execution.map(|p| JsPreExecutionStats {
                parsing_time_s: p.parsing_time_s,
                compilation_time_s: p.compilation_time_s,
                elapsed_s: p.elapsed_s,
                peak_memory_mb: p.peak_memory_mb,
            }),
            execution: s.execution.map(|e| JsExecutionStats {
                elapsed_s: e.elapsed_s,
                cpu_time_s: e.cpu_time_s,
                thread_time_s: e.thread_time_s,
                wait_time_s: e.wait_time_s,
                processed_rows_total: e.processed_rows_total.map(|v| v as f64),
                processed_rows_native: e.processed_rows_native.map(|v| v as f64),
                storage_access_time_s: e.storage_access_time_s,
                storage_access_count: e.storage_access_count.map(|v| v as f64),
                storage_access_bytes: e.storage_access_bytes.map(|v| v as f64),
                peak_memory_mb: e.peak_memory_mb,
            }),
            result_size_mb: s.result_size_mb,
            peak_result_buffer_memory_mb: s.peak_result_buffer_memory_mb,
            plan_cache_status: s.plan_cache_status,
            plan_cache_hit_count: s.plan_cache_hit_count.map(f64::from),
            statement_type: s.statement_type,
            rows: s.rows.map(|v| v as f64),
            cols: s.cols.map(f64::from),
            query_truncated: s.query_truncated,
        }
    }
}
