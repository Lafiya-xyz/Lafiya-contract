# Invariants

Each property the contracts must uphold, numbered so auditors, issues, and
tests can refer to it (for example "breaks INV-AR-3"). The **Checked by**
column maps each invariant to the unit tests (`test.rs`), property tests
(`fuzz_test.rs`, proptest), and integration tests that check it. No formal
proofs exist yet. A **gap** marks an invariant with no dedicated test;
auditors should give those extra attention.

Test paths are relative to `contracts/<crate>/src/`.

## Global (both registries)

| ID | Invariant | Checked by |
|---|---|---|
| INV-G-1 | `initialize` succeeds at most once; a second call returns `AlreadyInitialized`. | attester-registry `test.rs::initialize_twice_fails` |
| INV-G-2 | Every admin-only function requires the current admin's auth and fails with `NotInitialized` before `initialize`. | `test.rs::add_attester_without_admin_auth_fails`, `add_attester_before_initialize_fails`, `update_attester_info_without_admin_auth_fails`, attestation-registry `test.rs::set_attester_registry_by_non_admin_fails`, `revoke_attestation_without_admin_auth_fails`, `test_initialize_auth_matrix` |
| INV-G-3 | Admin transfer is two-step: only the proposed address can accept, and only while a proposal is pending. | both `test.rs::propose_admin_by_non_admin_fails`, `accept_admin_by_wrong_address_fails`, `accept_admin_with_no_pending_proposal_fails`, `successful_admin_transfer_flow`; attester-registry `second_propose_admin_call_overwrites_pending_proposal` |
| INV-G-4 | `upgrade` requires admin auth. | **gap** — no dedicated test |
| INV-G-5 | `migrate` requires admin auth, runs only when stored `SchemaVersion < SCHEMA_VERSION`, and is otherwise a no-op error. | **gap** — no dedicated test |
| INV-G-6 | `get_schema_version` returns the stored schema version, defaulting to 1 for pre-versioning deployments. | attester-registry `test.rs::get_schema_version_succeeds` |
| INV-G-7 | Every documented error code and event exists in the contract, and every contract error and event is documented. | attestation-registry `test.rs::test_error_codes_are_documented`, `test_contract_events_are_documented`; `make conformance` |
| INV-G-8 | `get_interface` reports the contract kind, `INTERFACE_VERSION`, feature list, schema version, and event version; `INTERFACE_VERSION` is bumped on every breaking interface change. | each contract's `test.rs::get_interface_reports_*`; `scripts/conformance/check_snapshot.py` |

## attester-registry (INV-AR)

| ID | Invariant | Checked by |
|---|---|---|
| INV-AR-1 | `is_attester(a)` is true if and only if `a` was added, has not been removed since, and is not suspended. | `fuzz_test.rs::is_attester_matches_model_after_any_op_sequence`; `test.rs::is_attester_false_before_allowlisting`, `remove_attester_revokes_allowlisting` |
| INV-AR-2 | Adding an existing attester is idempotent and does not consume cap. | `fuzz_test.rs::add_attester_is_idempotent_for_is_attester`; `test.rs::re_adding_an_existing_attester_does_not_consume_cap` |
| INV-AR-3 | The attester count never exceeds `max_attesters` through an add; lowering the cap never evicts. | `test.rs::add_attester_beyond_cap_fails`, `removing_an_attester_frees_cap_slot`, `lowering_max_attesters_below_current_count_does_not_evict`; `large_test.rs::large_attester_allowlist_load` |
| INV-AR-4 | Batch add and remove are bounded by `BATCH_LIMIT` and are all-or-nothing. | **gap** — no dedicated test |
| INV-AR-5 | Metadata can only be updated for a current attester, by the admin, while not paused. | `test.rs::update_attester_info_on_unknown_attester_fails`, `update_attester_info_on_removed_attester_fails`, `update_attester_info_without_admin_auth_fails`, `update_attester_info_while_paused_fails` |
| INV-AR-6 | `get_attester_status` agrees with `is_attester` and `get_attester_info`. | `test.rs::get_attester_status_reports_metadata_and_suspension_consistently`, `get_attester_status_for_removed_attester_is_none`, `get_attester_status_for_unknown_attester_is_none` |
| INV-AR-7 | While paused, no allowlist mutation succeeds. | `test.rs::update_attester_info_while_paused_fails` (partial — **gap** for add/remove/suspend) |
| INV-AR-8 | Persistent entries and the instance have their TTL extended on write, so live state is not archived under normal use. | **gap** — no dedicated test |

