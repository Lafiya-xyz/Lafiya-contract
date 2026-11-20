import { createHash, createPublicKey, verify } from "node:crypto";
import { readFileSync } from "node:fs";
import { describe, expect, it } from "vitest";
import * as api from "../dist/index.js";
import {
  P256_ORDER,
  decodeIntent,
  derToCompact,
  isLowS,
  normalizeLowS,
  parsePasskeyAssertion,
  passkeyChallenge,
  secp256r1PublicKeyFromSpki,
  webauthnSignedDigest,
} from "../dist/index.js";
import { fixtureEntry } from "./fixtures.js";

// Shared with contracts/multisig-account/src/passkey_vectors_test.rs, which
// checks that `normalized_compact_hex` is what __check_auth accepts and that
// the high-S `compact_hex` is rejected.
const VECTORS = JSON.parse(
  readFileSync(new URL("./vectors/passkey-vectors.json", import.meta.url), "utf8"),
);
const hex = (s) => Uint8Array.from(Buffer.from(s, "hex"));
const toHex = (b) => Buffer.from(b).toString("hex");
const webauthnVectors = VECTORS.vectors.filter((v) => v.kind === "webauthn");

function assertionFor(vector, overrides = {}) {
  return {
    authenticatorData: hex(vector.authenticator_data_hex),
    clientDataJSON: Uint8Array.from(Buffer.from(vector.client_data_json)),
    signature: hex(vector.der_hex),
    ...overrides,
  };
}

describe("DER and low-S handling (shared vectors)", () => {
  it.each(VECTORS.vectors.map((v) => [v.name, v]))("%s", (_name, vector) => {
    const compact = derToCompact(hex(vector.der_hex));
    expect(toHex(compact)).toBe(vector.compact_hex);
    expect(isLowS(compact)).toBe(vector.low_s);
    expect(toHex(normalizeLowS(compact))).toBe(vector.normalized_compact_hex);
  });

  it("high-S and low-S forms are both valid ECDSA, so normalization is required", () => {
    const key = createPublicKey({
      key: Buffer.from(VECTORS.public_key_spki_hex, "hex"),
      format: "der",
      type: "spki",
    });
    for (const vector of webauthnVectors) {
      // What the authenticator signs (node hashes it once more with SHA-256).
      const message = Buffer.concat([
        hex(vector.authenticator_data_hex),
        createHash("sha256").update(vector.client_data_json).digest(),
      ]);
      expect(toHex(createHash("sha256").update(message).digest())).toBe(vector.signed_digest_hex);
      for (const sig of [vector.compact_hex, vector.normalized_compact_hex]) {
        expect(verify("sha256", message, { key, dsaEncoding: "ieee-p1363" }, hex(sig))).toBe(true);
      }
    }
  });

  it("normalization is idempotent and maps s to n - s", () => {
    const high = VECTORS.vectors.find((v) => !v.low_s);
    const compact = hex(high.compact_hex);
    const once = normalizeLowS(compact);
    expect(toHex(normalizeLowS(once))).toBe(toHex(once));
    const s = BigInt("0x" + toHex(compact.subarray(32)));
    const sLow = BigInt("0x" + toHex(once.subarray(32)));
    expect(s + sLow).toBe(P256_ORDER);
  });

  it("rejects malformed DER", () => {
    const der = hex(VECTORS.vectors[0].der_hex);
    const bad = [
      new Uint8Array([0x31, ...der.subarray(1)]), // not a SEQUENCE
      Uint8Array.from([...der, 0x00]), // trailing byte
      der.subarray(0, der.length - 1), // truncated
      hex("3006020100020101"), // r = 0
      hex("30070202007f020101"), // non-minimal r
      hex("3006020181020101"), // negative r
    ];
    for (const sig of bad) {
      expect(() => derToCompact(sig)).toThrow(/invalid DER/);
    }
  });
});

// Builds a DER ECDSA signature from raw 32-byte r and s, applying the minimal
// INTEGER encoding (strip leading zeros, re-add 0x00 when the high bit is set).
function derFromRaw(r, s) {
  const encodeInt = (bytes) => {
    let i = 0;
    while (i < bytes.length - 1 && bytes[i] === 0) i++;
    let body = Array.from(bytes.subarray(i));
    if (body[0] & 0x80) body = [0x00, ...body];
    return [0x02, body.length, ...body];
  };
  const rEnc = encodeInt(r);
  const sEnc = encodeInt(s);
  const body = [...rEnc, ...sEnc];
  return Uint8Array.from([0x30, body.length, ...body]);
}

