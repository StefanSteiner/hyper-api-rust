// Copyright (c) 2026, Salesforce, Inc. All rights reserved.
// SPDX-License-Identifier: Apache-2.0 OR MIT

//! Backend (server-to-client) messages.
//!
//! This module parses messages sent from the Hyper server to the client.
//! Each message variant is discriminated by a single-byte tag; see the
//! `*_TAG` constants and the [`Message`] enum for the full catalog.
//!
//! # Authentication Flow
//!
//! The server initiates authentication with an `Authentication*` message.
//! Supported methods:
//!
//! | Method | Tag subtype | Flow |
//! |---|---|---|
//! | Trust | `R(0)` | Server sends `AuthenticationOk` immediately |
//! | Cleartext | `R(3)` | Server requests password; client sends `PasswordMessage` |
//! | MD5 | `R(5)` + 4-byte salt | Client hashes `md5(md5(password + user) + salt)` |
//! | SCRAM-SHA-256 | `R(10)` | Multi-step challenge-response via `SASLInitialResponse` / `SASLResponse` |
//!
//! After successful authentication the server sends `AuthenticationOk`,
//! followed by `ParameterStatus` messages, `BackendKeyData`, and finally
//! `ReadyForQuery`.
//!
//! # Attribution
//!
//! Portions of this module were adapted from
//! [`postgres-protocol`](https://github.com/sfackler/rust-postgres)'s
//! `message/backend.rs` (Copyright (c) 2016 Steven Fackler, MIT or
//! Apache-2.0). Adapted material includes the message-tag constants
//! (`PARSE_COMPLETE_TAG = b'1'`, `BIND_COMPLETE_TAG = b'2'`, ...,
//! `AUTHENTICATION_TAG = b'R'`, etc.), the `Header` struct shape, and
//! message-framing logic. Hyper-specific changes added on top include the
//! HyperBinary COPY format and Hyper-specific message variants. See the
//! `NOTICE` file at the repo root for the full upstream copyright and
//! reproduced license text.

use byteorder::{BigEndian, ByteOrder, ReadBytesExt};
use bytes::{Bytes, BytesMut};
use memchr::memchr;
use std::io::{self, Read};
use std::ops::Range;
use std::str;

use crate::types::Oid;

/// Message tag for `ParseComplete` ('1').
pub const PARSE_COMPLETE_TAG: u8 = b'1';
/// Message tag for `BindComplete` ('2').
pub const BIND_COMPLETE_TAG: u8 = b'2';
/// Message tag for `CloseComplete` ('3').
pub const CLOSE_COMPLETE_TAG: u8 = b'3';
/// Message tag for `NotificationResponse` ('A').
pub const NOTIFICATION_RESPONSE_TAG: u8 = b'A';
/// Message tag for `CopyDone` ('c').
pub const COPY_DONE_TAG: u8 = b'c';
/// Message tag for `CommandComplete` ('C').
pub const COMMAND_COMPLETE_TAG: u8 = b'C';
/// Message tag for `CopyData` ('d').
pub const COPY_DATA_TAG: u8 = b'd';
/// Message tag for `DataRow` ('D').
pub const DATA_ROW_TAG: u8 = b'D';
/// Message tag for `ErrorResponse` ('E').
pub const ERROR_RESPONSE_TAG: u8 = b'E';
/// Message tag for `CopyInResponse` ('G').
pub const COPY_IN_RESPONSE_TAG: u8 = b'G';
/// Message tag for `CopyOutResponse` ('H').
pub const COPY_OUT_RESPONSE_TAG: u8 = b'H';
/// Message tag for `EmptyQueryResponse` ('I').
pub const EMPTY_QUERY_RESPONSE_TAG: u8 = b'I';
/// Message tag for `BackendKeyData` ('K').
pub const BACKEND_KEY_DATA_TAG: u8 = b'K';
/// Message tag for `NoData` ('n').
pub const NO_DATA_TAG: u8 = b'n';
/// Message tag for `NoticeResponse` ('N').
pub const NOTICE_RESPONSE_TAG: u8 = b'N';
/// Message tag for Authentication ('R').
pub const AUTHENTICATION_TAG: u8 = b'R';
/// Message tag for `PortalSuspended` ('s').
pub const PORTAL_SUSPENDED_TAG: u8 = b's';
/// Message tag for `ParameterStatus` ('S').
pub const PARAMETER_STATUS_TAG: u8 = b'S';
/// Message tag for `ParameterDescription` ('t').
pub const PARAMETER_DESCRIPTION_TAG: u8 = b't';
/// Message tag for `RowDescription` ('T').
pub const ROW_DESCRIPTION_TAG: u8 = b'T';
/// Message tag for `ReadyForQuery` ('Z').
pub const READY_FOR_QUERY_TAG: u8 = b'Z';
/// Message header information.
///
/// `PostgreSQL` wire protocol messages start with a 5-byte header:
/// - 1 byte: message type tag
/// - 4 bytes: message length (`BigEndian`, including the length field itself)
#[derive(Debug, Copy, Clone)]
pub struct Header {
    /// Message type tag (e.g., 'R' for Authentication, 'T' for `RowDescription`).
    tag: u8,
    /// Total message length in bytes, including the 4-byte length field itself.
    len: i32,
}

