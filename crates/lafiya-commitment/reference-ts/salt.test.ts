// Checks the TypeScript salt derivation against the shared fixture
// `vectors/salt-derivation-vectors.json`, which `src/salt.rs` and
// `reference-py/salt_test.py` check too. Run with `node salt.test.ts`
// (Node 22+ strips the type annotations natively).

import { hkdfSync } from "node:crypto";
import { readFileSync } from "node:fs";
import { fileURLToPath } from "node:url";
import assert from "node:assert/strict";
import { commitV1, toHex, type FieldValue } from "./lrc1.ts";
import { deriveSalt, masterSecretFingerprint, saltInfo, SaltDerivationError } from "./salt.ts";

const vectorsPath = fileURLToPath(new URL("../vectors/salt-derivation-vectors.json", import.meta.url));
const doc = JSON.parse(readFileSync(vectorsPath, "utf8"));

const hex = (s: string) => Uint8Array.from(s.match(/../g) ?? [], (b) => parseInt(b, 16));
const utf8 = (s: string) => new TextEncoder().encode(s);

function toField(f: { kind: string; value: string }): FieldValue {
  if (f.kind === "text") return { kind: "text", value: f.value };
  if (f.kind === "int64") return { kind: "int64", value: BigInt(f.value) };
  throw new Error(`unsupported field kind in salt vectors: ${f.kind}`);
}

for (const tc of doc.hkdf_rfc5869) {
  const okm = new Uint8Array(hkdfSync("sha256", hex(tc.ikm_hex), hex(tc.salt_hex), hex(tc.info_hex), tc.length));
  assert.equal(toHex(okm), tc.okm_hex, `HKDF mismatch for ${tc.name}`);
}

for (const v of doc.vectors) {
  const secret = hex(v.master_secret_hex);
  const recordId = utf8(v.record_id);
  assert.equal(toHex(saltInfo(recordId, v.salt_version)), v.info_hex, `info: ${v.name}`);
  const salt = deriveSalt(secret, recordId, v.salt_version);
  assert.equal(toHex(salt), v.salt_hex, `salt: ${v.name}`);
  assert.equal(toHex(masterSecretFingerprint(secret)), v.fingerprint_hex, `fingerprint: ${v.name}`);
  if (v.commitment_hex !== undefined) {
    // commit_v1_salted: the salt is a Bytes field prepended to the record.
    const fields: FieldValue[] = [{ kind: "bytes", value: salt }, ...v.fields.map(toField)];
    assert.equal(toHex(commitV1(fields)), v.commitment_hex, `commitment: ${v.name}`);
  }
}

for (const v of doc.invalid) {
  assert.throws(
    () => deriveSalt(hex(v.master_secret_hex), utf8(v.record_id), v.salt_version),
    SaltDerivationError,
    `invalid vector ${v.name} was accepted`,
  );
}

console.log(
  `ok: ${doc.hkdf_rfc5869.length} HKDF, ${doc.vectors.length} derivation and ${doc.invalid.length} invalid vectors matched`,
);
