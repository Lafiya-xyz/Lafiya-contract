// Lafiya Record Commitment v1 (LRC-1) — TypeScript reference implementation.
//
// This mirrors `crates/lafiya-commitment/src/lib.rs` field for field, byte
// for byte. See `../README.md` and `docs/adr/0008-record-commitment-canonicalization.md`
// for the specification this implements. Run with `node lrc1.ts` (Node 22+
// strips the type annotations natively — no build step or dependency).

import { createHash } from "node:crypto";

/** A single record field in its fixed schema position. */
export type FieldValue =
  | { kind: "absent" }
  | { kind: "null" }
  | { kind: "text"; value: string }
  // Represented as bigint (not number) so values above 2^53 cannot silently
  // lose precision the way a plain JS number would.
  | { kind: "int64"; value: bigint }
  | { kind: "bool"; value: boolean }
  | { kind: "bytes"; value: Uint8Array };

const TAG_ABSENT = 0x00;
const TAG_NULL = 0x01;
const TAG_TEXT = 0x10;
const TAG_INT64 = 0x11;
const TAG_BOOL = 0x12;
const TAG_BYTES = 0x13;

/** Domain-separation tag mixed into every LRC-1 commitment. */
export const DOMAIN_TAG: Uint8Array = new TextEncoder().encode(
  "lafiya:record-commitment",
);

/** Canonicalization scheme version for this encoding. */
export const VERSION_V1 = 0x01;

/**
 * Reserved version byte for commitments recorded before LRC-1 existed.
 * Never assigned to a canonicalization scheme — see the ADR's
 * "Existing commitment compatibility" section.
 */
export const VERSION_LEGACY_UNVERSIONED = 0x00;

function concat(...parts: Uint8Array[]): Uint8Array {
  const total = parts.reduce((sum, part) => sum + part.length, 0);
  const out = new Uint8Array(total);
  let offset = 0;
  for (const part of parts) {
    out.set(part, offset);
    offset += part.length;
  }
  return out;
}

function u32be(n: number): Uint8Array {
  const buf = new Uint8Array(4);
  new DataView(buf.buffer).setUint32(0, n, false);
  return buf;
}

function i64be(n: bigint): Uint8Array {
  const buf = new Uint8Array(8);
  new DataView(buf.buffer).setBigInt64(0, n, false);
  return buf;
}

function encodeField(field: FieldValue): Uint8Array {
  switch (field.kind) {
    case "absent":
      return Uint8Array.of(TAG_ABSENT);
    case "null":
      return Uint8Array.of(TAG_NULL);
    case "text": {
      // Callers MUST supply Unicode Normalization Form C; this is a no-op
      // for already-normalized input and guards against silently emitting
      // a non-canonical form.
      const utf8 = new TextEncoder().encode(field.value.normalize("NFC"));
      return concat(Uint8Array.of(TAG_TEXT), u32be(utf8.length), utf8);
    }
    case "int64":
      return concat(Uint8Array.of(TAG_INT64), i64be(field.value));
    case "bool":
      return concat(Uint8Array.of(TAG_BOOL), Uint8Array.of(field.value ? 1 : 0));
    case "bytes":
      return concat(
        Uint8Array.of(TAG_BYTES),
        u32be(field.value.length),
        field.value,
      );
  }
}

/**
 * Encode an ordered list of record fields into the LRC-1 canonical payload.
 * Field order is fixed by the schema, not sorted — callers must always pass
 * fields in the same schema-defined order.
 */
export function encodePayload(fields: FieldValue[]): Uint8Array {
  return concat(...fields.map(encodeField));
}

/** Compute the LRC-1 commitment: SHA-256(DOMAIN_TAG || VERSION_V1 || payload). */
export function commitV1(fields: FieldValue[]): Uint8Array {
  const payload = encodePayload(fields);
  const hash = createHash("sha256");
  hash.update(Buffer.from(DOMAIN_TAG));
  hash.update(Buffer.from([VERSION_V1]));
  hash.update(Buffer.from(payload));
  return new Uint8Array(hash.digest());
}

export function toHex(bytes: Uint8Array): string {
  return Buffer.from(bytes).toString("hex");
}
