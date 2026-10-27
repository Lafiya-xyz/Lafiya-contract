//! Base45 encoder/decoder (RFC 9285) for QR alphanumeric mode.
//!
//! Base45 uses a 45-character alphabet that maps exactly to the QR code
//! alphanumeric character set, yielding roughly 25 % better data density
//! than base64 in alphanumeric mode.

/// The 45-character base45 alphabet (RFC 9285 §4).
const ALPHABET: &[u8; 45] = b"0123456789ABCDEFGHIJKLMNOPQRSTUVWXYZ $%*+-./:";

/// Encode bytes to base45.
pub fn encode(data: &[u8]) -> String {
    let mut out = Vec::new();
    let mut i = 0;
    while i < data.len() {
        if i + 1 < data.len() {
            // Encode two bytes as three base45 characters.
            let n = (data[i] as u32) * 256 + (data[i + 1] as u32);
            let c = n % 45;
            let n = n / 45;
            let b = n % 45;
            let a = n / 45;
            out.push(ALPHABET[c as usize]);
            out.push(ALPHABET[b as usize]);
            out.push(ALPHABET[a as usize]);
            i += 2;
        } else {
            // Encode one byte as two base45 characters.
            let n = data[i] as u32;
            let b = n % 45;
            let a = n / 45;
            out.push(ALPHABET[b as usize]);
            out.push(ALPHABET[a as usize]);
            i += 1;
        }
    }
    String::from_utf8(out).expect("base45 alphabet is ASCII")
}

/// Maximum number of input characters accepted by [`decode`].
///
/// This bounds the allocation performed by the decoder so that a malicious
/// (attacker-controlled) input cannot force unbounded memory use. The limit
/// is far above any legitimate QR payload.
pub const MAX_ENCODED_LEN: usize = 1 << 20; // 1 MiB of base45 characters

/// Decode base45 to bytes.
///
/// Returns an error string if the input contains characters outside the
/// base45 alphabet, if the encoded length is invalid, if the input exceeds
/// [`MAX_ENCODED_LEN`], or if a triplet/pair encodes an out-of-range value.
pub fn decode(input: &str) -> Result<Vec<u8>, String> {
    let chars = input.as_bytes();
    let len = chars.len();

    // Bound the input size before doing any allocation.
    if len > MAX_ENCODED_LEN {
        return Err(format!(
            "base45 input too long: {len} > {MAX_ENCODED_LEN} characters"
        ));
    }

    // Length must not be ≡ 1 (mod 3): each pair of bytes needs 3 chars,
    // each single byte needs 2 chars, so valid lengths satisfy len % 3 != 1.
    if len % 3 == 1 {
        return Err(format!(
            "invalid base45 length {len}: cannot have length ≡ 1 (mod 3)"
        ));
    }

    // Build reverse lookup table.
    let mut rev = [0xffu8; 256];
    for (i, &c) in ALPHABET.iter().enumerate() {
        rev[c as usize] = i as u8;
    }

    // Exact output size: every 3 chars → 2 bytes, every trailing 2 chars → 1.
    let out_len = (len / 3) * 2 + (len % 3) / 2;
    let mut out = Vec::with_capacity(out_len);
    let mut i = 0;
    while i < len {
        if i + 2 < len {
            // Three chars → two bytes.
            let c = lookup(&rev, chars[i])?;
            let b = lookup(&rev, chars[i + 1])?;
            let a = lookup(&rev, chars[i + 2])?;
            let n = (a as u32) * 45 * 45 + (b as u32) * 45 + (c as u32);
            if n > 0xffff {
                return Err(format!(
                    "base45 triplet at position {i} encodes value {n} > 65535"
                ));
            }
            out.push((n >> 8) as u8);
            out.push((n & 0xff) as u8);
            i += 3;
        } else {
            // Two chars → one byte.
            let b = lookup(&rev, chars[i])?;
            let a = lookup(&rev, chars[i + 1])?;
            let n = (a as u32) * 45 + (b as u32);
            if n > 0xff {
                return Err(format!(
                    "base45 pair at position {i} encodes value {n} > 255"
                ));
            }
            out.push(n as u8);
            i += 2;
        }
    }
    debug_assert_eq!(out.len(), out_len);
    Ok(out)
}

