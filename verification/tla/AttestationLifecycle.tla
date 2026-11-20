------------------------------ MODULE AttestationLifecycle ------------------------------
(***************************************************************************)
(* A TLA+ model of the patient-facing attestation lifecycle.               *)
(*                                                                         *)
(* This module is intentionally self-contained: it EXTENDS only Naturals   *)
(* and models no governance or multisig state, so TLC can explore it       *)
(* independently of the governance models in this directory.               *)
(*                                                                         *)
(* Modelled lifecycle:                                                     *)
(*   - consent: grant / expire / consume                                   *)
(*   - attest (single attestor)                                            *)
(*   - co-attest and quorum                                                *)
(*   - dispute / clear dispute                                             *)
(*   - revoke                                                              *)
(*   - withdraw                                                            *)
(*   - record versioning (previous_record_hash)                            *)
(*   - status-list publication                                             *)
(*                                                                         *)
(* Safety invariants:                                                      *)
(*   RevokedNeverVerified                                                  *)
(*   ConsentConsumedAtMostOnce                                             *)
(*   DisputedNeverVerifiedWithoutClear                                     *)
(*   QuorumMonotonicUnderRevocation                                        *)
(*                                                                         *)
(* Liveness (under fairness):                                              *)
(*   LivenessAttestEventuallyResolves                                      *)
(*   LivenessDisputeEventuallyClears                                       *)
(*                                                                         *)
(* The model is deliberately small so the state space stays tractable for  *)
(* TLC. See AttestationLifecycle.cfg for the model-checking configuration  *)
(* and the documented state-space size.                                    *)
(***************************************************************************)

EXTENDS Naturals

CONSTANTS
    \* Number of attestors participating in the lifecycle.
    Attestors,
    \* Quorum required for a record to be considered "verified".
    Quorum,
    \* Maximum number of consent grants that may be issued.
    MaxConsents,
    \* Maximum number of record versions (bounded for TLC).
    MaxVersions

ASSUME QuorumAssumption == Quorum \in (1 .. Attestors)
ASSUME AttestorsAssumption == Attestors \in Nat \ {0}
ASSUME MaxConsentsAssumption == MaxConsents \in Nat \ {0}
ASSUME MaxVersionsAssumption == MaxVersions \in Nat \ {0}

VARIABLES
    \* Consent lifecycle: "none", "granted", "consumed", "expired".
    consentState,
    \* Number of times consent has been consumed (must be <= 1).
    consentConsumedCount,
    \* Set of attestors who have co-attested the current record version.
    attestations,
    \* Whether the current record version is disputed.
    disputed,
    \* Whether the current record version has been revoked.
    revoked,
    \* Whether the current record version has been withdrawn.
    withdrawn,
    \* Current record version number (bounded by MaxVersions).
    version,
    \* Hash of the previous record version (abstracted as a version id).
    previous_record_hash,
    \* Whether the current record version has been published to the status list.
    published,
    \* Whether the current record version is considered "verified".
    verified

vars == << consentState, consentConsumedCount, attestations, disputed,
           revoked, withdrawn, version, previous_record_hash, published,
           verified >>

\* All variables are typed to keep the state space finite and explicit.
TypeOK ==
    /\ consentState \in { "none", "granted", "consumed", "expired" }
    /\ consentConsumedCount \in 0 .. 1
    /\ attestations \subseteq (1 .. Attestors)
    /\ disputed \in BOOLEAN
    /\ revoked \in BOOLEAN
    /\ withdrawn \in BOOLEAN
    /\ version \in 0 .. MaxVersions
    /\ previous_record_hash \in 0 .. MaxVersions
    /\ published \in BOOLEAN
    /\ verified \in BOOLEAN

\* A record is "verified" iff it has reached quorum, is not disputed,
\* not revoked, not withdrawn, and has been published.
IsVerified ==
    /\ Cardinality(attestations) >= Quorum
    /\ ~disputed
    /\ ~revoked
    /\ ~withdrawn
    /\ published

\* The record is "live" (eligible for further lifecycle actions) iff it
\* has not been revoked or withdrawn.
IsLive == ~revoked /\ ~withdrawn

---------------------------------------------------------------------------
\* Initial state.
---------------------------------------------------------------------------
Init ==
    /\ consentState = "none"
    /\ consentConsumedCount = 0
    /\ attestations = {}
    /\ disputed = FALSE
    /\ revoked = FALSE
    /\ withdrawn = FALSE
    /\ version = 0
    /\ previous_record_hash = 0
    /\ published = FALSE
    /\ verified = FALSE

---------------------------------------------------------------------------
\* Consent actions.
---------------------------------------------------------------------------

\* Grant consent (only from "none" or "expired").
GrantConsent ==
    /\ consentState \in { "none", "expired" }
    /\ consentState' = "granted"
    /\ UNCHANGED << consentConsumedCount, attestations, disputed, revoked,
                    withdrawn, version, previous_record_hash, published,
                    verified >>

\* Expire consent (only from "granted").
ExpireConsent ==
    /\ consentState = "granted"
    /\ consentState' = "expired"
    /\ UNCHANGED << consentConsumedCount, attestations, disputed, revoked,
                    withdrawn, version, previous_record_hash, published,
                    verified >>

\* Consume consent (only from "granted", at most once).
ConsumeConsent ==
    /\ consentState = "granted"
    /\ consentConsumedCount = 0
    /\ consentState' = "consumed"
    /\ consentConsumedCount' = consentConsumedCount + 1
    /\ UNCHANGED << attestations, disputed, revoked, withdrawn, version,
                    previous_record_hash, published, verified >>

