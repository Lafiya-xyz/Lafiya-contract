# API reference (rustdoc)

The rustdoc for this version is published alongside the site:

- [`attester_registry`](api/attester_registry/index.html)
- [`attestation_registry`](api/attestation_registry/index.html)
- [`facility_registry`](api/facility_registry/index.html)
- [`multisig_account`](api/multisig_account/index.html)
- [`lafiya_commitment`](api/lafiya_commitment/index.html)
- [`lafiya_config`](api/lafiya_config/index.html)
- [`lafiya_rpc_resilience`](api/lafiya_rpc_resilience/index.html)

Build it locally with `cargo doc --workspace --no-deps --open`.

## facility-registry

The `facility_registry` contract tracks enrolled health facilities, their
affiliations with attesters, and the deactivation lifecycle. The TypeScript
bindings live in `bindings/facility-registry` and mirror the contract entry
points; the `lafiya-cli facility` subcommands wrap them with simulation
previews and JSON output.

### Portable facility identifiers

Facilities are canonically identified by a portable `FacilityId`:

```
FacilityId = BytesN<32> = H(domain || jurisdiction || issuing_authority || facility_code)
```

where `H` is SHA-256 and `domain` is the fixed ASCII string
`"lafiya.facility.v1"`. The canonicalization rules and test vectors are
documented in `docs/specs/` (canonical facility ID spec). The portable ID is
stable across re-genesis (ADR-0016), multi-jurisdiction deployments
(ADR-0020), and federation, and is the identifier embedded in QR payloads and
verifiable credentials.

The contract supports:

- `get_facility_by_portable_id(id: FacilityId) -> Option<Facility>` — lookup by
  portable ID.
- `get_facility_by_code_hash(code_hash: BytesN<32>) -> Option<Facility>` —
  lookup by the hash of the external facility code.
- `get_facility(id: u64) -> Option<Facility>` — the legacy sequential `u64`
  alias, retained for backward compatibility during migration. New code should
  prefer the portable ID; the `u64` is a local alias only and must not be
  embedded in QR payloads or VCs.

### Affiliation-period history (paginated)

Affiliation periods are stored in fixed-size pages keyed by
`AffiliationPage(facility, attester, page_no)`, with a small header holding the
latest period so that `is_affiliated` is O(1). No single storage entry grows
without bound, regardless of how many affiliate/disaffiliate cycles occur.

- `get_affiliation_periods(facility, attester, cursor: Option<u32>, limit: u32) -> AffiliationPeriodPage`
  — returns a bounded page of affiliation periods. `cursor` is the page number
  to start from (`None` starts at page 0); `limit` caps the number of periods
  returned. The result includes `next_cursor: Option<u32>` which is `None` when
  the last page has been reached. Callers should iterate until `next_cursor`
  is `None`.
- `was_affiliated_at(facility, attester, timestamp: u64) -> bool` — binary
  searches the pages, so read cost is O(log n) storage entries.

The TypeScript bindings expose the same pagination shape
(`getAffiliationPeriods(facility, attester, cursor?, limit)` returning
`{ periods, nextCursor }`), and the CLI command
`lafiya-cli facility list-affiliations` accepts `--cursor` and `--limit`.

### Deactivation semantics

`deactivate_facility` records a `deactivation_reason` code and a timestamp:

| Code | Meaning |
| --- | --- |
| `closed` | Facility closed. |
| `licence_lapsed` | Facility licence lapsed. |
| `fraud` | Facility is untrusted (fraud). |

The reason code and timestamp are stored on the facility and emitted in the
deactivation event. `reactivate_facility` clears the active deactivation.

Verification of facility-scoped attestations (`attest_with_scope`) follows
these rules, mirroring attester compromise windows:

- Attestations made **before** deactivation remain valid for `closed` and
  `licence_lapsed`.
- For `fraud`, attestations made before the deactivation timestamp are marked
  `facility_untrusted` from that timestamp onward.

This model is implemented in the contract read path and in both SDKs, and is
covered by conformance vectors (see `docs/specs/` and the conformance suite).
The Rust model and the TypeScript SDK agree with the contract on these
outcomes.