impl Header {
    /// Parses a message header from a buffer.
    ///
    /// Returns `Ok(None)` if the buffer is too short (< 5 bytes).
    /// Returns `Err` if the parsed length is invalid (< 4).
    ///
    /// # Arguments
    ///
    /// * `buf` - Buffer containing at least the first 5 bytes of a message
    ///
    /// # Errors
    ///
    /// Returns [`io::ErrorKind::InvalidData`] if the parsed length field
    /// is less than 4 (which is the minimum valid length — the 4-byte
    /// length field itself).
    #[inline]
    pub fn parse(buf: &[u8]) -> io::Result<Option<Header>> {
        if buf.len() < 5 {
            return Ok(None);
        }

        let tag = buf[0];
        let len = BigEndian::read_i32(&buf[1..]);

        if len < 4 {
            return Err(io::Error::new(
                io::ErrorKind::InvalidData,
                "invalid message length: header length < 4",
            ));
        }

        Ok(Some(Header { tag, len }))
    }

    /// Returns the message tag.
    #[inline]
    #[must_use]
    pub fn tag(self) -> u8 {
        self.tag
    }

    /// Returns the message length (including itself).
    #[inline]
    #[must_use]
    pub fn len(self) -> i32 {
        self.len
    }

    /// Returns true if the message length is 0.
    ///
    /// Note: A valid message always has a length of at least 4 (for the length field itself),
    /// so this should always return false for valid messages.
    #[inline]
    #[must_use]
    pub fn is_empty(self) -> bool {
        self.len == 0
    }
}

/// An enum representing backend messages from Hyper server.
#[non_exhaustive]
#[derive(Debug)]
pub enum Message {
    /// Authentication request completed successfully.
    AuthenticationOk,
    /// Authentication via cleartext password.
    AuthenticationCleartextPassword,
    /// Authentication via MD5 password.
    AuthenticationMd5Password(AuthenticationMd5PasswordBody),
    /// Authentication via SASL.
    AuthenticationSasl(AuthenticationSaslBody),
    /// SASL authentication continue.
    AuthenticationSaslContinue(AuthenticationSaslContinueBody),
    /// SASL authentication final.
    AuthenticationSaslFinal(AuthenticationSaslFinalBody),
    /// Backend key data for cancel requests.
    BackendKeyData(BackendKeyDataBody),
    /// Bind operation complete.
    BindComplete,
    /// Close operation complete.
    CloseComplete,
    /// Command completed.
    CommandComplete(CommandCompleteBody),
    /// COPY data.
    CopyData(CopyDataBody),
    /// COPY operation done.
    CopyDone,
    /// COPY IN response (client should send data).
    CopyInResponse(CopyInResponseBody),
    /// COPY OUT response (server will send data).
    CopyOutResponse(CopyOutResponseBody),
    /// A row of data.
    DataRow(DataRowBody),
    /// Empty query response.
    EmptyQueryResponse,
    /// Error response.
    ErrorResponse(ErrorResponseBody),
    /// No data (for empty result sets).
    NoData,
    /// Notice response (warning).
    NoticeResponse(NoticeResponseBody),
    /// Notification from LISTEN/NOTIFY.
    NotificationResponse(NotificationResponseBody),
    /// Parameter description for prepared statement.
    ParameterDescription(ParameterDescriptionBody),
    /// Parameter status update.
    ParameterStatus(ParameterStatusBody),
    /// Parse operation complete.
    ParseComplete,
    /// Portal suspended (for partial results).
    PortalSuspended,
    /// Ready for query.
    ReadyForQuery(ReadyForQueryBody),
    /// Row description (column metadata).
    RowDescription(RowDescriptionBody),
}

