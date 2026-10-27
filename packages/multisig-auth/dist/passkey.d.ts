/** Order `n` of the P-256 group. */
export declare const P256_ORDER: bigint;
/**
 * The WebAuthn challenge for an auth payload: the payload bytes themselves.
 * Pass the result as `PublicKeyCredentialRequestOptions.challenge`.
 */
export declare function passkeyChallenge(payload: Uint8Array): Uint8Array;
/**
 * Convert a DER-encoded ECDSA signature to the 64-byte `r || s` form.
 * Strict: rejects trailing bytes, non-minimal integers, negative values and
 * scalars outside `[1, n - 1]`, so a malformed signature fails here instead
 * of on-chain.
 */
export declare function derToCompact(der: Uint8Array): Uint8Array;
/** Whether a compact `r || s` signature is already in low-S form. */
export declare function isLowS(signature: Uint8Array): boolean;
/**
 * Return the low-S form of a compact signature (`s` replaced by `n - s` when
 * `s > n / 2`). Both forms are valid ECDSA signatures over the same digest;
 * only the low-S one is accepted by the contract.
 */
export declare function normalizeLowS(signature: Uint8Array): Uint8Array;
/**
 * Extract the 65-byte uncompressed SEC-1 public key (the contract's
 * `SignerKey::Secp256r1`) from the SPKI DER returned by
 * `AuthenticatorAttestationResponse.getPublicKey()` at registration.
 */
export declare function secp256r1PublicKeyFromSpki(spki: Uint8Array): Uint8Array;
/** The digest an authenticator signs: `SHA-256(authenticatorData || SHA-256(clientDataJSON))`. */
export declare function webauthnSignedDigest(authenticatorData: Uint8Array, clientDataJSON: Uint8Array): Uint8Array;
/** The raw fields of a `PublicKeyCredential` assertion response. */
export interface PasskeyAssertion {
    authenticatorData: Uint8Array;
    clientDataJSON: Uint8Array;
    /** DER-encoded ECDSA signature, as returned by the authenticator. */
    signature: Uint8Array;
}
export interface PasskeyAssertionOptions {
    /** Expected `clientData.origin`, e.g. `window.location.origin`. */
    origin?: string;
    /** Expected relying-party id; checked against the authenticator's rpIdHash. */
    rpId?: string;
    /** Require the UV flag (PIN/biometric), not just user presence. */
    requireUserVerification?: boolean;
}
export interface ParsedPasskeyAssertion {
    /** Compact, low-S `r || s`, ready for the contract's `Signature.signature`. */
    signature: Uint8Array;
    /** Whether the authenticator returned a high-S signature that was normalized. */
    normalized: boolean;
    /** The digest the signature is over (see `webauthnSignedDigest`). */
    signedDigest: Uint8Array;
    origin: string;
    userVerified: boolean;
    signCount: number;
}
/**
 * Parse and check a passkey assertion produced for `payload`: the client
 * data must be a `webauthn.get` whose challenge is exactly
 * `passkeyChallenge(payload)`, the user-present flag must be set, and
 * origin / rpId / user verification are checked when requested. Throws on
 * any mismatch. The signature is converted to compact low-S form.
 */
export declare function parsePasskeyAssertion(assertion: PasskeyAssertion, payload: Uint8Array, options?: PasskeyAssertionOptions): ParsedPasskeyAssertion;
