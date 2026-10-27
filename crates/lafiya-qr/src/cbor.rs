//! Minimal CBOR encoder/decoder (RFC 8949) for the QR payload format.
//!
//! Only the CBOR types used by the Lafiya QR payload spec are implemented:
//! - Unsigned integers (major type 0)
//! - Negative integers (major type 1) — for i64 negative values
//! - Byte strings (major type 2)
//! - Text strings (major type 3)
//! - Arrays (major type 4)
//! - Maps with unsigned integer keys (major type 5)
//! - Simple values: false (0xf4), true (0xf5)

use super::CborValue;

/// Maximum number of elements allowed in a CBOR array or map.
///
/// This bounds allocation when decoding attacker-controlled input: a tiny
/// payload can otherwise claim a huge element count. The limit is far above
/// anything the QR payload spec needs.
pub const MAX_CONTAINER_LEN: u64 = 1024;

/// Maximum nesting depth for arrays/maps.
///
/// Prevents stack exhaustion from deeply nested attacker-controlled input.
pub const MAX_DEPTH: usize = 16;

// ---------------------------------------------------------------------------
// Encoder
// ---------------------------------------------------------------------------

pub fn encode_value(val: &CborValue, out: &mut Vec<u8>) {
    match val {
        CborValue::Uint(n) => encode_uint(*n, out),
        CborValue::Int(n) => {
            if *n >= 0 {
                encode_uint(*n as u64, out);
            } else {
                // Negative: major type 1, value = -1 - n
                let encoded = (-1 - *n) as u64;
                encode_type_len(1, encoded, out);
            }
        }
        CborValue::Bytes(b) => {
            encode_type_len(2, b.len() as u64, out);
            out.extend_from_slice(b);
        }
        CborValue::Text(s) => {
            let utf8 = s.as_bytes();
            encode_type_len(3, utf8.len() as u64, out);
            out.extend_from_slice(utf8);
        }
        CborValue::Array(arr) => {
            encode_type_len(4, arr.len() as u64, out);
            for item in arr {
                encode_value(item, out);
            }
        }
        CborValue::Map(entries) => {
            encode_type_len(5, entries.len() as u64, out);
            for (k, v) in entries {
                encode_uint(*k, out);
                encode_value(v, out);
            }
        }
        CborValue::Bool(b) => {
            out.push(if *b { 0xf5 } else { 0xf4 });
        }
    }
}

fn encode_uint(n: u64, out: &mut Vec<u8>) {
    encode_type_len(0, n, out);
}

fn encode_type_len(major: u8, n: u64, out: &mut Vec<u8>) {
    let mt = major << 5;
    if n <= 23 {
        out.push(mt | n as u8);
    } else if n <= 0xff {
        out.push(mt | 24);
        out.push(n as u8);
    } else if n <= 0xffff {
        out.push(mt | 25);
        out.extend_from_slice(&(n as u16).to_be_bytes());
    } else if n <= 0xffff_ffff {
        out.push(mt | 26);
        out.extend_from_slice(&(n as u32).to_be_bytes());
    } else {
        out.push(mt | 27);
        out.extend_from_slice(&n.to_be_bytes());
    }
}

// ---------------------------------------------------------------------------
// Decoder
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CborError(pub String);

impl core::fmt::Display for CborError {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        write!(f, "{}", self.0)
    }
}

impl std::error::Error for CborError {}

/// Decode a single CBOR value from the front of `data`.
///
/// Returns the decoded value and the unconsumed remainder. Callers that expect
/// exactly one value should use [`decode_exact`].
pub fn decode_value(data: &[u8]) -> Result<(CborValue, &[u8]), CborError> {
    decode_value_at_depth(data, 0)
}

/// Decode exactly one CBOR value, rejecting trailing bytes.
pub fn decode_exact(data: &[u8]) -> Result<CborValue, CborError> {
    let (value, rest) = decode_value(data)?;
    if !rest.is_empty() {
        return Err(CborError(format!(
            "trailing bytes after CBOR value: {}",
            rest.len()
        )));
    }
    Ok(value)
}

