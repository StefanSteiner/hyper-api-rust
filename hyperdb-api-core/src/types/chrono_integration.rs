// Copyright (c) 2026, Salesforce, Inc. All rights reserved.
// SPDX-License-Identifier: Apache-2.0 OR MIT

//! Chrono integration for Hyper types.
//!
//! This module provides conversions between Hyper's date/time types
//! and the [`chrono`](https://docs.rs/chrono) crate's types.
//!
//! # Epoch Difference
//!
//! Hyper uses 2000-01-01 as its epoch, while Unix/chrono uses 1970-01-01.
//! All conversions handle this automatically.
//!
//! # Fallible Conversions
//!
//! Conversions from Hyper types to chrono types use [`TryFrom`] because
//! extreme date/time values may fall outside chrono's representable range.
//! Conversions from chrono to Hyper types use [`From`] since they are infallible.
//!
//! # Examples
//!
//! ```rust,no_run
//! // Marked `no_run` to dodge a Windows Defender heuristic that intermittently
//! // refuses to launch this specific compiled doctest binary with
//! // `ERROR_ACCESS_DENIED`. The same conversions are exercised by the
//! // `tests::test_date_roundtrip` / `test_timestamp_roundtrip` unit tests so
//! // coverage is preserved.
//! # {
//! use hyperdb_api_core::types::{Date, Time, Timestamp, OffsetTimestamp};
//! use chrono::{Datelike, NaiveDate, NaiveTime, NaiveDateTime, Utc};
//!
//! // Date conversion
//! let hyper_date = Date::new(2024, 6, 15);
//! let chrono_date: NaiveDate = hyper_date.try_into().unwrap();
//! assert_eq!(chrono_date, NaiveDate::from_ymd_opt(2024, 6, 15).unwrap());
//!
//! let back: Date = chrono_date.into();
//! assert_eq!(back, hyper_date);
//!
//! // Timestamp conversion
//! let hyper_ts = Timestamp::new(Date::new(2024, 1, 1), Time::new(12, 0, 0, 0));
//! let chrono_ts: NaiveDateTime = hyper_ts.try_into().unwrap();
//! assert_eq!(chrono_ts.year(), 2024);
//! # }
//! ```

use std::fmt;

use chrono::{DateTime, NaiveDate, NaiveDateTime, NaiveTime, Utc};

use super::{Date, OffsetTimestamp, Time, Timestamp};

/// Error returned when a Hyper date/time value cannot be represented as a chrono type.
#[derive(Debug, Clone)]
pub struct ChronoConversionError {
    message: String,
}

impl fmt::Display for ChronoConversionError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "chrono conversion error: {}", self.message)
    }
}

impl std::error::Error for ChronoConversionError {}

// =============================================================================
// Date <-> chrono::NaiveDate
// =============================================================================

impl TryFrom<Date> for NaiveDate {
    type Error = ChronoConversionError;

    fn try_from(d: Date) -> std::result::Result<Self, Self::Error> {
        let (y, m, day) = d.to_ymd();
        NaiveDate::from_ymd_opt(y, m, day).ok_or_else(|| ChronoConversionError {
            message: format!("date {y}-{m}-{day} is out of range for chrono"),
        })
    }
}

impl From<NaiveDate> for Date {
    fn from(d: NaiveDate) -> Self {
        use chrono::Datelike;
        Date::new(d.year(), d.month(), d.day())
    }
}

// =============================================================================
// Time <-> chrono::NaiveTime
// =============================================================================

impl TryFrom<Time> for NaiveTime {
    type Error = ChronoConversionError;

    fn try_from(t: Time) -> std::result::Result<Self, Self::Error> {
        let (h, m, s, us) = t.to_hms_micro();
        NaiveTime::from_hms_micro_opt(h, m, s, us).ok_or_else(|| ChronoConversionError {
            message: format!("time {h}:{m}:{s}.{us} is out of range for chrono"),
        })
    }
}

