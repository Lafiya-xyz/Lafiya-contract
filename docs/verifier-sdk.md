# Verifier SDK (`@lafiya/verifier`)

`packages/verifier-sdk/` is the high-level library a verifier app uses to
check a scanned Lafiya card. It wraps the generated
[TypeScript bindings](typescript-bindings.md) and runs every step in one
place, so consumers cannot get a step subtly wrong:

1. Parse the QR payload and check its network and contract IDs against
   **trust anchors**. A contract ID is never trusted just because the QR
   payload names it.
2. Recompute the LRC-1 record commitment from the disclosed fields and
   compare it with the card's record hash.
3. Read the attestation from `attestation-registry`.
4. Read the attester's **current** status from `attester-registry`
   (ADR-0006).
5. Apply revocation, supersession, and maximum-age rules.
6. Aggregate co-attestations against the record class's quorum policy
   (distinct attesters only) and surface the count and policy result.
7. Surface any patient-initiated `Disputed` overlay for the record.
8. Return a single UI-ready verdict with ordered reasons.

## Usage

```ts
import { LafiyaVerifier, loadTrustAnchors } from "@lafiya/verifier";

const trust = await loadTrustAnchors({
  domain: "lafiya.example",          // publishes /.well-known/stellar.toml
  network: "testnet",
  rpcUrl: "https://soroban-testnet.stellar.org",
});
const verifier = new LafiyaVerifier({ network: "testnet", trust });

const result = await verifier.verifyCard(qrPayload, { disclosedFields, maxAgeDays: 365 });
// { verdict, reasons[], attestation?, attester?, checkedAt, source }
```

`verifyCard` never throws: RPC and parsing failures become verdicts.

### Cancellation, timeouts, and retries

Every chain read accepts an `AbortSignal` and a per-call deadline:

```ts
const controller = new AbortController();
const result = await verifier.verifyCard(qrPayload, {
  signal: controller.signal, // e.g. aborted when the user leaves the screen
  timeoutMs: 5_000,          // per read attempt
});
if (result.verdict === "unavailable") console.log(result.transportError?.kind);
```

`BindingsChainReader` defaults to a 10 s per-attempt deadline and retries
**reads** (all of its calls are idempotent simulations) at most twice, with
full-jitter exponential backoff (`random() * min(maxDelayMs, baseDelayMs * 2^n)`,
200 ms base, 2 s cap). Configure both with
`new BindingsChainReader({ timeoutMs, retry: { retries, baseDelayMs, maxDelayMs } })`.
An abort cancels the in-flight attempt and any pending backoff; it is never
retried. `callWithPolicy` is exported for integrators' own calls: it only
retries when `idempotent: true` is passed, so ambiguous writes (where the
first attempt may have landed) are never retried.

Custom `ChainReader` implementations receive the same `{ signal, timeoutMs }`
as the last argument of each read and should honour it.

### Transport error taxonomy

Failed reads reject with a `TransportError` (exported, with
`isTransportError(err, kind?)` and `classifyError(err)`); narrow on `kind`:

| `kind` | Meaning | Retried (idempotent reads) |
|---|---|---|
| `network` | No usable answer: connection refused/reset, DNS, HTTP 5xx or 429. | Yes |
| `timeout` | No answer within `timeoutMs`. | Yes |
| `aborted` | The caller's `AbortSignal` fired. | No |
| `protocol` | The server answered with something unusable: malformed response, simulation/contract error, HTTP 4xx. | No |
| `not_found` | The RPC endpoint or contract does not exist (HTTP 404). A missing *attestation* is not an error; it is `null`. | No |
| `untrusted` | Refused before sending because the transport is not trusted (plain HTTP without `allowHttp`). | No |

`error.attempts` is the number of attempts made. In `verifyCard` any of these
becomes the `unavailable` verdict, with the error on `result.transportError`.
The SDK's lint (`pnpm lint`, `@typescript-eslint/no-floating-promises` and
`no-misused-promises`) keeps failure paths free of unhandled rejections, and
the fake-timer tests in `test/transport.test.ts` assert none occur.

### Trust anchors

`loadTrustAnchors` accepts either explicit configuration
(`{ config: { testnet: { networkPassphrase, rpcUrl, attestationRegistry, attesterRegistry } } }`)
or SEP-1 discovery, where the home domain's `stellar.toml` publishes
`NETWORK_PASSPHRASE`, `LAFIYA_ATTESTATION_REGISTRY`, and
`LAFIYA_ATTESTER_REGISTRY`. Invalid anchors raise `TrustError`.

### Offline mode

Pass `chain: new OfflineChainReader(bundle)` to verify from a previously
downloaded bundle of attestations and attester states instead of RPC.
Results then report `source: "offline"` and `staleAsOf` (the snapshot's
capture time).

Persist bundles with `SnapshotCache` over a pluggable store
(`MemorySnapshotStore`, `FileSnapshotStore` for Node, `IndexedDbSnapshotStore`
for browsers). Each saved snapshot is sealed with a SHA-256 hash chain over the
bundle, ledger, and previous hash (plus an optional publisher signature
checked by `verifySignature`); the cache refuses ledger rollbacks, and
corrupt or unknown-schema records load as a miss.

