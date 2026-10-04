import { scValToNative, xdr } from "@stellar/stellar-sdk";
import {
  EVENT_SPECS,
  type EventKind,
  type EventPayloads,
} from "./generated.js";
import type { EventSpec, SpecType } from "./spec.js";

/** Where a decoded event came from on chain. */
export interface EventMeta {
  contractId: string;
  ledger: number;
  ledgerClosedAt: string;
  txHash: string;
  /** `getEvents` event id; pass as `cursor` to resume after this event. */
  cursor: string;
}

/** A decoded event: discriminated on `kind`, payload fields spread in. */
export type LafiyaEvent = {
  [K in EventKind]: { kind: K } & EventPayloads[K] & EventMeta;
}[EventKind];

/** The subset of a `getEvents` event this package reads. */
export interface RawEvent {
  id: string;
  ledger: number;
  ledgerClosedAt: string;
  txHash: string;
  contractId?: { toString(): string } | string;
  topic: xdr.ScVal[];
  value: xdr.ScVal;
}

/** An event matched a Lafiya prefix topic but none of its payload versions. */
export class EventDecodeError extends Error {
  constructor(
    readonly kind: EventKind,
    readonly eventId: string,
    reason: string,
  ) {
    super(`cannot decode ${kind} event ${eventId}: ${reason}`);
    this.name = "EventDecodeError";
  }
}

const BY_PREFIX = new Map<string, EventKind>();
for (const kind of Object.keys(EVENT_SPECS) as EventKind[]) {
  for (const spec of EVENT_SPECS[kind]) {
    BY_PREFIX.set(spec.prefixTopics.join("\u0000"), kind);
  }
}

function symbolOf(val: xdr.ScVal): string | undefined {
  return val.switch().name === "scvSymbol" ? val.sym().toString() : undefined;
}

function convert(val: xdr.ScVal, type: SpecType): unknown {
  if (typeof type === "object" && "option" in type) {
    return val.switch().name === "scvVoid"
      ? undefined
      : convert(val, type.option.value_type);
  }
  const native = scValToNative(val);
  if (type === "bytes" || (typeof type === "object" && "bytes_n" in type)) {
    return Array.from(native as Uint8Array, (b) =>
      b.toString(16).padStart(2, "0"),
    ).join("");
  }
  if (type === "u32" || type === "i32") return Number(native);
  if (typeof native === "number" && /^[iu](64|128|256)$/.test(String(type))) {
    return BigInt(native);
  }
  return native;
}

function tryDecode(spec: EventSpec, raw: RawEvent): Record<string, unknown> {
  const topicFields = spec.fields.filter((f) => f.location === "topic");
  const dataFields = spec.fields.filter((f) => f.location === "data");
  const expectedTopics = spec.prefixTopics.length + topicFields.length;
  if (raw.topic.length !== expectedTopics) {
    throw new Error(`expected ${expectedTopics} topics, got ${raw.topic.length}`);
  }

  const out: Record<string, unknown> = {};
  topicFields.forEach((field, i) => {
    out[field.key] = convert(raw.topic[spec.prefixTopics.length + i], field.type);
  });

  if (spec.dataFormat === "map") {
    const entries = new Map<string, xdr.ScVal>();
    if (raw.value.switch().name === "scvMap") {
      for (const entry of raw.value.map() ?? []) {
        entries.set(symbolOf(entry.key()) ?? "", entry.val());
      }
    } else if (dataFields.length > 0) {
      throw new Error(`expected map data, got ${raw.value.switch().name}`);
    }
    for (const field of dataFields) {
      const val = entries.get(field.name);
      if (val === undefined) throw new Error(`missing data field ${field.name}`);
      out[field.key] = convert(val, field.type);
    }
  } else if (dataFields.length === 1) {
    out[dataFields[0].key] = convert(raw.value, dataFields[0].type);
  } else if (dataFields.length > 1) {
    const items = raw.value.vec() ?? [];
    dataFields.forEach((field, i) => {
      out[field.key] = convert(items[i], field.type);
    });
  }
  return out;
}

/** The event kind for a raw event's prefix topic, or `undefined` if it isn't a Lafiya event. */
export function eventKindOf(topic: xdr.ScVal[]): EventKind | undefined {
  for (let n = 1; n <= topic.length; n++) {
    const prefix = topic.slice(0, n).map(symbolOf);
    if (prefix.some((p) => p === undefined)) return undefined;
    const kind = BY_PREFIX.get(prefix.join("\u0000"));
    if (kind) return kind;
  }
  return undefined;
}

/**
 * Decode one raw `getEvents` event. Returns `undefined` for events that are
 * not Lafiya contract events and throws {@link EventDecodeError} when a
 * Lafiya event matches no known payload version (a silent skip would hide
 * a contract change that needs regenerated decoders).
 */
export function decodeEvent(raw: RawEvent): LafiyaEvent | undefined {
  const kind = eventKindOf(raw.topic);
  if (!kind) return undefined;
  let lastError = "no payload versions";
  for (const spec of EVENT_SPECS[kind]) {
    try {
      const payload = tryDecode(spec, raw);
      return {
        kind,
        ...payload,
        contractId: raw.contractId?.toString() ?? "",
        ledger: raw.ledger,
        ledgerClosedAt: raw.ledgerClosedAt,
        txHash: raw.txHash,
        cursor: raw.id,
      } as LafiyaEvent;
    } catch (err) {
      lastError = (err as Error).message;
    }
  }
  throw new EventDecodeError(kind, raw.id, lastError);
}
