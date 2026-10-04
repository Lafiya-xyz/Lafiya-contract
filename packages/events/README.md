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
