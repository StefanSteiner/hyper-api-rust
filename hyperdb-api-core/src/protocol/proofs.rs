// Copyright (c) 2026, Salesforce, Inc. All rights reserved.
// SPDX-License-Identifier: Apache-2.0 OR MIT

//! Kani proof harnesses for formal verification of hyper-protocol.
//!
//! These harnesses verify:
//! - SQL escaping helper properties (is_valid_unquoted_identifier)
//! - COPY format read functions never panic on arbitrary input
//! - COPY read functions return correct errors for short buffers
//! - Protocol type roundtrip correctness (via concrete ParseError)
//!
//! NOTE: Harnesses that use BytesMut (write functions) or format! (SQL escaping
//! Display impls) are excluded because Kani's solver cannot handle the
//! complexity of the `bytes` crate internals and dynamic formatting.

#[cfg(kani)]
mod escape_proofs {
    use super::super::escape::is_valid_unquoted_identifier;

    #[kani::proof]
    fn empty_string_not_valid_identifier() {
        assert!(!is_valid_unquoted_identifier(""));
    }

    /// Verifies that any single ASCII letter or underscore is a valid identifier start.
    #[kani::proof]
    fn single_letter_is_valid_identifier() {
        let c: u8 = kani::any_where(|&b: &u8| b.is_ascii_alphabetic());
        let s = [c];
        let input = core::str::from_utf8(&s).unwrap();
        assert!(is_valid_unquoted_identifier(input));
    }

    #[kani::proof]
    fn underscore_is_valid_identifier() {
        assert!(is_valid_unquoted_identifier("_"));
    }

    /// A string starting with a digit is never a valid unquoted identifier.
    #[kani::proof]
    fn digit_start_is_invalid() {
        let d: u8 = kani::any_where(|&b: &u8| b >= b'0' && b <= b'9');
        let s = [d];
        let input = core::str::from_utf8(&s).unwrap();
        assert!(!is_valid_unquoted_identifier(input));
    }

    /// Strings containing spaces/hyphens are never valid unquoted identifiers.
    #[kani::proof]
    fn space_makes_invalid() {
        assert!(!is_valid_unquoted_identifier("a b"));
    }

    #[kani::proof]
    fn hyphen_makes_invalid() {
        assert!(!is_valid_unquoted_identifier("a-b"));
    }
}

#[cfg(kani)]
mod copy_proofs {
    use super::super::copy::*;

    // =========================================================================
    // Read functions never panic on arbitrary input
    // =========================================================================

    #[kani::proof]
    fn read_i16_no_panic() {
        let len: usize = kani::any_where(|&l: &usize| l <= 8);
        let mut buf = [0u8; 8];
        for i in 0..8 {
            if i < len {
                buf[i] = kani::any();
            }
        }
        let _ = read_i16(&buf[..len]);
    }

    #[kani::proof]
    fn read_i32_no_panic() {
        let len: usize = kani::any_where(|&l: &usize| l <= 8);
        let mut buf = [0u8; 8];
        for i in 0..8 {
            if i < len {
                buf[i] = kani::any();
            }
        }
        let _ = read_i32(&buf[..len]);
    }

    #[kani::proof]
    fn read_i64_no_panic() {
        let len: usize = kani::any_where(|&l: &usize| l <= 16);
        let mut buf = [0u8; 16];
        for i in 0..16 {
            if i < len {
                buf[i] = kani::any();
            }
        }
        let _ = read_i64(&buf[..len]);
    }

    #[kani::proof]
    fn read_data128_no_panic() {
        let len: usize = kani::any_where(|&l: &usize| l <= 20);
        let mut buf = [0u8; 20];
        for i in 0..20 {
            if i < len {
                buf[i] = kani::any();
            }
        }
        let _ = read_data128(&buf[..len]);
    }

    #[kani::proof]
    fn read_varbinary_no_panic() {
        let len: usize = kani::any_where(|&l: &usize| l <= 16);
        let mut buf = [0u8; 16];
        for i in 0..16 {
            if i < len {
                buf[i] = kani::any();
            }
        }
        let _ = read_varbinary(&buf[..len]);
    }

    // =========================================================================
    // Read roundtrip: manually construct LE bytes, verify read decodes correctly
    // =========================================================================

    #[kani::proof]
    fn read_i16_roundtrip() {
        let val: i16 = kani::any();
        let buf = val.to_le_bytes();
        let decoded = read_i16(&buf).unwrap();
        assert_eq!(val, decoded);
    }

    #[kani::proof]
    fn read_i32_roundtrip() {
        let val: i32 = kani::any();
        let buf = val.to_le_bytes();
        let decoded = read_i32(&buf).unwrap();
        assert_eq!(val, decoded);
    }

    #[kani::proof]
    fn read_i64_roundtrip() {
        let val: i64 = kani::any();
        let buf = val.to_le_bytes();
        let decoded = read_i64(&buf).unwrap();
        assert_eq!(val, decoded);
    }

    #[kani::proof]
    fn read_data128_roundtrip() {
        let val: [u8; 16] = kani::any();
        let decoded = read_data128(&val).unwrap();
        assert_eq!(val, decoded);
    }

    // =========================================================================
    // Read functions return correct errors for short buffers
    // =========================================================================

    #[kani::proof]
    fn read_i16_short_buffer_is_err() {
        let len: usize = kani::any_where(|&l: &usize| l < 2);
        let buf = [0u8; 1];
        assert!(read_i16(&buf[..len]).is_err());
    }