Staleness is governed by `staleness: { defaultMaxSeconds, byRiskClass, onStale }`
(default: 24 hours, `"fail"`) and `verifyCard(..., { riskClass })`. A stale
snapshot returns `unavailable` in `"fail"` mode; in `"warn"` mode negative
verdicts are still reported and only a would-be `verified` becomes
`unavailable`. See [Snapshot staleness](specs/verification-model.md#snapshot-staleness).

### Router resolution (optional)

`resolveContract(reader, store, routerId, name, { minInterfaceVersion })`
resolves a contract id through the optional stable-address router
([ADR-0018](adr/0018-stable-address-registry-router.md)), remembering the
last-known state and rejecting rollbacks. Direct contract ids in trust anchors
remain supported.

### Status-list sync

For bandwidth-constrained or offline verifiers, revocation can be checked
against the compact bitstring status list instead of per-record RPC calls.
The SDK syncs the list, validates its versioned commitment against the
signed root, and caches it:

```ts
import { StatusListSync } from "@lafiya/verifier";

const sync = new StatusListSync({ network: "testnet", trust });
await sync.refresh();                       // fetch + verify signed root
const revoked = sync.isRevoked(recordHash); // local bitstring lookup
```

`refresh()` throws `TrustError` if the signed root does not match the
fetched segments. Pass `statusList: sync` to `LafiyaVerifier` to use the
cached list for revocation checks; results then report
`source: "status-list"`. The list never reveals record identity — indices
are allocated independently of record hashes — so a status-list hit must
still be confirmed against `attestation-registry` before showing `revoked`
to a user.

## Verdicts

The verdict set is finite and ordered by precedence; when several reasons
apply, the highest-precedence one wins. All reasons are still returned.

| Verdict | Meaning |
|---|---|
| `invalid_payload` | The QR code is not a valid Lafiya card payload. |
| `untrusted_contract` | The payload's network or contract is not in the trust anchors. |
| `commitment_mismatch` | The disclosed record does not match the card's record hash. |
| `unavailable` | The attestation could not be checked (network or RPC failure). |
| `revoked` | The attestation was revoked by the registry admin. |
| `not_found` | No attestation exists for this record. |
| `superseded` | A newer attestation exists than the one on the card. |
| `attester_removed` | The attesting health worker is no longer on the allowlist. |
| `attester_suspended` | The attesting health worker is currently suspended. |
| `disputed` | The patient has flagged this record as disputed. |
| `quorum_not_met` | Fewer distinct attesters than the record class's quorum policy requires. |
| `expired` | The attestation is older than `maxAgeDays`. |
| `verified` | Attested by a currently trusted health worker (and quorum met, if required). |

Only `verified` should be shown to a user as a pass.

### Quorum fields

When the record class has a quorum policy, the result includes:

```ts
result.quorum // { required: number, distinctAttesters: number, satisfied: boolean }
```

`distinctAttesters` counts unique attester addresses only; a repeated
co-attestation from the same attester does not increase the count. Quorum
is evaluated after revocation and supersession, so revoking one attester's
co-attestation can drop a record below quorum and flip the verdict to
`quorum_not_met`. `quorum` is `undefined` for record classes without a
policy.

### Disputed records

A `disputed` verdict means the patient signed a dispute over the record
commitment. Disputes are an overlay: they never delete the attestation and
are cleared only by an attester or admin. The result includes
`result.dispute // { disputedAt, clearedAt? }` when present. Responder
guidance: treat `disputed` as a soft failure — do not delete the record,
but require re-verification by a health worker before relying on it.
Disputes are a linkability signal (they tie a patient signature to a
record), so do not log or display dispute metadata beyond what the UI
needs.

## Example app

`packages/verifier-sdk/example/index.html` is a minimal page that scans a QR
code with the camera (`BarcodeDetector`, with a text-box fallback) and shows
the verdict. Build the package, serve the **repository root** (the import map
resolves the bindings from `/bindings/`), and open
`/packages/verifier-sdk/example/`.

## Development

```sh
cd packages/verifier-sdk
pnpm install
pnpm typecheck && pnpm lint && pnpm test && pnpm build && pnpm size
pnpm lint:i18n       # catalog coverage, provenance, reading level (after build)
pnpm check:package   # publint + are-the-types-wrong on the packed tarball
pnpm smoke           # install the packed tarball; import from ESM, CJS, bundler, worker
```

Supported runtimes and minimum versions are in the package
[README](../packages/verifier-sdk/README.md#compatibility); `pnpm smoke`
fails if that table and the tested targets drift apart.

- Unit tests use a mocked chain reader; `test/verifier.test.ts` includes a
  truth table covering every verdict.
- `test/integration.test.ts` runs against a local quickstart with real
  contracts when `LAFIYA_IT_RPC_URL`, `LAFIYA_IT_ATTESTATION_REGISTRY`,
  `LAFIYA_IT_ATTESTER_REGISTRY`, and `LAFIYA_IT_RECORD_HASH` are set, and is
  skipped otherwise.
- `pnpm size` enforces the gzipped bundle budget (8 KiB) in CI.
- `test/status-list.test.ts` covers signed-root validation and cached
  bitstring lookups; `test/quorum.test.ts` covers distinct-attester
  counting and quorum monotonicity under revocation.
