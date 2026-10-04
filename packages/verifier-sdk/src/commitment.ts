/**
 * LRC-1 record commitment (ADR-0008), byte-for-byte compatible with
 * `crates/lafiya-commitment` and checked against its shared test vectors.
 * Uses WebCrypto, so it runs in browsers and Node alike.
 */
export type FieldValue =
  | { kind: "absent" }
  | { kind: "null" }
  | { kind: "text"; value: string }
  | { kind: "int64"; value: bigint }
  | { kind: "bool"; value: boolean }
  | { kind: "bytes"; value: Uint8Array };

const DOMAIN_TAG = new TextEncoder().encode("lafiya:record-commitment");
const VERSION_V1 = 0x01;

function u32be(n: number): number[] {
  return [(n >>> 24) & 0xff, (n >>> 16) & 0xff, (n >>> 8) & 0xff, n & 0xff];
}

/** Canonical LRC-1 payload for an ordered list of fields. */
export function encodePayload(fields: readonly FieldValue[]): Uint8Array {
  const out: number[] = [];
  for (const field of fields) {
    switch (field.kind) {
      case "absent":
        out.push(0x00);
        break;
      case "null":
        out.push(0x01);
        break;
      case "text": {
        // NFC is part of the scheme: visually identical strings must commit
        // identically.
        const utf8 = new TextEncoder().encode(field.value.normalize("NFC"));
        out.push(0x10, ...u32be(utf8.length), ...utf8);
        break;
      }
      case "int64": {
        const buf = new DataView(new ArrayBuffer(8));
        buf.setBigInt64(0, field.value, false);
        out.push(0x11, ...new Uint8Array(buf.buffer));
        break;
      }
      case "bool":
        out.push(0x12, field.value ? 1 : 0);
        break;
      case "bytes":
        out.push(0x13, ...u32be(field.value.length), ...field.value);
        break;
    }
  }
  return Uint8Array.from(out);
}

export function toHex(bytes: Uint8Array): string {
  return Array.from(bytes, (b) => b.toString(16).padStart(2, "0")).join("");
}

/** `SHA-256(DOMAIN_TAG || 0x01 || payload)` as lowercase hex. */
export async function commitV1(fields: readonly FieldValue[]): Promise<string> {
  const payload = encodePayload(fields);
  const input = new Uint8Array(DOMAIN_TAG.length + 1 + payload.length);
  input.set(DOMAIN_TAG, 0);
  input[DOMAIN_TAG.length] = VERSION_V1;
  input.set(payload, DOMAIN_TAG.length + 1);
  const digest = await globalThis.crypto.subtle.digest("SHA-256", input);
  return toHex(new Uint8Array(digest));
}
