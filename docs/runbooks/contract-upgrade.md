# Runbook: Upgrading a Lafiya Contract in Production

**Audience:** release manager / on-call operator performing an on-chain upgrade of
`attester-registry` or `attestation-registry`.

**Scope:** upgrading existing deployed instances (testnet or mainnet). Initial
deployment is out of scope for this runbook.

**Companion tooling:** [`scripts/upgrade.sh`](../../scripts/upgrade.sh) automates the
mechanical steps described here (build → size check → hash → upload → `upgrade()` →
optional `migrate()` → verify). Every step the script performs is also spelled out
manually below, so the procedure can be followed — and audited — without it.

> ⚠️ An upgrade is a security-critical operation: whoever holds the admin key can
> repoint the contract at **any** wasm. Never sign an `upgrade` call for a wasm hash
> you have not personally traced back to reviewed source (see
> [Verify the wasm hash](#3-verify-the-wasm-hash-matches-what-was-reviewed)).

---

## 1. How upgrades work here (background)

Each contract exposes three upgrade-related functions, gated by the admin address set
at `initialize` time:

| Function | Auth | Effect |
| --- | --- | --- |
| `upgrade(new_wasm_hash: BytesN<32>)` | admin | Replaces the contract's code with the already-uploaded wasm blob whose SHA-256 is `new_wasm_hash`. Storage (instance + persistent) is **untouched**. The swap takes effect when the invocation completes successfully. |
| `migrate()` | admin | Runs pending storage-schema migration steps, then records the new schema version. Only needed for schema-changing releases (including the first upgrade of a legacy, pre-versioning instance). |
| `get_schema_version() -> u32` | none | Returns the schema version recorded in instance storage. `0` = no version recorded yet (legacy pre-versioning deployment, or uninitialized contract). |

Key mechanics:

- **Wasm must be uploaded before `upgrade()` runs.** `upgrade()` takes only a
  *hash*; the ledger rejects the update if no code entry with that hash exists.
  `stellar contract upload --wasm <file>` both writes the blob and prints its hash.
- **The on-chain wasm hash is just SHA-256 of the `.wasm` file bytes.** Anyone can
  recompute it locally with `sha256sum` — this is what makes hash verification
  against reviewed source possible (§3).
- **Upgrades are reversible.** The previous code entry stays on the ledger, so a
  rollback is a second `upgrade()` call with the old hash (§6).
- **Schema versioning strategy.** The running code carries a `SCHEMA_VERSION`
  constant (currently `1` in both contracts); each instance records the version it
  is on in instance storage under `DataKey::SchemaVersion`. A release that changes
  the *shape or meaning* of stored data bumps `SCHEMA_VERSION` and extends
  `migrate()` with the step that moves stored data from `N-1` to `N`. Code-only
  releases leave it unchanged. `migrate()` refuses to re-run when no migration is
  pending (`MigrationNotRequired`). Developers: see §5.4 before bumping.

### Storage versioning strategy (developer summary)

1. `DataKey` enums and stored structs are **append-only**: never reorder, remove, or
   renumber variants/fields — `#[contracttype]` serializes by position.
2. New keys/fields come in as **new variants / new keys**, guarded so old instances
   read sanely.
3. A schema-changing release: bumps `SCHEMA_VERSION`, adds an ordered
   `if stored < N { ... }` step to `migrate()`, and ships a changelog entry stating
   the schema impact.
4. `get_schema_version()` is the source of truth tooling (and this runbook) uses to
   decide whether a migration is pending, and to verify one landed.

---

## 2. Pre-upgrade checklist

Work through this **in order**. Do not start §4 until every box is checked. The
script enforces items 4–6 automatically; the rest are on you.

- [ ] **1. Release is reviewed.** The upgrade targets a git tag/commit reviewed in
      the normal PR process (and, once required, backed by an audit report on file).
      Record the exact commit SHA: `git rev-parse HEAD`.
- [ ] **2. Changelog entry.** `CHANGELOG.md` describes the release: what changed,
      whether the **storage schema changed** (`SCHEMA_VERSION` bumped?), and — if so —
      what `migrate()` will do. If the attestation schema consumed by `lafiya-web`
      changes, the cross-repo follow-up is tracked there too (see README → Shared
      Contracts).
- [ ] **3. Tests green.** From a **clean checkout of the release tag**:
      `make check` passes (fmt + clippy + `cargo test --workspace --locked` + wasm
      build). No local modifications: `git status --porcelain` is empty.
- [ ] **4. Wasm-size budget checked.** Soroban caps a contract code entry at
      **65,536 bytes (64 KiB)**. Confirm each wasm fits with headroom:
      ```bash
      wc -c target/wasm32v1-none/release/attester_registry.wasm    # 8,386 B = 13% of cap
      wc -c target/wasm32v1-none/release/attestation_registry.wasm # 16,296 B = 25% of cap
      ```
      Above ~85% of the cap, treat the release as blocked: shrink the code (review
      dependencies/features) or symbol-strip further before proceeding. Current
      sizes are as of the upgrade-ability release; re-measure every release.
- [ ] **5. Expected hash computed and shared.** Build once from the clean tag
      (§4.1–4.2), compute `sha256sum` of each wasm, and send the hash(es) to at
      least one reviewer to **independently reproduce** from the same tag
      (§3). Everyone must agree on the hash *before* anything is uploaded.
- [ ] **6. Upgrade path classified.** Compare `SCHEMA_VERSION` in the release
      against the deployed instance (`get_schema_version`):
      - same version → **code-only upgrade** (skip `migrate()`);
      - release bumped `SCHEMA_VERSION`, or the instance reports `0`/the call fails
        (legacy, pre-versioning code) → **schema-changing upgrade** (§5; `migrate()`
        required after the swap).
- [ ] **7. Rehearsed.** Same steps run against testnet first — ideally the exact
      script invocation with `--dry-run` reviewed by a second operator — on a
      deployment with the same lineage (deployed version) as production.
- [ ] **8. Operator environment ready.** `stellar` CLI installed (`stellar
      version`; this runbook tracks CLI v22+), admin identity/secret loaded
      (`stellar keys ls`), admin account funded, and you know which `--network`
      you are targeting. **Triple-check mainnet vs testnet.**
- [ ] **9. Pre-upgrade snapshot recorded.** Current state for rollback & audit:
      ```bash
      stellar contract info interface --id <CONTRACT_ID> --network <NET>   # exported fns
      stellar contract fetch --id <CONTRACT_ID> --network <NET> --out-file prev.wasm
      sha256sum prev.wasm   # current wasm hash — your rollback target
      ```
      Also note `get_schema_version` output (or that the call failed → legacy),
      and one known live data point (e.g. an allowlisted attester address,
      a known `record_hash` attestation) for the post-upgrade spot checks.
- [ ] **10. Window & comms.** Maintenance window agreed; `lafiya-web` team notified
      if the attestation schema or either contract's function signatures changed
      (they consume these contracts directly).

---

## 3. Verify the wasm hash matches what was reviewed

The **only** thing binding code to contract is the wasm hash. Before the admin
signs anything, the hash must match, three ways:

| Party | Command | Produces |
| --- | --- | --- |
| Reviewer / auditor (from reviewed source) | `git checkout <tag> && cargo build --release --locked --target wasm32v1-none && sha256sum target/wasm32v1-none/release/<crate>.wasm` | `H_reviewed` |
| Operator (local build, step §4.2) | `sha256sum target/wasm32v1-none/release/<crate>.wasm` | `H_operator` |
| Network (after upload, step §4.3) | `stellar contract upload` output line | `H_uploaded` |

**Require `H_reviewed == H_operator == H_uploaded`.** `scripts/upgrade.sh` aborts
if its locally-computed hash differs from the hash the ledger reports for the
uploaded blob; the reviewer comparison is a human step in the checklist.

Failure modes that must stop the procedure:

- **Mismatch between operator and reviewer builds** → one of you is not on the
  claimed commit, or the toolchain differs (wasm builds embed nothing
  timestamp-like, but rustc/SDK versions do change output). Both rebuild with
  `--locked` inside a clean checkout of the same tag and the pinned toolchain
  (`rust-toolchain.toml`). Still mismatched → halt and investigate; do not proceed.
- **Mismatch between local `sha256sum` and the upload result** → the CLI rewrote
  the bytes. ⚠️ **CLI v22+ optimizes wasm by default on `upload`/`deploy`**, which
  changes the bytes and therefore the hash. Always pass `--optimize=false`
  (`upgrade.sh` does) so the uploaded blob is byte-identical to the reviewed file —
  unless the optimized artifact is itself the thing that was reviewed.
- Post-`upgrade()` sanity: `stellar contract fetch --id <ID> --out-file now.wasm`
  and `sha256sum now.wasm` must equal the hash you intended (§4.5). If it doesn't,
  someone changed code under you — investigate before continuing.

---

## 4. The `upgrade()` call sequence

Mechanical path — [`scripts/upgrade.sh`](../../scripts/upgrade.sh):

```bash
scripts/upgrade.sh \
  --contract attester-registry \
  --id <CONTRACT_ID> \
  --source <ADMIN_IDENTITY> \
  --network testnet \
  --expected-schema-version 1 \
  [--run-migrate]        # schema-changing upgrades only (§5)
# Rehearse first: add --dry-run to print every command without submitting anything.
```

The manual equivalent, and what the script does under the hood:

### 4.1 Build from the clean release tag

```bash
git checkout <release-tag>
cargo build --release --locked --target wasm32v1-none
```

### 4.2 Size check + hash

```bash
wc -c target/wasm32v1-none/release/attester_registry.wasm        # ≤ 65536
sha256sum target/wasm32v1-none/release/attester_registry.wasm    # → H_operator (64 hex chars)
```

### 4.3 Upload the wasm blob

```bash
stellar contract upload \
  --wasm target/wasm32v1-none/release/attester_registry.wasm \
  --source-account <ADMIN_IDENTITY> \
  --network testnet \
  --optimize=false
# prints H_uploaded — must equal H_operator (and H_reviewed)
```

### 4.4 Submit the upgrade

```bash
stellar contract invoke \
  --id <CONTRACT_ID> \
  --source-account <ADMIN_IDENTITY> \
  --network testnet \
  --send yes \
  -- upgrade --new-wasm-hash <H_uploaded>
```

Authorization: the admin must sign; contracts call `admin.require_auth()` first
thing, so any other signer fails the transaction.

### 4.5 Verify the swap landed

```bash
# Code now running == the blob you intended:
stellar contract fetch --id <CONTRACT_ID> --network <NET> --out-file now.wasm
sha256sum now.wasm            # == H_uploaded

# Instance survived with its storage:
stellar contract invoke --id <CONTRACT_ID> --network <NET> \
  --source-account <ADMIN_IDENTITY> --send no -- get_schema_version
```

(the script performs this fetch/version verification, asserts
`--expected-schema-version` when given, and aborts loudly on any mismatch).

---

## 5. Storage-schema-changing upgrades

Trigger: the release bumped `SCHEMA_VERSION`, **or** the instance is legacy
(`get_schema_version` returns `0` *or the function doesn't exist yet* — the first
upgrade of a pre-versioning deployment).

### 5.1 Sequence

1. Run §4 end-to-end (the swap itself). The contract now runs new code against
   **old-shaped storage** — deliberately fine, because the new code reads through
   the same append-only layout.
2. Immediately run the migration (same window, before other writes):

   ```bash
   stellar contract invoke \
     --id <CONTRACT_ID> \
     --source-account <ADMIN_IDENTITY> \
     --network <NET> \
     --send yes -- migrate
   ```

   or pass `--run-migrate` to `upgrade.sh` in step §4 and it does this next.

3. Verify: `get_schema_version` now equals the release's `SCHEMA_VERSION`
   (`--expected-schema-version` makes the script assert it).

### 5.2 Post-migration state spot-checks

Prove storage survived the code swap + migration before closing the window:

- `attester-registry`: `is_attester` for a known allowlisted address → `true`;
  `is_attester` for a random address → `false`.
- `attestation-registry`: `get_attestation` for a known `record_hash` → the same
  `Attestation { attester, timestamp }` recorded in snapshot step 9;
  `attester_registry` pointer still consults the right contract (an `attest` by an
  allowlisted attester succeeds on testnet rehearsal).
- Re-running `migrate()` fails with `MigrationNotRequired` — confirming the guard
  (expected behavior, not a problem).

### 5.3 Legacy bootstrap note

Legacy instances (deployed before schema versioning existed) have no
`SchemaVersion` key. Their first upgrade follows §5 exactly: `migrate()` records
version `1` — schema v1 is defined as layout-identical to the legacy layout, so no
data reshaping is needed, only the version write. After that, the instance is on
the normal versioning track.

### 5.4 For developers: authoring a schema-changing release

1. Append new `DataKey` variants / keys only; never reorder existing variants or
   struct fields.
2. Bump `SCHEMA_VERSION` (`1` → `2` → ...).
3. Add the migration step to `migrate()`, oldest first, guarded by the version it
   migrates *from*: `if stored < 2 { /* reshape v1 → v2 */ }`.
4. Unit-test the step with the `env.as_contract(...)` pattern that rewrites an
   instance to the old shape, runs `migrate()`, and asserts both the new version
   and data preservation (`contracts/*/src/test.rs`).
5. State the schema bump and migration behavior in `CHANGELOG.md` and the release
   notes for the operator.

---

## 6. Rollback

Upgrades reverse cleanly because the old code entry stays on the ledger:

1. Get the previous hash — from the pre-upgrade snapshot (§2 item 9), or rebuild
   the previous tag and `sha256sum` it (`H_reviewed` for the old release works the
   same three-way as §3).
2. Run the §4 sequence with the previous hash (`--wasm prev.wasm`, or
   `upload --wasm` + `invoke ... upgrade --new-wasm-hash <prev_hash>`).
3. Verify with `fetch` + `sha256sum` and the functional spot checks.

**Caveat for schema changes:** rollback restores *code*, not data shape. If
`migrate()` already ran, the old code may mis-read new-shaped data. Rule: prefer
**roll-forward** (a fix release on the new schema) once a migration has been
applied; practice rollback of a migrated instance on testnet before relying on it
in production.

---

## 7. Troubleshooting

| Symptom | Likely cause | Action |
| --- | --- | --- |
| `upgrade` tx fails with contract error `1` (`NotInitialized`) | wrong contract id, or instance never initialized | verify `--id`; initialize first |
| tx fails: authorization errors / `require_auth` | signer is not the admin | `stellar keys ls`; sign with the admin identity set at `initialize` |
| host error: wasm hash not found / update fails | blob never uploaded (step skipped) | run step 4.3 (`upload`) before `upgrade()` |
| `sha256sum` ≠ `upload` output | CLI optimized the wasm (`--optimize` default) | re-upload with `--optimize=false`; never mix artifacts |
| operator hash ≠ reviewer hash | dirty tree, wrong tag, toolchain drift | both rebuild clean with `--locked` and the pinned toolchain (§3) |
| `get_schema_version` → `0` | legacy instance, version not recorded | expected on first upgrade → run §5 migrate |
| `get_schema_version` → "function not found" | contract code predates versioning | expected on first upgrade → run §5 migrate |
| `migrate` fails with `MigrationNotRequired` (code `3`/`4`) | no migration pending | already on the current schema — nothing to do |
| wasm build > 65,536 bytes | size budget exceeded | stop; shrink the contract (see §2 item 4) |
| wasm upload works on testnet, prod fees look large | normal — uploads are size-metered | budget XLM for ~size-based fee; rehearse on testnet |

---

## 8. Reference

- Contract functions: `upgrade`, `migrate`, `get_schema_version` —
  `contracts/attester-registry/src/lib.rs`, `contracts/attestation-registry/src/lib.rs`
- Error codes: [`docs/error-codes.md`](../error-codes.md)
- Automation: [`scripts/upgrade.sh`](../../scripts/upgrade.sh) (`--help` for all flags)
- Change log expected by checklist item 2: `CHANGELOG.md`
- Stellar docs: *Upgrading contracts* (`update_current_contract_wasm`), Soroban RPC.

*Validated against Stellar testnet (2026-07-17, `stellar` CLI 27.0.0): real
upgrades between two contract versions, executed end-to-end by `scripts/upgrade.sh`.*

| Case | Contract instance | Txs | Result |
| --- | --- | --- | --- |
| Schema-changing, `attester-registry` v1→v2 | `CBCRV4OYENAUXO2OXWU3JMKDXD7NGVLGXSHOXC55P7XUSHM2MD6JTFZA` | upgrade `6fc4cd49ea081c35d3ae7e3cc59d5add42272b87bb31e20c3e7cd78d9df9f409`, migrate `f8691ed729f30d0a5cf0b13ed0ac8c5975fa88682434c9a98220e3b01e0ab16c` | version 1→2; allowlist intact; re-migration rejected with `MigrationNotRequired` (#3) |
| Schema-changing, `attestation-registry` v1→v2 | `CCWPKEVBYEEDBMX2T4AKBOTTPXCGWNTZQXBOQWOHLVJ7JOWAMX3G6EAX` | migrate `99286e80945d04e1597bc5450fdfa7cb0f394ce10157c9eb2cf661208851d55c` | version 1→2; seeded attestation intact |
| Code-only, `attestation-registry` v2→v2′ | same | upgrade `f1f82532cd21d5410be7022bdabc0eb4d6a4c066df2707fb7329cf960346172e` | version correctly unchanged at 2; new code hash verified on-chain; `attest` still works via the cross-contract allowlist check |

*Reproduce exactly this rehearsal (same contract lineage) before any mainnet run.*
