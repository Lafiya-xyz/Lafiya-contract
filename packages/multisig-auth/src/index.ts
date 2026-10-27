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
function buildAuthorizationEntryPreimage(
  entry: xdr.SorobanAuthorizationEntry,
  validUntilLedgerSeq: number,
  networkPassphrase: string,
): xdr.HashIdPreimage {
  const credentials = entry.credentials();
  if (
    credentials.switch().value !==
    xdr.SorobanCredentialsType.sorobanCredentialsAddress().value
  ) {
    throw new Error(
      `buildAuthPayload: unsupported credential type ${credentials.switch().name} (only ADDRESS is supported)`,
    );
  }
  const addrAuth = credentials.address();
  const networkId = hash(Buffer.from(networkPassphrase));
  return xdr.HashIdPreimage.envelopeTypeSorobanAuthorization(
    new xdr.HashIdPreimageSorobanAuthorization({
      networkId,
      nonce: addrAuth.nonce(),
      invocation: entry.rootInvocation(),
      signatureExpirationLedger: validUntilLedgerSeq,
    }),
  );
}

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
export function buildAuthPayload(
  entry: xdr.SorobanAuthorizationEntry,
  networkPassphrase: string,
  validUntilLedger: number,
): Uint8Array {
  const preimage = buildAuthorizationEntryPreimage(
    entry,
    validUntilLedger,
    networkPassphrase,
  );
  return hash(preimage.toXDR());
}

/**
 * Sort signatures by ascending public key (matching the exact byte-wise
 * comparison `__check_auth` performs on `BytesN<32>`, not string/locale
 * order on the G... strkey), and drop exact duplicates. Ordering signatures
 * any other way is precisely what produces `BadSignatureOrder`.
 */
function sortAndDedupe(sigs: CollectedSignature[]): CollectedSignature[] {
  const byPublicKey = new Map<string, CollectedSignature>();
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
} as const;

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
export function attachSignatures(
  entry: xdr.SorobanAuthorizationEntry,
  sigs: CollectedSignature[],
): xdr.SorobanAuthorizationEntry {
  if (
    entry.credentials().switch().value ===
    xdr.SorobanCredentialsType.sorobanCredentialsSourceAccount().value
  ) {
    return entry;
  }

  const clone = xdr.SorobanAuthorizationEntry.fromXDR(entry.toXDR());
  const ordered = sortAndDedupe(sigs);

  const sigScVals = ordered.map((sig) =>
    nativeToScVal(
      {
        public_key: StrKey.decodeEd25519PublicKey(sig.publicKey),
        signature: Buffer.from(sig.signature),
      },
      SIGNATURE_STRUCT_TYPE,
    ),
  );
  const signatureScVal = xdr.ScVal.scvVec(sigScVals);

  const credentials = clone.credentials();
  if (
    credentials.switch().value !==
    xdr.SorobanCredentialsType.sorobanCredentialsAddress().value
  ) {
    throw new Error(
      `attachSignatures: unsupported credential type ${credentials.switch().name} (only ADDRESS is supported)`,
    );
  }
  credentials.address().signature(signatureScVal);
  return clone;
}

// ---------------------------------------------------------------------------
// Ceremony files
//
// A signing ceremony collects N-of-M signatures over one payload across
// multiple signers (and, for a browser admin console, possibly multiple
// sessions) before the authorized call is submitted. The ceremony file is
// the shared state that lets a CLI-based signer and a browser-based signer
// take part in the same ceremony: whichever side collects a signature
// appends it and hands the (JSON, human-diffable) file to the next signer.
//
// Air-gapped interop (#534): the CLI's `tx prepare` envelope and `tx sign
// --offline` detached signature bundle use this same ceremony shape, so a
// browser signer can pick up a file produced on a networked host, sign it on
// an offline device, and hand the result back -- or vice versa. The optional
// `envelope` block below carries the CLI envelope's self-describing metadata
// (contract id, expiry, human-readable decode) and a digest of the entry so
// an offline signer can detect tampering in transit before signing. Files
// without it remain valid (version 1, browser-only ceremonies).
// ---------------------------------------------------------------------------