impl From<NaiveTime> for Time {
    fn from(t: NaiveTime) -> Self {
        use chrono::Timelike;
        let micros = u64::from(t.hour()) * Time::MICROS_PER_HOUR
            + u64::from(t.minute()) * Time::MICROS_PER_MINUTE
            + u64::from(t.second()) * Time::MICROS_PER_SECOND
            + u64::from(t.nanosecond() / 1000);
        Time::from_microseconds(micros)
    }
}

// =============================================================================
// Timestamp <-> chrono::NaiveDateTime
// =============================================================================

/// Microseconds between Unix epoch (1970-01-01) and Hyper epoch (2000-01-01).
///
/// Derivation: 1970-01-01 to 2000-01-01 spans 30 years with 7 leap years
/// (1972, 1976, 1980, 1984, 1988, 1992, 1996).
///   days  = 23 × 365 + 7 × 366 = 10_957
///   secs  = 10_957 × 86_400   = 946_684_800
///   µs    = 946_684_800 × 1_000_000 = 946_684_800_000_000
const UNIX_TO_HYPER_EPOCH_MICROS: i64 = 946_684_800_000_000;

impl TryFrom<Timestamp> for NaiveDateTime {
    type Error = ChronoConversionError;

    fn try_from(ts: Timestamp) -> std::result::Result<Self, Self::Error> {
        let hyper_micros = ts.microseconds();
        let unix_micros = hyper_micros.saturating_add(UNIX_TO_HYPER_EPOCH_MICROS);
        let secs = unix_micros.div_euclid(1_000_000);
        // `rem_euclid(1_000_000)` is in `0..1_000_000`; multiplied by 1000 is
        // `0..1_000_000_000`, which fits in u32 (max 4_294_967_295).
        let nanos = u32::try_from(unix_micros.rem_euclid(1_000_000) * 1000)
            .expect("rem_euclid(1_000_000) * 1000 is bounded by 1e9, fits in u32");
        DateTime::from_timestamp(secs, nanos)
            .map(|dt| dt.naive_utc())
            .ok_or_else(|| {
                let (date, time) = ts.to_date_time();
                ChronoConversionError {
                    message: format!(
                        "timestamp {date} {time} ({hyper_micros} Hyper-epoch µs, {unix_micros} Unix µs) is out of range for chrono"
                    ),
                }
            })
    }
}

impl From<NaiveDateTime> for Timestamp {
    fn from(dt: NaiveDateTime) -> Self {
        let unix_micros = dt.and_utc().timestamp_micros();
        Timestamp::from_microseconds(unix_micros - UNIX_TO_HYPER_EPOCH_MICROS)
    }
}

// =============================================================================
// OffsetTimestamp <-> chrono::DateTime<Utc>
// =============================================================================

impl TryFrom<OffsetTimestamp> for DateTime<Utc> {
    type Error = ChronoConversionError;

    fn try_from(ts: OffsetTimestamp) -> std::result::Result<Self, Self::Error> {
        let hyper_micros = ts.timestamp().microseconds();
        let unix_micros = hyper_micros.saturating_add(UNIX_TO_HYPER_EPOCH_MICROS);
        let secs = unix_micros.div_euclid(1_000_000);
        // Bounded by `1_000_000 * 1000 = 1e9`, fits in u32. See NaiveDateTime impl above.
        let nanos = u32::try_from(unix_micros.rem_euclid(1_000_000) * 1000)
            .expect("rem_euclid(1_000_000) * 1000 is bounded by 1e9, fits in u32");
        DateTime::from_timestamp(secs, nanos).ok_or_else(|| {
            let (date, time) = ts.timestamp().to_date_time();
            ChronoConversionError {
                message: format!(
                    "offset timestamp {} {} UTC{:+} ({} Hyper-epoch µs, {} Unix µs) \
                     is out of range for chrono",
                    date,
                    time,
                    ts.offset_minutes(),
                    hyper_micros,
                    unix_micros
                ),
            }
        })
    }
}

