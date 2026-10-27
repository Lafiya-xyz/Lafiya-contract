# Local sandbox

One command gives you a realistic local chain for developing `lafiya-web`,
`lafiya-verifier`, the indexer, and dashboards. It has both registries wired
together, attesters in every lifecycle state, multi-entry attestation histories,
revocations, and a recent admin transfer.

> **DEV ONLY.** Every sandbox key derives from a public, fixed seed, so anyone can
> recreate it. Never fund these keys, or reuse them, on testnet or mainnet.

## Requirements

- Docker (runs `stellar/quickstart` as the `lafiya-sandbox` container on port 8000)
- [`stellar` CLI](https://developers.stellar.org/docs/tools/developer-tools) (tested with v28)
- The Rust toolchain from `rust-toolchain.toml`, including the `wasm32v1-none` target

## Commands

Run these from the repository root:

```bash
cargo xtask sandbox up --scenario edge-cases   # start, deploy, apply (idempotent)
cargo xtask sandbox verify --scenario edge-cases  # exit 1 if the chain drifted from the scenario
cargo xtask sandbox down                       # remove the container and generated files
cargo xtask sandbox reset --scenario pilot     # down, then up from scratch
```

`--scenario` takes a name from `sandbox/scenarios/` or a path to a `.toml` file. The
default is `minimal`.

`up` reads the current on-chain state first and submits only the missing steps, so
running it again makes no changes. If the `lafiya-sandbox` container is already
running and `.env.local` points at live contracts, those contracts are reused.

## Shipped scenarios

| Scenario | Contents | Fresh `up` |
| --- | --- | --- |
| `minimal` | 2 attesters, 1 attested record | fastest |
| `pilot` | 50 generated CHWs, 500 attestations, plus suspended/removed attesters, a revocation, and an admin transfer | < 3 min |
| `edge-cases` | every attester state (active, suspended after attesting, suspended idle, removed after attesting, idle), histories from several attesters, repeated attestations, histories longer than `MAX_HISTORY`, revocations, and an admin transfer. CI applies and asserts this one. | ~1.5 min |
| `status-list` | a status-list contract deployed and initialized, a bulk set of records attested and then revoked so the list has populated segments, and a signed root published for verifier sync. Use it with the status-list sync demo below. | ~2 min |

## Scenario format

```toml
[admin]
initial = "admin"     # deploys and initializes both registries
current = "admin-2"   # optional: admin is transferred here (propose + accept)

[[attesters]]
name = "chw-lagos-1"
region = "NG_LA"      # optional; with it, add_attester_with_info (+ license hash)
status = "active"     # active | suspended | removed (default: active)

[[attestations]]
record = "patient-001"
by = ["chw-lagos-1", "clinician-2"]   # one attest() per entry, in order

[[revocations]]
record = "patient-007"                # attested by `by`, then revoked
by = ["chw-lagos-1"]

[generate]            # optional bulk data (metadata-less, batch-added attesters)
attesters = 50
attestations = 500
```

Attesters with a `region` are added one at a time with metadata. Attesters without
one are batch-added with `add_attesters`. Attesters that are on chain but not in the
scenario are left alone. Revoking a record deletes its history on chain, so `up`
records applied revocations in `sandbox/.revoked-<contract>` to stay idempotent.

## Generated files (git-ignored)

- **`sandbox/.env.local`** contains `LAFIYA_RPC_URL`, `LAFIYA_NETWORK_PASSPHRASE`,
  `LAFIYA_ATTESTER_REGISTRY_ID`, `LAFIYA_ATTESTATION_REGISTRY_ID`,
  `LAFIYA_ADMIN_ADDRESS`, and one `LAFIYA_DEV_KEY_<NAME>` public key per account.
  Load it with `set -a; . sandbox/.env.local; set +a` or your framework's dotenv
  support.
- **`sandbox/fixtures.json`** maps account names to addresses. For every record it
  gives the plaintext demo fields, the salt, the resulting `record_hash`, who attested
  it, and whether it was revoked. With these, the web app can recompute commitments
  and check them against the chain.
- **`sandbox/.env.local`** also contains `LAFIYA_STATUS_LIST_ID` and
  `LAFIYA_STATUS_LIST_ROOT_KEY` when the `status-list` scenario is applied, so the
  verifier SDK can sync and validate the signed root.

Demo commitments use LRC-1 (`lafiya-commitment::commit_v1`) over
`[patient_ref, blood_group, allergies]` as `Text`, followed by the salt as `Bytes`.
This is a demo schema, not a production record layout.

Signing keys are stored as `stellar` CLI identities named `lafiya-sandbox-<name>`. Use
them from your app or scripts with `--source lafiya-sandbox-chw-lagos-1`.

## Two-deployment federation sandbox

The `federation` scenario runs a second, independent mock deployment next to the
primary one so you can exercise the cross-deployment trust-anchor flow described in
[ADR 0012](../docs/adr/0012-multi-jurisdiction-deployment-topology.md) and the
federation trust-anchor spec. It is additive: `minimal`, `pilot`, and `edge-cases`
are unchanged and still run as single-deployment sandboxes.

```bash
cargo xtask sandbox up --scenario federation
cargo xtask sandbox verify --scenario federation
```

`up` deploys a second pair of registries (attester + attestation) under a separate
admin and governance key, writes a governance-signed trust-anchor document for that
deployment, and pins the primary deployment's anchor. It also seeds one foreign
attestation so the verifier has something to resolve.

### Generated federation files (git-ignored)

- **`sandbox/.env.federation.local`** adds `LAFIYA_FOREIGN_RPC_URL`,
  `LAFIYA_FOREIGN_NETWORK_PASSPHRASE`, `LAFIYA_FOREIGN_ATTESTER_REGISTRY_ID`,
  `LAFIYA_FOREIGN_ATTESTATION_REGISTRY_ID`, and `LAFIYA_FOREIGN_GOVERNANCE_ADDRESS`.
- **`sandbox/trust-anchors/`** holds the signed trust-anchor documents:
  `primary.anchor.json` (pinned by the verifier) and `foreign.anchor.json`
  (produced by the mock second deployment's governance).
- **`sandbox/fixtures.federation.json`** maps foreign records to their
  `record_hash`, attesting deployment, and the anchor that vouches for them.

### How the verifier resolves a foreign jurisdiction

1. Read the attestation's anchoring deployment ID from the on-chain record.
2. Look up that deployment's trust-anchor document in the verifier's pinned set
   (`sandbox/trust-anchors/`). An anchor that is not pinned is rejected.
3. Verify the anchor's governance signature against the pinned governance key for
   that deployment, and check it has not expired or been revoked.
4. Verify the foreign attestation against the foreign registry using the keys and
   endpoints named in the anchor.
5. Report the result as `trusted`, `untrusted`, or `unknown-deployment`.

### Expected read-only cross-registry check

`cargo xtask sandbox verify --scenario federation` performs a read-only check and
asserts:

- the seeded foreign attestation resolves to `trusted` via the pinned foreign anchor;
- a record attested only on the primary deployment resolves to `trusted` locally and
  is not misattributed to the foreign deployment;
- no state is written to either registry during the check.

### Rogue-deployment scenario

The scenario also stages a rogue mock deployment whose anchor is self-signed (its
governance key is not in the verifier's pinned set). The cross-registry check must
reject its attestations as `untrusted` and must not fall back to trusting the
self-signed anchor. `verify` fails if the rogue attestation is ever reported as
`trusted`.

## For `lafiya-web` developers

1. `cargo xtask sandbox up --scenario edge-cases`
2. Point the app at `LAFIYA_RPC_URL` and the two contract IDs from
   `sandbox/.env.local`.
3. Use `sandbox/fixtures.json` for demo records: `patient-001` has a multi-entry
   history, `patient-003` was attested by an attester who is now suspended,
   `patient-004` by one who has since been removed, and `patient-007` and
   `patient-008` are revoked.

## Status-list sync demo

The `status-list` scenario exercises the compact bulk-revocation path described in
`docs/adr/0012-w3c-vc-export-format.md` and implemented in `packages/verifier-sdk`.
Instead of one RPC per record, a verifier downloads the status-list contract's
versioned segments, checks them against the signed root, and answers revocation
queries locally from a cached bitstring.

1. `cargo xtask sandbox up --scenario status-list`
2. Load the environment: `set -a; . sandbox/.env.local; set +a`
3. Run the reference sync demo from `packages/verifier-sdk`:

   ```bash
   cargo run -p lafiya-verifier --example status_list_sync -- \
     --rpc-url "$LAFIYA_RPC_URL" \
     --status-list "$LAFIYA_STATUS_LIST_ID" \
     --root-key "$LAFIYA_STATUS_LIST_ROOT_KEY" \
     --fixtures sandbox/fixtures.json
   ```

   The demo fetches the signed root, syncs only the segments whose version changed
   since the last run, and prints the resulting bitstring size and sync time. It then
   checks every record in `fixtures.json` against the cached list and reports any
   mismatch with the on-chain revocation state.

4. Re-run the demo without changing the chain to see the cache hit: no segments are
   re-fetched and sync time drops to the root check only.

The demo prints measured size and sync time for the scenario's entry count. To
reproduce the 1M-entry measurement from the acceptance criteria, use the
`status-list` scenario's `[generate]` block with `attestations = 1000000` (expect a
long `up`; the sync itself is the part being measured).
