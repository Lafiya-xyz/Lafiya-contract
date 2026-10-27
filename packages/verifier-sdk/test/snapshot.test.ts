import { mkdtempSync, rmSync } from "node:fs";
import { tmpdir } from "node:os";
import { join } from "node:path";
import { describe, expect, it } from "vitest";
import {
  FileSnapshotStore,
  IndexedDbSnapshotStore,
  LafiyaVerifier,
  MemorySnapshotStore,
  openSnapshot,
  SnapshotCache,
  SnapshotError,
  sealSnapshot,
  VERDICTS,
  type OfflineBundle,
  type StalenessPolicy,
  type Verdict,
} from "../src/index.js";
import { CHW, NOW, NOW_S, REGISTRY, TRUST } from "./helpers.js";

const RECORD = "ab".repeat(32);
const HOUR = 3600;

function bundle(over: Partial<OfflineBundle> = {}): OfflineBundle {
  return {
    network: "testnet",
    snapshotAt: Number(NOW_S),
    attestations: { [RECORD]: [{ attester: CHW, timestamp: (NOW_S - 100n).toString() }] },
    attesters: { [CHW]: { suspended: false } },
    revoked: [],
    ...over,
  };
}

const card = `lafiya:v1?${new URLSearchParams({ network: "testnet", contract: REGISTRY, record: RECORD })}`;

async function verdictFor(b: OfflineBundle, staleness?: StalenessPolicy, riskClass?: string) {
  const reader = await openSnapshot(await sealSnapshot(b, 1, null));
  const verifier = new LafiyaVerifier({
    network: "testnet",
    trust: TRUST,
    chain: reader,
    now: () => NOW,
    ...(staleness ? { staleness } : {}),
  });
  return verifier.verifyCard(card, riskClass ? { riskClass } : {});
}

describe("snapshot integrity", () => {
  it("opens a sealed snapshot and rejects tampering", async () => {
    const env = await sealSnapshot(bundle(), 10, null);
    await expect(openSnapshot(env)).resolves.toBeDefined();
    const forged = { ...env, bundle: bundle({ revoked: [] , attesters: {} }) };
    await expect(openSnapshot(forged)).rejects.toBeInstanceOf(SnapshotError);
  });

  it("requires a valid signature when a verifier is configured", async () => {
    const check = (hash: string, sig: string) => sig === `sig:${hash}`;
    const signed = await sealSnapshot(bundle(), 10, null, (h) => `sig:${h}`);
    await expect(openSnapshot(signed, { verifySignature: check })).resolves.toBeDefined();
    const unsigned = await sealSnapshot(bundle(), 10, null);
    await expect(openSnapshot(unsigned, { verifySignature: check })).rejects.toBeInstanceOf(SnapshotError);
    await expect(
      openSnapshot({ ...signed, signature: "sig:other" }, { verifySignature: check }),
    ).rejects.toBeInstanceOf(SnapshotError);
  });
});

describe("snapshot cache", () => {
  it("chains snapshots and refuses ledger rollback", async () => {
    const cache = new SnapshotCache(new MemorySnapshotStore());
    const first = await cache.save(bundle(), 10);
    const second = await cache.save(bundle(), 11);
    expect(second.prev).toBe(first.hash);
    await expect(cache.save(bundle(), 9)).rejects.toBeInstanceOf(SnapshotError);
    expect((await cache.load("testnet"))?.snapshotAt).toBe(Number(NOW_S));
  });

  it("treats corrupt or unknown-schema records as a miss", async () => {
    const store = new MemorySnapshotStore();
    const cache = new SnapshotCache(store);
    await store.put("testnet", "{not json");
    expect(await cache.load("testnet")).toBeNull();
    await store.put("testnet", JSON.stringify({ ...(await sealSnapshot(bundle(), 1, null)), schema: 99 }));
    expect(await cache.load("testnet")).toBeNull();
    const env = await sealSnapshot(bundle(), 1, null);
    await store.put("testnet", JSON.stringify({ ...env, ledger: 2 }));
    expect(await cache.load("testnet")).toBeNull();
  });

  it("round-trips through the filesystem store (Node)", async () => {
    const dir = mkdtempSync(join(tmpdir(), "lafiya-snap-"));
    try {
      const cache = new SnapshotCache(new FileSnapshotStore(dir));
      expect(await cache.load("testnet")).toBeNull();
      await cache.save(bundle(), 5);
      expect((await cache.load("testnet"))?.bundle.network).toBe("testnet");
    } finally {
      rmSync(dir, { recursive: true, force: true });
    }
  });

  it("round-trips through the IndexedDB store (browser API, faked)", async () => {
    const rows = new Map<string, string>();
    const request = <T>(result: T) => {
      const req: { result: T; onsuccess?: () => void; onerror?: () => void } = { result };
      queueMicrotask(() => req.onsuccess?.());
      return req;
    };
    const objectStore = {
      get: (k: string) => request(rows.get(k)),
      put: (v: string, k: string) => (rows.set(k, v), request(k)),
    };
    const db = {
      createObjectStore: () => objectStore,
      transaction: () => ({ objectStore: () => objectStore }),
    };
    const factory = {
      open: () => {
        const req: { result: typeof db; onupgradeneeded?: () => void; onsuccess?: () => void } = { result: db };
        queueMicrotask(() => {
          req.onupgradeneeded?.();
          req.onsuccess?.();
        });
        return req;
      },
    } as unknown as IDBFactory;
    const cache = new SnapshotCache(new IndexedDbSnapshotStore(factory));
    await cache.save(bundle(), 3);
    expect((await cache.load("testnet"))?.snapshotAt).toBe(Number(NOW_S));
  });
});

