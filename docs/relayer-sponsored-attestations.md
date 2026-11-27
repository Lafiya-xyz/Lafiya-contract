# Relayer-Sponsored Attestations

The `attest` contract method requires authorization from the allowlisted
`attester` address, but does not require that address to be the transaction
source. A relayer can therefore submit the transaction and pay its network fee
without asking a community health worker (CHW) to fund an account or sign the
transaction envelope.

## Authorization and submission

1. The relayer builds an invocation of
   `attestation-registry::attest(attester, record_hash)` and simulates it to
   obtain the Soroban authorization entry required by the contract.
2. The attester signs that authorization entry. It is bound to the contract
   invocation and its arguments; the signature is not a signature on the
   relayer's transaction envelope.
3. The relayer includes the signed entry in the transaction, sets its own
   funded account as the transaction source (or uses a fee-bump transaction),
   signs the transaction as required, and submits it to Soroban RPC.
4. The contract checks `attester.require_auth()`, verifies that the address is
   allowlisted, checks that the contract is not paused, and records the
   attestation.

`attest_versioned` and `anchor_batch` use the same attester-authorization
pattern. Contract accounts can also be attesters: Soroban evaluates their
authorization through the account contract's `__check_auth` implementation.

The registry contract does not implement or operate a relayer. The application
integrating the contract must manage transaction simulation, signed
authorization-entry handling, fee payment, submission, retries, and signer
key security. Authorization entries must be created for the intended network,
contract, method, and arguments and must remain valid when submitted. The
relayer should never receive the CHW's signing key or request a signature over
the outer transaction.


## Argument-bound authorization

`attester.require_auth()` in Soroban commits the signature to the contract,
function name, and **all** arguments of the invocation. The auth tree for a
sponsored `attest` is a single root node with no sub-invocations:

```text
attestation-registry.attest(attester, patient, record_hash)
```

`attest_versioned`, `attest_version`, and `anchor_batch` bind their extra
fields (`commitment_version`, `previous_record_hash`, `root`, `leaf_count`) the
same way. A signature over hash A therefore cannot authorize hash B, and a
relayer cannot alter any bound field. `lafiya auth decode` prints every bound
argument by name so multisig reviewers can verify them before signing. No
public ABI change is required.

## Sponsored claims

Claiming a USDC payout from the incentive pool normally requires the CHW to pay
XLM network fees and hold the minimum account balance. That is a barrier for
the very people the pool exists to pay. The relayer therefore also sponsors
`claim` invocations, under a policy separate from attestations.

### Claim action type

`claim` is its own action type in the relayer policy, distinct from `attest`.
It is disabled unless explicitly enabled, and it carries its own quota and
spend cap so that claim traffic cannot consume the attestation budget (or vice
versa). The relayer builds an invocation of
`incentive-pool::claim(work_item_id)` (or the versioned equivalent), simulates
it to obtain the authorization entry, and has the CHW sign that entry. As with
attestations, the signature is bound to the contract, method, and arguments and
is not a signature over the relayer's transaction envelope.

### Validation

Before submitting a sponsored claim, the relayer must verify:

- The invocation targets the **configured incentive-pool contract** (network,
  contract ID, and method). Claims against any other contract are rejected.
- The `work_item_id` argument refers to a work item that **belongs to the
  signer** (the CHW whose authorization entry is attached). The relayer
  resolves the work item's owner from the registry/pool state and rejects the
  claim if the owner does not match the signer.
- The claim has not already been paid, and the pool is not paused.
- The claim falls within the claim action's quota and spend cap.

The relayer must never accept a pre-built, opaque transaction for a claim; it
builds the invocation itself from the validated `work_item_id` so that the
target contract and owner checks cannot be bypassed.

### Optional fee reimbursement (ADR-gated)

A small, capped fee reimbursement may be deducted from the payout to help fund
the sponsor. This is **not enabled by default** and must only be added if the
ADR approves it, because it complicates payout accounting and the amount the
CHW actually receives. If approved, the reimbursement is capped per claim and
per period, is recorded alongside the payout, and is surfaced to the CHW before
they sign the authorization entry.

## Threat model (claim-specific abuse cases)

In addition to the attestation abuse cases above, the relayer's threat model
must cover:

- **Claim for a work item not owned by the signer.** A malicious or buggy
  client submits a claim whose `work_item_id` belongs to another CHW. Mitigated
  by resolving the work item owner and requiring it to equal the signer, and by
  building the invocation from the validated ID rather than accepting an opaque
  transaction.
- **Claim against the wrong contract.** A claim invocation targeting a
  look-alike or attacker-controlled contract. Mitigated by pinning the
  configured pool contract ID and method and rejecting anything else.
- **Quota exhaustion / denial of service.** An attacker floods the relayer with
  claim requests to exhaust the claim quota or spend cap and starve legitimate
  CHWs. Mitigated by separate per-action quotas and spend caps, per-signer rate
  limits, and monitoring/alerting on quota consumption.
- **Replay and double-claim.** Re-submitting a previously paid claim or
  replaying a signed authorization entry. Mitigated by checking paid state
  before submission and by the argument-bound authorization entry (which
  cannot be repurposed for a different work item).
- **Fee-reimbursement gaming.** If reimbursement is enabled, an attacker
  inflates or repeats reimbursements to drain the sponsor. Mitigated by the
  ADR-gated, per-claim and per-period caps, recording reimbursements with the
  payout, and reconciling reimbursements against actual sponsored fees.
- **Budget cross-contamination.** Claim traffic consuming the attestation
  budget or vice versa. Mitigated by keeping claim and attest quotas and spend
  caps independent.
