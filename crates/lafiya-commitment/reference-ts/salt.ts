// Deterministic LRC-1 salt derivation (HKDF-SHA256) — TypeScript reference.
//
// Mirrors `crates/lafiya-commitment/src/salt.rs` byte for byte; specified in
// `docs/adr/0018-deterministic-salt-derivation.md`:
//
//   prk  = HKDF-Extract(salt = EXTRACT_SALT, ikm = masterSecret)
//   info = INFO_TAG || DERIVATION_V1 || u32be(len(recordId)) || recordId
//          || u32be(saltVersion)
//   salt = HKDF-Expand(prk, info, 32)
//
// Uses Node's built-in `hkdfSync`, an implementation independent of the
// hand-written HMAC in the Rust and Python references. Run the vectors with
// `node salt.test.ts` (Node 22+, no build step or dependency).

import { hkdfSync } from "node:crypto";

const encoder = new TextEncoder();

export const EXTRACT_SALT = encoder.encode("lafiya:salt-derivation:v1");
export const INFO_TAG = encoder.encode("lafiya:record-salt");
export const FINGERPRINT_INFO = encoder.encode("lafiya:master-secret-fingerprint");
export const DERIVATION_V1 = 0x01;

export const SALT_LEN = 32;
export const FINGERPRINT_LEN = 8;
export const MIN_MASTER_SECRET_LEN = 32;
export const MAX_MASTER_SECRET_LEN = 64;
export const MAX_RECORD_ID_LEN = 256;

export class SaltDerivationError extends Error {
  constructor(message: string) {
    super(message);
    this.name = "SaltDerivationError";
  }
}

function u32be(n: number): number[] {
  return [(n >>> 24) & 0xff, (n >>> 16) & 0xff, (n >>> 8) & 0xff, n & 0xff];
}

function checkMasterSecret(masterSecret: Uint8Array): void {
  if (masterSecret.length < MIN_MASTER_SECRET_LEN || masterSecret.length > MAX_MASTER_SECRET_LEN) {
    throw new SaltDerivationError(
      `master secret must be ${MIN_MASTER_SECRET_LEN}..=${MAX_MASTER_SECRET_LEN} bytes, got ${masterSecret.length}`,
    );
  }
}

/** The HKDF `info` for one record and salt version. */
export function saltInfo(recordId: Uint8Array, saltVersion: number): Uint8Array {
  if (recordId.length < 1 || recordId.length > MAX_RECORD_ID_LEN) {
    throw new SaltDerivationError(`record id must be 1..=${MAX_RECORD_ID_LEN} bytes, got ${recordId.length}`);
  }
  if (!Number.isInteger(saltVersion) || saltVersion < 1 || saltVersion > 0xffffffff) {
    throw new SaltDerivationError(`salt version must be 1..=2^32-1, got ${saltVersion}`);
  }
  return Uint8Array.from([
    ...INFO_TAG,
    DERIVATION_V1,
    ...u32be(recordId.length),
    ...recordId,
    ...u32be(saltVersion),
  ]);
}

/** The 32-byte LRC-1 salt for `recordId` at `saltVersion`. */
export function deriveSalt(masterSecret: Uint8Array, recordId: Uint8Array, saltVersion: number): Uint8Array {
  checkMasterSecret(masterSecret);
  const info = saltInfo(recordId, saltVersion);
  return new Uint8Array(hkdfSync("sha256", masterSecret, EXTRACT_SALT, info, SALT_LEN));
}

/** Public 8-byte id of a master secret, for telling rotated secrets apart. */
export function masterSecretFingerprint(masterSecret: Uint8Array): Uint8Array {
  checkMasterSecret(masterSecret);
  return new Uint8Array(hkdfSync("sha256", masterSecret, EXTRACT_SALT, FINGERPRINT_INFO, FINGERPRINT_LEN));
}
