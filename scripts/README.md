# Lafiya Scripts

> **Issue #402 — CLI Consolidation**
>
> All operational logic now lives in `lafiya-cli` (the Rust CLI). The Bash scripts in
> this directory are **thin deprecation wrappers** that call `lafiya-cli` and print a
> notice. They will be removed in a future release.
>
> **Use the CLI directly:**
> ```bash
> lafiya-cli --network testnet deploy ...
> lafiya-cli --network testnet upgrade --contract attester-registry ...
> lafiya-cli --network testnet smoke-test
> lafiya-cli --network testnet attester add G...
> lafiya-cli --network testnet attestation revoke-by-attester --attester G...
> ```

---

## Centralized Config

`config/networks.toml` is the single source of truth for all networks:

```toml
[testnet]
rpc_url = "https://soroban-testnet.stellar.org"
network_passphrase = "Test SDF Network ; September 2015"

[testnet.contracts]
attester_registry = "C..."
attestation_registry = "C..."
```

**Secrets policy:** Private keys, mnemonics, deployer secrets are **never** stored in
`networks.toml`. They are managed via `stellar` CLI identities or env vars.

## Rust CLI (`crates/lafiya-cli`) — primary entry point

```bash
# Config
lafiya-cli --network testnet config show
lafiya-cli config list
lafiya-cli --network testnet config env

# Attester management
lafiya-cli --network testnet attester is G...
lafiya-cli --network testnet --source admin attester add G...
lafiya-cli --network testnet --source admin attester remove G...

# Attestation queries
lafiya-cli --network testnet attestation get <64-hex>

# Revoke all attestations by a fraudulent attester (#400)
lafiya-cli --network testnet --source admin \
  attestation revoke-by-attester --attester G... --dry-run

# Deploy contracts (#402)
lafiya-cli --network testnet deploy --source deployer --admin G...
lafiya-cli --network testnet deploy --dry-run

# Upgrade a contract (#402)
lafiya-cli --network testnet upgrade \
  --contract attester-registry \
  --source admin \
  --expected-schema-version 1

# Smoke test (#402)
lafiya-cli --network testnet smoke-test

# Preflight checks only (#401)
lafiya-cli --network testnet config show
```

### Preflight checks (#401)

Every mutating command (`deploy`, `upgrade`, `attester add/remove`,
`attestation revoke-by-attester`) automatically runs preflight checks before signing:

1. `getNetwork` — compare RPC passphrase with config.
2. `getLedgerEntries` — compare on-chain wasm hash with release manifest.
3. `getLatestLedger` — detect stale/partitioned RPC.

Use `--skip-preflight` (non-mainnet only) to bypass. Use `--allow-unknown-wasm` to
warn instead of abort on a hash mismatch.

### Signer back-ends (#399)

Select the signing back-end with `--signer <uri>`:

```bash
lafiya-cli --signer identity://alice ...   # stellar CLI file identity (default)
lafiya-cli --signer ledger://0 ...         # Ledger hardware wallet
lafiya-cli --signer gcpkms://projects/... ...   # Google Cloud KMS
lafiya-cli --signer awskms://arn:aws:kms:... ... # AWS KMS
```

See `docs/signer-trust-boundaries.md` for trust boundaries per back-end.

---

## Deprecated Bash scripts

These scripts are compatibility wrappers that call `lafiya-cli` and print a deprecation
notice. Do not add new logic to them.

| Script | Replacement CLI command |
|---|---|
| `scripts/deploy.sh` | `lafiya-cli deploy` |
| `scripts/deploy-testnet.sh` | `lafiya-cli --network testnet deploy` |
| `scripts/upgrade.sh` | `lafiya-cli upgrade` |
| `scripts/admin.sh` | `lafiya-cli attester` / `lafiya-cli attestation` |
| `scripts/smoke-test.sh` | `lafiya-cli smoke-test` |

---

## `scripts/lib/` — shared shell helpers (still used by wrappers)

- `lib/config.sh` — TOML parser via Python `tomllib`/`tomli`; exports `LAFIYA_*` env vars.
- `lib/validate.sh` — offline input validation. Run self-test: `./scripts/lib/validate.sh --self-test`.

---

## Switching networks — one flag

```bash
lafiya-cli --network testnet deploy ...
lafiya-cli --network futurenet deploy ...
lafiya-cli --network local deploy ...
lafiya-cli --network mainnet deploy ...
```

---

## Input validation

All values are validated locally before any network call or stellar CLI invocation:

| Value | Rule |
| --- | --- |
| `--network` | 1–32 chars, letters/digits/`-`/`_` |
| Attester/admin address | 56-char `G...` or `C...` strkey, CRC16-verified |
| Contract IDs | 56-char `C...` strkey |
| Record hash | 64 hex chars |
| `--source` | Identity name or `G...`; secret keys rejected |
| `rpc_url` | `http://` or `https://` with a host |

---

## CI

```bash
make config-check    # validates networks.toml + lafiya-config tests
make config-list     # lists networks
make deploy NETWORK=testnet DRY_RUN=--dry-run
make upgrade CONTRACT=attester-registry NETWORK=testnet DRY_RUN=--dry-run
make smoke-test NETWORK=testnet DRY_RUN=--dry-run
make preflight NETWORK=testnet
```
