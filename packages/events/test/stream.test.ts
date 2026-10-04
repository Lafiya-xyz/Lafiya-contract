import { readFileSync } from "node:fs";
import { xdr } from "@stellar/stellar-sdk";
import { describe, expect, it } from "vitest";
import {
  ledgerOfCursor,
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
