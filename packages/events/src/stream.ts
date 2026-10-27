import type { rpc } from "@stellar/stellar-sdk";
import { decodeEvent, type LafiyaEvent, type RawEvent } from "./decode.js";
import type { EventKind } from "./generated.js";

/** The one RPC method streaming needs; `rpc.Server` satisfies it. */
export interface EventsRpc {
  getEvents(
    request: rpc.Api.GetEventsRequest,
  ): Promise<{
    events: RawEvent[];
    cursor: string;
    latestLedger: number;
    oldestLedger: number;
  }>;
}

/**
 * Where a stream persists its position. `load` is read once on start;
 * `save` is called with an event's `cursor` only after the consumer has
 * finished with that event (pulled the next one), and with the page cursor
 * once every event in a page has been consumed. A rejected `save` stops the
 * stream with that error rather than continuing unpersisted.
 */
export interface CursorStore {
  load(): Promise<string | undefined> | string | undefined;
  save(cursor: string): Promise<void> | void;
}

/** A {@link CursorStore} held in memory, for tests and short-lived consumers. */
export function memoryCursorStore(initial?: string): CursorStore & { cursor?: string } {
  return {
    cursor: initial,
    load() {
      return this.cursor;
    },
    save(cursor) {
      this.cursor = cursor;
    },
  };
}

/** Ledgers `fromLedger..=toLedger` could not be read from the RPC. */
export interface LedgerGap {
  fromLedger: number;
  toLedger: number;
}

export interface RetryOptions {
  /** Consecutive failed `getEvents` calls tolerated before throwing. Default 5. */
  attempts?: number;
  /** First backoff delay in ms; doubles per consecutive failure. Default 500. */
  baseDelayMs?: number;
  /** Backoff ceiling in ms. Default 30000. */
  maxDelayMs?: number;
}

export interface StreamOptions {
  rpc: EventsRpc;
  /** Contract IDs (`C...`) to read events from. */
  contracts: string[];
  /** Ledger to start from when there is no cursor. */
  startLedger?: number;
  /** Resume after this event (an event's `cursor`, or a page cursor). */
  cursor?: string;
  /**
   * Persist and restore the stream position. A cursor loaded from the store
   * takes precedence over `cursor` and `startLedger`.
   */
  cursorStore?: CursorStore;
  /**
   * Called when ledgers the stream needs are outside RPC retention (on
   * start, or after an outage). Backfill `gap` from an archive or indexer
   * before resolving; the stream then continues from the RPC's oldest
   * retained ledger. Without it the stream throws {@link RetentionGapError}.
   */
  onGap?: (gap: LedgerGap) => Promise<void> | void;
  /** Retry transient `getEvents` failures with exponential backoff. */
  retry?: RetryOptions;
  /** Only yield these kinds. Default: every Lafiya event. */
  kinds?: EventKind[];
  /** Events per `getEvents` call. Default 100. */
  pageSize?: number;
  /** Keep polling for new events once caught up. Default true. */
  follow?: boolean;
  /** Wait between polls when caught up, in ms. Default 5000. */
  pollIntervalMs?: number;
  signal?: AbortSignal;
}

/**
 * The requested start is older than the RPC's retention window, so events
 * between `requestedLedger` and `oldestLedger` can no longer be read from
 * this RPC. Streaming refuses to silently skip them: backfill the gap from
 * an archive (or an indexer) and resume from `oldestLedger`.
 */
export class RetentionGapError extends Error {
  constructor(
    readonly requestedLedger: number,
    readonly oldestLedger: number,
  ) {
    super(
      `events from ledger ${requestedLedger} are outside RPC retention (oldest available: ${oldestLedger})`,
    );
    this.name = "RetentionGapError";
  }
}

/**
 * Ledger an event id / paging cursor points into. Cursors are a TOID
 * (`ledger << 32 | tx << 12 | op`) optionally followed by `-<event index>`.
 */
export function ledgerOfCursor(cursor: string): number {
  const toid = BigInt(cursor.split("-")[0]);
  return Number(toid >> 32n);
}

/**
 * Total order over event ids / paging cursors: negative if `a` comes
 * before `b`, zero if equal, positive if after.
 */
export function compareCursors(a: string, b: string): number {
  const [toidA, indexA = "0"] = a.split("-");
  const [toidB, indexB = "0"] = b.split("-");
  const ta = BigInt(toidA);
  const tb = BigInt(toidB);
  if (ta !== tb) return ta < tb ? -1 : 1;
  return Number(indexA) - Number(indexB);
}

