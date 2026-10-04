# Testing Improvements & Infrastructure

**Issues:** #375, #376, #377, #378

## Issue #375: Upgrade Compatibility Harness

**Status:** Framework implemented

### What's new:
- `upgrade-fixtures/` directory for historical release WASMs
- `tests/upgrade_compatibility.rs` harness that:
  - Loads fixture WASMs from `upgrade-fixtures/vX.Y.Z/*.wasm`
  - Validates checksums against `checksums.toml`
  - Deploys old WASM and writes scenario data
  - Records pre-upgrade reads as golden files
  - Upgrades to current code and validates equivalence
  - Tests all write paths still work

### Directory structure:
```
upgrade-fixtures/
├── checksums.toml              (SHA-256 validation)
├── v0.1.0-dev/
│   ├── attester-registry.wasm
│   ├── attestation-registry.wasm
│   └── multisig.wasm
└── scenarios/
    └── attestation-registry-scenario.md
```

### Running tests:
```bash
cargo test --test upgrade_compatibility --release
FIXTURE_VERSION=v0.1.0-dev cargo test --release -- --nocapture
```

### Release checklist:
1. Build release: `cargo build --release`
2. Copy WASMs to `upgrade-fixtures/vX.Y.Z/`
3. Compute SHA-256 and update `checksums.toml`
4. Verify: `scripts/verify-fixtures.sh`
5. Commit and tag release

---

## Issue #376: Model-Based Stateful Property Testing

**Status:** Framework implemented

### What's new:
- `tests/stateful_property_testing.rs` with reference model for attestation-registry
- Reference model tracks:
  - Attestations: HashMap<Hash, Vec<(sequence, attester, data)>>
  - Attesters: active, suspended, allowlist
  - Admin, paused state
- Model transitions: Attest, Revoke, AddAttester, RemoveAttester, Suspend, Pause, Unpause
- Invariant checkers:
  - Sequence numbers are monotonic
  - No orphaned suspensions
  - History bounds respected (≤ MAX_HISTORY)

### Example test structure:
```rust
#[test]
fn model_based_property_test_attestation_registry() {
    let mut model = ReferenceModel::new("ADMIN");
    
    // Add attesters, run operations
    model.add_attester("A0").ok();
    model.attest("HASH_1", "A0", data).ok();
    
    // Check invariants
    model.check_sequence_invariant();
    model.check_suspension_invariant();
    model.check_history_invariant(10);
}
```

### Running tests:
```bash
cargo test --test stateful_property_testing --release
RUST_LOG=debug cargo test -- --nocapture  # Verbose output
```

### Next: Integrate proptest-state-machine
- Use proptest strategy generators for Attest(attester_idx, hash_idx)
- Shrink failures to minimal sequences
- Save failing seeds as regression tests
- Use real attester-registry + real allowlist gating

---

## Issue #377: Coverage-Guided Fuzzing

**Status:** Configuration ready (requires libFuzzer setup)

### What's needed:
1. Add `Cargo.toml` dependencies:
   ```toml
   [dev-dependencies]
   libfuzzer-sys = "0.4"
   arbitrary = { version = "1.3", features = ["derive"] }
   ```

2. Create fuzz targets in `fuzz/fuzz_targets/`:
   - `fuzz_attester_registry_ops.rs`
   - `fuzz_attestation_registry_ops.rs`
   - `fuzz_multisig_check_auth.rs`

3. Example target:
   ```rust
   #![no_main]
   use libfuzzer_sys::fuzz_target;
   use arbitrary::Arbitrary;
   use soroban_sdk::testutils::MockEnv;

   fuzz_target!(|data: &[u8]| {
       if let Ok(ops) = ArbitraryOps::from_bytes(data) {
           // Execute ops, check invariants
           assert_no_unexpected_panics();
           assert_documented_errors_only();
           assert_invariants();
       }
   });
   ```

4. Build and run:
   ```bash
   cargo +nightly fuzz run fuzz_attester_registry_ops
   ```

5. CI integration (nightly):
   - Run each target for 10 minutes
   - Upload crashes as artifacts
   - Maintain seed corpus in `fuzz/corpus/`

### Invariant oracles:
- ✅ No unexpected host panics
- ✅ Only documented errors returned
- ✅ State invariants maintained (after fuzz—see #376)

---

## Issue #378: Local Network Integration Tests in CI

**Status:** Workflow implemented

### What's fixed:
1. **Passphrase mismatch**: 
   - Removed hardcoded passphrase from `tests/integration/run.sh`
   - Now reads from `config/networks.toml` via `scripts/lib/config.sh`
   - Validated against Soroban container passphrase

2. **CI job** (`.github/workflows/integration-tests.yml`):
   - Pulls `stellar/quickstart` pinned by digest
   - Runs with `--local --enable-soroban-rpc`
   - Waits for RPC health check with bounded timeout
   - Downloads contract artifacts from build job
   - Runs `tests/integration/run.sh`
   - Uploads logs on failure
   - Fails fast with readable summary

3. **Network configuration**:
   ```bash
   # Get passphrase from single source
   PASSPHRASE=$(grep -A2 'name = "local"' config/networks.toml | grep network_passphrase)
   export NETWORK_PASSPHRASE="$PASSPHRASE"
   export SOROBAN_RPC_HOST="http://localhost:8000"
   ```

### Running locally:
```bash
# Start local Soroban
docker run -p 8000:8000 -p 11626:11626 \
  stellar/quickstart:latest \
  --local --enable-soroban-rpc

# Run integration suite
cd tests/integration
SOROBAN_RPC_HOST=http://localhost:8000 \
  NETWORK_PASSPHRASE="Standalone Network ; February 2017" \
  bash run.sh
```

### Integration test checklist:
- ✅ Deployment transactions
- ✅ XDR encoding via CLI
- ✅ Real fees calculation
- ✅ Footprint validation
- ✅ Contract storage persistence
- ✅ Multi-step workflows
- ✅ Error paths

---

## Testing Pyramid Summary

```
           │
           │  Integration (CI: local network)
           │  └─ Real WASM deployment, fees, footprints
           │
           │  Stateful Property Testing (this PR)
           │  └─ Model-based state machine verification
           │
           │  Coverage-Guided Fuzzing (this PR)
           │  └─ libFuzzer finds edge cases, panics
           │
           │  Unit Tests (existing)
           │  └─ Individual functions, happy paths
           │
           ▼
    ┌──────────────┐
    │  Invariants  │
    │  Validated   │
    └──────────────┘
```

---

## Next Steps

### Immediate (this PR):
- ✅ Upgrade fixture framework
- ✅ Model-based property testing
- ✅ CI integration + passphrase fix
- 📋 Coverage-guided fuzzing templates

### Follow-up PRs:
- [ ] Integrate proptest-state-machine for automated shrinking
- [ ] Add realistic property generators (collisions, history overflow)
- [ ] Connect real attester-registry allowlist gating
- [ ] Fuzz target implementations + nightly CI
- [ ] Seed corpus from manual testing
- [ ] Crash triage + regression test suite

---

## References

- [Upgrade Fixtures README](upgrade-fixtures/README.md)
- [CI Workflow](.github/workflows/integration-tests.yml)
- [Model-Based Testing](tests/stateful_property_testing.rs)
- [Upgrade Compatibility](tests/upgrade_compatibility.rs)
