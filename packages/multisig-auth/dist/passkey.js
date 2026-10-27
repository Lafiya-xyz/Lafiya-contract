/**
 * WebAuthn / passkey (secp256r1) signing helpers for `multisig-account`.
 *
 * A passkey never exposes its private key: the browser asks the
 * authenticator to sign, and only the assertion (authenticator data, client
 * data JSON, DER signature) comes back. Everything here works on that
 * assertion and on public keys -- there is no API that takes, derives, or
 * returns private key material.
 *
 * The three things that are easy to get wrong:
 *
 * 1. **Challenge.** The WebAuthn challenge is the 32-byte auth payload from
 *    `buildAuthPayload`, so the assertion is bound to exactly that Soroban
 *    authorization (network, nonce, invocation tree, expiry ledger).
 * 2. **DER to compact.** Authenticators return an ASN.1 DER
 *    `SEQUENCE { r INTEGER, s INTEGER }`; the contract's `Signature`
 *    takes the 64-byte `r || s` form.
 * 3. **Low-S.** ECDSA signatures are malleable: `(r, s)` and `(r, n - s)`
 *    both verify. Soroban's `secp256r1_verify` only accepts the low-S form
 *    (`s <= n / 2`), and authenticators may return either, so every
 *    signature is normalized before it is attached.
 */