describe("staleness semantics", () => {
  const fail: StalenessPolicy = { defaultMaxSeconds: 6 * HOUR, onStale: "fail" };
  const warn: StalenessPolicy = { ...fail, onStale: "warn" };

  it("verifies a fresh snapshot and reports staleAsOf", async () => {
    const result = await verdictFor(bundle(), fail);
    expect(result).toMatchObject({ verdict: "verified", source: "offline" });
    expect(result.staleAsOf).toEqual(NOW);
  });

  it("hard-fails a stale snapshot as unavailable", async () => {
    const old = bundle({ snapshotAt: Number(NOW_S) - 7 * HOUR });
    const result = await verdictFor(old, fail);
    expect(result.verdict).toBe("unavailable");
    expect(result.staleAsOf).toEqual(new Date(old.snapshotAt * 1000));
  });

  it("in warn mode keeps negative verdicts but never verifies a stale snapshot", async () => {
    const age = Number(NOW_S) - 7 * HOUR;
    expect((await verdictFor(bundle({ snapshotAt: age }), warn)).verdict).toBe("unavailable");
    expect((await verdictFor(bundle({ snapshotAt: age, revoked: [RECORD], attestations: {} }), warn)).verdict).toBe(
      "revoked",
    );
  });

  it("applies per-risk-class limits and rejects snapshots from the future", async () => {
    const policy: StalenessPolicy = { ...fail, byRiskClass: { emergency: 30 * 24 * HOUR } };
    const old = bundle({ snapshotAt: Number(NOW_S) - 48 * HOUR });
    expect((await verdictFor(old, policy)).verdict).toBe("unavailable");
    expect((await verdictFor(old, policy, "emergency")).verdict).toBe("verified");
    expect((await verdictFor(bundle({ snapshotAt: Number(NOW_S) + HOUR }), policy)).verdict).toBe("unavailable");
  });

  it("property: a stale snapshot never yields a stronger verdict than the same data fresh", async () => {
    // Deterministic xorshift so failures reproduce.
    let seed = 0x9e3779b9;
    const rand = () => ((seed ^= seed << 13), (seed ^= seed >>> 17), (seed ^= seed << 5), (seed >>> 0) / 2 ** 32);
    const strength = (v: Verdict) => VERDICTS.indexOf(v); // lower index = weaker

    for (let i = 0; i < 200; i++) {
      const data = bundle({
        attestations: rand() < 0.8 ? bundle().attestations : {},
        attesters: rand() < 0.8 ? { [CHW]: { suspended: rand() < 0.3 } } : {},
        revoked: rand() < 0.2 ? [RECORD] : [],
      });
      const policy: StalenessPolicy = {
        defaultMaxSeconds: 6 * HOUR,
        onStale: rand() < 0.5 ? "fail" : "warn",
      };
      const fresh = await verdictFor({ ...data, snapshotAt: Number(NOW_S) }, policy);
      const stale = await verdictFor({ ...data, snapshotAt: Number(NOW_S) - Math.floor(rand() * 100) * HOUR }, policy);
      if (stale.staleAsOf && Number(NOW_S) - stale.staleAsOf.getTime() / 1000 > 6 * HOUR) {
        expect(stale.verdict).not.toBe("verified");
      }
      expect(strength(stale.verdict)).toBeLessThanOrEqual(strength(fresh.verdict));
    }
  });
});