impl From<DateTime<Utc>> for OffsetTimestamp {
    fn from(dt: DateTime<Utc>) -> Self {
        let unix_micros = dt.timestamp_micros();
        let hyper_micros = unix_micros - UNIX_TO_HYPER_EPOCH_MICROS;
        let ts = Timestamp::from_microseconds(hyper_micros);
        OffsetTimestamp::new(ts, 0)
    }
}

// =============================================================================
// OffsetTimestamp <-> chrono::DateTime<chrono::FixedOffset>
// =============================================================================

impl TryFrom<OffsetTimestamp> for DateTime<chrono::FixedOffset> {
    type Error = ChronoConversionError;

    fn try_from(ts: OffsetTimestamp) -> std::result::Result<Self, Self::Error> {
        let utc: DateTime<Utc> = ts.try_into()?;
        let offset = chrono::FixedOffset::east_opt(i32::from(ts.offset_minutes()) * 60)
            .ok_or_else(|| ChronoConversionError {
                message: format!(
                    "UTC offset of {} minutes is out of range for chrono",
                    ts.offset_minutes()
                ),
            })?;
        Ok(utc.with_timezone(&offset))
    }
}

impl From<DateTime<chrono::FixedOffset>> for OffsetTimestamp {
    fn from(dt: DateTime<chrono::FixedOffset>) -> Self {
        let unix_micros = dt.timestamp_micros();
        let hyper_micros = unix_micros - UNIX_TO_HYPER_EPOCH_MICROS;
        let ts = Timestamp::from_microseconds(hyper_micros);
        let offset_secs = dt.offset().local_minus_utc();
        // chrono's FixedOffset accepts `-86_399..=86_399` seconds, so `offset_secs / 60`
        // is bounded by `±1440`, which fits comfortably in i16.
        let offset_minutes = i16::try_from(offset_secs / 60)
            .expect("chrono FixedOffset bounds offset to ±86_399s, so /60 fits in i16");
        OffsetTimestamp::new(ts, offset_minutes)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::{Datelike, Timelike};

    #[test]
    fn test_date_roundtrip() {
        let hyper = Date::new(2024, 6, 15);
        let chrono: NaiveDate = hyper.try_into().unwrap();
        assert_eq!(chrono.year(), 2024);
        assert_eq!(chrono.month(), 6);
        assert_eq!(chrono.day(), 15);
        let back: Date = chrono.into();
        assert_eq!(back, hyper);
    }

    #[test]
    fn test_date_epoch() {
        let epoch = Date::from_days(0);
        let chrono: NaiveDate = epoch.try_into().unwrap();
        assert_eq!(chrono, NaiveDate::from_ymd_opt(2000, 1, 1).unwrap());
    }

    #[test]
    fn test_date_before_epoch() {
        let d = Date::new(1999, 12, 31);
        let chrono: NaiveDate = d.try_into().unwrap();
        assert_eq!(chrono, NaiveDate::from_ymd_opt(1999, 12, 31).unwrap());
        let back: Date = chrono.into();
        assert_eq!(back, d);
    }

    #[test]
    fn test_time_roundtrip() {
        let hyper = Time::new(14, 30, 45, 123456);
        let chrono: NaiveTime = hyper.try_into().unwrap();
        assert_eq!(chrono.hour(), 14);
        assert_eq!(chrono.minute(), 30);
        assert_eq!(chrono.second(), 45);
        // chrono stores nanoseconds, so microseconds * 1000
        assert_eq!(chrono.nanosecond(), 123456000);
        let back: Time = chrono.into();
        assert_eq!(back, hyper);
    }

    #[test]
    fn test_time_midnight() {
        let midnight = Time::new(0, 0, 0, 0);
        let chrono: NaiveTime = midnight.try_into().unwrap();
        assert_eq!(chrono, NaiveTime::from_hms_opt(0, 0, 0).unwrap());
    }

    #[test]
    fn test_timestamp_roundtrip() {
        let hyper = Timestamp::new(Date::new(2024, 3, 14), Time::new(15, 9, 26, 535897));
        let chrono: NaiveDateTime = hyper.try_into().unwrap();
        assert_eq!(chrono.year(), 2024);
        assert_eq!(chrono.month(), 3);
        assert_eq!(chrono.day(), 14);
        assert_eq!(chrono.hour(), 15);
        assert_eq!(chrono.minute(), 9);
        assert_eq!(chrono.second(), 26);
        let back: Timestamp = chrono.into();
        assert_eq!(back, hyper);
    }

    #[test]
    fn test_timestamp_epoch() {
        let epoch = Timestamp::from_microseconds(0);
        let chrono: NaiveDateTime = epoch.try_into().unwrap();
        assert_eq!(
            chrono,
            NaiveDateTime::new(
                NaiveDate::from_ymd_opt(2000, 1, 1).unwrap(),
                NaiveTime::from_hms_opt(0, 0, 0).unwrap()
            )
        );
    }

    #[test]
    fn test_offset_timestamp_utc_roundtrip() {
        let ts = Timestamp::new(Date::new(2024, 6, 15), Time::new(10, 30, 0, 0));
        let hyper = OffsetTimestamp::new(ts, 0);
        let chrono: DateTime<Utc> = hyper.try_into().unwrap();
        assert_eq!(chrono.year(), 2024);
        assert_eq!(chrono.month(), 6);
        assert_eq!(chrono.day(), 15);
        assert_eq!(chrono.hour(), 10);
        let back: OffsetTimestamp = chrono.into();
        assert_eq!(back.timestamp(), hyper.timestamp());
    }

    #[test]
    fn test_offset_timestamp_fixed_offset() {
        let ts = Timestamp::new(Date::new(2024, 6, 15), Time::new(10, 30, 0, 0));
        let hyper = OffsetTimestamp::new(ts, 120); // +02:00
        let chrono: DateTime<chrono::FixedOffset> = hyper.try_into().unwrap();
        assert_eq!(chrono.offset().local_minus_utc(), 7200); // 120 minutes = 7200 seconds
                                                             // Local time should be 12:30 (+02:00 from 10:30 UTC)
        assert_eq!(chrono.hour(), 12);
        assert_eq!(chrono.minute(), 30);
    }

    // =========================================================================
    // Epoch constant verification and edge cases
    // =========================================================================

    #[test]
    fn test_epoch_constant_matches_chrono() {
        // Verify UNIX_TO_HYPER_EPOCH_MICROS by computing it independently via chrono.
        let unix_epoch = NaiveDate::from_ymd_opt(1970, 1, 1)
            .unwrap()
            .and_hms_opt(0, 0, 0)
            .unwrap();
        let hyper_epoch = NaiveDate::from_ymd_opt(2000, 1, 1)
            .unwrap()
            .and_hms_opt(0, 0, 0)
            .unwrap();
        let diff = hyper_epoch
            .signed_duration_since(unix_epoch)
            .num_microseconds()
            .unwrap();
        assert_eq!(diff, UNIX_TO_HYPER_EPOCH_MICROS);
    }

    #[test]
    fn test_timestamp_leap_year_feb29() {
        // 2000 is a leap year (divisible by 400)
        let hyper = Timestamp::new(Date::new(2000, 2, 29), Time::new(23, 59, 59, 999999));
        let chrono: NaiveDateTime = hyper.try_into().unwrap();
        assert_eq!(chrono.year(), 2000);
        assert_eq!(chrono.month(), 2);
        assert_eq!(chrono.day(), 29);
        assert_eq!(chrono.hour(), 23);
        assert_eq!(chrono.minute(), 59);
        assert_eq!(chrono.second(), 59);
        let back: Timestamp = chrono.into();
        assert_eq!(back, hyper);
    }

    #[test]
    fn test_timestamp_leap_year_2024_feb29() {
        let hyper = Timestamp::new(Date::new(2024, 2, 29), Time::new(12, 0, 0, 0));
        let chrono: NaiveDateTime = hyper.try_into().unwrap();
        assert_eq!(chrono.month(), 2);
        assert_eq!(chrono.day(), 29);
        let back: Timestamp = chrono.into();
        assert_eq!(back, hyper);
    }

    #[test]
    fn test_timestamp_before_hyper_epoch() {
        // 1999-12-31 23:59:59 — negative Hyper microseconds
        let hyper = Timestamp::new(Date::new(1999, 12, 31), Time::new(23, 59, 59, 0));
        assert!(hyper.microseconds() < 0);
        let chrono: NaiveDateTime = hyper.try_into().unwrap();
        assert_eq!(chrono.year(), 1999);
        assert_eq!(chrono.month(), 12);
        assert_eq!(chrono.day(), 31);
        let back: Timestamp = chrono.into();
        assert_eq!(back, hyper);
    }

    #[test]
    fn test_timestamp_before_unix_epoch() {
        // 1969-07-20 20:17:40 — before Unix epoch (Apollo 11 landing)
        let hyper = Timestamp::new(Date::new(1969, 7, 20), Time::new(20, 17, 40, 0));
        let chrono: NaiveDateTime = hyper.try_into().unwrap();
        assert_eq!(chrono.year(), 1969);
        assert_eq!(chrono.month(), 7);
        assert_eq!(chrono.day(), 20);
        let back: Timestamp = chrono.into();
        assert_eq!(back, hyper);
    }

    #[test]
    fn test_timestamp_far_future() {
        // 9999-12-31 — maximum reasonable date
        let hyper = Timestamp::new(Date::new(9999, 12, 31), Time::new(23, 59, 59, 999999));
        let chrono: NaiveDateTime = hyper.try_into().unwrap();
        assert_eq!(chrono.year(), 9999);
        let back: Timestamp = chrono.into();
        assert_eq!(back, hyper);
    }

    #[test]
    fn test_timestamp_error_includes_date_time() {
        // Construct a timestamp so extreme that chrono cannot represent it
        let extreme = Timestamp::from_microseconds(i64::MAX);
        let err = NaiveDateTime::try_from(extreme).unwrap_err();
        let msg = err.to_string();
        // Error should contain both the decoded date/time AND the microsecond values
        assert!(msg.contains("Hyper-epoch µs"), "missing Hyper µs in: {msg}");
        assert!(msg.contains("Unix µs"), "missing Unix µs in: {msg}");
    }

    #[test]
    fn test_offset_timestamp_error_includes_context() {
        let extreme = Timestamp::from_microseconds(i64::MAX);
        let ots = OffsetTimestamp::new(extreme, 60);
        let err = DateTime::<Utc>::try_from(ots).unwrap_err();
        let msg = err.to_string();
        assert!(msg.contains("Hyper-epoch µs"), "missing Hyper µs in: {msg}");
        assert!(msg.contains("UTC"), "missing offset in: {msg}");
    }

    #[test]
    fn test_date_roundtrip_century_boundary() {
        // Non-leap century year (1900 is NOT a leap year)
        let d1900 = Date::new(1900, 3, 1);
        let chrono: NaiveDate = d1900.try_into().unwrap();
        assert_eq!(chrono, NaiveDate::from_ymd_opt(1900, 3, 1).unwrap());
        let back: Date = chrono.into();
        assert_eq!(back, d1900);

        // Leap century year (2000 IS a leap year — divisible by 400)
        let d2000 = Date::new(2000, 2, 29);
        let chrono: NaiveDate = d2000.try_into().unwrap();
        assert_eq!(chrono, NaiveDate::from_ymd_opt(2000, 2, 29).unwrap());
        let back: Date = chrono.into();
        assert_eq!(back, d2000);
    }
}
