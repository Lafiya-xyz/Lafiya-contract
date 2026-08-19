# Spike: Contract Interface Conformance & Artifact Compatibility Testing

Tracks [#137](https://github.com/Lafiya-xyz/Lafiya-contract/issues/137). Prototype
tooling lives in [`scripts/conformance/`](../../scripts/conformance); run it with
`make conformance` (see that directory's `README.md` for details).

## Question

What automated conformance suite proves that a deployed contract's Wasm, its
generated TypeScript bindings, its documented events/errors, and downstream
consumer assumptions stay in sync as the contracts evolve?

## TL;DR

The prototype found that the checks it proposes are not hypothetical --
**`bindings/attester-registry` and `bindings/attestation-registry`, as
committed on `main` today, are already badly out of sync with the contracts
they claim to wrap**: 24 of the 33 combined public functions across both
registries, and 8 of 13 error codes, are missing from the generated clients
entirely (details in [Finding 1](#finding-1-the-committed-bindings-are-already-non-conformant)).
Nobody would know this without either reading both files side by side or
running `make bindings` and diffing -- which is exactly the gap this spike
was asked to evaluate solutions for.

## Approaches Evaluated

Four complementary techniques were built and run against the real
`attester-registry` and `attestation-registry` contracts. They are
complementary, not competing -- each catches a different failure mode.

### A. Wasm interface spec snapshotting

`stellar contract info interface --wasm <path> --output json` reads the
`contractspecv0` custom section that `soroban-sdk` embeds in the compiled
Wasm and returns every function signature, the `#[contracterror]` enum, every
`#[contractevent]` schema, and every shared `#[contracttype]` struct as
structured data (`scripts/conformance/extract_interface.py`). Docstrings are
stripped and entries sorted so the normalized output only changes when the
interface itself changes. That gets committed as a golden snapshot
(`scripts/conformance/snapshots/<contract>.json`) and diffed on every run
(`check_snapshot.py`).

Because it reads the *compiled artifact*, not `lib.rs`, it would have caught
issue [#103](https://github.com/Lafiya-xyz/Lafiya-contract/issues/103) (a
merge that landed docs/tests for a feature whose contract-side implementation
never made it into the diff) -- the missing function simply wouldn't appear
in the snapshot.

### B. Binding regeneration diff

Re-run `stellar contract bindings typescript` into a scratch directory and
diff the regenerated `src/index.ts` client surface (exported function names,
`Errors` map) against the committed `bindings/<contract>/src/index.ts`
(`check_bindings_drift.py`). This is the check that surfaced Finding 1 below.
It's a strict subset of what approach A can express (the bindings are just a
rendering of the same spec) but it checks the artifact contributors and CI
actually forgot to regenerate, which is the everyday failure mode, not a
hypothetical one.

### C. Local-deployed conformance testing

Static spec extraction proves the Wasm's *declared* interface; it says
nothing about whether the deployed contract *behaves* the way the docs claim.
To test that, the prototype spun up a local `stellar/quickstart` node
(matching `tests/integration/run.sh`'s pattern), deployed
`attester-registry`, and:

- Confirmed `stellar contract info interface --contract-id <id> --network
  local` (spec pulled from the on-chain instance) is byte-for-byte identical
  to the spec pulled from the local `.wasm` file -- i.e. approach A's
  cheaper local-file read is a valid proxy for the deployed artifact, at
  least for a freshly deployed, unmodified build.
- Called `pause` then `add_attester` and confirmed the transaction fails
  with `Error(Contract, #4)` -- matching `docs/error-codes.md`'s claim that
  `ContractPaused = 4`. This is a *behavioral* check static spec diffing
  cannot do: two contracts can have identical signatures and error enums
  while one silently returns the wrong code or never enforces the pause.

### D. Documentation cross-checks

Two narrower, cheap checks that close specific gaps `check_readme_contracts.py`
(the repo's existing README-vs-`lib.rs` check) doesn't cover:

- **`check_error_docs.py`** -- parses `docs/error-codes.md`'s per-contract
  tables and cross-checks every `(code, variant)` pair against the Wasm's
  error enum, in both directions (doc has a stale code; Wasm has an
  undocumented one).
- **`gen_events_doc.py`** -- generates `docs/events.md`, a full event schema
  reference (topics, data fields, types) directly from the Wasm, and
  `--check` mode fails CI if it's stale. `docs/architecture/event-indexing.md`
  names events consumers should expect but never pinned their shape; this
  gives that shape a single generated source of truth.

## Evaluation

| Criterion | A. Spec snapshot | B. Binding diff | C. Local-deployed | D. Doc cross-check |
|---|---|---|---|---|
| **Detection power** | High -- any signature/error/event/type change | Medium -- only what reaches the generated client | Highest -- catches behavioral bugs static checks can't | Narrow but precise -- doc/spec text mismatches only |
| **False-positive rate** | Low (docs stripped, sorted); doc-only wording edits and *intentional* changes without a snapshot bump both "fail" until the snapshot is updated -- by design | Low for function/error surface; raw `index.ts` text diff is noisy (doc comments, `@stellar/stellar-sdk` peer version) so the checker only diffs the parsed surface, not full text | Low, but only exercises the specific calls the test script makes -- coverage is a function of how much of the state machine you script, not the tool | Low |
| **CI cost** | ~1s per contract (spec extraction only, no network) | ~2-5s per contract (full binding codegen) | High -- needs Docker + a running RPC node, ~30-60s+ per run | ~1s |
| **Maintainability** | Snapshots are auto-generated (`--update`), reviewed as a diff in PRs like any other generated file | Same -- no hand-maintained fixture | Test script must be hand-written per behavior under test and updated as flows change | `docs/events.md` is generated; `docs/error-codes.md` stays hand-maintained (its prose descriptions are the point) so it can drift if a contributor forgets it |
| **Compatibility coverage** | Structural: functions, params, return/error types, event topics/data, shared structs | Structural, client-surface only | Behavioral: auth gates, state transitions, actual error codes returned | Docs-to-spec only |
| **Ease of use for contributors** | `make conformance-update` after a deliberate change; diff shows up in `git diff` like any file | Same fix as today (`make bindings`), just now enforced | Requires Docker locally; not something you'd run on every save | Same |

None of these four alone is "the" answer; A+B+D are cheap enough to run on
every PR and catch the large majority of drift, while C is valuable but
expensive enough that it belongs at a release gate, not per-commit CI.

## Prototype

Scope: both registry contracts end to end (`attester-registry` was the
primary target per the acceptance criteria; `attestation-registry` was added
because, once the harness existed, extending coverage cost nothing and
immediately paid for itself -- see Finding 1). `multisig-account` is not yet
covered (see [Migration Plan](#migration-plan)).

```
scripts/conformance/
  contracts.py              # registry of covered contracts + paths
  extract_interface.py      # wraps `stellar contract info interface`, normalizes output
  spec_types.py             # renders spec type nodes as readable strings
  check_snapshot.py         # approach A
  check_bindings_drift.py   # approach B
  check_error_docs.py       # approach D (errors)
  gen_events_doc.py         # approach D (events) -- also generates docs/events.md
  snapshots/*.json          # committed golden snapshots
  README.md
```

Run everything with `make conformance` (add `-update` to regenerate
snapshots/`docs/events.md` after a deliberate change). See
`scripts/conformance/README.md` for details. Approach C (local-deployed) is
documented above with its exact commands rather than checked in as a script,
since it needs Docker and a running node -- not something to invoke on every
`make conformance` run (see [Recommended CI Stages](#recommended-ci-stages)).

## Failure Examples & Measurements

### Finding 1: The committed bindings are already non-conformant

Running `check_bindings_drift.py` against `main` as it stands today (no
synthetic changes) reports:

| Contract | Wasm functions | Bindings functions | Missing | Wasm errors | Bindings errors | Missing |
|---|---|---|---|---|---|---|
| `attester-registry` | 20 | 6 | **14** (70%) | 6 | 2 | **4** (67%) |
| `attestation-registry` | 13 | 3 | **10** (77%) | 7 | 3 | **4** (57%) |

Missing from `attester-registry`'s client entirely: `pause`, `unpause`,
`is_paused`, `migrate`, `upgrade`, the whole two-step admin-transfer flow
(`propose_admin`/`accept_admin`/`get_admin`), and every allowlist-cap
function (`get_max_attesters`/`set_max_attesters`/`get_attester_count`/
`get_schema_version`/`suspend_attester`/`reinstate_attester`). A `lafiya-web`
developer working only from `bindings/attester-registry` would not know
these entry points exist. This is exactly the "generated interface
diverges from what consumers expect" failure the issue asks about, already
present, not a hypothetical.

Fixing this is a `make bindings && git commit` away and is intentionally
**out of scope for this spike PR** -- it's the natural first task for
[#126](https://github.com/Lafiya-xyz/Lafiya-contract/issues/126) ("Add CI
enforcement for generated TypeScript binding drift"), which should land the
regeneration and the CI gate together so the fix doesn't immediately regress.

### Finding 2: prototype detects an intentional interface break

To confirm the snapshot check actually catches drift (not just passes
vacuously), `contracts/attester-registry/src/lib.rs` was temporarily edited
to (a) rename the public `get_attester_count` entry point to
`attester_count_current`, and (b) change `Error::AllowlistFull`'s value from
`5` to `7` (both changes compile cleanly -- `cargo build` alone would not
catch either). Rebuilding and running `check_snapshot.py` reported:

```
[attester-registry] INTERFACE DRIFT vs attester-registry.json:
  - removed function_v0: get_attester_count
  + added   function_v0: attester_count_current
  ~ changed udt_error_enum_v0: Error
```

The edit was reverted before committing this branch; no contract source
changed in this PR.

### Finding 3: local-deployed parity and behavioral check

Deployed `attester-registry` to a local `stellar/quickstart` node:

- `stellar contract info interface --contract-id <id> --network local`
  (spec read back from the on-chain instance) was byte-for-byte identical to
  the spec read from the local `.wasm` file -- validating that approach A's
  file-based check is a sound (and far cheaper) proxy for testing the
  deployed artifact in ordinary CI.
- After `initialize` + `pause`, calling `add_attester` failed with
  `Error(Contract, #4)`, matching `docs/error-codes.md`'s `ContractPaused =
  4` -- confirming runtime behavior, not just the declared signature, agrees
  with the docs for this path.

## Recommended CI Stages

Proposed, not wired into `.github/workflows/ci.yml` in this PR (per the
issue's "Recommended CI stages" deliverable being a plan, and to keep this
PR focused on the spike itself):

1. **Blocking, on every PR** (~5-10s total): `make conformance` minus the
   binding-drift step's current failures -- i.e. land this *after* the
   bindings are fixed (Finding 1), otherwise it blocks all PRs on day one.
   Covers approaches A, B, D.
2. **Non-blocking initially, promote once stable** (mirrors the existing
   `fuzz` job's `continue-on-error: true` pattern): the same job, added as
   `continue-on-error: true` first so the team can watch it for a cycle
   before making it a hard gate -- the same rollout pattern already used for
   the fuzz job in `ci.yml`.
3. **Release gate only, not per-PR** (approach C): a local-deployed smoke
   test (extending `tests/integration/run.sh`'s pattern with an interface
   parity check and a couple of behavioral assertions like Finding 3) run
   before publishing bindings to `lafiya-web` or tagging a release. Too slow
   and infra-heavy for per-commit CI.
4. Pin the `stellar` CLI version used in CI (e.g. alongside
   `rust-toolchain.toml`) so binding/spec output is deterministic across
   runs -- the `@stellar/stellar-sdk` peer-dependency version in generated
   `package.json` files tracks the CLI version, and an unpinned CLI would
   make `check_bindings_drift.py` flag version churn as if it were interface
   drift.

## Migration Plan for Existing Artifacts

1. **Immediate, separate PR (recommend pairing with #126):** regenerate and
   commit both `bindings/` directories (`make bindings`), fixing Finding 1.
   Coordinate with `lafiya-web` before merging -- its currently-pinned
   binding version is missing the majority of the contracts' surface, so
   consuming the regenerated client is itself a (long overdue) breaking
   change for that repo, not a silent one.
2. Once bindings are current, wire `make conformance` into CI per the
   [staging plan](#recommended-ci-stages) above.
3. Extend `contracts.py` to cover `multisig-account` (not yet included here;
   it has no generated TS bindings today, so only approaches A and D apply
   until/unless bindings are added for it).
4. Fold `check_readme_contracts.py`'s function-table check into
   `make conformance` as a fifth stage so all doc-sync checks live in one
   place, rather than being split across a standalone CI job and this suite.
5. Track schema-version bumps (`SCHEMA_VERSION` in each contract) as a
   trigger to snapshot the *pre-upgrade* interface as a versioned fixture
   (`snapshots/<contract>-v1.json`, etc.) -- not built here, but the
   normalized-JSON format this prototype produces is the right input for
   that once an actual upgrade ships.

## Cross-Repository Impact

- **`bindings/`** -- both directories need regeneration (Finding 1);
  ongoing drift prevention via CI per above.
- **`lafiya-web`** -- consumes `bindings/` per
  `docs/typescript-bindings.md`'s git-dependency strategy; needs to pick up
  the regenerated client and adjust for the many now-available functions
  it previously couldn't call.
- **`lafiya-verifier`** and other **event-indexing consumers** -- benefit
  from `docs/events.md` as a pinned, generated schema instead of
  reconstructing event shapes from `docs/architecture/event-indexing.md`'s
  prose list or the Rust source directly.
- **CI** -- see [Recommended CI Stages](#recommended-ci-stages).

## Follow-Up Opportunities

- Interface snapshot CI (blocking), per the staging plan.
- Binding drift CI (blocking, once Finding 1 is fixed) -- likely the
  concrete implementation for #126.
- Event/error compatibility CI (blocking) -- `check_error_docs.py` +
  `gen_events_doc.py --check`.
- A release-gate job running the local-deployed behavioral checks
  (approach C) before tagging or publishing.
- Versioned compatibility fixtures once the first real contract upgrade
  ships (see Migration Plan item 5).