import { Buffer } from "buffer";
import { hash } from "@stellar/stellar-sdk";
/** Order `n` of the P-256 group. */
export const P256_ORDER = BigInt("0xffffffff00000000ffffffffffffffffbce6faada7179e84f3b9cac2fc632551");
const HALF_ORDER = P256_ORDER >> 1n;
/** DER `SubjectPublicKeyInfo` prefix for an uncompressed P-256 point. */
const P256_SPKI_PREFIX = Buffer.from("3059301306072a8648ce3d020106082a8648ce3d030107034200", "hex");
/** authenticatorData flag bits (WebAuthn §6.1). */
const FLAG_USER_PRESENT = 0x01;
const FLAG_USER_VERIFIED = 0x04;
function toBigInt(bytes) {
    return BigInt("0x" + (Buffer.from(bytes).toString("hex") || "0"));
}
function toBytes32(value) {
    return Uint8Array.from(Buffer.from(value.toString(16).padStart(64, "0"), "hex"));
}
function base64UrlEncode(bytes) {
    return Buffer.from(bytes)
        .toString("base64")
        .replace(/\+/g, "-")
        .replace(/\//g, "_")
        .replace(/=+$/, "");
}
/**
 * The WebAuthn challenge for an auth payload: the payload bytes themselves.
 * Pass the result as `PublicKeyCredentialRequestOptions.challenge`.
 */
export function passkeyChallenge(payload) {
    if (payload.length !== 32) {
        throw new Error(`passkeyChallenge: payload must be 32 bytes, got ${payload.length}`);
    }
    return Uint8Array.from(payload);
}
/**
 * Convert a DER-encoded ECDSA signature to the 64-byte `r || s` form.
 * Strict: rejects trailing bytes, non-minimal integers, negative values and
 * scalars outside `[1, n - 1]`, so a malformed signature fails here instead
 * of on-chain.
 */
export function derToCompact(der) {
    const fail = (why) => {
        throw new Error(`derToCompact: invalid DER signature (${why})`);
    };
    let offset = 0;
    const readInteger = () => {
        if (der[offset++] !== 0x02)
            fail("expected INTEGER");
        const length = der[offset++];
        if (length === undefined || length === 0 || length > 33)
            fail("bad INTEGER length");
        const bytes = der.subarray(offset, offset + length);
        if (bytes.length !== length)
            fail("truncated INTEGER");
        if (bytes[0] & 0x80)
            fail("negative INTEGER");
        if (length > 1 && bytes[0] === 0 && !(bytes[1] & 0x80))
            fail("non-minimal INTEGER");
        offset += length;
        const value = toBigInt(bytes);
        if (value === 0n || value >= P256_ORDER)
            fail("scalar out of range");
        return value;
    };
    if (der[offset++] !== 0x30)
        fail("expected SEQUENCE");
    const seqLength = der[offset++];
    if (seqLength === undefined || seqLength & 0x80 || offset + seqLength !== der.length) {
        fail("bad SEQUENCE length");
    }
    const r = readInteger();
    const s = readInteger();
    if (offset !== der.length)
        fail("trailing bytes");
    const out = new Uint8Array(64);
    out.set(toBytes32(r), 0);
    out.set(toBytes32(s), 32);
    return out;
}
/** Whether a compact `r || s` signature is already in low-S form. */
export function isLowS(signature) {
    if (signature.length !== 64) {
        throw new Error(`isLowS: signature must be 64 bytes, got ${signature.length}`);
    }
    return toBigInt(signature.subarray(32)) <= HALF_ORDER;
}
/**
 * Return the low-S form of a compact signature (`s` replaced by `n - s` when
 * `s > n / 2`). Both forms are valid ECDSA signatures over the same digest;
 * only the low-S one is accepted by the contract.
 */
export function normalizeLowS(signature) {
    if (isLowS(signature)) {
        return Uint8Array.from(signature);
    }
    const out = Uint8Array.from(signature);
    out.set(toBytes32(P256_ORDER - toBigInt(signature.subarray(32))), 32);
    return out;
}
/**
 * Extract the 65-byte uncompressed SEC-1 public key (the contract's
 * `SignerKey::Secp256r1`) from the SPKI DER returned by
 * `AuthenticatorAttestationResponse.getPublicKey()` at registration.
 */
export function secp256r1PublicKeyFromSpki(spki) {
    const prefix = spki.subarray(0, P256_SPKI_PREFIX.length);
    if (spki.length !== P256_SPKI_PREFIX.length + 65 || !P256_SPKI_PREFIX.equals(Buffer.from(prefix))) {
        throw new Error("secp256r1PublicKeyFromSpki: not an uncompressed P-256 SPKI public key");
    }
    return Uint8Array.from(spki.subarray(P256_SPKI_PREFIX.length));
}
/** The digest an authenticator signs: `SHA-256(authenticatorData || SHA-256(clientDataJSON))`. */
export function webauthnSignedDigest(authenticatorData, clientDataJSON) {
    return Uint8Array.from(hash(Buffer.concat([Buffer.from(authenticatorData), hash(Buffer.from(clientDataJSON))])));
}
/**
 * Parse and check a passkey assertion produced for `payload`: the client
 * data must be a `webauthn.get` whose challenge is exactly
 * `passkeyChallenge(payload)`, the user-present flag must be set, and
 * origin / rpId / user verification are checked when requested. Throws on
 * any mismatch. The signature is converted to compact low-S form.
 */
export function parsePasskeyAssertion(assertion, payload, options = {}) {
    const clientData = JSON.parse(Buffer.from(assertion.clientDataJSON).toString("utf8"));
    if (clientData.type !== "webauthn.get") {
        throw new Error(`passkey assertion: unexpected clientData.type ${String(clientData.type)}`);
    }
    if (clientData.challenge !== base64UrlEncode(passkeyChallenge(payload))) {
        throw new Error("passkey assertion: challenge does not match this auth payload");
    }
    if (typeof clientData.origin !== "string") {
        throw new Error("passkey assertion: clientData.origin missing");
    }
    if (options.origin !== undefined && clientData.origin !== options.origin) {
        throw new Error(`passkey assertion: unexpected origin ${clientData.origin}`);
    }
    const authData = Buffer.from(assertion.authenticatorData);
    if (authData.length < 37) {
        throw new Error("passkey assertion: authenticatorData too short");
    }
    if (options.rpId !== undefined && !hash(Buffer.from(options.rpId)).equals(authData.subarray(0, 32))) {
        throw new Error(`passkey assertion: rpIdHash does not match ${options.rpId}`);
    }
    const flags = authData[32];
    if (!(flags & FLAG_USER_PRESENT)) {
        throw new Error("passkey assertion: user-present flag not set");
    }
    const userVerified = (flags & FLAG_USER_VERIFIED) !== 0;
    if (options.requireUserVerification && !userVerified) {
        throw new Error("passkey assertion: user verification required but not performed");
    }
    const compact = derToCompact(assertion.signature);
    return {
        signature: normalizeLowS(compact),
        normalized: !isLowS(compact),
        signedDigest: webauthnSignedDigest(assertion.authenticatorData, assertion.clientDataJSON),
        origin: clientData.origin,
        userVerified,
        signCount: authData.readUInt32BE(33),
    };
}
