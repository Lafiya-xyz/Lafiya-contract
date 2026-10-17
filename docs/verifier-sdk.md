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
6. Return a single UI-ready verdict with ordered reasons.

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

### Trust anchors

`loadTrustAnchors` accepts either explicit configuration
(`{ config: { testnet: { networkPassphrase, rpcUrl, attestationRegistry, attesterRegistry } } }`)
or SEP-1 discovery, where the home domain's `stellar.toml` publishes
`NETWORK_PASSPHRASE`, `LAFIYA_ATTESTATION_REGISTRY`, and
`LAFIYA_ATTESTER_REGISTRY`. Invalid anchors raise `TrustError`.

### Offline mode

Pass `chain: new OfflineChainReader(bundle)` to verify from a previously
downloaded bundle of attestations and attester states instead of RPC.
Results then report `source: "offline"`.

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
| `expired` | The attestation is older than `maxAgeDays`. |
| `verified` | Attested by a currently trusted health worker. |

Only `verified` should be shown to a user as a pass.

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
pnpm typecheck && pnpm test && pnpm build && pnpm size
```

- Unit tests use a mocked chain reader; `test/verifier.test.ts` includes a
  truth table covering every verdict.
- `test/integration.test.ts` runs against a local quickstart with real
  contracts when `LAFIYA_IT_RPC_URL`, `LAFIYA_IT_ATTESTATION_REGISTRY`,
  `LAFIYA_IT_ATTESTER_REGISTRY`, and `LAFIYA_IT_RECORD_HASH` are set, and is
  skipped otherwise.
- `pnpm size` enforces a gzipped budget per entry point and module format
  (`.` ESM 8 KiB, CJS 9 KiB) over the SDK's own code: the entry file plus
  every module it reaches by relative import. The `verifier-sdk` workflow
  fails on regression and posts the report on the pull request. Raise a
  budget only in the PR that needs it, with the reason.
- `pnpm check:supply-chain` enforces the policy below.

## Supply chain

The verifier runs in safety-critical UIs, often on slow connections, so
every runtime dependency is attack surface and download weight. Policy,
checked by `scripts/check-supply-chain.mjs` in CI:

- **Allowlisted runtime dependencies only.** Adding one means editing the
  allowlist in the script, with a reason, in review.
- **No install scripts.** `.npmrc` sets `ignore-scripts=true`, and
  `pnpm-workspace.yaml` sets `strictDepBuilds: true` with no dependency
  allowed to build, so an install fails if a dependency starts shipping a
  lifecycle script. The package itself defines no install hooks.
- **Lockfile integrity.** CI installs with `--frozen-lockfile`. Every
  registry package in `pnpm-lock.yaml` must carry an `sha512` integrity hash
  (pnpm checks each tarball against it), and nothing may resolve from a
  tarball URL or git. The only non-registry entries allowed are the local
  bindings.
- **Provenance.** `publishConfig.provenance` is `true`, and tags
  `verifier-sdk-v*` publish from CI with `npm publish --provenance`, so every
  release is attested to the commit and workflow that built it. The publish
  job refuses to run while the bindings are still `file:` dependencies (see
  [PUBLISHING.md](../PUBLISHING.md)).

### Dependency inventory

`pnpm ls --prod --depth 1`:

```text
@lafiya/verifier@0.1.0
├─┬ @lafiya/attestation-registry@file:../../bindings/attestation-registry
│ ├─┬ @stellar/stellar-sdk@14.6.1
│ │ ├── @stellar/stellar-base@14.1.0
│ │ ├── axios@1.20.0
│ │ ├── bignumber.js@9.3.1
│ │ ├── commander@14.0.3
│ │ ├── eventsource@2.0.2
│ │ ├── feaxios@0.0.23
│ │ ├── randombytes@2.1.0
│ │ ├── toml@3.0.0
│ │ └── urijs@1.19.11
│ └── buffer@6.0.3
└─┬ @lafiya/attester-registry@file:../../bindings/attester-registry
  ├── @stellar/stellar-sdk@14.6.1 [deduped]
  └── buffer@6.0.3 [deduped]
```

| Dependency | Status | Justification |
| --- | --- | --- |
| `@lafiya/attestation-registry`, `@lafiya/attester-registry` | Kept | Generated clients for the two registries; the single source of truth for their interfaces. Only `src/rpc.ts` (`BindingsChainReader`) imports them. |
| `@stellar/stellar-sdk` (transitive, ~60 packages) | Kept | XDR encoding, simulation and Soroban RPC. Re-implementing them would trade an audited, widely used dependency for unaudited code. Consumers that verify offline (`OfflineChainReader`) or bring their own `ChainReader` never execute it, and the ESM build is `sideEffects: false`, so bundlers can drop it. |
| `buffer` | Removed as a direct dependency | It was only used to build `BytesN` arguments; the contract spec encoder accepts a `Uint8Array`, so `rpc.ts` now passes one. It remains a transitive dependency of the bindings. |

QR parsing, trust-anchor checks, LRC-1 commitments (WebCrypto) and the
verdict logic use no dependencies at all.