/** Human-readable decode of what a ceremony authorizes, for offline review. */
export interface CeremonyDecode {
  /** The multisig-account contract the call targets. */
  contractId: string;
  /** The contract function being invoked. */
  functionName: string;
  /** Ledger sequence the authorization expires at. */
  validUntilLedger: number;
  /** Free-form, human-readable summary of the invocation arguments. */
  summary: string;
}

/**
 * Self-describing envelope metadata shared with the CLI's `tx prepare`
 * output. Present when the ceremony originated from (or is destined for) an
 * air-gapped CLI signing flow.
 */
export interface CeremonyEnvelope {
  /** Envelope format version, for forward compatibility. */
  version: 1;
  /** The multisig-account contract this ceremony authorizes a call on. */
  contractId: string;
  /** Network passphrase the payload hash was computed under. */
  networkPassphrase: string;
  /** Ledger sequence the authorization is valid until. */
  validUntilLedger: number;
  /** Human-readable decode of the invocation, for offline review. */
  decode: CeremonyDecode;
  /** hex-encoded SHA-256 of the raw `entryXdr` bytes, so an offline signer
   * can detect tampering with the entry in transit before signing. */
  entryDigestHex: string;
}

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
  signatures: Array<{ publicKey: string; signatureHex: string }>;
  /** Optional self-describing envelope metadata (air-gapped CLI interop). */
  envelope?: CeremonyEnvelope;
}

/** Compute the hex-encoded SHA-256 digest of an entry's raw XDR bytes. */
function entryDigestHex(entry: xdr.SorobanAuthorizationEntry): string {
  return Buffer.from(hash(entry.toXDR())).toString("hex");
}

/** Build a fresh, unsigned ceremony file for `entry`. */
export function createCeremony(
  entry: xdr.SorobanAuthorizationEntry,
  contractId: string,
  networkPassphrase: string,
  validUntilLedger: number,
  decode?: CeremonyDecode,
): CeremonyFile {
  const payload = buildAuthPayload(entry, networkPassphrase, validUntilLedger);
  const ceremony: CeremonyFile = {
    version: 1,
    contractId,
    networkPassphrase,
    validUntilLedger,
    entryXdr: entry.toXDR("base64"),
    payloadHex: Buffer.from(payload).toString("hex"),
    signatures: [],
  };
  if (decode) {
    ceremony.envelope = {
      version: 1,
      contractId,
      networkPassphrase,
      validUntilLedger,
      decode,
      entryDigestHex: entryDigestHex(entry),
    };
  }
  return ceremony;
}

