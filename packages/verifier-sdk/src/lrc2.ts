/**
 * LRC-2 selective-disclosure verification (ADR-0023), byte-for-byte
 * compatible with `crates/lafiya-commitment/src/lrc2.rs` and checked against
 * the shared fixture `crates/lafiya-commitment/vectors/lrc2-test-vectors.json`.
 *
 *   leaf_i = SHA-256("lafiya:lrc2:leaf" || u8(i) || salt_i[32] || encodeLrc1(field_i))
 *   root   = SHA-256("lafiya:lrc2:root" || leaf_0 || leaf_1 || … || leaf_{n-1})
 *
 * Re-uses the existing LRC-1 encoder and WebCrypto SHA-256 path; no new
 * runtime dependencies.
 */
import { type FieldValue, encodePayload, toHex } from "./commitment.js";

const DOMAIN_LEAF = new TextEncoder().encode("lafiya:lrc2:leaf");
const DOMAIN_ROOT = new TextEncoder().encode("lafiya:lrc2:root");

/** LRC-2 version byte carried in application-layer contexts (e.g. QR). */
export const VERSION_LRC2 = 0x02;

/** A single disclosed field together with its position and salt. */
export interface Disclosure {
  /** Zero-based field index within the schema (0..=255). */
  index: number;
  /** Per-field random salt, exactly 32 bytes. */
  salt: Uint8Array;
  /** The disclosed field value. */
  field: FieldValue;
}

/** Closed set of disclosure-verification failures. */
export type DisclosureErrorKind =
  | "invalid-salt-length"
  | "invalid-index"
  | "duplicate-index"
  | "invalid-leaf-length"
  | "invalid-root-length"
  | "empty-proof"
  | "too-many-fields"
  | "index-out-of-range"
  | "root-mismatch";

export class DisclosureError extends Error {
  readonly kind: DisclosureErrorKind;
  constructor(kind: DisclosureErrorKind, message: string) {
    super(message);
    this.name = "DisclosureError";
    this.kind = kind;
  }
}

async function sha256(input: Uint8Array): Promise<Uint8Array> {
  const digest = await globalThis.crypto.subtle.digest("SHA-256", input);
  return new Uint8Array(digest);
}

/** `leaf_i = SHA-256(DOMAIN_LEAF || u8(i) || salt || encodeLrc1(field))`. */
export async function makeLeaf(
  index: number,
  salt: Uint8Array,
  field: FieldValue,
): Promise<Uint8Array> {
  if (!Number.isInteger(index) || index < 0 || index > 255) {
    throw new DisclosureError("invalid-index", `field index ${index} is not a u8`);
  }
  if (salt.length !== 32) {
    throw new DisclosureError(
      "invalid-salt-length",
      `salt must be 32 bytes, got ${salt.length}`,
    );
  }
  const encoded = encodePayload([field]);
  const input = new Uint8Array(DOMAIN_LEAF.length + 1 + 32 + encoded.length);
  input.set(DOMAIN_LEAF, 0);
  input[DOMAIN_LEAF.length] = index;
  input.set(salt, DOMAIN_LEAF.length + 1);
  input.set(encoded, DOMAIN_LEAF.length + 1 + 32);
  return sha256(input);
}

/** `root = SHA-256(DOMAIN_ROOT || leaf_0 || … || leaf_{n-1})`. */
export async function makeRoot(leaves: readonly Uint8Array[]): Promise<Uint8Array> {
  const input = new Uint8Array(DOMAIN_ROOT.length + leaves.length * 32);
  input.set(DOMAIN_ROOT, 0);
  leaves.forEach((leaf, i) => {
    if (leaf.length !== 32) {
      throw new DisclosureError(
        "invalid-leaf-length",
        `leaf[${i}] must be 32 bytes, got ${leaf.length}`,
      );
    }
    input.set(leaf, DOMAIN_ROOT.length + i * 32);
  });
  return sha256(input);
}

