--------------------------- MODULE MultisigRecovery ---------------------------
(***************************************************************************)
(* Design check for multisig signer rotation racing a guardian recovery.   *)
(* Neither timelocked rotation nor guardian recovery is implemented in     *)
(* contracts/multisig-account yet; this spec fixes the design requirement  *)
(* before implementation.                                                  *)
(*                                                                         *)
(* Both changes are proposed (and validated) against the configuration at  *)
(* proposal time and applied later, after a delay. Without a config nonce, *)
(* a stale threshold rotation can land after a recovery replaced the       *)
(* signer set and lock everyone out. With UseNonce = TRUE, any change that *)
(* was proposed against an older configuration is rejected.                *)
(***************************************************************************)
EXTENDS Naturals, FiniteSets

CONSTANTS
    Keys,           \* every ed25519 key that could be a signer
    Available,      \* keys whose holders can still sign (others are lost)
    InitSigners,    \* signers passed to __constructor
    InitThreshold,  \* threshold passed to __constructor
    RecoverySets,   \* signer sets a guardian may install
    UseNonce,       \* reject changes proposed against an older config
    MaxNonce,       \* bound on applied configuration changes
    None            \* model value: "no pending change"

ASSUME
    /\ Available \subseteq Keys
    /\ InitSigners \subseteq Keys
    /\ RecoverySets \subseteq SUBSET Keys
    /\ UseNonce \in BOOLEAN
    /\ MaxNonce \in Nat

VARIABLES signers, threshold, nonce, pendingRotation, pendingRecovery

vars == <<signers, threshold, nonce, pendingRotation, pendingRecovery>>

Thresholds == 1..Cardinality(Keys)

\* A configuration its available holders can still operate.
Operable(S, t) == t >= 1 /\ t <= Cardinality(S \cap Available)

LockedOut == ~Operable(signers, threshold)

Init ==
    /\ signers = InitSigners
    /\ threshold = InitThreshold
    /\ nonce = 0
    /\ pendingRotation = None
    /\ pendingRecovery = None

\* A signer quorum proposes a new threshold, valid for the current signers.
ProposeRotation(t) ==
    /\ ~LockedOut
    /\ pendingRotation = None
    /\ t # threshold
    /\ Operable(signers, t)
    /\ pendingRotation' = [t |-> t, n |-> nonce]
    /\ UNCHANGED <<signers, threshold, nonce, pendingRecovery>>

\* The guardian proposes a full replacement configuration.
ProposeRecovery(S, t) ==
    /\ pendingRecovery = None
    /\ Operable(S, t)
    /\ pendingRecovery' = [s |-> S, t |-> t, n |-> nonce]
    /\ UNCHANGED <<signers, threshold, nonce, pendingRotation>>

Fresh(p) == ~UseNonce \/ p.n = nonce

ApplyRotation ==
    /\ pendingRotation # None
    /\ Fresh(pendingRotation)
    /\ nonce < MaxNonce
    /\ threshold' = pendingRotation.t
    /\ nonce' = nonce + 1
    /\ pendingRotation' = None
    /\ UNCHANGED <<signers, pendingRecovery>>

ApplyRecovery ==
    /\ pendingRecovery # None
    /\ Fresh(pendingRecovery)
    /\ nonce < MaxNonce
    /\ signers' = pendingRecovery.s
    /\ threshold' = pendingRecovery.t
    /\ nonce' = nonce + 1
    /\ pendingRecovery' = None
    /\ UNCHANGED pendingRotation

\* A stale proposal can only be discarded.
DropStale ==
    /\ UseNonce
    /\ \/ /\ pendingRotation # None /\ ~Fresh(pendingRotation)
          /\ pendingRotation' = None
          /\ UNCHANGED pendingRecovery
       \/ /\ pendingRecovery # None /\ ~Fresh(pendingRecovery)
          /\ pendingRecovery' = None
          /\ UNCHANGED pendingRotation
    /\ UNCHANGED <<signers, threshold, nonce>>

Next ==
    \/ \E t \in Thresholds : ProposeRotation(t)
    \/ \E S \in RecoverySets, t \in Thresholds : ProposeRecovery(S, t)
    \/ ApplyRotation
    \/ ApplyRecovery
    \/ DropStale

Spec == Init /\ [][Next]_vars

\* Safety: no sequence of rotations and recoveries locks everyone out.
NeverLockedOut == ~LockedOut

=============================================================================