/** Parse a ceremony file's JSON text, validating its shape. */
export function readCeremony(json: string): CeremonyFile {
  const parsed = JSON.parse(json) as Partial<CeremonyFile>;
  if (parsed.version !== 1) {
    throw new Error(
      `unsupported ceremony file version: ${String(parsed.version)} (expected 1)`,
    );
  }
  const required: (keyof CeremonyFile)[] = [
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
  if (parsed.envelope !== undefined) {
    const env = parsed.envelope as Partial<CeremonyEnvelope>;
    if (env.version !== 1) {
      throw new Error(
        `unsupported ceremony envelope version: ${String(env.version)} (expected 1)`,
      );
    }
    for (const field of [
      "contractId",
      "networkPassphrase",
      "validUntilLedger",
      "decode",
      "entryDigestHex",
    ] as const) {
      if (env[field] === undefined) {
        throw new Error(`ceremony envelope missing required field: ${field}`);
      }
    }
  }
  return parsed as CeremonyFile;
}

/** Serialize a ceremony file back to JSON text (pretty-printed for diffs). */
export function writeCeremony(ceremony: CeremonyFile): string {
  return JSON.stringify(ceremony, null, 2) + "\n";
}

/**
 * Re-decode a ceremony on the signing device and verify it is internally
 * consistent before signing: the entry must hash to the recorded
 * `entryDigestHex`, the payload must re-derive from the entry, and the
 * envelope's contract/network/expiry must match the ceremony's own fields.
 * Throws on any mismatch, so tampering in transit is caught before a key is
 * ever used. Returns the ceremony unchanged when it has no envelope.
 */
export function verifyCeremony(ceremony: CeremonyFile): CeremonyFile {
  const entry = xdr.SorobanAuthorizationEntry.fromXDR(ceremony.entryXdr, "base64");
  const derivedPayload = buildAuthPayload(
    entry,
    ceremony.networkPassphrase,
    ceremony.validUntilLedger,
  );
  if (Buffer.from(derivedPayload).toString("hex") !== ceremony.payloadHex) {
    throw new Error(
      "ceremony payloadHex does not match the entry (tampered or stale ceremony)",
    );
  }
  const envelope = ceremony.envelope;
  if (!envelope) {
    return ceremony;
  }
  if (entryDigestHex(entry) !== envelope.entryDigestHex) {
    throw new Error(
      "ceremony entry digest does not match the envelope (tampered in transit)",
    );
  }
  if (
    envelope.contractId !== ceremony.contractId ||
    envelope.networkPassphrase !== ceremony.networkPassphrase ||
    envelope.validUntilLedger !== ceremony.validUntilLedger
  ) {
    throw new Error(
      "ceremony envelope metadata does not match the ceremony (tampered in transit)",
    );
  }
  return ceremony;
}

/**
 * Add one signature to a ceremony, verifying it is over the ceremony's own
 * `payloadHex` before accepting it (a signature collected against a stale
 * payload -- e.g. after `validUntilLedger` was bumped -- must never be
 * silently merged in). Returns a new ceremony; `ceremony` is not mutated.
 */
export function addSignatureToCeremony(
  ceremony: CeremonyFile,
  publicKey: string,
  signature: Uint8Array,
): CeremonyFile {
  const payload = Buffer.from(ceremony.payloadHex, "hex");
  if (!Keypair.fromPublicKey(publicKey).verify(payload, Buffer.from(signature))) {
    throw new Error(
      `signature from ${publicKey} does not verify against this ceremony's payload`,
    );
  }
  return {
    ...ceremony,
    signatures: [
      ...ceremony.signatures.filter((s) => s.publicKey !== publicKey),
      { publicKey, signatureHex: Buffer.from(signature).toString("hex") },
    ],
  };
}

/**
 * Merge a detached signature bundle (the CLI's `tx sign --offline` output,
 * which is a ceremony file carrying only `signatures`) into `ceremony`,
 * verifying each signature against the ceremony's payload first. The bundle
 * must describe the same entry and payload as `ceremony`. Returns a new
 * ceremony; neither input is mutated.
 */
export function mergeSignatureBundle(
  ceremony: CeremonyFile,
  bundle: CeremonyFile,
): CeremonyFile {
  if (
    bundle.entryXdr !== ceremony.entryXdr ||
    bundle.payloadHex !== ceremony.payloadHex
  ) {
    throw new Error(
      "signature bundle was produced for a different entry or payload",
    );
  }
  let merged = ceremony;
  for (const sig of bundle.signatures) {
    merged = addSignatureToCeremony(
      merged,
      sig.publicKey,
      Buffer.from(sig.signatureHex, "hex"),
    );
  }
  return merged;
}

/** Rebuild the signed `SorobanAuthorizationEntry` from a ceremony's
 * collected signatures, sorted and encoded via `attachSignatures`. */
export function finalizeCeremony(
  ceremony: CeremonyFile,
): xdr.SorobanAuthorizationEntry {
  const entry = xdr.SorobanAuthorizationEntry.fromXDR(ceremony.entryXdr, "base64");
  const sigs: CollectedSignature[] = ceremony.signatures.map((s) => ({
    publicKey: s.publicKey,
    signature: Buffer.from(s.signatureHex, "hex"),
  }));
  return attachSignatures(entry, sigs);
}

export * from "./passkey.js";
export * from "./intent.js";
