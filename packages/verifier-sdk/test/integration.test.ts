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

  it("syncs status-list segments, validates the signed root, and checks revocation offline", async () => {
    const statusListId = env.LAFIYA_IT_STATUS_LIST_ID;
    if (!statusListId) {
      // Status-list contract not deployed in this environment; nothing to sync.
      return;
    }

    const sync = await verifier.syncStatusList({
      network: "local",
      statusListId,
      // Sync only the segments covering the record under test.
      recordHashes: [env.LAFIYA_IT_RECORD_HASH!],
    });

    // The signed root must validate against the configured trust anchor.
    expect(sync.rootValid).toBe(true);
    expect(sync.segments.length).toBeGreaterThan(0);

    // Inclusion of the record's index must be provable against the signed root.
    for (const segment of sync.segments) {
      expect(verifier.validateStatusListInclusion(segment, sync.root)).toBe(true);
    }

    // Offline revocation check: no further RPC calls, uses the cached segments.
    const offline = await verifier.checkRevocationOffline({
      network: "local",
      statusListId,
      recordHash: env.LAFIYA_IT_RECORD_HASH!,
      cache: sync,
    });
    expect(["revoked", "not_revoked"]).toContain(offline.status);
  });
});
