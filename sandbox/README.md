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

Demo commitments use LRC-1 (`lafiya-commitment::commit_v1`) over
`[patient_ref, blood_group, allergies]` as `Text`, followed by the salt as `Bytes`.
This is a demo schema, not a production record layout.

Signing keys are stored as `stellar` CLI identities named `lafiya-sandbox-<name>`. Use
them from your app or scripts with `--source lafiya-sandbox-chw-lagos-1`.

## For `lafiya-web` developers

1. `cargo xtask sandbox up --scenario edge-cases`
2. Point the app at `LAFIYA_RPC_URL` and the two contract IDs from
   `sandbox/.env.local`.
3. Use `sandbox/fixtures.json` for demo records: `patient-001` has a multi-entry
   history, `patient-003` was attested by an attester who is now suspended,
   `patient-004` by one who has since been removed, and `patient-007` and
   `patient-008` are revoked.