impl Message {
    /// Parses a backend message from a buffer.
    ///
    /// Reads the message header (5 bytes: tag + length) and parses the complete
    /// message body. Returns `Ok(None)` if the buffer doesn't contain a complete
    /// message yet. The buffer is advanced when a complete message is parsed.
    ///
    /// # Arguments
    ///
    /// * `buf` - Buffer containing message data (may be partial)
    ///
    /// # Errors
    ///
    /// - Returns [`io::ErrorKind::InvalidInput`] if the declared length
    ///   field is less than 4 (below the minimum valid message size).
    /// - Returns an I/O error from the inner `Buffer` reads when a body
    ///   field is truncated, a C-string is not NUL-terminated, or a
    ///   string field contains invalid UTF-8.
    /// - Returns [`io::ErrorKind::InvalidInput`] for unexpected
    ///   authentication sub-tags or unknown top-level message tags.
    ///
    /// # Panics
    ///
    /// Does not panic in practice. The `read_u32` call on `&buf[1..5]`
    /// is proven infallible by the preceding `buf.len() < 5` short-circuit.
    #[inline]
    pub fn parse(buf: &mut BytesMut) -> io::Result<Option<Message>> {
        if buf.len() < 5 {
            let to_read = 5 - buf.len();
            buf.reserve(to_read);
            return Ok(None);
        }

        let tag = buf[0];
        let len = (&buf[1..5]).read_u32::<BigEndian>().unwrap();

        if len < 4 {
            return Err(io::Error::new(
                io::ErrorKind::InvalidInput,
                "invalid message length: parsing u32",
            ));
        }

        // Defensively use checked arithmetic. `len` is u32 from the wire — a
        // hostile or buggy server could send u32::MAX, which on 32-bit platforms
        // wraps usize and on 64-bit produces an oversized allocation.
        let Some(total_len) = (len as usize).checked_add(1) else {
            return Err(io::Error::new(
                io::ErrorKind::InvalidInput,
                format!("invalid message length: {len} + 1 overflows usize"),
            ));
        };
        if buf.len() < total_len {
            let to_read = total_len - buf.len();
            buf.reserve(to_read);
            return Ok(None);
        }

        let mut buf = Buffer {
            bytes: buf.split_to(total_len).freeze(),
            idx: 5,
        };

        let message = match tag {
            PARSE_COMPLETE_TAG => Message::ParseComplete,
            BIND_COMPLETE_TAG => Message::BindComplete,
            CLOSE_COMPLETE_TAG => Message::CloseComplete,
            NOTIFICATION_RESPONSE_TAG => {
                let process_id = buf.read_i32::<BigEndian>()?;
                let channel = buf.read_cstr()?;
                let message = buf.read_cstr()?;
                Message::NotificationResponse(NotificationResponseBody {
                    process_id,
                    channel,
                    message,
                })
            }
            COPY_DONE_TAG => Message::CopyDone,
            COMMAND_COMPLETE_TAG => {
                let tag = buf.read_cstr()?;
                Message::CommandComplete(CommandCompleteBody { tag })
            }
            COPY_DATA_TAG => {
                let storage = buf.read_all();
                Message::CopyData(CopyDataBody { storage })
            }
            DATA_ROW_TAG => {
                let len = buf.read_u16::<BigEndian>()?;
                let storage = buf.read_all();
                Message::DataRow(DataRowBody { storage, len })
            }
            ERROR_RESPONSE_TAG => {
                let storage = buf.read_all();
                Message::ErrorResponse(ErrorResponseBody { storage })
            }
            COPY_IN_RESPONSE_TAG => {
                let format = buf.read_u8()?;
                let len = buf.read_u16::<BigEndian>()?;
                let storage = buf.read_all();
                Message::CopyInResponse(CopyInResponseBody {
                    format,
                    len,
                    storage,
                })
            }
            COPY_OUT_RESPONSE_TAG => {
                let format = buf.read_u8()?;
                let len = buf.read_u16::<BigEndian>()?;
                let storage = buf.read_all();
                Message::CopyOutResponse(CopyOutResponseBody {
                    format,
                    len,
                    storage,
                })
            }
            EMPTY_QUERY_RESPONSE_TAG => Message::EmptyQueryResponse,
            BACKEND_KEY_DATA_TAG => {
                let process_id = buf.read_i32::<BigEndian>()?;
                let secret_key = buf.read_i32::<BigEndian>()?;
                Message::BackendKeyData(BackendKeyDataBody {
                    process_id,
                    secret_key,
                })
            }
            NO_DATA_TAG => Message::NoData,
            NOTICE_RESPONSE_TAG => {
                let storage = buf.read_all();
                Message::NoticeResponse(NoticeResponseBody { storage })
            }
            AUTHENTICATION_TAG => match buf.read_i32::<BigEndian>()? {
                0 => Message::AuthenticationOk,
                3 => Message::AuthenticationCleartextPassword,
                5 => {
                    let mut salt = [0; 4];
                    buf.read_exact(&mut salt)?;
                    Message::AuthenticationMd5Password(AuthenticationMd5PasswordBody { salt })
                }
                10 => {
                    let storage = buf.read_all();
                    Message::AuthenticationSasl(AuthenticationSaslBody(storage))
                }
                11 => {
                    let storage = buf.read_all();
                    Message::AuthenticationSaslContinue(AuthenticationSaslContinueBody(storage))
                }
                12 => {
                    let storage = buf.read_all();
                    Message::AuthenticationSaslFinal(AuthenticationSaslFinalBody(storage))
                }
                tag => {
                    return Err(io::Error::new(
                        io::ErrorKind::InvalidInput,
                        format!("unknown authentication tag `{tag}`"),
                    ));
                }
            },
            PORTAL_SUSPENDED_TAG => Message::PortalSuspended,
            PARAMETER_STATUS_TAG => {
                let name = buf.read_cstr()?;
                let value = buf.read_cstr()?;
                Message::ParameterStatus(ParameterStatusBody { name, value })
            }
            PARAMETER_DESCRIPTION_TAG => {
                let len = buf.read_u16::<BigEndian>()?;
                let storage = buf.read_all();
                Message::ParameterDescription(ParameterDescriptionBody { storage, len })
            }
            ROW_DESCRIPTION_TAG => {
                let len = buf.read_u16::<BigEndian>()?;
                let storage = buf.read_all();
                Message::RowDescription(RowDescriptionBody { storage, len })
            }
            READY_FOR_QUERY_TAG => {
                let status = buf.read_u8()?;
                Message::ReadyForQuery(ReadyForQueryBody { status })
            }
            tag => {
                return Err(io::Error::new(
                    io::ErrorKind::InvalidInput,
                    format!("unknown message tag `{tag}`"),
                ));
            }
        };

        if !buf.is_empty() {
            return Err(io::Error::new(
                io::ErrorKind::InvalidInput,
                "invalid message length: expected buffer to be empty",
            ));
        }

        Ok(Some(message))
    }
}

/// Internal buffer for parsing message bodies.
///
/// Maintains a position index to track parsing progress through the message bytes.
struct Buffer {
    /// The complete message bytes.
    bytes: Bytes,
    /// Current parsing position within the bytes.
    idx: usize,
}

impl Buffer {
    /// Returns a slice of the remaining unparsed bytes.
    #[inline]
    fn slice(&self) -> &[u8] {
        &self.bytes[self.idx..]
    }

    /// Returns true if all bytes have been consumed.
    #[inline]
    fn is_empty(&self) -> bool {
        self.slice().is_empty()
    }

    /// Reads a null-terminated C string from the buffer.
    ///
    /// Advances the position past the null terminator.
    /// Returns the bytes without the null terminator.
    #[inline]
    fn read_cstr(&mut self) -> io::Result<Bytes> {
        match memchr(0, self.slice()) {
            Some(pos) => {
                let start = self.idx;
                let end = start + pos;
                let cstr = self.bytes.slice(start..end);
                self.idx = end + 1;
                Ok(cstr)
            }
            None => Err(io::Error::new(
                io::ErrorKind::UnexpectedEof,
                "unexpected EOF",
            )),
        }
    }