const sleep = (ms: number, signal?: AbortSignal) =>
  new Promise<void>((resolve) => {
    const timer = setTimeout(resolve, ms);
    signal?.addEventListener("abort", () => {
      clearTimeout(timer);
      resolve();
    });
  });

/**
 * Stream decoded Lafiya events with cursor-based pagination.
 *
 * - **Ordering**: events are yielded in strictly increasing cursor order
 *   (ledger, transaction, operation, event index). Events at or before the
 *   last one seen, e.g. from a duplicated page after a reconnect, are
 *   dropped, so a cursor is never yielded twice by one stream.
 * - **Backpressure**: the next page is fetched only once the consumer has
 *   pulled every event of the current one, so at most one page (`pageSize`
 *   events) is buffered however slow the consumer is.
 * - **Resumption**: every yielded event carries `cursor`. With a
 *   `cursorStore` the stream persists it after the consumer moves past the
 *   event and restores it on start; without one, pass the last cursor back
 *   as `cursor`.
 * - **Gaps**: ledgers outside RPC retention are reported to `onGap` for
 *   backfill, or thrown as {@link RetentionGapError}; they are never
 *   skipped silently.
 *
 * Delivery is at-least-once across restarts: an event whose processing was
 * interrupted is yielded again. Make handlers idempotent on `event.cursor`
 * (or commit the cursor in the same transaction as the handler's effects)
 * for exactly-once effects at the consumer.
 */
export async function* streamEvents(
  options: StreamOptions,
): AsyncGenerator<LafiyaEvent> {
  const {
    rpc,
    contracts,
    kinds,
    cursorStore,
    onGap,
    pageSize = 100,
    follow = true,
    pollIntervalMs = 5000,
    signal,
  } = options;
  const { attempts = 5, baseDelayMs = 500, maxDelayMs = 30_000 } = options.retry ?? {};
  const wanted = kinds ? new Set<EventKind>(kinds) : undefined;
  const filters = [{ type: "contract" as const, contractIds: contracts }];
  let cursor = (await cursorStore?.load()) ?? options.cursor;
  let startLedger = options.startLedger;
  if (cursor === undefined && startLedger === undefined) {
    throw new Error("streamEvents needs a startLedger or a cursor");
  }
  // Highest raw event id seen, for dropping duplicates and regressions.
  let lastSeen = cursor;
  // Retention is checked on the first page and on the first page after any
  // failure, since an outage is when the cursor can fall out of retention.
  let checkRetention = true;
  let failures = 0;

  const requestedLedger = () =>
    cursor !== undefined ? ledgerOfCursor(cursor) : startLedger!;

  const gap = async (requested: number, oldestLedger: number) => {
    if (!onGap) throw new RetentionGapError(requested, oldestLedger);
    await onGap({ fromLedger: requested, toLedger: oldestLedger - 1 });
    cursor = undefined;
    startLedger = oldestLedger;
  };

  while (!signal?.aborted) {
    const request: rpc.Api.GetEventsRequest =
      cursor !== undefined
        ? { filters, cursor, limit: pageSize }
        : { filters, startLedger: startLedger!, limit: pageSize };

    let page;
    try {
      page = await rpc.getEvents(request);
    } catch (err) {
      // RPC rejects a start outside retention with an error that names the
      // range; surface it as a typed gap when we can.
      const match = /ledger range:?\s*(\d+)\s*-\s*(\d+)/i.exec(String(err));
      if (match) {
        await gap(requestedLedger(), Number(match[1]));
        continue;
      }
      if (++failures >= attempts) throw err;
      checkRetention = true;
      await sleep(Math.min(baseDelayMs * 2 ** (failures - 1), maxDelayMs), signal);
      continue;
    }
    failures = 0;

    if (checkRetention) {
      const requested = requestedLedger();
      if (requested < page.oldestLedger) {
        await gap(requested, page.oldestLedger);
        continue;
      }
      checkRetention = false;
    }

    for (const raw of page.events) {
      if (lastSeen !== undefined && compareCursors(raw.id, lastSeen) <= 0) continue;
      lastSeen = raw.id;
      const event = decodeEvent(raw);
      if (event && (!wanted || wanted.has(event.kind))) {
        yield event;
        // Resumed: the consumer is done with `event`.
        await cursorStore?.save(event.cursor);
      }
    }
    const next = page.cursor || page.events.at(-1)?.id;
    if (next && (cursor === undefined || compareCursors(next, cursor) > 0)) {
      cursor = next;
      await cursorStore?.save(cursor);
    }

    if (page.events.length < pageSize) {
      if (!follow) return;
      await sleep(pollIntervalMs, signal);
    }
  }
}
