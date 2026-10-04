# Upgrade Compatibility Fixtures

Historical release WASMs for testing upgrade compatibility.

## Directory Structure

```
upgrade-fixtures/
├── README.md                 (this file)
├── checksums.toml            (SHA-256 checksums + release metadata)
├── v0.1.0-dev/
│   ├── attester-registry.wasm
│   ├── attestation-registry.wasm
│   └── multisig.wasm
├── v0.2.0/
│   ├── attester-registry.wasm
│   ├── attestation-registry.wasm
│   └── multisig.wasm
└── scenarios/
    ├── attester-registry-scenario.md
    ├── attestation-registry-scenario.md
    └── multisig-scenario.md
```

## Checksum Verification

Every fixture WASM must be validated against `checksums.toml`:

```toml
["v0.1.0-dev".attester-registry]
sha256 = "abc123..."
released = "2026-01-15"
source = "https://github.com/Lafiya-xyz/Lafiya-contract/releases/tag/v0.1.0-dev"

["v0.1.0-dev".attestation-registry]
sha256 = "def456..."
released = "2026-01-15"
source = "..."
```

## Test Scenarios

Each contract has a scenario script that:
1. Registers the old WASM
2. Writes representative test data
3. Runs read operations and records results
4. Uploads new WASM and calls `upgrade()`
5. Calls `migrate()` if needed
6. Re-runs all read operations and asserts equivalence
7. Validates all write paths still work

### Attestation Registry Scenario

Representative data to write:
- 3+ attesters (mix of active, suspended, revoked)
- Attesters with and without metadata
- Attestations with full history (MAX_HISTORY entries)
- Revocations with counter resets
- Pending admin proposals
- Paused state toggles

Expected reads:
- `get_attestation(hash, attester)` — full history
- `get_attestation_history(hash)` — ordered by sequence
- `get_attesters()` — active list
- `get_attester(address)` — metadata
- Error cases: suspended, revoked, allowlist gated

## Release Checklist

When tagging a release:

1. Build the release WASM:
   ```bash
   cargo build --release
   cp target/wasm32-unknown-unknown/release/*.wasm upgrade-fixtures/vX.Y.Z/
   ```

2. Compute SHA-256:
   ```bash
   shasum -a 256 upgrade-fixtures/vX.Y.Z/*.wasm
   ```

3. Update `checksums.toml` with hashes and release date

4. Verify integrity:
   ```bash
   scripts/verify-fixtures.sh
   ```

5. Commit and tag:
   ```bash
   git add upgrade-fixtures/
   git tag vX.Y.Z
   ```

## Running Upgrade Tests

```bash
# Test all fixtures against current code
cargo test --test upgrade_compatibility --release

# Test a specific version
FIXTURE_VERSION=v0.1.0-dev cargo test --test upgrade_compatibility --release

# Verbose output
RUST_LOG=debug cargo test --test upgrade_compatibility --release -- --nocapture
```

## Fixture Maintenance

- Fixtures are immutable once released
- Never modify a released fixture WASM
- New versions always test against the previous version (cascading upgrades)
- Keep only the last 3-5 versions to manage repository size
- Archive old fixtures to external storage if needed

## Known Issues & Workarounds

- **Lazy migration:** if a data migration is deferred, the scenario must run `migrate()` explicitly after `upgrade()`
- **Event schema changes:** the scenario must parse events from both old and new schemas if the event type changes
- **New storage keys:** if new keys are added without migration, the scenario should verify they're initialized to defaults
