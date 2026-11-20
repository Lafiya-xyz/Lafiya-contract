import { describe, expect, it } from "vitest";
import {
  decodeQrChunks,
  encodeQrPayload,
  evaluateFacilityTrust,
  loadTrustAnchors,
  parseQrPayload,
  QrPayloadError,
  TrustError,
} from "../src/index.js";
import { ATTESTER_REGISTRY, FACILITY_ID, REGISTRY, TRUST } from "./helpers.js";

const HASH = "ab".repeat(32);
const NOW = 1_700_000_000n;

describe("QR payload", () => {
  it("round-trips", () => {
    const payload = { network: "testnet", contractId: REGISTRY, recordHash: HASH, attestedAt: 42n };
    expect(parseQrPayload(encodeQrPayload(payload))).toEqual(payload);
  });

  it.each([
    "https://example.com",
    `lafiya:v1?network=testnet&contract=${REGISTRY}&record=zz`,
    `lafiya:v1?network=testnet&contract=GABC&record=${HASH}`,
    `lafiya:v1?contract=${REGISTRY}&record=${HASH}`,
    `lafiya:v1?network=testnet&contract=${REGISTRY}&record=${HASH}&attested=-1`,
  ])("rejects %s", (text) => {
    expect(() => parseQrPayload(text)).toThrow(QrPayloadError);
  });

  it("round-trips freshness fields", () => {
    const payload = {
      network: "testnet",
      contractId: REGISTRY,
      recordHash: HASH,
      attestedAt: 42n,
      issuedAt: 100n,
      validUntil: 200n,
    };
    expect(parseQrPayload(encodeQrPayload(payload))).toEqual(payload);
  });

  it("omits freshness fields when absent", () => {
    const text = encodeQrPayload({ network: "testnet", contractId: REGISTRY, recordHash: HASH, attestedAt: 42n });
    expect(text).not.toContain("issued");
    expect(text).not.toContain("valid_until");
  });

  it.each([
    `lafiya:v1?network=testnet&contract=${REGISTRY}&record=${HASH}&attested=42&issued=-1`,
    `lafiya:v1?network=testnet&contract=${REGISTRY}&record=${HASH}&attested=42&valid_until=abc`,
    `lafiya:v1?network=testnet&contract=${REGISTRY}&record=${HASH}&attested=42&issued=200&valid_until=100`,
  ])("rejects malformed freshness %s", (text) => {
    expect(() => parseQrPayload(text)).toThrow(QrPayloadError);
  });

  it("round-trips a portable facility id", () => {
    const payload = {
      network: "testnet",
      contractId: REGISTRY,
      recordHash: HASH,
      attestedAt: 42n,
      facilityId: FACILITY_ID,
    };
    expect(parseQrPayload(encodeQrPayload(payload))).toEqual(payload);
  });

  it("omits the facility id when absent", () => {
    const text = encodeQrPayload({ network: "testnet", contractId: REGISTRY, recordHash: HASH, attestedAt: 42n });
    expect(text).not.toContain("facility");
  });

  it.each([
    `lafiya:v1?network=testnet&contract=${REGISTRY}&record=${HASH}&attested=42&facility=zz`,
    `lafiya:v1?network=testnet&contract=${REGISTRY}&record=${HASH}&attested=42&facility=${"ab".repeat(31)}`,
    `lafiya:v1?network=testnet&contract=${REGISTRY}&record=${HASH}&attested=42&facility=${"AB".repeat(32)}`,
  ])("rejects malformed facility id %s", (text) => {
    expect(() => parseQrPayload(text)).toThrow(QrPayloadError);
  });
});

describe("QR freshness policy", () => {
  const base = { network: "testnet", contractId: REGISTRY, recordHash: HASH, attestedAt: 42n };

  it("treats missing freshness fields as unknown", () => {
    const result = parseQrPayload(encodeQrPayload(base), { now: NOW, staleness: "warn" });
    expect(result.freshness).toBe("unknown");
  });

  it("accepts a payload inside its validity window", () => {
    const text = encodeQrPayload({ ...base, issuedAt: NOW - 10n, validUntil: NOW + 10n });
    const result = parseQrPayload(text, { now: NOW, staleness: "hard-fail" });
    expect(result.freshness).toBe("fresh");
  });

  it("flags an expired payload", () => {
    const text = encodeQrPayload({ ...base, issuedAt: NOW - 20n, validUntil: NOW - 10n });
    expect(parseQrPayload(text, { now: NOW, staleness: "warn" }).freshness).toBe("stale");
    expect(parseQrPayload(text, { now: NOW, staleness: "soft-fail" }).freshness).toBe("stale");
    expect(() => parseQrPayload(text, { now: NOW, staleness: "hard-fail" })).toThrow(QrPayloadError);
  });

  it("flags a future-dated payload", () => {
    const text = encodeQrPayload({ ...base, issuedAt: NOW + 10n, validUntil: NOW + 20n });
    expect(parseQrPayload(text, { now: NOW, staleness: "warn" }).freshness).toBe("future");
    expect(() => parseQrPayload(text, { now: NOW, staleness: "hard-fail" })).toThrow(QrPayloadError);
  });

  it("does not hard-fail when the clock is unreliable", () => {
    const text = encodeQrPayload({ ...base, issuedAt: NOW - 20n, validUntil: NOW - 10n });
    const result = parseQrPayload(text, { now: NOW, staleness: "hard-fail", clockReliable: false });
    expect(result.freshness).toBe("stale");
  });
});