## attestation-registry (INV-AT)

| ID | Invariant | Checked by |
|---|---|---|
| INV-AT-1 | `attest` succeeds only with the attester's own auth and only if the configured attester-registry reports `is_attester(attester) == true` at call time. | `test.rs::attest_by_allowlisted_attester_succeeds`, `attest_by_non_allowlisted_attester_fails`, `attest_without_attester_auth_fails`, `test_attest_auth_matrix` |
| INV-AT-2 | A suspended or removed attester cannot create new attestations. | covered through INV-AR-1 plus INV-AT-1; **gap** — no end-to-end test |
| INV-AT-3 | `attest` never panics on any 32-byte `record_hash`, before or after `initialize`. | `fuzz_test.rs::attest_never_panics_on_arbitrary_record_hash`, `attest_before_initialize_never_panics`, `repeated_attest_on_same_hash_never_panics` |
| INV-AT-4 | `get_attestation` returns the latest attestation; history holds at most `MAX_HISTORY` entries in chronological order. | `test.rs::re_attest_overwrites_previous_attestation`, `get_attestation_history_returns_all_attestations`, `attestation_history_is_bounded`, `get_attestation_history_boundary_at_max_history` |
| INV-AT-5 | After `revoke_attestation(h)`, neither `get_attestation(h)` nor `get_attestation_history(h)` reports the revoked attestation. | `test.rs::revoke_attestation_happy_path`, `revoke_attestation_clears_get_attestation_history` |
| INV-AT-6 | `initialize` only accepts a contract address that implements `is_attester`. | `test.rs::initialize_rejects_non_contract_address`, `initialize_rejects_unrelated_contract_without_is_attester`, `initialize_accepts_real_attester_registry` |
| INV-AT-7 | `set_attester_registry` requires admin auth and emits `AttesterRegistryRepointed`. | `test.rs::set_attester_registry_by_admin_succeeds`, `set_attester_registry_by_non_admin_fails`, `set_attester_registry_before_initialize_fails` |
| INV-AT-8 | While paused, `attest` fails with `ContractPaused`. | **gap** — no dedicated test |
| INV-AT-9 | Every successful `attest` emits exactly one attestation event. | `test.rs::attest_emits_event` |

## multisig-account (INV-MS)

| ID | Invariant | Checked by |
|---|---|---|
| INV-MS-1 | Construction rejects a zero threshold, a threshold above the signer count, and duplicate signers, and never panics outside `panic_with_error!`. | `test.rs::zero_threshold_is_rejected`, `zero_signers_and_zero_threshold_is_rejected`, `threshold_above_signer_count_is_rejected`, `duplicate_configured_signer_is_rejected`; `fuzz_test.rs::construction_never_panics_on_arbitrary_inputs` |
| INV-MS-2 | `__check_auth` succeeds only with at least `threshold` valid signatures from distinct registered signers over the exact payload. | `test.rs::one_of_one_signer_authorizes`, `two_of_three_signers_authorize`, `three_of_five_signers_authorize`, `fewer_than_threshold_is_rejected`, `unknown_signer_is_rejected`, `signature_for_another_payload_is_rejected`; `fuzz_test.rs::valid_signatures_authorize`, `insufficient_signatures_rejected` |
| INV-MS-3 | Signatures must be strictly ascending by public key, so a signer cannot be counted twice. | `test.rs::duplicate_signature_is_rejected`, `same_signer_appearing_twice_in_signatures_is_rejected`, `signatures_out_of_order_are_rejected`; `fuzz_test.rs::out_of_order_signatures_rejected` |
| INV-MS-4 | The number of signatures is bounded by the signer count (budget-exhaustion protection). | `test.rs::too_many_signatures_is_rejected` |
| INV-MS-5 | Successful auth extends the instance TTL, so the account is not archived while in use. | `test.rs::check_auth_extends_ttl_on_success` |
| INV-MS-6 | The multisig can administer both registries end to end. | `integration_test.rs::multisig_address_administers_both_registries` |

## Commitment scheme (optional scope)

| ID | Invariant | Checked by |
|---|---|---|
| INV-C-1 | The Rust and TypeScript LRC-1 implementations produce identical commitments for every published test vector. | `crates/lafiya-commitment` tests, `crates/lafiya-commitment/reference-ts/lrc1.test.ts`, `vectors/lrc1-test-vectors.json` |
