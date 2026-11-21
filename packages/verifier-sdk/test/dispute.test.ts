import { readFileSync } from "node:fs";
import { describe, expect, it } from "vitest";
import { DISPUTE_DOMAIN_V2, disputeMessageV2, disputePreimageV2, toHex } from "../src/index.js";

interface Vector {
  name: string;
  network_id_hex: string;
  contract: string;
  record_hash_hex: string;
  signer: string;
  signer_key_hex: string;
  nonce: string;
  expires_at: string;
  preimage_hex: string;
  message_hex: string;
}

const doc: { domain: string; vectors: Vector[] } = JSON.parse(
  readFileSync(
    new URL("../../../docs/specs/vectors/dispute-v2-vectors.json", import.meta.url),
    "utf8",
  ),
);

const hex = (s: string) => Uint8Array.from(Buffer.from(s, "hex"));

describe("dispute signature v2 vectors", () => {
  it("uses the shared domain tag", () => {
    expect(doc.domain).toBe(DISPUTE_DOMAIN_V2);
  });

  for (const v of doc.vectors) {
    it(v.name, async () => {
      const input = {
        networkId: hex(v.network_id_hex),
        contract: v.contract,
        recordHash: hex(v.record_hash_hex),
        signer: v.signer,
        signerKey: hex(v.signer_key_hex),
        nonce: BigInt(v.nonce),
        expiresAt: BigInt(v.expires_at),
      };
      expect(toHex(disputePreimageV2(input))).toBe(v.preimage_hex);
      expect(toHex(await disputeMessageV2(input))).toBe(v.message_hex);
    });
  }
});
