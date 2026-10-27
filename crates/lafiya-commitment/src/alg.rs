//! Reserved cryptographic algorithm identifiers (ADR-0019).
//!
//! One byte per algorithm, in three namespaces. These are identifiers only:
//! nothing in the current wire formats carries them yet, and only the
//! algorithms marked *active* are implemented. Reserving them now lets a
//! format gain an algorithm field before the v1 freeze without a later
//! registry collision. Never reuse or renumber an id. Retired ids stay
//! reserved forever, like contract error codes.
//!
//! In every namespace `0x00` means "implied by the enclosing format version"
//! (the meaning of every v1 field that has no explicit algorithm id), and
//! `0xF0..=0xFF` is for private experiments and must never be accepted by a
//! production verifier.

/// Hash functions (commitments, Merkle trees, signature pre-hashing).
pub mod hash {
    /// Implied by the enclosing format version.
    pub const IMPLIED: u8 = 0x00;
    /// SHA-256 (FIPS 180-4). Active: LRC-1, LRC-2, Merkle batches, QR.
    pub const SHA256: u8 = 0x01;
    /// SHA3-256 (FIPS 202). Reserved: dual-hash successor candidate.
    pub const SHA3_256: u8 = 0x02;
    /// Keccak-256 (pre-FIPS padding). Reserved: Soroban host function.
    pub const KECCAK256: u8 = 0x03;
    /// SHA-512/256 (FIPS 180-4). Reserved.
    pub const SHA512_256: u8 = 0x04;
    /// Poseidon over BN254. Reserved: ZK-friendly commitments.
    pub const POSEIDON_BN254: u8 = 0x10;
}

/// Signature schemes (attester receipts, patient signatures, multisig).
pub mod sig {
    /// Implied by the enclosing format version.
    pub const IMPLIED: u8 = 0x00;
    /// Ed25519 (RFC 8032). Active: Stellar accounts, receipts, patient signatures.
    pub const ED25519: u8 = 0x01;
    /// ECDSA over P-256 (secp256r1). Active in `multisig-account` (passkeys).
    pub const ECDSA_P256: u8 = 0x02;
    /// ECDSA over secp256k1 (public-key recovery). Reserved.
    pub const ECDSA_SECP256K1: u8 = 0x03;
    /// ML-DSA-44 (FIPS 204). Reserved: post-quantum.
    pub const ML_DSA_44: u8 = 0x10;
    /// ML-DSA-65 (FIPS 204). Reserved: post-quantum.
    pub const ML_DSA_65: u8 = 0x11;
    /// SLH-DSA-SHA2-128s (FIPS 205). Reserved: post-quantum, hash-based.
    pub const SLH_DSA_SHA2_128S: u8 = 0x12;
    /// FN-DSA-512 (Falcon). Reserved: post-quantum, pending FIPS 206.
    pub const FN_DSA_512: u8 = 0x13;
    /// Ed25519 and ML-DSA-44, both required to verify. Reserved: transition.
    pub const HYBRID_ED25519_ML_DSA_44: u8 = 0x20;
}

/// Key-derivation functions.
pub mod kdf {
    /// Implied by the enclosing format version.
    pub const IMPLIED: u8 = 0x00;
    /// HKDF-SHA256 (RFC 5869). Active: salt derivation (ADR-0018).
    pub const HKDF_SHA256: u8 = 0x01;
}

/// Lowest id of the private-use range in every namespace.
pub const PRIVATE_USE_START: u8 = 0xF0;

#[cfg(test)]
mod tests {
    use super::*;

    fn assert_unique(ids: &[u8]) {
        for (i, a) in ids.iter().enumerate() {
            assert!(
                *a < PRIVATE_USE_START,
                "id {a:#04x} is in the private-use range"
            );
            for b in &ids[i + 1..] {
                assert_ne!(a, b, "duplicate algorithm id {a:#04x}");
            }
        }
    }

    #[test]
    fn ids_are_unique_per_namespace() {
        assert_unique(&[
            hash::IMPLIED,
            hash::SHA256,
            hash::SHA3_256,
            hash::KECCAK256,
            hash::SHA512_256,
            hash::POSEIDON_BN254,
        ]);
        assert_unique(&[
            sig::IMPLIED,
            sig::ED25519,
            sig::ECDSA_P256,
            sig::ECDSA_SECP256K1,
            sig::ML_DSA_44,
            sig::ML_DSA_65,
            sig::SLH_DSA_SHA2_128S,
            sig::FN_DSA_512,
            sig::HYBRID_ED25519_ML_DSA_44,
        ]);
        assert_unique(&[kdf::IMPLIED, kdf::HKDF_SHA256]);
    }
}
