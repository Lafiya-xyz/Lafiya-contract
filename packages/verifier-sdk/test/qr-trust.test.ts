import { describe, expect, it } from "vitest";
import { encodeQrPayload, loadTrustAnchors, parseQrPayload, QrPayloadError, TrustError } from "../src/index.js";
import { ATTESTER_REGISTRY, REGISTRY, TRUST } from "./helpers.js";

const HASH = "ab".repeat(32);

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
