import { readFileSync } from "node:fs";
import { xdr } from "@stellar/stellar-sdk";
import { describe, expect, it } from "vitest";
import {
  compareCursors,
  ledgerOfCursor,
  memoryCursorStore,
  RetentionGapError,
  streamEvents,
  type EventsRpc,
  type LafiyaEvent,
  type RawEvent,
} from "../src/index.js";

const raws: RawEvent[] = JSON.parse(
  readFileSync(new URL("./fixtures/events.json", import.meta.url), "utf8"),
).map((f: any) => ({
  ...f.raw,
  topic: f.raw.topic.map((t: string) => xdr.ScVal.fromXDR(t, "base64")),
  value: xdr.ScVal.fromXDR(f.raw.value, "base64"),
}));

/** A fake RPC holding `raws`, serving pages the way getEvents does. */
function fakeRpc(oldestLedger = 900): EventsRpc & { calls: any[] } {
  const calls: any[] = [];
  return {
    calls,
    async getEvents(req: any) {
      calls.push(req);
      const start =
        req.cursor !== undefined
          ? raws.findIndex((r) => r.id === req.cursor) + 1
          : raws.findIndex((r) => r.ledger >= req.startLedger);
      const events = start < 0 ? [] : raws.slice(start, start + req.limit);
      return {
        events,
        cursor: events.at(-1)?.id ?? req.cursor ?? "",
        latestLedger: 2000,
        oldestLedger,
      };
    },
  };
}

async function collect(iter: AsyncIterable<LafiyaEvent>, max = Infinity) {
  const out: LafiyaEvent[] = [];
  for await (const ev of iter) {
    out.push(ev);
    if (out.length >= max) break;
  }
  return out;
}

describe("streamEvents", () => {
  it("pages through every event with cursors", async () => {
    const rpc = fakeRpc();
    const events = await collect(
      streamEvents({ rpc, contracts: ["C"], startLedger: 1000, pageSize: 4, follow: false }),
    );
    expect(events.map((e) => e.cursor)).toEqual(raws.map((r) => r.id));
    expect(rpc.calls[0]).toMatchObject({ startLedger: 1000, limit: 4 });
    expect(rpc.calls[1]).toMatchObject({ cursor: raws[3].id });
  });

  it("filters by kind", async () => {
    const events = await collect(
      streamEvents({
        rpc: fakeRpc(),
        contracts: ["C"],
        startLedger: 1000,
        kinds: ["AttestationRecorded"],
        follow: false,
      }),
    );
    expect(events.map((e) => e.kind)).toEqual(["AttestationRecorded"]);
  });

  it("resumes exactly after the last persisted cursor", async () => {
    const first = await collect(
      streamEvents({ rpc: fakeRpc(), contracts: ["C"], startLedger: 1000, follow: false }),
      5,
    );
    const resumed = await collect(
      streamEvents({ rpc: fakeRpc(), contracts: ["C"], cursor: first[4].cursor, follow: false }),
    );
    expect([...first, ...resumed].map((e) => e.cursor)).toEqual(raws.map((r) => r.id));
  });

  it("throws a RetentionGapError instead of skipping unretained ledgers", async () => {
    await expect(
      collect(streamEvents({ rpc: fakeRpc(1005), contracts: ["C"], startLedger: 1000 })),
    ).rejects.toMatchObject({ name: "RetentionGapError", requestedLedger: 1000, oldestLedger: 1005 });

    await expect(
      collect(streamEvents({ rpc: fakeRpc(1005), contracts: ["C"], cursor: raws[1].id })),
    ).rejects.toBeInstanceOf(RetentionGapError);
  });

  it("maps an RPC out-of-range error to a RetentionGapError", async () => {
    const rpc: EventsRpc = {
      async getEvents() {
        throw new Error("startLedger must be within the ledger range: 1200 - 2000");
      },
    };
    await expect(
      collect(streamEvents({ rpc, contracts: ["C"], startLedger: 1000 })),
    ).rejects.toMatchObject({ requestedLedger: 1000, oldestLedger: 1200 });
  });

  it("yields every stored event before waiting for new ones", async () => {
    const controller = new AbortController();
    const iter = streamEvents({
      rpc: fakeRpc(),
      contracts: ["C"],
      startLedger: 1000,
      pollIntervalMs: 10_000,
      signal: controller.signal,
    });
    const events = await collect(
      (async function* () {
        for await (const ev of iter) {
          yield ev;
        }
      })(),
      raws.length,
    );
    controller.abort();
    expect(events).toHaveLength(raws.length);
  });

  it("reads the ledger out of a cursor", () => {
    expect(ledgerOfCursor(raws[2].id)).toBe(raws[2].ledger);
  });
});

