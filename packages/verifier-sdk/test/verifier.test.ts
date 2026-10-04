import { describe, expect, it } from "vitest";
import {
  commitV1,
  LafiyaVerifier,
  OfflineChainReader,
  VERDICTS,
  type FieldValue,
  type Verdict,
} from "../src/index.js";
import { CHW, fakeChain, NOW, NOW_S, OTHER_CONTRACT, REGISTRY, TRUST, type FakeState } from "./helpers.js";

const FIELDS: FieldValue[] = [
  { kind: "text", value: "lafiya.emergency_record" },
  { kind: "bytes", value: new Uint8Array(16).fill(7) },
  { kind: "text", value: "allergy" },
  { kind: "text", value: "penicillin" },
];
const RECORD = await commitV1(FIELDS);
const DAY = 86_400n;
const fresh = { attester: CHW, timestamp: NOW_S - 10n * DAY };

function qr(overrides: Record<string, string> = {}) {
  const params = new URLSearchParams({
    network: "testnet",
    contract: REGISTRY,
    record: RECORD,
    attested: fresh.timestamp.toString(),
    ...overrides,
  });
  return `lafiya:v1?${params}`;
}

async function verify(state: FakeState, payload = qr(), disclosedFields: FieldValue[] = FIELDS) {
  const verifier = new LafiyaVerifier({
    network: "testnet",
    trust: TRUST,
    chain: fakeChain(state),
    now: () => NOW,
  });
  return verifier.verifyCard(payload, { disclosedFields, maxAgeDays: 365 });
}

/** One row per verdict: the state that must produce it. */
const TRUTH_TABLE: Record<Verdict, () => ReturnType<typeof verify>> = {
  invalid_payload: () => verify({ history: [fresh] }, "not a lafiya card"),
  untrusted_contract: () => verify({ history: [fresh] }, qr({ contract: OTHER_CONTRACT })),
  commitment_mismatch: () =>
    verify({ history: [fresh] }, qr(), [...FIELDS.slice(0, 3), { kind: "text", value: "none" }]),
  unavailable: () => verify({ history: [fresh], fail: true }),
  revoked: () => verify({ history: [], revoked: true }),
  not_found: () => verify({ history: [] }),
  superseded: () =>
    verify({ history: [fresh, { attester: CHW, timestamp: fresh.timestamp + DAY }] }),
  attester_removed: () => verify({ history: [fresh], attester: null }),
  attester_suspended: () => verify({ history: [fresh], attester: { suspended: true } }),
  expired: () =>
    verify(
      { history: [{ attester: CHW, timestamp: NOW_S - 400n * DAY }] },
      qr({ attested: (NOW_S - 400n * DAY).toString() }),
    ),
  verified: () => verify({ history: [fresh] }),
};

describe("verdict truth table", () => {
  it("covers every verdict", () => {
    expect(Object.keys(TRUTH_TABLE).sort()).toEqual([...VERDICTS].sort());
  });

  for (const verdict of VERDICTS) {
    it(`yields ${verdict}`, async () => {
      const result = await TRUTH_TABLE[verdict]();
      expect(result.verdict).toBe(verdict);
      expect(result.reasons[0].code).toBe(verdict);
      expect(result.checkedAt).toEqual(NOW);
    });
  }
});

describe("LafiyaVerifier", () => {
  it("never reads a contract taken from the QR code", async () => {
    const chain = fakeChain({ history: [fresh] });
    const verifier = new LafiyaVerifier({ network: "testnet", trust: TRUST, chain, now: () => NOW });
    await verifier.verifyCard(qr({ contract: OTHER_CONTRACT }));
    await verifier.verifyCard(qr({ network: "mainnet" }));
    expect(chain.calls).toEqual([]);
  });

  it("returns a UI-ready verified result", async () => {
    const result = await verify({ history: [fresh], attester: { suspended: false, region: "kano" } });
    expect(result).toMatchObject({
      verdict: "verified",
      attester: { address: CHW, status: "active", region: "kano" },
      attestation: { attester: CHW, timestamp: fresh.timestamp },
      source: "rpc",
    });
    expect(result.attestation?.attestedAt).toEqual(new Date(Number(fresh.timestamp) * 1000));
  });

  it("reports every failure, highest precedence first", async () => {
    const result = await verify({
      history: [{ attester: CHW, timestamp: NOW_S - 400n * DAY }],
      attester: { suspended: true },
    }, qr({ attested: (NOW_S - 500n * DAY).toString() }));
    expect(result.reasons.map((r) => r.code)).toEqual(["superseded", "attester_suspended", "expired"]);
    expect(result.verdict).toBe("superseded");
  });

  it("verifies offline from a snapshot bundle", async () => {
    const chain = new OfflineChainReader({
      network: "testnet",
      snapshotAt: Number(NOW_S),
      attestations: { [RECORD]: [{ attester: CHW, timestamp: fresh.timestamp.toString() }] },
      attesters: { [CHW]: { suspended: false } },
      revoked: [],
    });
    const verifier = new LafiyaVerifier({ network: "testnet", trust: TRUST, chain, now: () => NOW });
    const result = await verifier.verifyCard(qr(), { disclosedFields: FIELDS });
    expect(result).toMatchObject({ verdict: "verified", source: "offline" });

    chain.bundle.attestations = {};
    chain.bundle.revoked = [RECORD];
    expect((await verifier.verifyCard(qr())).verdict).toBe("revoked");
  });
});
