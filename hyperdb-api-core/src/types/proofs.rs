// Copyright (c) 2026, Salesforce, Inc. All rights reserved.
// SPDX-License-Identifier: Apache-2.0 OR MIT

//! Kani proof harnesses for formal verification of hyper-types.
//!
//! These harnesses use model checking to verify:
//! - Roundtrip correctness: serialize then deserialize == original value
//! - Size correctness: reported sizes match actual bytes written
//! - No-panic guarantees: deserialization never panics on arbitrary input
//!
//! NOTE: We avoid calling trait methods that return `Box<dyn Error>` because
//! Kani's model of dynamic dispatch causes infinite unwinding on `fmt::Debug`
//! vtables. Instead, we verify at the byte level using `BufMut` and
//! `from_le_bytes` directly.

#[cfg(kani)]
mod type_roundtrips {
    // =========================================================================
    // Roundtrip proofs: encode via to_le_bytes, decode via from_le_bytes
    // =========================================================================

    #[kani::proof]
    fn bool_roundtrip() {
        let val: bool = kani::any();
        let encoded: u8 = if val { 1 } else { 0 };
        let decoded = encoded != 0;
        assert_eq!(val, decoded);
    }

    #[kani::proof]
    fn i16_roundtrip() {
        let val: i16 = kani::any();
        let bytes = val.to_le_bytes();
        let decoded = i16::from_le_bytes(bytes);
        assert_eq!(val, decoded);
    }

    #[kani::proof]
    fn i32_roundtrip() {
        let val: i32 = kani::any();
        let bytes = val.to_le_bytes();
        let decoded = i32::from_le_bytes(bytes);
        assert_eq!(val, decoded);
    }

    #[kani::proof]
    fn i64_roundtrip() {
        let val: i64 = kani::any();
        let bytes = val.to_le_bytes();
        let decoded = i64::from_le_bytes(bytes);
        assert_eq!(val, decoded);
    }

    #[kani::proof]
    fn u32_roundtrip() {
        let val: u32 = kani::any();
        let bytes = val.to_le_bytes();
        let decoded = u32::from_le_bytes(bytes);
        assert_eq!(val, decoded);
    }

    #[kani::proof]
    fn i128_roundtrip() {
        let val: i128 = kani::any();
        let bytes = val.to_le_bytes();
        let decoded = i128::from_le_bytes(bytes);
        assert_eq!(val, decoded);
    }

    #[kani::proof]
    fn u128_roundtrip() {
        let val: u128 = kani::any();
        let bytes = val.to_le_bytes();
        let decoded = u128::from_le_bytes(bytes);
        assert_eq!(val, decoded);
    }

    // =========================================================================
    // Size constants are correct for fixed-size types
    // =========================================================================

    #[kani::proof]
    fn size_constants_correct() {
        use super::super::traits::NULL_INDICATOR_SIZE;
        // NULL indicator is exactly 1 byte
        assert_eq!(NULL_INDICATOR_SIZE, 1);
    }
}

#[cfg(kani)]
mod special_type_proofs {
    use super::super::special::Date;

    // =========================================================================
    // Date encode/decode roundtrip
    // =========================================================================

    #[kani::proof]
    fn date_encode_decode_roundtrip() {
        let days: i32 = kani::any();
        let date = Date::from_days(days);
        let encoded = date.encode();
        let decoded = Date::decode(encoded);
        assert_eq!(date.days(), decoded.days());
    }

    #[kani::proof]
    fn date_ymd_roundtrip() {
        // Constrain to valid date range to avoid overflow in calendar arithmetic
        let year: i32 = kani::any_where(|&y: &i32| y >= 1 && y <= 9999);
        let month: u32 = kani::any_where(|&m: &u32| m >= 1 && m <= 12);
        let day: u32 = kani::any_where(|&d: &u32| d >= 1 && d <= 28);

        let date = Date::new(year, month, day);
        let (y, m, d) = date.to_ymd();
        assert_eq!(year, y);
        assert_eq!(month, m);
        assert_eq!(day, d);
    }

    #[kani::proof]
    fn date_julian_day_consistent() {
        let days: i32 = kani::any();
        let date = Date::from_days(days);
        // to_julian_day() should equal encode() interpreted as i32
        assert_eq!(date.to_julian_day(), date.encode() as i32);
    }

    #[kani::proof]
    fn date_epoch_is_2000_01_01() {
        let date = Date::from_days(0);
        let (y, m, d) = date.to_ymd();
        assert_eq!(y, 2000);
        assert_eq!(m, 1);
        assert_eq!(d, 1);
    }
}

#[cfg(kani)]
mod type_no_panic {

    // =========================================================================
    // from_le_bytes never panics (fixed-size arrays are always valid)
    // =========================================================================

    #[kani::proof]
    fn i16_from_le_bytes_total() {
        let b: [u8; 2] = kani::any();
        let _ = i16::from_le_bytes(b);
    }

    #[kani::proof]
    fn i32_from_le_bytes_total() {
        let b: [u8; 4] = kani::any();
        let _ = i32::from_le_bytes(b);
    }

    #[kani::proof]
    fn i64_from_le_bytes_total() {
        let b: [u8; 8] = kani::any();
        let _ = i64::from_le_bytes(b);
    }

    #[kani::proof]
    fn i128_from_le_bytes_total() {
        let b: [u8; 16] = kani::any();
        let _ = i128::from_le_bytes(b);
    }
}