const ids = raws.map((r) => r.id);
const noBackoff = { attempts: 3, baseDelayMs: 0 };

/** Wraps `inner`, failing the calls whose 0-based index is in `failOn`. */
function flaky(inner: EventsRpc, failOn: Set<number>): EventsRpc & { calls: number } {
  const rpc = {
    calls: 0,
    async getEvents(req: any) {
      if (failOn.has(rpc.calls++)) throw new Error("socket hang up");
      return inner.getEvents(req);
    },
  };
  return rpc;
}

describe("streamEvents fault tolerance", () => {
  it("retries through disconnects without losing or repeating events", async () => {
    const rpc = flaky(fakeRpc(), new Set([0, 2, 3, 5]));
    const events = await collect(
      streamEvents({ rpc, contracts: ["C"], startLedger: 1000, pageSize: 3, follow: false, retry: noBackoff }),
    );
    expect(events.map((e) => e.cursor)).toEqual(ids);
  });

  it("throws once consecutive failures exhaust the retry budget", async () => {
    const rpc = flaky(fakeRpc(), new Set([1, 2, 3]));
    await expect(
      collect(
        streamEvents({ rpc, contracts: ["C"], startLedger: 1000, pageSize: 3, follow: false, retry: noBackoff }),
      ),
    ).rejects.toThrow("socket hang up");
    expect(rpc.calls).toBe(4);
  });

  it("drops duplicated and overlapping pages, keeping cursor order", async () => {
    const inner = fakeRpc();
    let call = 0;
    // Serves each page twice, and every second page starts two events early,
    // as a load-balanced RPC replaying from a stale node might.
    const rpc: EventsRpc = {
      async getEvents(req: any) {
        const n = call++;
        if (n % 2 === 1 && req.cursor !== undefined) {
          const at = ids.indexOf(req.cursor);
          return inner.getEvents({ ...req, cursor: at >= 2 ? ids[at - 2] : req.cursor });
        }
        return inner.getEvents(req);
      },
    };
    const events = await collect(
      streamEvents({ rpc, contracts: ["C"], startLedger: 1000, pageSize: 4, follow: false }),
    );
    expect(events.map((e) => e.cursor)).toEqual(ids);
  });

  it("persists the cursor only after the consumer is done with an event", async () => {
    const store = memoryCursorStore();
    const iter = streamEvents({ rpc: fakeRpc(), contracts: ["C"], startLedger: 1000, cursorStore: store });
    const first = await iter.next();
    expect(store.cursor).toBeUndefined();
    await iter.next();
    expect(store.cursor).toBe(first.value!.cursor);
    await iter.return(undefined);
  });

  it("resumes from the store after a restart, exactly once at an idempotent consumer", async () => {
    const store = memoryCursorStore();
    const applied = new Map<string, number>();
    const handle = (cursor: string) => applied.set(cursor, (applied.get(cursor) ?? 0) + 1);
    const seen = new Set<string>();

    // First run "crashes" while processing its fifth event.
    let n = 0;
    for await (const ev of streamEvents({ rpc: fakeRpc(), contracts: ["C"], startLedger: 1000, cursorStore: store, pageSize: 3 })) {
      if (++n === 5) break;
      if (!seen.has(ev.cursor)) {
        seen.add(ev.cursor);
        handle(ev.cursor);
      }
    }
    // Restart: `startLedger` is ignored in favour of the stored cursor.
    for await (const ev of streamEvents({ rpc: fakeRpc(), contracts: ["C"], startLedger: 1000, cursorStore: store, follow: false })) {
      if (!seen.has(ev.cursor)) {
        seen.add(ev.cursor);
        handle(ev.cursor);
      }
    }
    expect([...applied.keys()]).toEqual(ids);
    expect([...applied.values()].every((count) => count === 1)).toBe(true);
  });

  it("reports a start-of-stream gap to onGap and continues from retention", async () => {
    const gaps: unknown[] = [];
    const events = await collect(
      streamEvents({
        rpc: fakeRpc(1005),
        contracts: ["C"],
        startLedger: 1000,
        follow: false,
        onGap: (gap) => {
          gaps.push(gap);
        },
      }),
    );
    expect(gaps).toEqual([{ fromLedger: 1000, toLedger: 1004 }]);
    expect(events.map((e) => e.cursor)).toEqual(raws.filter((r) => r.ledger >= 1005).map((r) => r.id));
  });

  it("detects a gap opened by an outage mid-stream", async () => {
    let oldest = 900;
    const inner = fakeRpc();
    let calls = 0;
    const rpc: EventsRpc = {
      async getEvents(req: any) {
        calls++;
        if (calls === 2) {
          // Outage: by the time the RPC is back, retention has moved on.
          oldest = 1008;
          throw new Error("ECONNRESET");
        }
        return { ...(await inner.getEvents(req)), oldestLedger: oldest };
      },
    };
    const gaps: unknown[] = [];
    const events = await collect(
      streamEvents({
        rpc,
        contracts: ["C"],
        startLedger: 1000,
        pageSize: 3,
        follow: false,
        retry: noBackoff,
        onGap: (gap) => {
          gaps.push(gap);
        },
      }),
    );
    expect(gaps).toEqual([{ fromLedger: 1002, toLedger: 1007 }]);
    expect(events.map((e) => e.ledger)).toEqual([1000, 1001, 1002, 1008, 1009, 1010, 1011, 1012]);
  });

  it("throws RetentionGapError for a mid-stream gap without onGap", async () => {
    let calls = 0;
    const inner = fakeRpc();
    const rpc: EventsRpc = {
      async getEvents(req: any) {
        if (++calls === 2) throw new Error("startLedger must be within the ledger range: 1008 - 2000");
        return inner.getEvents(req);
      },
    };
    await expect(
      collect(streamEvents({ rpc, contracts: ["C"], startLedger: 1000, pageSize: 3, follow: false })),
    ).rejects.toMatchObject({ requestedLedger: 1002, oldestLedger: 1008 });
  });

  it("buffers at most one page for a slow consumer", async () => {
    // An endless RPC: event i lives in ledger 1000 + i.
    let served = 0;
    const synth = (i: number): RawEvent => ({
      ...raws[i % raws.length],
      ledger: 1000 + i,
      id: `${((BigInt(1000 + i) << 32n)).toString().padStart(19, "0")}-0000000000`,
    });
    const rpc: EventsRpc = {
      async getEvents(req: any) {
        const start = req.cursor !== undefined ? Number(BigInt(req.cursor.split("-")[0]) >> 32n) - 999 : 0;
        const events = Array.from({ length: req.limit }, (_, k) => synth(start + k));
        served += events.length;
        return { events, cursor: events.at(-1)!.id, latestLedger: 1e9, oldestLedger: 1 };
      },
    };
    const pageSize = 10;
    let consumed = 0;
    let maxInFlight = 0;
    for await (const _ of streamEvents({ rpc, contracts: ["C"], startLedger: 1000, pageSize })) {
      await new Promise((r) => setTimeout(r, 1));
      consumed++;
      maxInFlight = Math.max(maxInFlight, served - consumed);
      if (consumed === 200) break;
    }
    expect(maxInFlight).toBeLessThan(pageSize);
    expect(served).toBe(200);
  });

  it("orders cursors by TOID, then event index", () => {
    expect(compareCursors(ids[0], ids[1])).toBeLessThan(0);
    expect(compareCursors(ids[1], ids[1])).toBe(0);
    expect(compareCursors(`${ids[1].split("-")[0]}-0000000002`, ids[1])).toBeGreaterThan(0);
  });
});