function rawFromCompact(compact) {
  return { r: compact.subarray(0, 32), s: compact.subarray(32) };
}

function bigTo32(n) {
  const out = new Uint8Array(32);
  let v = n;
  for (let i = 31; i >= 0; i--) {
    out[i] = Number(v & 0xffn);
    v >>= 8n;
  }
  return out;
}

describe("DER edge cases (high-S, short-r, leading zero)", () => {
  const named = (name) => {
    const v = VECTORS.vectors.find((x) => x.name === name);
    expect(v, `missing vector ${name}`).toBeDefined();
    return v;
  };

  it("normalizes a high-S vector to low-S", () => {
    const v = named("webauthn-high-s");
    const compact = derToCompact(hex(v.der_hex));
    expect(toHex(compact)).toBe(v.compact_hex);
    expect(isLowS(compact)).toBe(false);
    const normalized = normalizeLowS(compact);
    expect(toHex(normalized)).toBe(v.normalized_compact_hex);
    expect(isLowS(normalized)).toBe(true);
  });

  it("handles a short-r vector (r < 2^248, encoded with fewer bytes)", () => {
    const v = named("webauthn-short-r");
    const compact = derToCompact(hex(v.der_hex));
    expect(toHex(compact)).toBe(v.compact_hex);
    // r must be left-padded to 32 bytes in the compact form.
    expect(compact[0]).toBe(0);
    expect(toHex(normalizeLowS(compact))).toBe(v.normalized_compact_hex);
  });

  it("handles a leading-zero vector (r with 0x00 pad byte in DER)", () => {
    const v = named("webauthn-leading-zero");
    const compact = derToCompact(hex(v.der_hex));
    expect(toHex(compact)).toBe(v.compact_hex);
    // The DER INTEGER carries a 0x00 sign pad; the compact r keeps the value.
    expect(hex(v.der_hex)[4]).toBe(0x00);
    expect(toHex(normalizeLowS(compact))).toBe(v.normalized_compact_hex);
  });

  it("round-trips raw r/s through DER for every vector", () => {
    for (const v of VECTORS.vectors) {
      const compact = hex(v.compact_hex);
      const { r, s } = rawFromCompact(compact);
      const der = derFromRaw(r, s);
      expect(toHex(derToCompact(der))).toBe(v.compact_hex);
    }
  });

  it("pads short r and s to 32 bytes each", () => {
    const r = bigTo32(0x7fn);
    const s = bigTo32(0x01n);
    const der = derFromRaw(r, s);
    const compact = derToCompact(der);
    expect(compact.length).toBe(64);
    expect(toHex(compact.subarray(0, 32))).toBe(toHex(r));
    expect(toHex(compact.subarray(32))).toBe(toHex(s));
  });
});

describe("low-S normalization property", () => {
  // Deterministic PRNG so failures are reproducible.
  function mulberry32(seed) {
    let a = seed >>> 0;
    return () => {
      a = (a + 0x6d2b79f5) >>> 0;
      let t = a;
      t = Math.imul(t ^ (t >>> 15), t | 1);
      t ^= t + Math.imul(t ^ (t >>> 7), t | 61);
      return ((t ^ (t >>> 14)) >>> 0) / 4294967296;
    };
  }

  const HALF_ORDER = P256_ORDER >> 1n;

  function randomScalar(rand) {
    // Uniform-ish scalar in [1, n-1].
    for (;;) {
      let n = 0n;
      for (let i = 0; i < 32; i++) n = (n << 8n) | BigInt(Math.floor(rand() * 256));
      if (n > 0n && n < P256_ORDER) return n;
    }
  }

  it("always produces a low-S signature", () => {
    const rand = mulberry32(0x5eed);
    for (let i = 0; i < 500; i++) {
      const r = randomScalar(rand);
      const s = randomScalar(rand);
      const compact = new Uint8Array([...bigTo32(r), ...bigTo32(s)]);
      const normalized = normalizeLowS(compact);
      expect(normalized.length).toBe(64);
      expect(isLowS(normalized)).toBe(true);
      const sOut = BigInt("0x" + toHex(normalized.subarray(32)));
      expect(sOut).toBeLessThanOrEqual(HALF_ORDER);
    }
  });

  it("maps high-S inputs to the same compact form as their low-S counterparts", () => {
    const rand = mulberry32(0xc0ffee);
    for (let i = 0; i < 500; i++) {
      const r = randomScalar(rand);
      const s = randomScalar(rand);
      const sLow = s > HALF_ORDER ? P256_ORDER - s : s;
      const sHigh = s > HALF_ORDER ? s : P256_ORDER - s;
      const lowCompact = new Uint8Array([...bigTo32(r), ...bigTo32(sLow)]);
      const highCompact = new Uint8Array([...bigTo32(r), ...bigTo32(sHigh)]);
      expect(isLowS(lowCompact)).toBe(true);
      expect(isLowS(highCompact)).toBe(false);
      expect(toHex(normalizeLowS(highCompact))).toBe(toHex(normalizeLowS(lowCompact)));
      expect(toHex(normalizeLowS(highCompact))).toBe(toHex(lowCompact));
    }
  });

  it("normalizes DER-encoded random signatures to low-S", () => {
    const rand = mulberry32(0xdeadbeef);
    for (let i = 0; i < 200; i++) {
      const r = randomScalar(rand);
      const s = randomScalar(rand);
      const der = derFromRaw(bigTo32(r), bigTo32(s));
      const normalized = normalizeLowS(derToCompact(der));
      expect(isLowS(normalized)).toBe(true);
    }
  });
});

