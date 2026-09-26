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

