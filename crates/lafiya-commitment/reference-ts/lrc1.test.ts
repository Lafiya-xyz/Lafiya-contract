// Verifies the TypeScript reference implementation against the shared
// cross-language fixture. Run with `node lrc1.test.ts` — no build step or
// dependency (Node 22+ strips the type annotations natively).

import { readFileSync } from "node:fs";
import { fileURLToPath } from "node:url";
import assert from "node:assert/strict";
import { commitV1, encodePayload, toHex, type FieldValue } from "./lrc1.ts";

const vectorsPath = fileURLToPath(
  new URL("../vectors/lrc1-test-vectors.json", import.meta.url),
);

type JsonField =
  | { kind: "absent" }
  | { kind: "null" }
  | { kind: "text"; value: string }
  | { kind: "int64"; value: string }
  | { kind: "bool"; value: boolean }
  | { kind: "bytes"; value: string };

interface Vector {
  name: string;
  description: string;
  fields: JsonField[];
  payload_hex: string;
  commitment_hex: string;
}

function hexToBytes(hex: string): Uint8Array {
  const out = new Uint8Array(hex.length / 2);
  for (let i = 0; i < out.length; i++) {
    out[i] = parseInt(hex.slice(i * 2, i * 2 + 2), 16);
  }
  return out;
}

function toFieldValue(f: JsonField): FieldValue {
  switch (f.kind) {
    case "absent":
      return { kind: "absent" };
    case "null":
      return { kind: "null" };
    case "text":
      return { kind: "text", value: f.value };
    case "int64":
      return { kind: "int64", value: BigInt(f.value) };
    case "bool":
      return { kind: "bool", value: f.value };
    case "bytes":
      return { kind: "bytes", value: hexToBytes(f.value) };
  }
}

const vectors: Vector[] = JSON.parse(readFileSync(vectorsPath, "utf8"));
assert.ok(vectors.length > 0, "fixture must not be empty");

for (const vector of vectors) {
  const fields = vector.fields.map(toFieldValue);

  const payload = encodePayload(fields);
  assert.equal(
    toHex(payload),
    vector.payload_hex,
    `payload mismatch for vector \`${vector.name}\``,
  );

  const commitment = commitV1(fields);
  assert.equal(
    toHex(commitment),
    vector.commitment_hex,
    `commitment mismatch for vector \`${vector.name}\``,
  );
}

console.log(`ok: ${vectors.length} vectors matched`);