    /// Reads all remaining bytes from the buffer.
    ///
    /// Advances the position to the end of the buffer.
    #[inline]
    fn read_all(&mut self) -> Bytes {
        let buf = self.bytes.slice(self.idx..);
        self.idx = self.bytes.len();
        buf
    }
}

impl Read for Buffer {
    #[inline]
    fn read(&mut self, buf: &mut [u8]) -> io::Result<usize> {
        let len = {
            let slice = self.slice();
            let len = std::cmp::min(slice.len(), buf.len());
            buf[..len].copy_from_slice(&slice[..len]);
            len
        };
        self.idx += len;
        Ok(len)
    }
}

// Message body types

/// MD5 password authentication body.
///
/// Contains the 4-byte salt used for MD5 password hashing.
/// The client must hash the password with this salt and send it back.
#[derive(Debug)]
pub struct AuthenticationMd5PasswordBody {
    /// 4-byte random salt for MD5 password hashing.
    salt: [u8; 4],
}

impl AuthenticationMd5PasswordBody {
    /// Returns the salt.
    #[inline]
    #[must_use]
    pub fn salt(&self) -> [u8; 4] {
        self.salt
    }
}

/// SASL authentication body.
///
/// Contains a list of supported SASL authentication mechanisms,
/// each as a null-terminated string. The list ends with a double null.
#[derive(Debug)]
pub struct AuthenticationSaslBody(Bytes);

impl AuthenticationSaslBody {
    /// Returns the raw data.
    #[inline]
    pub fn data(&self) -> &[u8] {
        &self.0
    }

    /// Returns an iterator over the available SASL mechanisms.
    #[inline]
    pub fn mechanisms(&self) -> SaslMechanisms<'_> {
        SaslMechanisms { buf: &self.0 }
    }
}

/// Iterator over SASL mechanism names in an authentication message.
///
/// Parses null-terminated strings from the SASL authentication body.
#[derive(Debug)]
pub struct SaslMechanisms<'a> {
    /// Buffer containing null-terminated mechanism names.
    buf: &'a [u8],
}

impl<'a> Iterator for SaslMechanisms<'a> {
    type Item = &'a str;

    fn next(&mut self) -> Option<Self::Item> {
        if self.buf.is_empty() || self.buf[0] == 0 {
            return None;
        }

        match memchr(0, self.buf) {
            Some(pos) => {
                let mechanism = str::from_utf8(&self.buf[..pos]).ok()?;
                self.buf = &self.buf[pos + 1..];
                Some(mechanism)
            }
            None => None,
        }
    }
}

/// SASL continue authentication body.
///
/// Contains server challenge data for the next step in SASL authentication.
/// The format depends on the specific SASL mechanism being used.
#[derive(Debug)]
pub struct AuthenticationSaslContinueBody(Bytes);

impl AuthenticationSaslContinueBody {
    /// Returns the raw data.
    #[inline]
    pub fn data(&self) -> &[u8] {
        &self.0
    }
}

/// SASL final authentication body.
///
/// Contains the final server response in SASL authentication.
/// Typically includes a server signature for verification.
#[derive(Debug)]
pub struct AuthenticationSaslFinalBody(Bytes);

impl AuthenticationSaslFinalBody {
    /// Returns the raw data.
    #[inline]
    pub fn data(&self) -> &[u8] {
        &self.0
    }
}

/// Backend key data body.
///
/// Contains information needed to cancel a running query.
/// The client can use these values to send a cancel request.
#[derive(Debug)]
pub struct BackendKeyDataBody {
    /// Process ID of the backend server process.
    process_id: i32,
    /// Secret key for authenticating cancel requests.
    secret_key: i32,
}

impl BackendKeyDataBody {
    /// Returns the process ID.
    #[inline]
    #[must_use]
    pub fn process_id(&self) -> i32 {
        self.process_id
    }

    /// Returns the secret key.
    #[inline]
    #[must_use]
    pub fn secret_key(&self) -> i32 {
        self.secret_key
    }
}

/// Command complete body.
///
/// Sent after a command (SELECT, INSERT, UPDATE, DELETE, etc.) completes.
/// The tag indicates what command was executed and may include row counts.
#[derive(Debug)]
pub struct CommandCompleteBody {
    /// Command tag (e.g., "SELECT 42", "INSERT 0 1", "UPDATE 5").
    tag: Bytes,
}

impl CommandCompleteBody {
    /// Returns the command tag.
    ///
    /// # Errors
    ///
    /// Returns [`io::ErrorKind::InvalidInput`] if the tag bytes are not
    /// valid UTF-8. The server sends ASCII command tags like `SELECT 42`,
    /// so this path is only reachable on protocol corruption.
    #[inline]
    pub fn tag(&self) -> io::Result<&str> {
        str::from_utf8(&self.tag).map_err(|e| io::Error::new(io::ErrorKind::InvalidInput, e))
    }
}

/// COPY data body.
///
/// Contains a chunk of COPY data during COPY IN or COPY OUT operations.
/// The format depends on the COPY format (text or binary).
#[derive(Debug)]
pub struct CopyDataBody {
    /// Raw COPY data bytes.
    storage: Bytes,
}

impl CopyDataBody {
    /// Returns the data.
    #[inline]
    pub fn data(&self) -> &[u8] {
        &self.storage
    }

    /// Consumes self and returns the bytes.
    #[inline]
    pub fn into_bytes(self) -> Bytes {
        self.storage
    }
}

/// COPY IN response body.
///
/// Sent by the server to indicate it's ready to receive COPY data.
/// The client should send `CopyData` messages until `CopyDone`.
#[derive(Debug)]
pub struct CopyInResponseBody {
    /// Overall format: 0 = text, 1 = binary (`HyperBinary`).
    format: u8,
    /// Number of columns in the COPY operation.
    len: u16,
    /// Kept for memory ownership of parsed data.
    #[expect(
        dead_code,
        reason = "owns the backing buffer referenced by `format`/`len`; must live as long as the message"
    )]
    storage: Bytes,
}

