import { readFileSync } from "node:fs";
import { xdr } from "@stellar/stellar-sdk";
import { describe, expect, it } from "vitest";
import {
  decodeEvent,
  EVENT_SPECS,
  EventDecodeError,
  type EventKind,
  type RawEvent,
} from "../src/index.js";

interface Fixture {
  kind: EventKind;
  raw: Omit<RawEvent, "topic" | "value"> & { topic: string[]; value: string };
  expected: Record<string, string>;
}

const fixtures: Fixture[] = JSON.parse(
  readFileSync(new URL("./fixtures/events.json", import.meta.url), "utf8"),
);

function toRaw(f: Fixture): RawEvent {
  return {
    ...f.raw,
    topic: f.raw.topic.map((t) => xdr.ScVal.fromXDR(t, "base64")),
    value: xdr.ScVal.fromXDR(f.raw.value, "base64"),
  };
}

const plain = (v: unknown) =>
  JSON.parse(JSON.stringify(v, (_, x) => (typeof x === "bigint" ? x.toString() : x)));

describe("generated decoders", () => {
  it("have a golden fixture for every event kind", () => {
    expect(fixtures.map((f) => f.kind).sort()).toEqual(Object.keys(EVENT_SPECS).sort());
  });

  for (const fixture of fixtures) {
    it(`decode ${fixture.kind}`, () => {
      const event = decodeEvent(toRaw(fixture));
      expect(event?.kind).toBe(fixture.kind);
      expect(event?.cursor).toBe(fixture.raw.id);
      expect(event?.txHash).toBe(fixture.raw.txHash);
      expect(event?.ledger).toBe(fixture.raw.ledger);
      const payload = Object.fromEntries(
        Object.keys(fixture.expected).map((k) => [k, (event as any)[k]]),
      );
      expect(plain(payload)).toEqual(fixture.expected);
    });
  }

  it("types u64 fields as bigint", () => {
    const recorded = fixtures.find((f) => f.kind === "AttestationRecorded")!;
    const event = decodeEvent(toRaw(recorded));
    expect(event?.kind === "AttestationRecorded" && typeof event.timestamp).toBe("bigint");
  });

  it("ignores events that are not Lafiya events", () => {
    const raw = toRaw(fixtures[0]);
    raw.topic = [xdr.ScVal.scvSymbol("transfer")];
    expect(decodeEvent(raw)).toBeUndefined();
  });

  it("fails loudly when a Lafiya event no longer matches its schema", () => {
    const recorded = fixtures.find((f) => f.kind === "AttestationRecorded")!;
    const raw = toRaw(recorded);
    raw.value = xdr.ScVal.scvMap([]);
    expect(() => decodeEvent(raw)).toThrow(EventDecodeError);
  });
});
