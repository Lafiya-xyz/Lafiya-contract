# @lafiya/events

Typed decoding and resumable streaming of Lafiya contract events
(`AttesterAdded`, `AttestationRecorded`, `AttestationRevoked`,
`AdminTransferred`, `Paused`, ...), so consumers don't match `getEvents`
topics or decode `ScVal`s by hand.

```ts
import { rpc } from "@stellar/stellar-sdk";
import { streamEvents } from "@lafiya/events";

const server = new rpc.Server("https://soroban-testnet.stellar.org");
for await (const ev of streamEvents({
  rpc: server,
  contracts: [attestationRegistryId],
  startLedger,
  kinds: ["AttestationRecorded"],
})) {
  ev.kind; ev.recordHash; ev.attester; ev.timestamp; ev.ledger; ev.txHash;
  saveCheckpoint(ev.cursor); // pass back as `cursor` to resume after it
}
```

- **`decodeEvent(raw)`** decodes one `getEvents` event into a discriminated
  `LafiyaEvent` (payload fields in camelCase, `u64` as `bigint`, `BytesN` as
  hex, addresses as strkeys). Non-Lafiya events return `undefined`. A Lafiya
  event that matches none of its known payload versions throws
  `EventDecodeError` rather than being skipped silently.
- **`streamEvents(options)`** pages with `getEvents` cursors, resumes from any
  event's `cursor`, and keeps polling when `follow` is true (the default).
  If the start is older than the RPC's retention window it throws
  `RetentionGapError` (with `requestedLedger` and `oldestLedger`) instead of
  skipping the missing events. Backfill that range from an archive or an
  indexer, then resume.

## Long-lived consumers

```ts
import { streamEvents, type CursorStore } from "@lafiya/events";

const store: CursorStore = {
  load: () => db.get("lafiya-cursor"),
  save: (cursor) => db.put("lafiya-cursor", cursor),
};

for await (const ev of streamEvents({
  rpc: server,
  contracts: [attestationRegistryId],
  startLedger,          // used only when the store is empty
  cursorStore: store,   // restored on start, saved as you go
  onGap: async ({ fromLedger, toLedger }) => {
    await backfillFromIndexer(fromLedger, toLedger);
  },
  retry: { attempts: 5, baseDelayMs: 500, maxDelayMs: 30_000 },
})) {
  await handle(ev);
}
```

### Guarantees

- **Ordering.** Events are yielded in strictly increasing `cursor` order
  (ledger, then transaction, operation and event index; `compareCursors`
  implements it). Anything at or before the last event seen, such as a page
  replayed after a reconnect or served by a lagging RPC node, is dropped, so
  one stream never yields the same cursor twice.
- **Backpressure.** `streamEvents` is a pull-based async iterator: the next
  `getEvents` page is requested only after the consumer has pulled every
  event of the current one. At most one page (`pageSize` events) is held in
  memory, however slow the consumer is.
- **Persistence.** With a `cursorStore`, a loaded cursor wins over `cursor`
  and `startLedger`. An event's cursor is saved only once the consumer pulls
  the *next* event, i.e. after the loop body for it has finished, and the
  page cursor is saved once a page is fully consumed so filtered-out events
  are not re-scanned. A rejected `save` ends the stream with that error.
- **Transient failures.** Failed `getEvents` calls are retried with
  exponential backoff; `retry.attempts` consecutive failures rethrow the last
  error. After any failure the next page re-checks retention, because an
  outage is when a cursor falls out of the RPC's window.
- **Gaps.** When needed ledgers are outside retention (at start, or after an
  outage), `onGap({ fromLedger, toLedger })` is awaited and the stream then
  continues from the oldest retained ledger. `fromLedger` is the ledger of
  the last cursor, so the range is inclusive and may overlap events already
  delivered. Without `onGap`, `RetentionGapError` is thrown. Backfill must
  be idempotent: a restart before the next event is saved reports the same
  gap again.

### Exactly-once at the consumer

Delivery is **at-least-once across restarts**: if the process stops while
handling an event (or the loop `break`s), that event's cursor was never
saved and it is yielded again on the next run. Get exactly-once *effects*
with either idiom:

1. **Commit together.** In the loop body, write the handler's effects and
   `ev.cursor` in one database transaction. Give the stream a store whose
   `load` reads that cursor and whose `save` does nothing: the cursor then
   moves exactly when the effects do.
2. **Idempotent handler.** Key every effect on `ev.cursor` (a unique,
   ordered id), e.g. `INSERT ... ON CONFLICT (cursor) DO NOTHING`, or skip
   events whose cursor is `<=` the last one you applied.

Events from a gap backfill should be keyed the same way so they deduplicate
against events the stream delivers.

## Generated decoders

`src/generated.ts` is generated from the `#[contractevent]` entries in the
committed interface snapshots (`scripts/conformance/snapshots/`), which
`make conformance` keeps in sync with the built Wasm:

```bash
python3 scripts/conformance/gen_events_ts.py          # regenerate
python3 scripts/conformance/gen_events_ts.py --check  # CI drift check
```

`EVENT_SPECS` lists each event's payload versions, newest first. When an
event changes shape under the event-versioning convention, keep the old
version in the list so historical events still decode. The multisig account
declares no events today, so it contributes no decoders.

## Tests

```bash
pnpm install && pnpm test
```

`test/fixtures/events.json` holds one raw event per kind (topics and data as
base64 XDR, as Soroban RPC returns them) with its expected payload. It is
built by `test/fixtures/make-fixtures.mjs` (run `pnpm build` first). Swap in
events captured from a local-quickstart run when one is available; the
format is the same.
