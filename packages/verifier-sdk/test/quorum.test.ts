import { describe, expect, it } from "vitest";
import { QuorumChainReader, QuorumError } from "../src/index.js";
import { CHW, TRUST, fakeChain } from "./helpers.js";

const trust = TRUST.testnet!;
const at = (ts: bigint) => ({ history: [{ attester: CHW, timestamp: ts }] });
const hash = "00".repeat(32);

describe("QuorumChainReader", () => {
  it("returns the value when k providers agree", async () => {
    const q = new QuorumChainReader([{ reader: fakeChain(at(1n)) }, { reader: fakeChain(at(1n)) }]);
    expect((await q.getAttestation(trust, hash))?.timestamp).toBe(1n);
  });

  it("refuses when one provider lies about attestation existence", async () => {
    const q = new QuorumChainReader([
      { reader: fakeChain(at(1n)) },
      { reader: fakeChain({ history: [] }) },
      { reader: fakeChain({ history: [] }) },
    ]);
    await expect(q.getAttestation(trust, hash)).rejects.toMatchObject({ kind: "disagreement" });
    expect(q.metrics.disagreements).toBe(1);
  });

  it("excludes a stale provider and cannot reach quorum on its word", async () => {
    const q = new QuorumChainReader([
      { reader: fakeChain({ attester: { suspended: false } }), latestLedger: async () => 10 },
      { reader: fakeChain({ attester: { suspended: true } }), latestLedger: async () => 100 },
    ]);
    const err = await q.getAttesterStatus(trust, CHW).catch((e) => e);
    expect(err).toBeInstanceOf(QuorumError);
    expect(err.kind).toBe("insufficient");
    expect(q.metrics.staleExcluded).toBe(1);
  });

  it("counts failed providers and records latency", async () => {
    const q = new QuorumChainReader([{ reader: fakeChain({ fail: true }) }, { reader: fakeChain(at(1n)) }]);
    await expect(q.getAttestation(trust, hash)).rejects.toMatchObject({ kind: "insufficient" });
    expect(q.metrics.providerFailures).toBe(1);
    expect(q.metrics.lastLatencyMs).toBeGreaterThanOrEqual(0);
  });
});