---------------------------------------------------------------------------
\* Attestation actions.
---------------------------------------------------------------------------

\* An attestor co-attests the current record version. Requires consumed
\* consent, a live record, and that the attestor has not already attested.
Attest(a) ==
    /\ a \in (1 .. Attestors)
    /\ a \notin attestations
    /\ consentState = "consumed"
    /\ IsLive
    /\ attestations' = attestations \cup { a }
    /\ UNCHANGED << consentState, consentConsumedCount, disputed, revoked,
                    withdrawn, version, previous_record_hash, published,
                    verified >>

\* Publish the record to the status list once quorum is reached and the
\* record is not disputed, revoked, or withdrawn.
Publish ==
    /\ ~published
    /\ Cardinality(attestations) >= Quorum
    /\ ~disputed
    /\ IsLive
    /\ published' = TRUE
    /\ verified' = TRUE
    /\ UNCHANGED << consentState, consentConsumedCount, attestations,
                    disputed, revoked, withdrawn, version,
                    previous_record_hash >>

---------------------------------------------------------------------------
\* Dispute actions.
---------------------------------------------------------------------------

\* Dispute the current record version (only if live and not already disputed).
Dispute ==
    /\ IsLive
    /\ ~disputed
    /\ disputed' = TRUE
    /\ verified' = FALSE
    /\ UNCHANGED << consentState, consentConsumedCount, attestations,
                    revoked, withdrawn, version, previous_record_hash,
                    published >>

\* Clear a dispute (only if disputed and live).
ClearDispute ==
    /\ IsLive
    /\ disputed
    /\ disputed' = FALSE
    /\ UNCHANGED << consentState, consentConsumedCount, attestations,
                    revoked, withdrawn, version, previous_record_hash,
                    published, verified >>

---------------------------------------------------------------------------
\* Revocation and withdrawal.
---------------------------------------------------------------------------

\* Revoke the current record version. Revocation is terminal for the
\* current version: it clears attestations and marks the record revoked.
Revoke ==
    /\ IsLive
    /\ revoked' = TRUE
    /\ verified' = FALSE
    /\ attestations' = {}
    /\ UNCHANGED << consentState, consentConsumedCount, disputed, withdrawn,
                    version, previous_record_hash, published >>

\* Withdraw the current record version (patient-initiated). Withdrawal is
\* terminal for the current version and also clears verification.
Withdraw ==
    /\ IsLive
    /\ withdrawn' = TRUE
    /\ verified' = FALSE
    /\ UNCHANGED << consentState, consentConsumedCount, attestations,
                    disputed, revoked, version, previous_record_hash,
                    published >>

---------------------------------------------------------------------------
\* Record versioning.
---------------------------------------------------------------------------

\* Create a new record version, chaining to the previous version via
\* previous_record_hash. Resets per-version state (attestations, dispute,
\* revocation, withdrawal, publication, verification) but preserves the
\* consent lifecycle.
NewVersion ==
    /\ version < MaxVersions
    /\ version' = version + 1
    /\ previous_record_hash' = version
    /\ attestations' = {}
    /\ disputed' = FALSE
    /\ revoked' = FALSE
    /\ withdrawn' = FALSE
    /\ published' = FALSE
    /\ verified' = FALSE
    /\ UNCHANGED << consentState, consentConsumedCount >>

---------------------------------------------------------------------------
\* Next-state relation.
---------------------------------------------------------------------------
Next ==
    \/ GrantConsent
    \/ ExpireConsent
    \/ ConsumeConsent
    \/ \E a \in (1 .. Attestors) : Attest(a)
    \/ Publish
    \/ Dispute
    \/ ClearDispute
    \/ Revoke
    \/ Withdraw
    \/ NewVersion

Spec == Init /\ [][Next]_vars

---------------------------------------------------------------------------
\* Safety invariants.
---------------------------------------------------------------------------

\* A revoked record is never "verified".
RevokedNeverVerified ==
    revoked => ~verified

\* Consent is consumed at most once.
ConsentConsumedAtMostOnce ==
    consentConsumedCount <= 1

\* A disputed record is never "verified" without a clear.
\* (A clear is modelled as disputed = FALSE; while disputed, verified must
\*  be FALSE.)
DisputedNeverVerifiedWithoutClear ==
    disputed => ~verified

\* Quorum monotonicity under revocation: once a record has been revoked,
\* the attestation set stays empty (revocation clears attestations and no
\* further attestations are accepted on a revoked record).
QuorumMonotonicUnderRevocation ==
    revoked => attestations = {}

\* The "verified" flag agrees with the structural definition of verified.
VerifiedAgreesWithStructure ==
    verified = IsVerified

---------------------------------------------------------------------------
\* Liveness properties (under fairness).
---------------------------------------------------------------------------

\* Under weak fairness on Publish, a record that has reached quorum, is
\* live, and is not disputed will eventually be published and verified.
LivenessAttestEventuallyResolves ==
    /\ WF_vars(Publish)
    /\ WF_vars(Dispute)
    => ( (Cardinality(attestations) >= Quorum /\ IsLive /\ ~disputed)
         ~> verified )

\* Under weak fairness on ClearDispute, a disputed live record will
\* eventually have its dispute cleared.
LivenessDisputeEventuallyClears ==
    WF_vars(ClearDispute) =>
        ( (disputed /\ IsLive) ~> ~disputed )

=============================================================================