describe("QR codec parity", () => {
  const vectors = [
    { name: "minimal", payload: { network: "testnet", contractId: REGISTRY, recordHash: HASH, attestedAt: 42n } },
    {
      name: "freshness",
      payload: {
        network: "testnet",
        contractId: REGISTRY,
        recordHash: HASH,
        attestedAt: 42n,
        issuedAt: 100n,
        validUntil: 200n,
      },
    },
  ];

  it.each(vectors)("TS codec round-trips $name", ({ payload }) => {
    expect(parseQrPayload(encodeQrPayload(payload))).toEqual(payload);
  });

  it("reassembles chunked payloads", () => {
    const payload = { network: "testnet", contractId: REGISTRY, recordHash: HASH, attestedAt: 42n };
    const text = encodeQrPayload(payload);
    const mid = Math.ceil(text.length / 2);
    const chunks = [
      `lafiya:v1:1/2:${text.slice(0, mid)}`,
      `lafiya:v1:2/2:${text.slice(mid)}`,
    ];
    expect(decodeQrChunks(chunks)).toBe(text);
    expect(parseQrPayload(decodeQrChunks(chunks))).toEqual(payload);
  });

  it("rejects incomplete or out-of-order chunks", () => {
    expect(() => decodeQrChunks(["lafiya:v1:1/2:abc"])).toThrow(QrPayloadError);
    expect(() => decodeQrChunks(["lafiya:v1:2/2:b", "lafiya:v1:1/2:a"])).toThrow(QrPayloadError);
  });

  it("round-trips the portable facility id vector", () => {
    const payload = {
      network: "testnet",
      contractId: REGISTRY,
      recordHash: HASH,
      attestedAt: 42n,
      facilityId: FACILITY_ID,
    };
    expect(parseQrPayload(encodeQrPayload(payload))).toEqual(payload);
  });
});

describe("facility deactivation trust", () => {
  const attestedAt = 1_000n;

  it("keeps pre-deactivation attestations valid for non-fraud reasons", () => {
    for (const reason of ["closed", "licence_lapsed"] as const) {
      const status = { active: false, reason, deactivatedAt: 2_000n };
      expect(evaluateFacilityTrust({ attestedAt, status })).toBe("valid");
    }
  });

  it("marks pre-deactivation attestations untrusted on fraud", () => {
    const status = { active: false, reason: "fraud" as const, deactivatedAt: 2_000n };
    expect(evaluateFacilityTrust({ attestedAt, status })).toBe("facility_untrusted");
  });

  it("keeps attestations valid when the facility is active", () => {
    expect(evaluateFacilityTrust({ attestedAt, status: { active: true } })).toBe("valid");
  });

  it("treats attestations after deactivation as untrusted", () => {
    const status = { active: false, reason: "closed" as const, deactivatedAt: 500n };
    expect(evaluateFacilityTrust({ attestedAt, status })).toBe("facility_untrusted");
  });
});

describe("trust anchors", () => {
  it("validates explicit config", async () => {
    expect(await loadTrustAnchors({ config: TRUST })).toEqual(TRUST);
    await expect(
      loadTrustAnchors({ config: { testnet: { ...TRUST.testnet, attesterRegistry: "nope" } } }),
    ).rejects.toThrow(TrustError);
  });

  it("discovers anchors from SEP-1 stellar.toml", async () => {
    const toml = [
      'NETWORK_PASSPHRASE="Test SDF Network ; September 2015"',
      `LAFIYA_ATTESTATION_REGISTRY="${REGISTRY}"`,
      `LAFIYA_ATTESTER_REGISTRY="${ATTESTER_REGISTRY}"`,
      "[[PRINCIPALS]]",
      'LAFIYA_ATTESTATION_REGISTRY="CIGNORED"',
    ].join("\n");
    let requested = "";
    const fetch = (async (url: string) => {
      requested = url;
      return new Response(toml);
    }) as unknown as typeof globalThis.fetch;
    const anchors = await loadTrustAnchors({
      domain: "lafiya.example",
      network: "testnet",
      rpcUrl: "https://rpc.invalid",
      fetch,
    });
    expect(requested).toBe("https://lafiya.example/.well-known/stellar.toml");
    expect(anchors).toEqual(TRUST);
  });
});