impl CopyInResponseBody {
    /// Returns the overall format (0 = text, 1 = binary).
    #[inline]
    pub fn format(&self) -> u8 {
        self.format
    }

    /// Returns the number of columns.
    #[inline]
    pub fn column_count(&self) -> u16 {
        self.len
    }
}

/// COPY OUT response body.
///
/// Sent by the server to indicate it will send COPY data.
/// The client should expect `CopyData` messages until `CopyDone`.
#[derive(Debug)]
pub struct CopyOutResponseBody {
    /// Overall format: 0 = text, 1 = binary (`HyperBinary`).
    format: u8,
    /// Number of columns in the COPY operation.
    len: u16,
    /// Kept for memory ownership of parsed data.
    #[expect(
        dead_code,
        reason = "owns the backing buffer referenced by `format`/`len`; must live as long as the message"
    )]
    storage: Bytes,
}

impl CopyOutResponseBody {
    /// Returns the overall format (0 = text, 1 = binary).
    #[inline]
    pub fn format(&self) -> u8 {
        self.format
    }

    /// Returns the number of columns.
    #[inline]
    pub fn column_count(&self) -> u16 {
        self.len
    }
}

/// Data row body.
///
/// Contains a single row of data from a query result.
/// Each column is prefixed with a 4-byte length (BigEndian):
/// - Positive length: non-NULL value of that length
/// - -1: NULL value
#[derive(Debug, Clone)]
pub struct DataRowBody {
    /// Raw row data bytes.
    storage: Bytes,
    /// Number of columns in this row.
    len: u16,
}

impl DataRowBody {
    /// Returns an iterator over the column ranges.
    #[inline]
    pub fn ranges(&self) -> DataRowRanges<'_> {
        DataRowRanges {
            buf: &self.storage,
            len: self.storage.len(),
            remaining: self.len,
        }
    }

    /// Returns the raw buffer.
    #[inline]
    pub fn buffer(&self) -> &[u8] {
        &self.storage
    }

    /// Returns the number of columns.
    #[inline]
    pub fn column_count(&self) -> u16 {
        self.len
    }

    /// Pre-computes offsets for **all** columns regardless of count.
    ///
    /// Computes offsets for all columns in this row.
    ///
    /// This provides O(1) access for any column index using dynamic allocation.
    /// Each entry is:
    /// - `Some((start, end))` for non-NULL values (byte range within the buffer)
    /// - `None` for NULL values or when parsing fails due to truncated data
    ///
    /// # Performance Tradeoffs
    ///
    /// This method allocates a `Vec` for all columns upfront. For tables with many
    /// columns where only a few are accessed, this may use more memory than needed.
    ///
    /// **When to use this method:**
    /// - Accessing multiple columns from the same row
    /// - Random access patterns where column indices vary
    /// - Bulk processing where setup cost is amortized
    ///
    /// **Alternative for sparse access:**
    /// If you only need one or two columns from a wide table, consider using
    /// `get_raw(index)` which computes offset on-demand (O(n) per call, but no
    /// allocation).
    ///
    /// # Memory Usage
    ///
    /// Allocates `column_count * size_of::<Option<(usize, usize)>>()` bytes.
    /// For a table with 100 columns, this is approximately 2.4 KB per row.
    ///
    /// # Panics
    ///
    /// Does not panic in practice. The `usize::try_from(len)` call is
    /// guarded by a `len >= 0` check, so the conversion from `i32` to
    /// `usize` is always infallible.
    #[inline]
    pub fn compute_all_offsets(&self) -> Vec<Option<(usize, usize)>> {
        let mut offsets = Vec::with_capacity(self.len as usize);

        let mut pos = 0usize;
        let buf = &self.storage[..];

        for _ in 0..self.len as usize {
            if pos + 4 > buf.len() {
                break;
            }
            let len = i32::from_be_bytes([buf[pos], buf[pos + 1], buf[pos + 2], buf[pos + 3]]);
            pos += 4;

            if len >= 0 {
                let len = usize::try_from(len).expect("len >= 0 checked above");
                let start = pos;
                let end = start.saturating_add(len);
                if end > buf.len() {
                    break;
                }
                offsets.push(Some((start, end)));
                pos = end;
            } else {
                offsets.push(None);
            }
        }

        offsets
    }

    /// Gets raw bytes for a column by index, computing offset on demand.
    /// Returns None if NULL or out of bounds.
    /// This avoids allocating a Vec of ranges.
    ///
    /// # Panics
    ///
    /// Does not panic in practice. The `usize::try_from(len)` calls are
    /// guarded by a `len >= 0` check in each branch, making the
    /// `i32 -> usize` conversion infallible.
    #[inline]
    pub fn get_column_bytes(&self, idx: usize) -> Option<&[u8]> {
        if idx >= self.len as usize {
            return None;
        }

        let mut buf = &self.storage[..];

        // Skip to the requested column
        for _ in 0..idx {
            if buf.len() < 4 {
                return None;
            }
            let len = i32::from_be_bytes([buf[0], buf[1], buf[2], buf[3]]);
            buf = &buf[4..];
            if len >= 0 {
                let len = usize::try_from(len).expect("len >= 0 checked above");
                if buf.len() < len {
                    return None;
                }
                buf = &buf[len..];
            }
        }

        // Read the target column
        if buf.len() < 4 {
            return None;
        }
        let len = i32::from_be_bytes([buf[0], buf[1], buf[2], buf[3]]);
        if len < 0 {
            None // NULL
        } else {
            let len = usize::try_from(len).expect("len >= 0 checked above");
            let data_start = 4;
            if buf.len() < data_start + len {
                return None;
            }
            Some(&buf[data_start..data_start + len])
        }
    }

    /// Checks if a column is NULL.
    ///
    /// # Panics
    ///
    /// Does not panic in practice. Each `usize::try_from(len)` is guarded
    /// by a `len >= 0` check, so the conversion from `i32` cannot fail.
    #[inline]
    pub fn is_column_null(&self, idx: usize) -> bool {
        if idx >= self.len as usize {
            return true;
        }

        let mut buf = &self.storage[..];

        // Skip to the requested column
        for _ in 0..idx {
            if buf.len() < 4 {
                return true;
            }
            let len = i32::from_be_bytes([buf[0], buf[1], buf[2], buf[3]]);
            buf = &buf[4..];
            if len >= 0 {
                let len = usize::try_from(len).expect("len >= 0 checked above");
                if buf.len() < len {
                    return true;
                }
                buf = &buf[len..];
            }
        }

        // Check the target column
        if buf.len() < 4 {
            return true;
        }
        let len = i32::from_be_bytes([buf[0], buf[1], buf[2], buf[3]]);
        len < 0
    }
}

