# Spec: Lafiya `stellar.toml` (SEP-1)

Third-party verifiers (clinics, partner NGOs, responder apps) need a domain-anchored way
to find which contract IDs and signing keys really belong to Lafiya on each network.
Lafiya publishes a [SEP-1](https://github.com/stellar/stellar-protocol/blob/master/ecosystem/sep-0001.md)
`stellar.toml` at:

```
https://lafiya.xyz/.well-known/stellar.toml
```

The file is **generated, never edited by hand** (see [Generation](#generation)).

## Content

| Section / key | Standard | Content |
| --- | --- | --- |
| `SIGNING_KEY` | SEP-1 | `G...` key that signs trust bundles and off-chain artifacts. Omitted until provisioned. |
| `[DOCUMENTATION]` | SEP-1 | `ORG_NAME`, `ORG_URL`, `ORG_GITHUB`. |
| `[[PRINCIPALS]]` | SEP-1 | Security contact. Lafiya takes vulnerability reports through GitHub private advisories (see `SECURITY.md`), so this lists the GitHub org rather than an email address. |
| `[[LAFIYA_CONTRACTS]]` | Lafiya extension | One entry per deployed contract per network (below). |

SEP-1 has no general-purpose section for listing smart contracts (only a `contract` field on
`[[CURRENCIES]]` token entries). Lafiya therefore uses the namespaced `LAFIYA_CONTRACTS`
array of tables, which SEP-1 consumers that don't know it will ignore.

### `[[LAFIYA_CONTRACTS]]`

| Key | Type | Meaning |
| --- | --- | --- |
| `name` | string | Contract key as in `config/networks.toml`: `attester_registry`, `attestation_registry` (and `multisig_account` once `networks.toml` records it). |
| `network` | string | Network name as in `config/networks.toml` (e.g. `testnet`, `mainnet`). |
| `network_passphrase` | string | Stellar network passphrase, so the entry is unambiguous without the name. |
| `contract_id` | string | `C...` contract address. |
| `wasm_hash` | string | Hex SHA-256 of the wasm the instance runs (matches the on-chain instance's executable). |
| `release_manifest` | string | URL of the [release manifest](../releasing.md#release-manifest) the contract was deployed from. |

Contracts that are not deployed on a network (empty ID in `networks.toml`) are not listed.

Example:

```toml
SIGNING_KEY = "GABC..."

[DOCUMENTATION]
ORG_NAME = "Lafiya"
ORG_URL = "https://lafiya.xyz"
ORG_GITHUB = "Lafiya-xyz"

[[PRINCIPALS]]
name = "Lafiya security contact (private advisories)"
github = "Lafiya-xyz"

[[LAFIYA_CONTRACTS]]
name = "attester_registry"
network = "testnet"
network_passphrase = "Test SDF Network ; September 2015"
contract_id = "CA..."
wasm_hash = "b6d658b0..."
release_manifest = "https://github.com/Lafiya-xyz/Lafiya-contract/releases/download/v0.1.0/release-manifest.json"
```

## Generation

`scripts/generate_stellar_toml.py` (`make stellar-toml`) writes `config/stellar.toml` from:

- `config/networks.toml`: deployed contract IDs and passphrases;
- the release manifest (`--manifest release-manifest.json`): wasm hashes and the manifest URL;
- `config/stellar-toml.meta.toml`: org metadata, `SIGNING_KEY`, and the manifest URL template.
  This is the only hand-edited input.

Without `--manifest`, a deployed contract's `wasm_hash` and `release_manifest` are carried
forward from the committed `config/stellar.toml` entry with the same contract ID. A newly
deployed contract with no manifest is an error.

CI runs `scripts/generate_stellar_toml.py --check`, which fails when the committed
`config/stellar.toml` no longer matches `networks.toml` and the metadata.

## Verification

```bash
lafiya-cli --network testnet trust verify --domain lafiya.xyz
```

fetches the file over HTTPS (WebPKI roots; `--ca-cert <pem>` for a staging host), then for
the selected network:

1. compares each `[[LAFIYA_CONTRACTS]]` contract ID and passphrase with the local resolved
   config (`config show`);
2. reads each contract's instance from the chain (`getLedgerEntries`, trying every
   `rpc_urls` endpoint) and compares its wasm hash with `wasm_hash` (`--skip-chain` skips this).

Any mismatch is printed as `FAIL ...` and the command exits non-zero. Verifier
integrations should use the same flow: fetch the domain's `stellar.toml` over HTTPS, then
trust only the contract IDs it lists, after checking their on-chain wasm hashes.

Hosting and update procedure: [`docs/releasing.md`](../releasing.md#publishing-stellartoml).
