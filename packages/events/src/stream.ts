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

export interface StreamOptions {
  rpc: EventsRpc;
  /** Contract IDs (`C...`) to read events from. */
  contracts: string[];
  /** Ledger to start from when there is no cursor. */
  startLedger?: number;
  /** Resume after this event (an event's `cursor`, or a page cursor). */
  cursor?: string;
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

const sleep = (ms: number, signal?: AbortSignal) =>
  new Promise<void>((resolve) => {
    const timer = setTimeout(resolve, ms);
    signal?.addEventListener("abort", () => {
      clearTimeout(timer);
      resolve();
    });
  });

/**
 * Stream decoded Lafiya events with cursor-based pagination. Every yielded
 * event carries `cursor`; persisting the last one and passing it back as
 * `cursor` resumes exactly after it. Throws {@link RetentionGapError}
 * instead of skipping events the RPC no longer holds.
 */
export async function* streamEvents(
  options: StreamOptions,
): AsyncGenerator<LafiyaEvent> {
  const {
    rpc,
    contracts,
    kinds,
    pageSize = 100,
    follow = true,
    pollIntervalMs = 5000,
    signal,
  } = options;
  if (options.cursor === undefined && options.startLedger === undefined) {
    throw new Error("streamEvents needs a startLedger or a cursor");
  }
  const wanted = kinds ? new Set<EventKind>(kinds) : undefined;
  const filters = [{ type: "contract" as const, contractIds: contracts }];
  let cursor = options.cursor;
  let checkedRetention = false;

  while (!signal?.aborted) {
    const request: rpc.Api.GetEventsRequest =
      cursor !== undefined
        ? { filters, cursor, limit: pageSize }
        : { filters, startLedger: options.startLedger!, limit: pageSize };

    let page;
    try {
      page = await rpc.getEvents(request);
    } catch (err) {
      // RPC rejects a startLedger outside retention with an error that
      // names the range; surface it as a typed gap when we can.
      const match = /ledger range:?\s*(\d+)\s*-\s*(\d+)/i.exec(String(err));
      if (!checkedRetention && match) {
        const requested =
          cursor !== undefined ? ledgerOfCursor(cursor) : options.startLedger!;
        throw new RetentionGapError(requested, Number(match[1]));
      }
      throw err;
    }

    if (!checkedRetention) {
      const requested =
        cursor !== undefined ? ledgerOfCursor(cursor) : options.startLedger!;
      if (requested < page.oldestLedger) {
        throw new RetentionGapError(requested, page.oldestLedger);
      }
      checkedRetention = true;
    }

    for (const raw of page.events) {
      const event = decodeEvent(raw);
      if (event && (!wanted || wanted.has(event.kind))) yield event;
    }
    cursor = page.cursor || page.events.at(-1)?.id || cursor;

    if (page.events.length < pageSize) {
      if (!follow) return;
      await sleep(pollIntervalMs, signal);
    }
  }
}
