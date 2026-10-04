/**
 * @lafiya/multisig-auth
 *
 * Helper for authorizing a Soroban call against the `multisig-account`
 * contract from a browser or Node signer (governance dashboards, passkey /
 * browser-wallet admin flows). Without this, a caller has to hand-build the
 * `HashIdPreimage::SorobanAuthorization` hash and hand-encode a
 * `Vec<Signature>` ScVal with the exact field names and public-key ordering
 * `__check_auth` expects -- a single ordering mistake there produces
 * `BadSignatureOrder` (contracts/multisig-account/src/lib.rs).
 *
 * `buildAuthPayload` and `attachSignatures` are the two calls a caller
 * needs; everything else here is the ceremony file format that lets a
 * browser signer and the CLI take part in one signing ceremony (see
 * `readCeremony`/`writeCeremony` below).
 */
import { Buffer } from "buffer";
import { Keypair, StrKey, hash, nativeToScVal, xdr } from "@stellar/stellar-sdk";
/**
 * Build the `HashIdPreimage::SorobanAuthorization` for `entry`'s `ADDRESS`
 * credentials. `@stellar/stellar-sdk` builds this same preimage internally
 * inside `authorizeEntry` (via its own `buildAuthorizationEntryPreimage`),
 * but that helper isn't part of this SDK's ESM build's exports in the
 * currently pinned version, so it's reimplemented here from the documented
 * XDR shape rather than depended on. See the package README's "Testing"
 * note.
 */
function buildAuthorizationEntryPreimage(entry, validUntilLedgerSeq, networkPassphrase) {
    const credentials = entry.credentials();
    if (credentials.switch().value !==
        xdr.SorobanCredentialsType.sorobanCredentialsAddress().value) {
        throw new Error(`buildAuthPayload: unsupported credential type ${credentials.switch().name} (only ADDRESS is supported)`);
    }
    const addrAuth = credentials.address();
    const networkId = hash(Buffer.from(networkPassphrase));
    return xdr.HashIdPreimage.envelopeTypeSorobanAuthorization(new xdr.HashIdPreimageSorobanAuthorization({
        networkId,
        nonce: addrAuth.nonce(),
        invocation: entry.rootInvocation(),
        signatureExpirationLedger: validUntilLedgerSeq,
    }));
}
/**
 * Compute the `HashIdPreimage::SorobanAuthorization` (or, for ADDRESS_V2/
 * ADDRESS_WITH_DELEGATES credentials, `...WithAddress`) hash for `entry` --
 * the exact 32 bytes each signer must sign with their ed25519 key, and the
 * exact bytes `__check_auth`'s `signature_payload` argument will be.
 *
 * Constructs the same `HashIdPreimage::SorobanAuthorization` XDR shape
 * `@stellar/stellar-sdk`'s own `authorizeEntry` builds internally (see the
 * local `buildAuthorizationEntryPreimage` above), then hashes it with the
 * SDK's own `hash`, so a payload built here matches what the SDK -- and any
 * other implementation of the same XDR spec -- would compute for the same
 * entry.
 */
export function buildAuthPayload(entry, networkPassphrase, validUntilLedger) {
    const preimage = buildAuthorizationEntryPreimage(entry, validUntilLedger, networkPassphrase);
    return hash(preimage.toXDR());
}
/**
 * Sort signatures by ascending public key (matching the exact byte-wise
 * comparison `__check_auth` performs on `BytesN<32>`, not string/locale
 * order on the G... strkey), and drop exact duplicates. Ordering signatures
 * any other way is precisely what produces `BadSignatureOrder`.
 */
function sortAndDedupe(sigs) {
    const byPublicKey = new Map();
    for (const sig of sigs) {
        byPublicKey.set(sig.publicKey, sig); // last write wins on an exact duplicate
    }
    return [...byPublicKey.values()].sort((a, b) => {
        const rawA = StrKey.decodeEd25519PublicKey(a.publicKey);
        const rawB = StrKey.decodeEd25519PublicKey(b.publicKey);
        return Buffer.compare(rawA, rawB);
    });
}
const SIGNATURE_STRUCT_TYPE = {
    type: {
        public_key: ["symbol", null],
        signature: ["symbol", null],
    },
};
/**
 * Sort, deduplicate, and encode `sigs` as the `Vec<Signature>` ScVal
 * `__check_auth` expects, then set it as `entry`'s credentials signature.
 * Returns a new entry; `entry` itself is not mutated.
 *
 * Only `ADDRESS` credentials are supported -- the only kind
 * `multisig-account` is ever invoked with (source-account credentials need
 * no signature and are returned unchanged, matching `authorizeEntry`'s own
 * behavior).
 */
