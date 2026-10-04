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

pub fn decode_value(data: &[u8]) -> Result<(CborValue, &[u8]), CborError> {
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
            let v = -1i64 - n as i64;
            Ok((CborValue::Int(v), rest))
        }
        2 => {
            // Byte string
            let (len, rest) = decode_uint_additional(additional, rest)?;
            let len = len as usize;
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
            let len = len as usize;
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
            let mut arr = Vec::with_capacity(count.min(64) as usize);
            for _ in 0..count {
                let (item, r) = decode_value(rest)?;
                arr.push(item);
                rest = r;
            }
            Ok((CborValue::Array(arr), rest))
        }
        5 => {
            // Map (we only support unsigned integer keys for this spec)
            let (count, mut rest) = decode_uint_additional(additional, rest)?;
            let mut entries: Vec<(u64, CborValue)> = Vec::with_capacity(count.min(64) as usize);
            for _ in 0..count {
                let (key_val, r) = decode_value(rest)?;
                let key = match key_val {
                    CborValue::Uint(k) => k,
                    other => {
                        return Err(CborError(format!(
                            "map key must be unsigned int, got {other:?}"
                        )))
                    }
                };
                let (val, r2) = decode_value(r)?;
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
}