fn decode_value_at_depth(data: &[u8], depth: usize) -> Result<(CborValue, &[u8]), CborError> {
    if depth > MAX_DEPTH {
        return Err(CborError(format!(
            "CBOR nesting depth exceeds maximum of {MAX_DEPTH}"
        )));
    }
    if data.is_empty() {
        return Err(CborError("unexpected end of CBOR input".into()));
    }
    let initial = data[0];
    let major = initial >> 5;
    let additional = initial & 0x1f;
    let rest = &data[1..];

    match major {
        0 => {
            // Unsigned integer
            let (n, rest) = decode_uint_additional(additional, rest)?;
            Ok((CborValue::Uint(n), rest))
        }
        1 => {
            // Negative integer: value is -1 - n
            let (n, rest) = decode_uint_additional(additional, rest)?;
            // n may exceed i64::MAX; reject rather than overflow.
            if n > i64::MAX as u64 {
                return Err(CborError(format!(
                    "negative integer magnitude {n} out of range"
                )));
            }
            let v = -1i64 - n as i64;
            Ok((CborValue::Int(v), rest))
        }
        2 => {
            // Byte string
            let (len, rest) = decode_uint_additional(additional, rest)?;
            let len = usize::try_from(len)
                .map_err(|_| CborError(format!("byte string length {len} out of range")))?;
            if rest.len() < len {
                return Err(CborError(format!(
                    "byte string of length {len} exceeds remaining input ({})",
                    rest.len()
                )));
            }
            Ok((CborValue::Bytes(rest[..len].to_vec()), &rest[len..]))
        }
        3 => {
            // Text string
            let (len, rest) = decode_uint_additional(additional, rest)?;
            let len = usize::try_from(len)
                .map_err(|_| CborError(format!("text string length {len} out of range")))?;
            if rest.len() < len {
                return Err(CborError(format!(
                    "text string of length {len} exceeds remaining input ({})",
                    rest.len()
                )));
            }
            let s = String::from_utf8(rest[..len].to_vec())
                .map_err(|e| CborError(format!("CBOR text string is not valid UTF-8: {e}")))?;
            Ok((CborValue::Text(s), &rest[len..]))
        }
        4 => {
            // Array
            let (count, mut rest) = decode_uint_additional(additional, rest)?;
            check_container_len(count, "array")?;
            // Each element needs at least one byte, so a count larger than the
            // remaining input can never be satisfied.
            if count > rest.len() as u64 {
                return Err(CborError(format!(
                    "array count {count} exceeds remaining input ({})",
                    rest.len()
                )));
            }
            let mut arr = Vec::with_capacity(count as usize);
            for _ in 0..count {
                let (item, r) = decode_value_at_depth(rest, depth + 1)?;
                arr.push(item);
                rest = r;
            }
            Ok((CborValue::Array(arr), rest))
        }
        5 => {
            // Map (we only support unsigned integer keys for this spec)
            let (count, mut rest) = decode_uint_additional(additional, rest)?;
            check_container_len(count, "map")?;
            // Each entry needs at least two bytes (key + value).
            if count > (rest.len() as u64) / 2 {
                return Err(CborError(format!(
                    "map count {count} exceeds remaining input ({})",
                    rest.len()
                )));
            }
            let mut entries: Vec<(u64, CborValue)> = Vec::with_capacity(count as usize);
            for _ in 0..count {
                let (key_val, r) = decode_value_at_depth(rest, depth + 1)?;
                let key = match key_val {
                    CborValue::Uint(k) => k,
                    other => {
                        return Err(CborError(format!(
                            "map key must be unsigned int, got {other:?}"
                        )))
                    }
                };
                let (val, r2) = decode_value_at_depth(r, depth + 1)?;
                entries.push((key, val));
                rest = r2;
            }
            Ok((CborValue::Map(entries), rest))
        }
        7 => {
            // Simple/float values (we only handle false/true)
            match additional {
                20 => Ok((CborValue::Bool(false), rest)),
                21 => Ok((CborValue::Bool(true), rest)),
                _ => Err(CborError(format!(
                    "unsupported CBOR simple value: additional={additional}"
                ))),
            }
        }
        _ => Err(CborError(format!("unsupported CBOR major type: {major}"))),
    }
}

fn check_container_len(count: u64, kind: &str) -> Result<(), CborError> {
    if count > MAX_CONTAINER_LEN {
        return Err(CborError(format!(
            "{kind} count {count} exceeds maximum of {MAX_CONTAINER_LEN}"
        )));
    }
    Ok(())
}