/// Iterator over data row column ranges.
///
/// Yields byte ranges for each column in a `DataRow`.
/// Returns `None` for NULL columns, `Some(range)` for non-NULL columns.
#[derive(Debug)]
pub struct DataRowRanges<'a> {
    /// Buffer containing the row data.
    buf: &'a [u8],
    /// Original buffer length (for computing absolute offsets).
    len: usize,
    /// Number of columns remaining to parse.
    remaining: u16,
}

impl Iterator for DataRowRanges<'_> {
    type Item = io::Result<Option<Range<usize>>>;

    fn next(&mut self) -> Option<Self::Item> {
        if self.remaining == 0 {
            return if self.buf.is_empty() {
                None
            } else {
                Some(Err(io::Error::new(
                    io::ErrorKind::InvalidInput,
                    "invalid message length: extra data in data row",
                )))
            };
        }

        self.remaining -= 1;

        if self.buf.len() < 4 {
            return Some(Err(io::Error::new(
                io::ErrorKind::UnexpectedEof,
                "unexpected EOF reading column length",
            )));
        }

        let len = i32::from_be_bytes([self.buf[0], self.buf[1], self.buf[2], self.buf[3]]);
        self.buf = &self.buf[4..];

        if len < 0 {
            Some(Ok(None)) // NULL value
        } else {
            let len = usize::try_from(len).expect("len >= 0 in else branch");
            if self.buf.len() < len {
                return Some(Err(io::Error::new(
                    io::ErrorKind::UnexpectedEof,
                    "unexpected EOF",
                )));
            }
            let base = self.len - self.buf.len();
            self.buf = &self.buf[len..];
            Some(Ok(Some(base..base + len)))
        }
    }
}

/// Error response body.
///
/// Contains error information in the form of key-value pairs.
/// Each field has a type code (single byte) followed by a null-terminated value.
/// The message ends with a null byte (type code 0).
#[derive(Debug)]
pub struct ErrorResponseBody {
    /// Raw error message bytes.
    storage: Bytes,
}

impl ErrorResponseBody {
    /// Returns an iterator over error fields.
    #[inline]
    pub fn fields(&self) -> ErrorFields<'_> {
        ErrorFields { buf: &self.storage }
    }
}

/// Iterator over error fields in an `ErrorResponse` or `NoticeResponse`.
///
/// Each field consists of a type code byte followed by a null-terminated value string.
#[derive(Debug)]
pub struct ErrorFields<'a> {
    /// Buffer containing the error/notice fields.
    buf: &'a [u8],
}

impl<'a> Iterator for ErrorFields<'a> {
    type Item = io::Result<ErrorField<'a>>;

    fn next(&mut self) -> Option<Self::Item> {
        if self.buf.is_empty() {
            return None;
        }

        let type_ = self.buf[0];
        self.buf = &self.buf[1..];

        if type_ == 0 {
            return None;
        }

        match memchr(0, self.buf) {
            Some(pos) => {
                let value = &self.buf[..pos];
                self.buf = &self.buf[pos + 1..];
                Some(Ok(ErrorField { type_, value }))
            }
            None => Some(Err(io::Error::new(
                io::ErrorKind::UnexpectedEof,
                "unexpected EOF in error field",
            ))),
        }
    }
}

/// A single error or notice field.
///
/// Common field type codes:
/// - 'S': Severity
/// - 'C': Code (SQLSTATE)
/// - 'M': Message
/// - 'D': Detail
/// - 'H': Hint
/// - 'P': Position
/// - 'p': Internal position
/// - 'q': Internal query
/// - 'W': Where
/// - 's': Schema name
/// - 't': Table name
/// - 'c': Column name
/// - 'd': Data type name
/// - 'n': Constraint name
#[derive(Debug)]
pub struct ErrorField<'a> {
    /// Field type code (single byte).
    type_: u8,
    /// Field value (without null terminator).
    value: &'a [u8],
}

impl ErrorField<'_> {
    /// Returns the field type code.
    #[inline]
    #[must_use]
    pub fn type_(&self) -> u8 {
        self.type_
    }

    /// Returns the field value as bytes.
    #[inline]
    #[must_use]
    pub fn value_bytes(&self) -> &[u8] {
        self.value
    }

    /// Returns the field value as a string.
    ///
    /// # Errors
    ///
    /// Returns [`io::ErrorKind::InvalidInput`] if the field value is not
    /// valid UTF-8. Server-sent fields are expected to be UTF-8; this path
    /// is only reached on protocol corruption.
    #[inline]
    pub fn value(&self) -> io::Result<&str> {
        str::from_utf8(self.value).map_err(|e| io::Error::new(io::ErrorKind::InvalidInput, e))
    }
}

