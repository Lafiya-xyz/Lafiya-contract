import { describe, expect, it } from "vitest";
import {
  MemorySnapshotStore,
  resolveContract,
  RouterError,
  type RouterReader,
  type RouterState,
} from "../src/index.js";
import { OTHER_CONTRACT, REGISTRY } from "./helpers.js";

const ROUTER = "C" + "R".repeat(55);
const state = (seq: number, contractId = REGISTRY, interfaceVersion = 2): RouterState => ({
  seq,
  entries: { "attestation-registry": { contractId, interfaceVersion } },
});
const reader = (s: () => RouterState): RouterReader => ({ getState: async () => s() });
const down: RouterReader = {
  getState: async () => {
    throw new Error("rpc down");
  },
};

describe("resolveContract", () => {
  it("resolves, remembers, and follows router updates", async () => {
    const store = new MemorySnapshotStore();
    const a = await resolveContract(reader(() => state(1)), store, ROUTER, "attestation-registry");
    expect(a).toMatchObject({ contractId: REGISTRY, cached: false, seq: 1 });
    const b = await resolveContract(reader(() => state(2, OTHER_CONTRACT)), store, ROUTER, "attestation-registry");
    expect(b.contractId).toBe(OTHER_CONTRACT);
  });

  it("falls back to last-known state when the router is unreachable", async () => {
    const store = new MemorySnapshotStore();
    await resolveContract(reader(() => state(1)), store, ROUTER, "attestation-registry");
    const r = await resolveContract(down, store, ROUTER, "attestation-registry");
    expect(r).toMatchObject({ contractId: REGISTRY, cached: true });
    await expect(resolveContract(down, new MemorySnapshotStore(), ROUTER, "x")).rejects.toBeInstanceOf(RouterError);
  });

  it("rejects a rolled-back router state and an unsupported interface version", async () => {
    const store = new MemorySnapshotStore();
    await resolveContract(reader(() => state(5)), store, ROUTER, "attestation-registry");
    await expect(
      resolveContract(reader(() => state(4, OTHER_CONTRACT)), store, ROUTER, "attestation-registry"),
    ).rejects.toThrow(/rolled back/);
    await expect(
      resolveContract(reader(() => state(6)), store, ROUTER, "attestation-registry", { minInterfaceVersion: 3 }),
    ).rejects.toThrow(/interface v2/);
  });
});