export function attachSignatures(entry, sigs) {
    if (entry.credentials().switch().value ===
        xdr.SorobanCredentialsType.sorobanCredentialsSourceAccount().value) {
        return entry;
    }
    const clone = xdr.SorobanAuthorizationEntry.fromXDR(entry.toXDR());
    const ordered = sortAndDedupe(sigs);
    const sigScVals = ordered.map((sig) => nativeToScVal({
        public_key: StrKey.decodeEd25519PublicKey(sig.publicKey),
        signature: Buffer.from(sig.signature),
    }, SIGNATURE_STRUCT_TYPE));
    const signatureScVal = xdr.ScVal.scvVec(sigScVals);
    const credentials = clone.credentials();
    if (credentials.switch().value !==
        xdr.SorobanCredentialsType.sorobanCredentialsAddress().value) {
        throw new Error(`attachSignatures: unsupported credential type ${credentials.switch().name} (only ADDRESS is supported)`);
    }
    credentials.address().signature(signatureScVal);
    return clone;
}
/** Build a fresh, unsigned ceremony file for `entry`. */
export function createCeremony(entry, contractId, networkPassphrase, validUntilLedger) {
    const payload = buildAuthPayload(entry, networkPassphrase, validUntilLedger);
    return {
        version: 1,
        contractId,
        networkPassphrase,
        validUntilLedger,
        entryXdr: entry.toXDR("base64"),
        payloadHex: Buffer.from(payload).toString("hex"),
        signatures: [],
    };
}
/** Parse a ceremony file's JSON text, validating its shape. */
export function readCeremony(json) {
    const parsed = JSON.parse(json);
    if (parsed.version !== 1) {
        throw new Error(`unsupported ceremony file version: ${String(parsed.version)} (expected 1)`);
    }
    const required = [
        "contractId",
        "networkPassphrase",
        "validUntilLedger",
        "entryXdr",
        "payloadHex",
        "signatures",
    ];
    for (const field of required) {
        if (parsed[field] === undefined) {
            throw new Error(`ceremony file missing required field: ${field}`);
        }
    }
    return parsed;
}
/** Serialize a ceremony file back to JSON text (pretty-printed for diffs). */
export function writeCeremony(ceremony) {
    return JSON.stringify(ceremony, null, 2) + "\n";
}
/**
 * Add one signature to a ceremony, verifying it is over the ceremony's own
 * `payloadHex` before accepting it (a signature collected against a stale
 * payload -- e.g. after `validUntilLedger` was bumped -- must never be
 * silently merged in). Returns a new ceremony; `ceremony` is not mutated.
 */
export function addSignatureToCeremony(ceremony, publicKey, signature) {
    const payload = Buffer.from(ceremony.payloadHex, "hex");
    if (!Keypair.fromPublicKey(publicKey).verify(payload, Buffer.from(signature))) {
        throw new Error(`signature from ${publicKey} does not verify against this ceremony's payload`);
    }
    return {
        ...ceremony,
        signatures: [
            ...ceremony.signatures.filter((s) => s.publicKey !== publicKey),
            { publicKey, signatureHex: Buffer.from(signature).toString("hex") },
        ],
    };
}
/** Rebuild the signed `SorobanAuthorizationEntry` from a ceremony's
 * collected signatures, sorted and encoded via `attachSignatures`. */
export function finalizeCeremony(ceremony) {
    const entry = xdr.SorobanAuthorizationEntry.fromXDR(ceremony.entryXdr, "base64");
    const sigs = ceremony.signatures.map((s) => ({
        publicKey: s.publicKey,
        signature: Buffer.from(s.signatureHex, "hex"),
    }));
    return attachSignatures(entry, sigs);
}
