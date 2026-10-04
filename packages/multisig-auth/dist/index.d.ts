import { xdr } from "@stellar/stellar-sdk";
/** One collected ed25519 signature, keyed by the signer's G... address. */
export interface CollectedSignature {
    /** The signer's ed25519 public key, as a Stellar G... strkey address. */
    publicKey: string;
    /** Raw 64-byte ed25519 signature over the payload from `buildAuthPayload`. */
    signature: Uint8Array;
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
export declare function buildAuthPayload(entry: xdr.SorobanAuthorizationEntry, networkPassphrase: string, validUntilLedger: number): Uint8Array;
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
export declare function attachSignatures(entry: xdr.SorobanAuthorizationEntry, sigs: CollectedSignature[]): xdr.SorobanAuthorizationEntry;
/** On-disk / wire shape of one signing ceremony. */
export interface CeremonyFile {
    /** Ceremony format version, for forward compatibility. */
    version: 1;
    /** The multisig-account contract this ceremony authorizes a call on. */
    contractId: string;
    /** Network passphrase the payload hash was computed under. */
    networkPassphrase: string;
    /** Ledger sequence the authorization is valid until. */
    validUntilLedger: number;
    /** The XDR (base64) of the unsigned `SorobanAuthorizationEntry`. */
    entryXdr: string;
    /** hex-encoded `buildAuthPayload` output, included so a signer can verify
     * what they are about to sign without re-deriving it. */
    payloadHex: string;
    /** Signatures collected so far, in the order they were added (not
     * necessarily sorted -- `attachSignatures` sorts at submission time). */
    signatures: Array<{
        publicKey: string;
        signatureHex: string;
    }>;
}
/** Build a fresh, unsigned ceremony file for `entry`. */
export declare function createCeremony(entry: xdr.SorobanAuthorizationEntry, contractId: string, networkPassphrase: string, validUntilLedger: number): CeremonyFile;
/** Parse a ceremony file's JSON text, validating its shape. */
export declare function readCeremony(json: string): CeremonyFile;
/** Serialize a ceremony file back to JSON text (pretty-printed for diffs). */
export declare function writeCeremony(ceremony: CeremonyFile): string;
/**
 * Add one signature to a ceremony, verifying it is over the ceremony's own
 * `payloadHex` before accepting it (a signature collected against a stale
 * payload -- e.g. after `validUntilLedger` was bumped -- must never be
 * silently merged in). Returns a new ceremony; `ceremony` is not mutated.
 */
export declare function addSignatureToCeremony(ceremony: CeremonyFile, publicKey: string, signature: Uint8Array): CeremonyFile;
/** Rebuild the signed `SorobanAuthorizationEntry` from a ceremony's
 * collected signatures, sorted and encoded via `attachSignatures`. */
export declare function finalizeCeremony(ceremony: CeremonyFile): xdr.SorobanAuthorizationEntry;