/// Notice response body (same format as error).
///
/// Contains warning or informational messages.
/// Uses the same field format as `ErrorResponse` but indicates a warning, not an error.
#[derive(Debug)]
pub struct NoticeResponseBody {
    /// Raw notice message bytes.
    storage: Bytes,
}

impl NoticeResponseBody {
    /// Returns an iterator over notice fields.
    #[inline]
    pub fn fields(&self) -> ErrorFields<'_> {
        ErrorFields { buf: &self.storage }
    }
}

/// Notification response body.
///
/// Sent when a NOTIFY event occurs that the client is listening to.
/// Used for asynchronous notifications between database sessions.
#[derive(Debug)]
pub struct NotificationResponseBody {
    /// Process ID of the backend that sent the notification.
    process_id: i32,
    /// Channel name (null-terminated).
    channel: Bytes,
    /// Notification payload (null-terminated).
    message: Bytes,
}

impl NotificationResponseBody {
    /// Returns the process ID.
    #[inline]
    pub fn process_id(&self) -> i32 {
        self.process_id
    }

    /// Returns the channel name.
    ///
    /// # Errors
    ///
    /// Returns [`io::ErrorKind::InvalidInput`] if the channel bytes are
    /// not valid UTF-8 (protocol corruption — server always sends UTF-8).
    #[inline]
    pub fn channel(&self) -> io::Result<&str> {
        str::from_utf8(&self.channel).map_err(|e| io::Error::new(io::ErrorKind::InvalidInput, e))
    }

    /// Returns the message.
    ///
    /// # Errors
    ///
    /// Returns [`io::ErrorKind::InvalidInput`] if the payload bytes are
    /// not valid UTF-8 (protocol corruption — server always sends UTF-8).
    #[inline]
    pub fn message(&self) -> io::Result<&str> {
        str::from_utf8(&self.message).map_err(|e| io::Error::new(io::ErrorKind::InvalidInput, e))
    }
}

/// Parameter description body.
///
/// Sent in response to a Parse message, describing the parameter types
/// expected by the prepared statement.
#[derive(Debug)]
pub struct ParameterDescriptionBody {
    /// Raw parameter OID bytes.
    storage: Bytes,
    /// Number of parameters.
    len: u16,
}

impl ParameterDescriptionBody {
    /// Returns an iterator over parameter OIDs.
    #[inline]
    pub fn parameters(&self) -> Parameters<'_> {
        Parameters {
            buf: &self.storage,
            remaining: self.len,
        }
    }
}

/// Iterator over parameter OIDs in a `ParameterDescription`.
///
/// Each parameter is represented by a 4-byte OID (`BigEndian`).
#[derive(Debug)]
pub struct Parameters<'a> {
    /// Buffer containing parameter OIDs.
    buf: &'a [u8],
    /// Number of parameters remaining to parse.
    remaining: u16,
}

impl Iterator for Parameters<'_> {
    type Item = io::Result<Oid>;

    fn next(&mut self) -> Option<Self::Item> {
        if self.remaining == 0 {
            return None;
        }

        self.remaining -= 1;

        if self.buf.len() < 4 {
            return Some(Err(io::Error::new(
                io::ErrorKind::UnexpectedEof,
                "unexpected EOF",
            )));
        }

        let oid = u32::from_be_bytes([self.buf[0], self.buf[1], self.buf[2], self.buf[3]]);
        self.buf = &self.buf[4..];
        Some(Ok(Oid::new(oid)))
    }
}

/// Parameter status body.
///
/// Sent by the server to inform the client about configuration parameter changes.
/// Common parameters include "`application_name`", "`server_version`", etc.
#[derive(Debug)]
pub struct ParameterStatusBody {
    /// Parameter name (null-terminated).
    name: Bytes,
    /// Parameter value (null-terminated).
    value: Bytes,
}

impl ParameterStatusBody {
    /// Returns the parameter name.
    ///
    /// # Errors
    ///
    /// Returns [`io::ErrorKind::InvalidInput`] if the parameter name bytes
    /// are not valid UTF-8 (protocol corruption).
    #[inline]
    pub fn name(&self) -> io::Result<&str> {
        str::from_utf8(&self.name).map_err(|e| io::Error::new(io::ErrorKind::InvalidInput, e))
    }

    /// Returns the parameter value.
    ///
    /// # Errors
    ///
    /// Returns [`io::ErrorKind::InvalidInput`] if the parameter value bytes
    /// are not valid UTF-8 (protocol corruption).
    #[inline]
    pub fn value(&self) -> io::Result<&str> {
        str::from_utf8(&self.value).map_err(|e| io::Error::new(io::ErrorKind::InvalidInput, e))
    }
}

/// Ready for query body.
///
/// Sent by the server to indicate it's ready to accept a new query.
/// The status indicates the current transaction state.
#[derive(Debug)]
pub struct ReadyForQueryBody {
    /// Transaction status: 'I' = idle, 'T' = in transaction, 'E' = in failed transaction.
    status: u8,
}

impl ReadyForQueryBody {
    /// Returns the transaction status.
    /// 'I' = idle, 'T' = in transaction, 'E' = in failed transaction
    #[inline]
    #[must_use]
    pub fn status(&self) -> u8 {
        self.status
    }
}

/// Row description body.
///
/// Sent in response to a Describe or Execute message, describing the columns
/// that will be returned by a query. Each field description includes name,
/// type OID, format, and other metadata.
#[derive(Debug)]
pub struct RowDescriptionBody {
    /// Raw field description bytes.
    storage: Bytes,
    /// Number of fields (columns).
    len: u16,
}