describe("parsePasskeyAssertion", () => {
  const [vector] = webauthnVectors;
  const payload = hex(vector.payload_hex);

  it("returns the compact low-S signature and the signed digest", () => {
    for (const v of webauthnVectors) {
      const parsed = parsePasskeyAssertion(assertionFor(v), hex(v.payload_hex), {
        origin: v.origin,
        rpId: v.rp_id,
        requireUserVerification: true,
      });
      expect(toHex(parsed.signature)).toBe(v.normalized_compact_hex);
      expect(parsed.normalized).toBe(!v.low_s);
      expect(toHex(parsed.signedDigest)).toBe(v.signed_digest_hex);
      expect(parsed.userVerified).toBe(true);
      expect(parsed.signCount).toBe(1);
    }
  });

  it("uses the auth payload as the challenge", () => {
    expect(toHex(passkeyChallenge(payload))).toBe(vector.payload_hex);
    expect(() => passkeyChallenge(payload.subarray(1))).toThrow(/32 bytes/);
  });

  it("rejects an assertion made for a different payload", () => {
    const other = Uint8Array.from(payload);
    other[0] ^= 1;
    expect(() => parsePasskeyAssertion(assertionFor(vector), other)).toThrow(/challenge/);
  });

  it("rejects wrong origin, rpId, type, and missing flags", () => {
    expect(() =>
      parsePasskeyAssertion(assertionFor(vector), payload, { origin: "https://evil.example" }),
    ).toThrow(/origin/);
    expect(() =>
      parsePasskeyAssertion(assertionFor(vector), payload, { rpId: "evil.example" }),
    ).toThrow(/rpIdHash/);

    const created = vector.client_data_json.replace("webauthn.get", "webauthn.create");
    expect(() =>
      parsePasskeyAssertion(
        assertionFor(vector, { clientDataJSON: Uint8Array.from(Buffer.from(created)) }),
        payload,
      ),
    ).toThrow(/type/);

    const noFlags = hex(vector.authenticator_data_hex);
    noFlags[32] = 0;
    expect(() =>
      parsePasskeyAssertion(assertionFor(vector, { authenticatorData: noFlags }), payload),
    ).toThrow(/user-present/);

    const presentOnly = hex(vector.authenticator_data_hex);
    presentOnly[32] = 0x01;
    expect(() =>
      parsePasskeyAssertion(assertionFor(vector, { authenticatorData: presentOnly }), payload, {
        requireUserVerification: true,
      }),
    ).toThrow(/user verification/);
  });
});

describe("public keys", () => {
  it("extracts the contract's 65-byte key from registration SPKI", () => {
    const key = secp256r1PublicKeyFromSpki(hex(VECTORS.public_key_spki_hex));
    expect(toHex(key)).toBe(VECTORS.public_key_uncompressed_hex);
    expect(() => secp256r1PublicKeyFromSpki(key)).toThrow(/P-256 SPKI/);
  });

  it("exposes no API that accepts or returns private key material", () => {
    const names = Object.keys(api).join(" ");
    expect(names).not.toMatch(/private|secret|seed|keypair/i);
  });
});

describe("decodeIntent", () => {
  it("decodes the invocation a signer is approving", () => {
    const entry = fixtureEntry();
    const intent = decodeIntent(entry, 1000);
    expect(intent.validUntilLedger).toBe(1000);
    expect(intent.signer).toMatch(/^G/);
    expect(intent.invocation.function).toBe("add_attester");
    expect(intent.invocation.subInvocations).toEqual([]);
  });
});