fn decode_uint_additional<'a>(
    additional: u8,
    data: &'a [u8],
) -> Result<(u64, &'a [u8]), CborError> {
    match additional {
        0..=23 => Ok((additional as u64, data)),
        24 => {
            if data.is_empty() {
                return Err(CborError("truncated: need 1 byte for uint8".into()));
            }
            Ok((data[0] as u64, &data[1..]))
        }
        25 => {
            if data.len() < 2 {
                return Err(CborError("truncated: need 2 bytes for uint16".into()));
            }
            let n = u16::from_be_bytes([data[0], data[1]]) as u64;
            Ok((n, &data[2..]))
        }
        26 => {
            if data.len() < 4 {
                return Err(CborError("truncated: need 4 bytes for uint32".into()));
            }
            let n = u32::from_be_bytes([data[0], data[1], data[2], data[3]]) as u64;
            Ok((n, &data[4..]))
        }
        27 => {
            if data.len() < 8 {
                return Err(CborError("truncated: need 8 bytes for uint64".into()));
            }
            let n = u64::from_be_bytes([
                data[0], data[1], data[2], data[3],
                data[4], data[5], data[6], data[7],
            ]);
            Ok((n, &data[8..]))
        }
        _ => Err(CborError(format!(
            "unsupported CBOR additional value: {additional}"
        ))),
    }
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    fn roundtrip(val: &CborValue) -> CborValue {
        let mut buf = Vec::new();
        encode_value(val, &mut buf);
        let (decoded, rest) = decode_value(&buf).expect("decode");
        assert!(rest.is_empty(), "trailing bytes after decode");
        decoded
    }

    #[test]
    fn uint_roundtrip() {
        for n in [0u64, 1, 23, 24, 255, 256, 65535, 65536, u32::MAX as u64, u64::MAX] {
            let v = CborValue::Uint(n);
            assert_eq!(roundtrip(&v), v, "n={n}");
        }
    }

    #[test]
    fn negative_int_roundtrip() {
        for n in [-1i64, -24, -255, -256, i64::MIN] {
            let v = CborValue::Int(n);
            assert_eq!(roundtrip(&v), v, "n={n}");
        }
    }

    #[test]
    fn bytes_roundtrip() {
        let v = CborValue::Bytes(vec![0x01, 0x02, 0xfe, 0xff]);
        assert_eq!(roundtrip(&v), v);
    }

    #[test]
    fn text_roundtrip() {
        let v = CborValue::Text("hello, world".into());
        assert_eq!(roundtrip(&v), v);
    }

    #[test]
    fn bool_roundtrip() {
        assert_eq!(roundtrip(&CborValue::Bool(true)), CborValue::Bool(true));
        assert_eq!(roundtrip(&CborValue::Bool(false)), CborValue::Bool(false));
    }

    #[test]
    fn map_roundtrip() {
        let v = CborValue::Map(vec![
            (1, CborValue::Uint(42)),
            (2, CborValue::Text("test".into())),
        ]);
        assert_eq!(roundtrip(&v), v);
    }

    #[test]
    fn array_roundtrip() {
        let v = CborValue::Array(vec![
            CborValue::Uint(1),
            CborValue::Bool(false),
            CborValue::Bytes(vec![0xaa]),
        ]);
        assert_eq!(roundtrip(&v), v);
    }

    #[test]
    fn rejects_empty_input() {
        assert!(decode_value(&[]).is_err());
    }

    #[test]
    fn decode_exact_rejects_trailing_bytes() {
        let mut buf = Vec::new();
        encode_value(&CborValue::Uint(1), &mut buf);
        buf.push(0x00);
        assert!(decode_exact(&buf).is_err());
    }

    #[test]
    fn decode_exact_accepts_single_value() {
        let mut buf = Vec::new();
        encode_value(&CborValue::Text("ok".into()), &mut buf);
        assert_eq!(decode_exact(&buf).unwrap(), CborValue::Text("ok".into()));
    }

    #[test]
    fn rejects_truncated_uint() {
        // 0x1b = uint64 follows, but no payload bytes.
        assert!(decode_value(&[0x1b]).is_err());
        assert!(decode_value(&[0x1a, 0x00]).is_err());
        assert!(decode_value(&[0x19, 0x00]).is_err());
        assert!(decode_value(&[0x18]).is_err());
    }

    #[test]
    fn rejects_truncated_byte_string() {
        // Byte string of length 4 with only 2 bytes present.
        assert!(decode_value(&[0x44, 0x01, 0x02]).is_err());
    }

    #[test]
    fn rejects_truncated_text_string() {
        // Text string of length 4 with only 2 bytes present.
        assert!(decode_value(&[0x64, b'a', b'b']).is_err());
    }

    #[test]
    fn rejects_oversized_array_count() {
        // Array claiming 2^32 elements with no payload.
        assert!(decode_value(&[0x9a, 0xff, 0xff, 0xff, 0xff]).is_err());
    }

    #[test]
    fn rejects_oversized_map_count() {
        // Map claiming 2^32 entries with no payload.
        assert!(decode_value(&[0xba, 0xff, 0xff, 0xff, 0xff]).is_err());
    }

    #[test]
    fn rejects_array_count_above_max_container_len() {
        // Array claiming MAX_CONTAINER_LEN + 1 elements; supply enough bytes
        // that the remaining-input check alone would not reject it.
        let count = MAX_CONTAINER_LEN + 1;
        let mut buf = Vec::new();
        encode_type_len(4, count, &mut buf);
        buf.extend(std::iter::repeat(0x00).take(count as usize));
        assert!(decode_value(&buf).is_err());
    }

    #[test]
    fn rejects_excessive_nesting() {
        // MAX_DEPTH + 2 nested single-element arrays.
        let mut buf = vec![0x81u8; MAX_DEPTH + 2];
        buf.push(0x00);
        assert!(decode_value(&buf).is_err());
    }

    #[test]
    fn rejects_negative_int_out_of_range() {
        // Major type 1 with value u64::MAX would overflow i64.
        let mut buf = Vec::new();
        encode_type_len(1, u64::MAX, &mut buf);
        assert!(decode_value(&buf).is_err());
    }

    #[test]
    fn accepts_container_at_max_len() {
        let count = MAX_CONTAINER_LEN;
        let mut buf = Vec::new();
        encode_type_len(4, count, &mut buf);
        buf.extend(std::iter::repeat(0x00).take(count as usize));
        let (value, rest) = decode_value(&buf).expect("decode");
        assert!(rest.is_empty());
        match value {
            CborValue::Array(arr) => assert_eq!(arr.len(), count as usize),
            other => panic!("expected array, got {other:?}"),
        }
    }
}
