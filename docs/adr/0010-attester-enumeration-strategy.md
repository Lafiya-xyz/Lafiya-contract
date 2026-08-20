# ADR-0010: Attester directory strategy — event-derived, not on-chain enumeration

- **Status:** Proposed
- **Date:** 2026-08-20
- **Deciders:** Lafiya contract maintainers

## Context

`attester-registry` answers "is this address allowlisted" (`is_attester`,
`get_attester_status`) and "how many attesters are there" (`get_attester_count`), but has
no way to answer "list the attesters." `scripts/admin.sh`'s `attester list` subcommand is a
placeholder that prints a note telling operators to "track attesters off-chain or via
events" — there is no supported recovery path today if that off-chain tracking is lost.

[Issue #134](https://github.com/Lafiya-xyz/Lafiya-contract/issues/134) asks which
enumeration strategy — paginated on-chain storage, an event-derived directory, or a
hybrid — gives operators an authoritative, recoverable directory view without unbounded
Soroban storage or resource costs, and requests at least two prototyped/benchmarked
strategies, documented mutation and recovery semantics, resource costs at representative
allowlist sizes, and an explicit ownership boundary.

This ADR does not operate in a vacuum: `docs/architecture/event-indexing.md` already
specifies an event-indexing service design (polling/streaming, persisted cursor,
idempotent replay, catch-up sweep on restart) built for syncing `lafiya-web`'s Supabase
profile data from these same contract events. This ADR evaluates whether that existing
design, extended to also materialize the attester *directory* (not just per-profile
verification state), is an adequate foundation for enumeration and recovery — rather than
re-specifying event indexing from scratch — and prototypes the on-chain alternative to
give a real cost comparison.

## Decision

**Do not add on-chain enumeration storage to `attester-registry`.** The attester directory
is derived off-chain from the events the contract already emits
(`AttesterAdded`, `AttesterInfoUpdated`, `AttesterRemoved`, `AttesterSuspended`,
`AttesterReinstated`), using the indexer design in `docs/architecture/event-indexing.md`.
`is_attester` / `get_attester_status` remain the sole on-chain source of truth for
individual membership; the directory is an off-chain, eventually-consistent *view* over
those events, never the authority for a single membership check.

To make that view's completeness verifiable rather than merely assumed, add one small
on-chain primitive — a monotonic mutation cursor:

```rust
DataKey::MutationSeq  // u64, instance storage
```

incremented by one on every `add_attester`, `add_attester_with_info`,
`update_attester_info`, `remove_attester`, `suspend_attester`, and `reinstate_attester`
call, alongside the existing `AttesterCount` write and `extend_ttl` bump those functions
already perform in the same transaction — so it adds one storage write to an operation
that already writes instance storage, not a new write class. Expose it via
`get_mutation_seq() -> u64`. This is the "hybrid contract cursor" the issue asks about: it
does not store the directory on-chain, but it lets an indexer prove, after any replay, that
it has observed every mutation rather than merely a count that happens to match (see
"Recovery procedure").

This makes the ownership boundary explicit: **the contract owns membership authority, the
attester count, and the mutation cursor; the indexer owns the enumerable directory and its
history.** See "Recommended ownership boundary" below.

## Alternatives considered

### Paginated on-chain address storage (prototyped)

`contracts/attester-registry/src/enumeration_spike.rs` prototypes this as a standalone
`PaginatedDirectory` contract (kept separate from `AttesterRegistry` so the benchmark
doesn't require shipping unused entry points): `add` appends to a `Vec<Address>` page
capped at `PAGE_SIZE = 200`, allocating a new page every 200 entries; `page(i)` reads one
page. See "Resource benchmarks" for measured costs.

Rejected as the primary mechanism:

- **Removal has no free option.** None of the three ways to remove an address from a
  paginated, page-indexed list are free: a tombstone (mark-and-skip) permanently wastes a
  slot and rent even after the attester is gone; swap-with-last-and-pop keeps pages dense
  but changes which page/offset every subsequent address lives at, so a cursor an operator
  or client saved between calls silently points at the wrong address after any removal
  elsewhere in the list; full page compaction is correct but is an O(directory size)
  rewrite, which is exactly the unbounded-cost shape this issue is trying to avoid.
  `AttesterRegistry` already supports remove/suspend/reinstate as O(1) operations today;
  adding a directory structure that turns removal into a real design problem is a
  regression, not a neutral addition.
- **Archival/TTL bookkeeping scales with directory size.** `AttesterRegistry`'s existing
  `extend_ttl` calls only bump *instance* storage (one TTL for `Admin`, `AttesterCount`,
  etc.), not the per-address `Attester(Address)` persistent entries, which already carry
  their own independent TTL today. A paginated directory adds up to
  `50,000 / 200 = 250` more persistent entries (at `DEFAULT_MAX_ATTESTERS`), each with its
  own TTL that nothing currently extends — a real archival gap the prototype surfaces:
  without new admin tooling to walk and bump every chunk, pages silently expire and must
  be individually restored before they are next readable.
- **It duplicates data the contract already has.** Every address in a page also exists as
  a `DataKey::Attester(Address)` entry; paginated storage roughly doubles the
  per-attester persistent-storage footprint (and rent) to serve a capability
  (enumeration) that only the admin/indexer needs, not the `is_attester` hot path used by
  `attestation-registry`.
- **It does not remove the off-chain dependency it's meant to replace.** Operators still
  need an off-chain client to page through results, format them, and reconcile against
  suspension state — the "no indexer" benefit is smaller than it looks, since
  `is_attester` already requires an off-chain caller for every other query pattern.

It remains the right fallback if a future requirement demands enumeration with *zero*
off-chain trust (e.g., a client that must reconstruct the directory using only the
contract, with no indexer or archive available at all) — see "Follow-up."

### Pure event-derived directory, no on-chain cursor

Reconstruct the directory purely from `getEvents`, per
`docs/architecture/event-indexing.md`, with no contract change at all. This is the
cheapest option (zero new on-chain cost) and was seriously considered as the full
decision.

Rejected as insufficient on its own because it has no cheap way to *prove* completeness
after a recovery sweep. `get_attester_count()` lets a recovered indexer sanity-check its
reconstructed directory size, but count alone cannot detect a "gap that nets to zero" —
e.g., one `remove_attester` and one `add_attester` landing in a missed window leaves the
count unchanged while the directory content is wrong. The monotonic `MutationSeq` in the
Decision closes exactly this gap for one extra instance-storage write, which is why the
hybrid was chosen over the pure event-derived option.

### Comparison matrix

| Criterion | Paginated on-chain (Strategy A) | Event-derived only | Hybrid: event-derived + cursor (chosen) |
| --- | --- | --- | --- |
| Correctness (steady state) | Authoritative by construction | Depends on indexer correctness | Depends on indexer correctness, but verifiably so |
| Recovery after indexer downtime | Always available from the contract alone | Replay from persisted cursor; full rebuild needs historical event access if the RPC retention window was exceeded | Same as event-derived, plus an exact "did I miss anything" check via `get_mutation_seq()` |
| Storage rent | New cost: ~250 extra persistent entries at the 50k cap, each independently subject to archival | No new on-chain storage | One extra `u64` in existing instance storage (already TTL-managed) |
| Query cost (directory read) | `page()` cost independent of directory size (see benchmark) | Off-chain (indexer's own storage/query cost, not a contract cost) | Off-chain, same as event-derived |
| Mutation cost | New: `add()`/removal write(s) on top of existing `Attester(Address)` writes | Unchanged — today's `add_attester`/`remove_attester` cost | Unchanged today's cost + one `u64` instance write already covered by the existing `extend_ttl` transaction |
| Implementation complexity | Medium (contract) + still needs an off-chain paging client | Low (contract) + indexer per event-indexing.md | Low (contract) + indexer per event-indexing.md |
| Operational ownership | Contract owns directory; no separate service, but no removal-safe design either | Indexer owns directory; contract has no completeness signal | Contract owns membership/count/cursor; indexer owns directory, verifiably |

## Mutation and recovery semantics

**Mutations.** All state-changing entry points that affect membership already exist and
are unchanged by this ADR: `add_attester`, `add_attester_with_info`,
`update_attester_info`, `remove_attester`, `suspend_attester`, `reinstate_attester`. Each
already emits a distinct event (`AttesterAdded`, `AttesterInfoUpdated`, `AttesterRemoved`,
`AttesterSuspended`, `AttesterReinstated`) — the indexer applies these exactly as
`docs/architecture/event-indexing.md` §"Profile Updater" describes, extended to write an
`attesters` directory table (that document's design already lists this as one of its
target write paths). This ADR adds one thing to each of those six entry points: increment
and persist `DataKey::MutationSeq`.

Removal, suspension, and reinstatement remain semantically distinct and stay that way in
the directory: `AttesterRemoved` means the entry is gone (never re-appears without a new
`AttesterAdded`, which is itself distinguishable from `AttesterInfoUpdated`);
`AttesterSuspended`/`AttesterReinstated` toggle an active flag without removing directory
history, matching `get_attester_status`'s existing `suspended: bool` semantics. The
indexer must apply events in ledger order per address (already required by
`event-indexing.md`'s idempotency design) — this ADR does not change that requirement.

**Recovery procedure**, extending `event-indexing.md`'s existing catch-up sweep:

1. On restart, the indexer loads its persisted cursor (last processed ledger) and its
   last-recorded `mutation_seq` value.
2. It runs its normal catch-up sweep (`getEvents` from the persisted cursor to the current
   ledger) and applies any missed mutation events to its directory table, exactly as
   `event-indexing.md` specifies.
3. After the sweep, it calls `get_mutation_seq()` and confirms the on-chain value equals
   `(its last-recorded mutation_seq) + (number of mutation events it just applied)`. A
   match proves the directory reflects every mutation since the indexer's last known-good
   state, not merely that the count happens to agree.
4. **If they don't match**, the gap is larger than the RPC node's event retention window
   (Stellar RPC nodes retain events for a bounded ledger window, not indefinitely) and
   incremental replay cannot close it. The indexer must fall back to a full historical
   source — a Stellar history archive or an archive-backed indexer (e.g. a Galexie/Hubble-
   class pipeline) — to rebuild the directory from contract genesis, or, for a small
   allowlist, an admin-assisted backfill that re-checks `is_attester` for every
   historically-known candidate address. Until the rebuild completes and the cursor check
   in step 3 passes, the directory must be served as `stale` (see "Downstream API
   implications").
5. This is the one scenario where paginated on-chain storage (Strategy A) would have given
   a strictly stronger guarantee — the directory would never need external historical
   data at all. It is not chosen as the default because that guarantee is paid for on
   every mutation and every ledger of storage, not only during the rare recovery case; see
   "Follow-up" for when to revisit that trade-off.

## Resource benchmarks

Measured via `env.cost_estimate().budget()` on native (non-Wasm) `Env`, cumulative from
`Env` creation — the same methodology `large_test.rs` already uses for `AttesterRegistry`
itself. These are relative-regression guardrails, not network fee predictions, but they
are directly comparable to each other because both prototypes use the same harness. Run
with `cargo test -p attester-registry --lib -- --nocapture`; numbers below are from a run
on 2026-08-20:

| Strategy | Operation | Attesters (cumulative) | CPU instructions | Memory (bytes) |
| --- | --- | --- | --- | --- |
| Event-derived (today's `add_attester`, `large_test.rs`) | `add_attester` | 10 | 150,113 | 50,865 |
| | | 100 | 326,579 | 124,305 |
| | | 1,000 | 1,953,691 | 858,705 |
| Paginated (prototype `add`, `enumeration_spike.rs`) | `PaginatedDirectory::add` | 10 | 110,885 | 44,167 |
| | | 100 | 230,765 | 140,467 |
| | | 1,000 | 366,105 | 248,283 |
| Paginated (prototype `page`, `enumeration_spike.rs`) | `page(0)` vs. `page(4)` (last of 5 pages @ `PAGE_SIZE=200`, 1,000 entries) | — | 0 vs. 0 | — |

The `page()` result is the most important line in this table: reading the *last* page —
behind 1,000 entries' worth of other pages the read never touches — costs exactly as
little as reading the first, confirming pagination gives O(1) read cost independent of
directory size, the property Strategy A exists to provide. (The two costs measuring as
identically zero, rather than merely close, reflects that the native cost model attaches
its charge to the storage *write* path far more than to single-key reads at this scale;
either way, the comparison — not the absolute number — is what the prototype needed to
show.)

The `add` comparison is the more consequential number for the Decision above: paginated
`add()` is cheaper per call in this prototype (366,105 vs. 1,953,691 cumulative CPU at
1,000 attesters) only because it writes a smaller struct (`AttesterInfo` plus an
`Option<BytesN<32>>`/`Option<Symbol>` in the real contract vs. a bare `Address` in the
prototype) and skips `AttesterRegistry`'s cap check, count bookkeeping, and TTL
extension — it is not a fair apples-to-apples cost of "enumeration" alone. What it does
not show, and what the "Alternatives considered" section is about instead, is the *rent*
and *archival* cost of the up-to-250 extra chunk keys that storage would add on top of
`AttesterRegistry`'s existing per-attester entries — a cost this cumulative-CPU benchmark
does not capture at all, since `cost_estimate()` measures instruction/memory budget, not
ledger rent accrual over time.

At the configured `DEFAULT_MAX_ATTESTERS` cap (50,000): the paginated strategy needs
`50,000 / 200 = 250` chunk entries plus 1 `ChunkCount` plus 1 `TotalCount` — 252 persistent
storage entries, each independently subject to TTL/archival (see "Alternatives
considered"). The hybrid strategy adds exactly one `u64` to existing instance storage,
regardless of allowlist size.

## Recommended ownership boundary

- **`attester-registry` (this repo) owns:** membership authority (`is_attester`,
  `get_attester_status`), the attester count, the event catalog, and the new mutation
  cursor. It never owns the enumerable list itself.
- **The event indexer (design in `docs/architecture/event-indexing.md`, target repo
  `lafiya-event-indexer` per that document's "Repository Ownership" section) owns:** the
  materialized attester directory table, its persisted cursor, replay/idempotency, and the
  recovery procedure in this ADR, including the `mutation_seq` completeness check.
- **`lafiya-web` and `lafiya-verifier` own:** presenting directory data to end users and
  verifiers, sourced from the indexer, never from a direct contract enumeration call
  (there isn't one).
- **The operator CLI (`scripts/admin.sh`, and any successor in `lafiya-cli`) owns:**
  exposing directory queries backed by the indexer's API, and a documented manual-recovery
  path (step 4 above) for the case where the indexer itself needs rebuilding.

## Downstream API implications

- `scripts/admin.sh`'s `attester list` placeholder (`"Note: attester-registry does not
  support enumeration on-chain... track attesters off-chain or via events."`) should be
  replaced with a call to the indexer's directory API once it exists, or explicitly
  documented as "requires the indexer service" in the interim — tracked as follow-up, not
  done in this ADR.
- Any client currently polling `get_attester_count()` to detect directory drift should
  also read `get_mutation_seq()` once available, since count alone cannot detect a
  same-size add+remove pair (see "Alternatives considered").
- The event catalog in `docs/architecture/event-indexing.md` becomes a formal
  contract-to-indexer interface once an indexer is built against it. Changing any of
  `AttesterAdded` / `AttesterInfoUpdated` / `AttesterRemoved` / `AttesterSuspended` /
  `AttesterReinstated`'s shape is a shared-contract change under `CONTRIBUTING.md`'s
  existing cross-repo-impact rule, exactly as attestation schema changes already are.
- A directory served by the indexer must expose its own freshness (last-reconciled
  `mutation_seq`/cursor) alongside the list, so `lafiya-web` and operators can distinguish
  "authoritative" from "stale, rebuilding" per the recovery procedure — this is a new
  requirement on the indexer's response shape, not on the contract.

## Consequences

### Positive

- No new persistent on-chain storage or per-attester rent; the 50,000-attester cap stays
  cheap regardless of enumeration needs.
- Removal, suspension, and reinstatement keep their existing O(1), tombstone-free contract
  semantics — the directory strategy does not force a compaction or indexing scheme onto
  the contract itself.
- The mutation cursor gives recovery a precise, cheap correctness check instead of relying
  on "the count looks right" or blind trust in the indexer.
- Builds directly on `docs/architecture/event-indexing.md` rather than duplicating or
  contradicting it.

### Trade-offs and risks

- The directory is only as available as the indexer; until one is built (see "Follow-up"),
  operators have no enumeration path beyond the manual `is_attester` checks
  `scripts/admin.sh` already documents.
- Recovery beyond the RPC event-retention window depends on access to a full historical
  ledger source (archive/Galexie-class pipeline), which is new infrastructure this repo
  does not provide or own.
- `MutationSeq` is a new piece of contract state and a new (additive, non-breaking) public
  read method, which is itself a small increase in the contract's public surface and
  storage-schema footprint (tracked the same way `SchemaVersion`/`migrate()` already
  version other additions).

## Follow-up

- Implement `DataKey::MutationSeq` and `get_mutation_seq()` in `AttesterRegistry` as a
  focused implementation PR against this ADR.
- Build (or scope as a new repo, per `event-indexing.md`'s "Repository Ownership") the
  event indexer that materializes the attester directory table and implements the
  recovery procedure above, including the `mutation_seq` completeness check and the
  stale/authoritative freshness flag.
- Replace `scripts/admin.sh`'s `attester list` placeholder with a real call to the
  indexer's directory API, and add an operator-facing manual-recovery command for the
  full-historical-rebuild case.
- Revisit paginated on-chain storage (Strategy A) only if a concrete requirement emerges
  for enumeration that must work with zero off-chain infrastructure available (no indexer,
  no archive) — at that point, also design the removal semantics this ADR found
  unresolved (tombstone vs. swap-remove vs. compaction) rather than reusing the
  append-only prototype as-is.
- Snapshot tooling: periodic indexer-side directory snapshots (content-addressed, per
  `mutation_seq`) so a recovering indexer can start from a recent trusted snapshot instead
  of always replaying from genesis.

## References

- [Issue #134](https://github.com/Lafiya-xyz/Lafiya-contract/issues/134)
- [`docs/architecture/event-indexing.md`](../architecture/event-indexing.md) — existing
  event-indexing service design this ADR builds on
- [`contracts/attester-registry/src/enumeration_spike.rs`](../../contracts/attester-registry/src/enumeration_spike.rs) —
  paginated on-chain storage prototype and benchmark
- [`contracts/attester-registry/src/large_test.rs`](../../contracts/attester-registry/src/large_test.rs) —
  existing `add_attester` cost benchmark, reused here as the event-derived strategy's
  on-chain cost baseline
- [`scripts/admin.sh`](../../scripts/admin.sh) — operator CLI referencing this gap today
- [`docs/adr/0003-single-admin-initial-model.md`](0003-single-admin-initial-model.md)