    #[kani::proof]
    fn read_i32_short_buffer_is_err() {
        let len: usize = kani::any_where(|&l: &usize| l < 4);
        let mut buf = [0u8; 3];
        for i in 0..3 {
            if i < len {
                buf[i] = kani::any();
            }
        }
        assert!(read_i32(&buf[..len]).is_err());
    }

    #[kani::proof]
    fn read_i64_short_buffer_is_err() {
        let len: usize = kani::any_where(|&l: &usize| l < 8);
        let mut buf = [0u8; 7];
        for i in 0..7 {
            if i < len {
                buf[i] = kani::any();
            }
        }
        assert!(read_i64(&buf[..len]).is_err());
    }

    // =========================================================================
    // Header constants consistency
    // =========================================================================

    #[kani::proof]
    fn header_size_matches_constant() {
        assert_eq!(HYPER_BINARY_HEADER.len(), HYPER_BINARY_HEADER_SIZE);
    }

    #[kani::proof]
    fn header_starts_with_signature() {
        assert!(HYPER_BINARY_HEADER.starts_with(HYPER_BINARY_SIGNATURE));
    }
}

#[cfg(kani)]
mod types_proofs {
    use super::super::types::*;

    // =========================================================================
    // No-panic on arbitrary input (concrete ParseError, no Box<dyn Error>)
    // =========================================================================

    #[kani::proof]
    fn bool_from_hyper_binary_no_panic() {
        let len: usize = kani::any_where(|&l: &usize| l <= 4);
        let mut buf = [0u8; 4];
        for i in 0..4 {
            if i < len {
                buf[i] = kani::any();
            }
        }
        let _ = bool_from_hyper_binary(&buf[..len]);
    }

    #[kani::proof]
    fn i16_from_hyper_binary_no_panic() {
        let len: usize = kani::any_where(|&l: &usize| l <= 4);
        let mut buf = [0u8; 4];
        for i in 0..4 {
            if i < len {
                buf[i] = kani::any();
            }
        }
        let _ = i16_from_hyper_binary(&buf[..len]);
    }

    #[kani::proof]
    fn i32_from_hyper_binary_no_panic() {
        let len: usize = kani::any_where(|&l: &usize| l <= 8);
        let mut buf = [0u8; 8];
        for i in 0..8 {
            if i < len {
                buf[i] = kani::any();
            }
        }
        let _ = i32_from_hyper_binary(&buf[..len]);
    }

    #[kani::proof]
    fn i64_from_hyper_binary_no_panic() {
        let len: usize = kani::any_where(|&l: &usize| l <= 16);
        let mut buf = [0u8; 16];
        for i in 0..16 {
            if i < len {
                buf[i] = kani::any();
            }
        }
        let _ = i64_from_hyper_binary(&buf[..len]);
    }

    #[kani::proof]
    fn f32_from_hyper_binary_no_panic() {
        let len: usize = kani::any_where(|&l: &usize| l <= 8);
        let mut buf = [0u8; 8];
        for i in 0..8 {
            if i < len {
                buf[i] = kani::any();
            }
        }
        let _ = f32_from_hyper_binary(&buf[..len]);
    }

    #[kani::proof]
    fn f64_from_hyper_binary_no_panic() {
        let len: usize = kani::any_where(|&l: &usize| l <= 16);
        let mut buf = [0u8; 16];
        for i in 0..16 {
            if i < len {
                buf[i] = kani::any();
            }
        }
        let _ = f64_from_hyper_binary(&buf[..len]);
    }

    // =========================================================================
    // Roundtrip via manually-constructed LE bytes (no BytesMut)
    // =========================================================================

    #[kani::proof]
    fn bool_roundtrip_via_protocol() {
        let val: bool = kani::any();
        let buf = [if val { 1u8 } else { 0u8 }];
        let decoded = bool_from_hyper_binary(&buf).unwrap();
        assert_eq!(val, decoded);
    }

    #[kani::proof]
    fn i16_roundtrip_via_protocol() {
        let val: i16 = kani::any();
        let buf = val.to_le_bytes();
        let decoded = i16_from_hyper_binary(&buf).unwrap();
        assert_eq!(val, decoded);
    }

    #[kani::proof]
    fn i32_roundtrip_via_protocol() {
        let val: i32 = kani::any();
        let buf = val.to_le_bytes();
        let decoded = i32_from_hyper_binary(&buf).unwrap();
        assert_eq!(val, decoded);
    }

    #[kani::proof]
    fn i64_roundtrip_via_protocol() {
        let val: i64 = kani::any();
        let buf = val.to_le_bytes();
        let decoded = i64_from_hyper_binary(&buf).unwrap();
        assert_eq!(val, decoded);
    }

    // =========================================================================
    // Wrong-size buffers return InvalidLength error
    // =========================================================================

    #[kani::proof]
    fn bool_wrong_size_is_err() {
        let len: usize = kani::any_where(|&l: &usize| l != 1 && l <= 4);
        let buf = [0u8; 4];
        assert!(bool_from_hyper_binary(&buf[..len]).is_err());
    }

    #[kani::proof]
    fn i16_wrong_size_is_err() {
        let len: usize = kani::any_where(|&l: &usize| l != 2 && l <= 4);
        let buf = [0u8; 4];
        assert!(i16_from_hyper_binary(&buf[..len]).is_err());
    }

    #[kani::proof]
    fn i32_wrong_size_is_err() {
        let len: usize = kani::any_where(|&l: &usize| l != 4 && l <= 8);
        let buf = [0u8; 8];
        assert!(i32_from_hyper_binary(&buf[..len]).is_err());
    }
}