/** Compute the LRC-2 root and per-field leaves for a full record. */
export async function commitLrc2(
  fields: readonly FieldValue[],
  salts: readonly Uint8Array[],
): Promise<{ root: Uint8Array; leaves: Uint8Array[] }> {
  if (fields.length !== salts.length) {
    throw new DisclosureError(
      "invalid-salt-length",
      `fields (${fields.length}) and salts (${salts.length}) must have the same length`,
    );
  }
  if (fields.length === 0) {
    throw new DisclosureError("empty-proof", "field list must not be empty");
  }
  if (fields.length > 256) {
    throw new DisclosureError("too-many-fields", "LRC-2 supports at most 256 fields");
  }
  const leaves: Uint8Array[] = [];
  for (let i = 0; i < fields.length; i++) {
    leaves.push(await makeLeaf(i, salts[i], fields[i]));
  }
  return { root: await makeRoot(leaves), leaves };
}

function bytesEqual(a: Uint8Array, b: Uint8Array): boolean {
  if (a.length !== b.length) return false;
  let diff = 0;
  for (let i = 0; i < a.length; i++) diff |= a[i] ^ b[i];
  return diff === 0;
}

/**
 * Verify a selective-disclosure proof against a known root.
 *
 * `undisclosedLeaves` are the raw leaf hashes of the fields that were *not*
 * disclosed, in ascending index order; the disclosed indices fill the gaps.
 *
 * Resolves `true` iff the reconstructed root matches `root`. Throws
 * `DisclosureError` for structurally invalid proofs (see
 * `DisclosureErrorKind`). A root mismatch is reported as
 * `kind: "root-mismatch"` rather than `false` so callers cannot confuse a
 * forged proof with a malformed one.
 *
 * Security note: this only proves the disclosed fields are consistent with
 * the attested root. The caller must also confirm the root has a valid
 * on-chain attestation.
 */
export async function verifyDisclosure(
  root: Uint8Array,
  disclosedFields: readonly Disclosure[],
  undisclosedLeaves: readonly Uint8Array[],
): Promise<boolean> {
  if (root.length !== 32) {
    throw new DisclosureError("invalid-root-length", `root must be 32 bytes, got ${root.length}`);
  }
  const total = disclosedFields.length + undisclosedLeaves.length;
  if (total === 0) {
    throw new DisclosureError("empty-proof", "proof discloses and hides nothing");
  }
  if (total > 256) {
    throw new DisclosureError("too-many-fields", "LRC-2 supports at most 256 fields");
  }

  const leafMap = new Map<number, Uint8Array>();
  for (const d of disclosedFields) {
    if (!Number.isInteger(d.index) || d.index < 0 || d.index > 255) {
      throw new DisclosureError("invalid-index", `field index ${d.index} is not a u8`);
    }
    if (leafMap.has(d.index)) {
      throw new DisclosureError("duplicate-index", `field index ${d.index} disclosed twice`);
    }
    leafMap.set(d.index, await makeLeaf(d.index, d.salt, d.field));
  }

  const undisclosedIndices: number[] = [];
  for (let i = 0; i < total; i++) {
    if (!leafMap.has(i)) undisclosedIndices.push(i);
  }
  if (undisclosedIndices.length !== undisclosedLeaves.length) {
    throw new DisclosureError(
      "index-out-of-range",
      `disclosed indices leave ${undisclosedIndices.length} gaps but ${undisclosedLeaves.length} hidden leaves were supplied`,
    );
  }
  undisclosedIndices.forEach((idx, i) => {
    const leaf = undisclosedLeaves[i];
    if (leaf.length !== 32) {
      throw new DisclosureError(
        "invalid-leaf-length",
        `undisclosed leaf[${i}] must be 32 bytes, got ${leaf.length}`,
      );
    }
    leafMap.set(idx, leaf);
  });

  const ordered: Uint8Array[] = [];
  for (let i = 0; i < total; i++) ordered.push(leafMap.get(i)!);
  const recomputed = await makeRoot(ordered);
  if (!bytesEqual(recomputed, root)) {
    throw new DisclosureError("root-mismatch", "recomputed root does not match attested root");
  }
  return true;
}

/** Lowercase hex helper re-exported for vector fixtures. */
export { toHex };