impl RowDescriptionBody {
    /// Returns an iterator over field descriptions.
    #[inline]
    pub fn fields(&self) -> Fields<'_> {
        Fields {
            buf: &self.storage,
            remaining: self.len,
        }
    }

    /// Returns the number of fields.
    #[inline]
    pub fn field_count(&self) -> u16 {
        self.len
    }
}

/// Iterator over field descriptions in a `RowDescription`.
///
/// Each field contains name, table OID, column ID, type OID, type size,
/// type modifier, and format code.
#[derive(Debug)]
pub struct Fields<'a> {
    /// Buffer containing field descriptions.
    buf: &'a [u8],
    /// Number of fields remaining to parse.
    remaining: u16,
}

impl<'a> Iterator for Fields<'a> {
    type Item = io::Result<Field<'a>>;

    fn next(&mut self) -> Option<Self::Item> {
        if self.remaining == 0 {
            return None;
        }

        self.remaining -= 1;

        // Parse field name (null-terminated string)
        let Some(name_end) = memchr(0, self.buf) else {
            return Some(Err(io::Error::new(
                io::ErrorKind::UnexpectedEof,
                "unexpected EOF in field name",
            )));
        };

        let name = match str::from_utf8(&self.buf[..name_end]) {
            Ok(s) => s,
            Err(e) => return Some(Err(io::Error::new(io::ErrorKind::InvalidInput, e))),
        };
        self.buf = &self.buf[name_end + 1..];

        // Parse fixed fields (18 bytes total)
        if self.buf.len() < 18 {
            return Some(Err(io::Error::new(
                io::ErrorKind::UnexpectedEof,
                "unexpected EOF in field description",
            )));
        }

        let table_oid = u32::from_be_bytes([self.buf[0], self.buf[1], self.buf[2], self.buf[3]]);
        let column_id = i16::from_be_bytes([self.buf[4], self.buf[5]]);
        let type_oid = u32::from_be_bytes([self.buf[6], self.buf[7], self.buf[8], self.buf[9]]);
        let type_size = i16::from_be_bytes([self.buf[10], self.buf[11]]);
        let type_modifier =
            i32::from_be_bytes([self.buf[12], self.buf[13], self.buf[14], self.buf[15]]);
        let format = i16::from_be_bytes([self.buf[16], self.buf[17]]);
        self.buf = &self.buf[18..];

        Some(Ok(Field {
            name,
            table_oid: Oid::new(table_oid),
            column_id,
            type_oid: Oid::new(type_oid),
            type_size,
            type_modifier,
            format,
        }))
    }
}

/// A field description in a `RowDescription`.
///
/// Describes a single column that will be returned by a query.
#[derive(Debug)]
pub struct Field<'a> {
    /// Column name.
    name: &'a str,
    /// OID of the table this column belongs to (0 if not from a table).
    table_oid: Oid,
    /// Column number within the table (attribute number).
    column_id: i16,
    /// OID of the column's data type.
    type_oid: Oid,
    /// Size of the type in bytes (-1 for variable-length types).
    type_size: i16,
    /// Type modifier (e.g., precision/scale for numeric types).
    type_modifier: i32,
    /// Format code: 0 = text, 1 = binary.
    format: i16,
}

impl<'a> Field<'a> {
    /// Returns the field name.
    #[inline]
    #[must_use]
    pub fn name(&self) -> &'a str {
        self.name
    }

    /// Returns the table OID.
    #[inline]
    #[must_use]
    pub fn table_oid(&self) -> Oid {
        self.table_oid
    }

    /// Returns the column ID within the table.
    #[inline]
    #[must_use]
    pub fn column_id(&self) -> i16 {
        self.column_id
    }

    /// Returns the type OID.
    #[inline]
    #[must_use]
    pub fn type_oid(&self) -> Oid {
        self.type_oid
    }

    /// Returns the type size.
    #[inline]
    #[must_use]
    pub fn type_size(&self) -> i16 {
        self.type_size
    }

    /// Returns the type modifier.
    #[inline]
    #[must_use]
    pub fn type_modifier(&self) -> i32 {
        self.type_modifier
    }

    /// Returns the format code (0 = text, 1 = binary).
    #[inline]
    #[must_use]
    pub fn format(&self) -> i16 {
        self.format
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn compute_all_offsets_handles_many_columns() {
        // Build a row with 40 columns to verify dynamic allocation works correctly.
        let mut buf = Vec::new();
        for i in 0u8..40 {
            buf.extend_from_slice(&1i32.to_be_bytes());
            buf.push(i);
        }
        let row = DataRowBody {
            storage: Bytes::from(buf),
            len: 40,
        };

        let offsets = row.compute_all_offsets();
        assert_eq!(offsets.len(), 40);

        let expect_range = |idx: usize, val: u8| {
            let (start, end) = offsets[idx].expect("expected non-null");
            let slice = &row.buffer()[start..end];
            assert_eq!(slice, &[val]);
        };

        expect_range(0, 0);
        expect_range(10, 10);
        expect_range(39, 39);
    }

    #[test]
    fn compute_all_offsets_tracks_nulls() {
        // Build: col0=1 byte, col1=NULL, col2=1 byte
        let mut buf = Vec::new();
        buf.extend_from_slice(&1i32.to_be_bytes());
        buf.push(0xAA);
        buf.extend_from_slice(&(-1i32).to_be_bytes()); // NULL
        buf.extend_from_slice(&1i32.to_be_bytes());
        buf.push(0xBB);

        let row = DataRowBody {
            storage: Bytes::from(buf),
            len: 3,
        };

        let offsets = row.compute_all_offsets();
        assert_eq!(offsets.len(), 3);
        assert!(offsets[0].is_some());
        assert!(offsets[1].is_none());
        assert!(offsets[2].is_some());
    }
}
