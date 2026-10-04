------------------------------ MODULE Governance ------------------------------
(***************************************************************************)
(* Admin lifecycle, pause, and upgrade/migrate state machines of the       *)
(* attester-registry and attestation-registry contracts.                   *)
(*                                                                         *)
(* Each registry is modelled independently (they share no governance       *)
(* state), but both are explored together so TLC interleaves operations   *)
(* submitted in parallel by different signers.                             *)
(*                                                                         *)
(* Action -> contract function mapping is kept in README.md.               *)
(***************************************************************************)
EXTENDS Naturals

CONSTANTS
    Accounts,       \* every address an admin could propose
    Signers,        \* accounts whose holders can actually produce a signature
    InitAdmin,      \* admin passed to `initialize`
    Registries,     \* contract instances being modelled
    Upgradable,     \* registries exposing `upgrade`/`migrate`
    MaxSchema,      \* highest SCHEMA_VERSION a future build may declare
    AllowRollback,  \* may `upgrade` install a build with an older schema?
    NoAccount       \* model value: "no pending admin"

ASSUME
    /\ Signers \subseteq Accounts
    /\ InitAdmin \in Signers
    /\ Upgradable \subseteq Registries
    /\ MaxSchema \in Nat \ {0}
    /\ AllowRollback \in BOOLEAN
    /\ NoAccount \notin Accounts

VARIABLES
    admin,          \* DataKey::Admin
    pending,        \* DataKey::PendingAdmin (NoAccount when absent)
    proposer,       \* admin that wrote the current pending proposal (ghost)
    paused,         \* DataKey::Paused
    codeSchema,     \* SCHEMA_VERSION compiled into the installed Wasm
    storedSchema    \* DataKey::SchemaVersion

vars == <<admin, pending, proposer, paused, codeSchema, storedSchema>>

\* `require_auth` succeeds only for an account whose holder can sign.
CanAuth(a) == a \in Signers

TypeOK ==
    /\ admin \in [Registries -> Accounts]
    /\ pending \in [Registries -> Accounts \cup {NoAccount}]
    /\ proposer \in [Registries -> Accounts \cup {NoAccount}]
    /\ paused \in [Registries -> BOOLEAN]
    /\ codeSchema \in [Registries -> 1..MaxSchema]
    /\ storedSchema \in [Registries -> 1..MaxSchema]

Init ==
    /\ admin = [r \in Registries |-> InitAdmin]
    /\ pending = [r \in Registries |-> NoAccount]
    /\ proposer = [r \in Registries |-> NoAccount]
    /\ paused = [r \in Registries |-> FALSE]
    /\ codeSchema = [r \in Registries |-> 1]
    /\ storedSchema = [r \in Registries |-> 1]

-----------------------------------------------------------------------------
(* Admin lifecycle *)

\* propose_admin: overwrites any existing proposal (most recent call wins).
ProposeAdmin(r, a) ==
    /\ CanAuth(admin[r])
    /\ pending' = [pending EXCEPT ![r] = a]
    /\ proposer' = [proposer EXCEPT ![r] = admin[r]]
    /\ UNCHANGED <<admin, paused, codeSchema, storedSchema>>

\* accept_admin: requires the pending admin's signature, clears the proposal.
AcceptAdmin(r) ==
    /\ pending[r] # NoAccount
    /\ CanAuth(pending[r])
    /\ admin' = [admin EXCEPT ![r] = pending[r]]
    /\ pending' = [pending EXCEPT ![r] = NoAccount]
    /\ proposer' = [proposer EXCEPT ![r] = NoAccount]
    /\ UNCHANGED <<paused, codeSchema, storedSchema>>

-----------------------------------------------------------------------------
(* Pause *)

Pause(r) ==
    /\ CanAuth(admin[r])
    /\ paused' = [paused EXCEPT ![r] = TRUE]
    /\ UNCHANGED <<admin, pending, proposer, codeSchema, storedSchema>>

Unpause(r) ==
    /\ CanAuth(admin[r])
    /\ paused' = [paused EXCEPT ![r] = FALSE]
    /\ UNCHANGED <<admin, pending, proposer, codeSchema, storedSchema>>

-----------------------------------------------------------------------------
(* Upgrade and migrate (attester-registry only today) *)

\* upgrade: installs any Wasm hash; the new build declares schema `s`.
Upgrade(r, s) ==
    /\ r \in Upgradable
    /\ CanAuth(admin[r])
    /\ AllowRollback \/ s >= storedSchema[r]
    /\ codeSchema' = [codeSchema EXCEPT ![r] = s]
    /\ UNCHANGED <<admin, pending, proposer, paused, storedSchema>>

\* migrate: fails with MigrationNotRequired unless stored < SCHEMA_VERSION.
Migrate(r) ==
    /\ r \in Upgradable
    /\ CanAuth(admin[r])
    /\ storedSchema[r] < codeSchema[r]
    /\ storedSchema' = [storedSchema EXCEPT ![r] = codeSchema[r]]
    /\ UNCHANGED <<admin, pending, proposer, paused, codeSchema>>

-----------------------------------------------------------------------------

Next ==
    \E r \in Registries :
        \/ \E a \in Accounts : ProposeAdmin(r, a)
        \/ AcceptAdmin(r)
        \/ Pause(r)
        \/ Unpause(r)
        \/ \E s \in 1..MaxSchema : Upgrade(r, s)
        \/ Migrate(r)

\* Liveness assumes an honest, available admin quorum: fairness on the
\* recovery actions only.
Fairness ==
    \A r \in Registries : WF_vars(Unpause(r)) /\ WF_vars(Migrate(r))

Spec == Init /\ [][Next]_vars /\ Fairness

-----------------------------------------------------------------------------
(* Safety *)

\* There is always exactly one admin, and it can sign: a transfer to an
\* address nobody controls can never complete.
AdminAlwaysCanSign == \A r \in Registries : CanAuth(admin[r])

\* A proposal is only ever acceptable if the *current* admin made it; a
\* proposal from a previous admin never survives a transfer.
NoStaleProposal ==
    \A r \in Registries : pending[r] # NoAccount => proposer[r] = admin[r]

\* Pausing never blocks the admin from unpausing.
UnpauseAlwaysEnabled ==
    \A r \in Registries : paused[r] => ENABLED Unpause(r)

\* A pending migration can always be applied, even while paused.
MigrateAlwaysEnabled ==
    \A r \in Upgradable :
        storedSchema[r] < codeSchema[r] => ENABLED Migrate(r)

\* Storage is never newer than the code reading it. Violated when
\* AllowRollback = TRUE (see README "Findings").
StoredSchemaNotAhead ==
    \A r \in Registries : storedSchema[r] <= codeSchema[r]

(* Liveness *)

EventuallyUnpaused == \A r \in Registries : paused[r] ~> ~paused[r]

EventuallyMigrated ==
    \A r \in Upgradable :
        (storedSchema[r] < codeSchema[r]) ~> (storedSchema[r] = codeSchema[r])

=============================================================================