fn lookup(rev: &[u8; 256], c: u8) -> Result<u8, String> {
    let v = rev[c as usize];
    if v == 0xff {
        Err(format!(
            "invalid base45 character: '{}' (0x{:02x})",
            char::from(c),
            c
        ))
    } else {
        Ok(v)
    }
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rfc9285_test_vector_ab() {
        // From RFC 9285 §4.5: "AB" encodes as "BB8"
        assert_eq!(encode(b"AB"), "BB8");
        assert_eq!(decode("BB8").unwrap(), b"AB");
    }

    #[test]
    fn rfc9285_test_vector_hello_world() {
        // From RFC 9285 §4.5: "Hello!!" encodes as "%69 VD92EX0"
        assert_eq!(encode(b"Hello!!"), "%69 VD92EX0");
        assert_eq!(decode("%69 VD92EX0").unwrap(), b"Hello!!");
    }

    #[test]
    fn rfc9285_test_vector_base45() {
        // "base-45" encodes as "UJCLQE7W581"
        assert_eq!(encode(b"base-45"), "UJCLQE7W581");
        assert_eq!(decode("UJCLQE7W581").unwrap(), b"base-45");
    }

    #[test]
    fn round_trip_empty() {
        let data = b"";
        assert_eq!(decode(&encode(data)).unwrap(), data);
    }

    #[test]
    fn round_trip_single_byte() {
        for b in 0u8..=255 {
            let data = [b];
            assert_eq!(decode(&encode(&data)).unwrap(), &data);
        }
    }

    #[test]
    fn round_trip_all_bytes() {
        let data: Vec<u8> = (0u8..=255).collect();
        assert_eq!(decode(&encode(&data)).unwrap(), data);
    }

    #[test]
    fn rejects_invalid_character() {
        let err = decode("!!INVALID").unwrap_err();
        assert!(err.contains("invalid base45 character"));
    }

    #[test]
    fn rejects_invalid_length_mod3_eq1() {
        // A length of 1 is ≡ 1 (mod 3) and must be rejected.
        let err = decode("A").unwrap_err();
        assert!(err.contains("invalid base45 length"), "{err}");
    }

    #[test]
    fn rejects_triplet_over_65535() {
        // ":::" = 44*45*45 + 44*45 + 44 = 91124 > 65535.
        let err = decode(":::").unwrap_err();
        assert!(err.contains("> 65535"), "{err}");
    }

    #[test]
    fn rejects_pair_over_255() {
        // "::" = 44*45 + 44 = 2024 > 255.
        let err = decode("::").unwrap_err();
        assert!(err.contains("> 255"), "{err}");
    }

    #[test]
    fn rejects_oversized_input() {
        let input = "0".repeat(MAX_ENCODED_LEN + 1);
        let err = decode(&input).unwrap_err();
        assert!(err.contains("too long"), "{err}");
    }

    #[test]
    fn round_trip_lengths() {
        // Exercise every length up to a few hundred bytes, including the
        // odd/even boundary handled by the trailing single-byte case.
        for len in 0..300usize {
            let data: Vec<u8> = (0..len).map(|i| (i * 31 + 7) as u8).collect();
            let encoded = encode(&data);
            assert_eq!(decode(&encoded).unwrap(), data, "len {len}");
        }
    }

    #[test]
    fn malformed_inputs_never_panic() {
        // Deterministic corpus of malformed inputs; mirrors the fuzz target.
        let cases = [
            "",
            "A",
            "::",
            ":::",
            "!!!!",
            "ZZZ",
            "  ",
            "0",
            "00",
            "000",
            "0000",
            "00000",
            "000000",
            "0000000",
            "\u{0}",
            "\u{7f}",
            "é",
        ];
        for case in cases {
            // Must return Ok or Err, never panic.
            let _ = decode(case);
        }
    }

    #[test]
    fn decode_encode_round_trip_stability() {
        // Decoding then re-encoding a valid encoding must be stable.
        let data = b"the quick brown fox jumps over the lazy dog";
        let encoded = encode(data);
        let decoded = decode(&encoded).unwrap();
        assert_eq!(encode(&decoded), encoded);
    }
}
