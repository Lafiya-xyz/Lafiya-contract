// Integration test against a local quickstart with real contracts:
//
//   LAFIYA_IT_RPC_URL=http://localhost:8000/soroban/rpc \
//   LAFIYA_IT_ATTESTATION_REGISTRY=C... LAFIYA_IT_ATTESTER_REGISTRY=C... \
//   LAFIYA_IT_RECORD_HASH=<64 hex, attested by an active attester> pnpm test
//
// Skipped unless those variables are set (see tests/integration/run.sh for
// deploying the contracts to a local quickstart).
import { describe, expect, it } from "vitest";
import { BindingsChainReader, encodeQrPayload, LafiyaVerifier } from "../src/index.js";

const env = process.env;
const enabled = Boolean(
  env.LAFIYA_IT_RPC_URL &&
    env.LAFIYA_IT_ATTESTATION_REGISTRY &&
    env.LAFIYA_IT_ATTESTER_REGISTRY &&
    env.LAFIYA_IT_RECORD_HASH,
);

describe.skipIf(!enabled)("quickstart integration", () => {
  const trust = {
    local: {
      networkPassphrase: env.LAFIYA_IT_PASSPHRASE ?? "Standalone Network ; February 2017",
      rpcUrl: env.LAFIYA_IT_RPC_URL!,
      attestationRegistry: env.LAFIYA_IT_ATTESTATION_REGISTRY!,
      attesterRegistry: env.LAFIYA_IT_ATTESTER_REGISTRY!,
    },
  };
  const verifier = new LafiyaVerifier({
    network: "local",
    trust,
    chain: new BindingsChainReader({ allowHttp: true }),
  });
  const card = (record: string) =>
    encodeQrPayload({ network: "local", contractId: trust.local.attestationRegistry, recordHash: record });

  it("verifies an attested record", async () => {
    expect((await verifier.verifyCard(card(env.LAFIYA_IT_RECORD_HASH!))).verdict).toBe("verified");
  });

  it("reports an unknown record as not_found", async () => {
    expect((await verifier.verifyCard(card("00".repeat(32)))).verdict).toBe("not_found");
  });
});
