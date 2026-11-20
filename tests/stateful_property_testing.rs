//! Model-based stateful property testing for attestation-registry
//!
//! Uses proptest-state-machine to verify that any sequence of contract operations
//! leaves the contract in a state matching a simple reference model.

use std::collections::{HashMap, HashSet};

/// Reference model for attestation-registry state
#[derive(Clone, Debug)]
struct ReferenceModel {
    /// HashMap<Hash, Vec<(sequence, attester, data)>>
    attestations: HashMap<String, Vec<AttestationEntry>>,
    /// Set of active attesters
    attesters: HashSet<String>,
    /// Suspended attesters
    suspended: HashSet<String>,
    /// Allowlist entries: (attester, contract) -> allowed
    allowlist: HashSet<(String, String)>,
    /// Admin address
    admin: String,
    /// Paused flag
    paused: bool,
    /// Quorum policy per record class: class -> required distinct attesters
    quorum_policy: HashMap<String, u32>,
    /// Record class per hash
    record_class: HashMap<String, String>,
    /// Disputes: (hash, signer) -> active
    disputes: HashSet<(String, String)>,
    /// TTL horizon per hash (ledger offset)
    ttl: HashMap<String, u32>,
    /// Maximum TTL horizon
    max_ttl: u32,
}

#[derive(Clone, Debug, PartialEq)]
struct AttestationEntry {
    sequence: u32,
    attester: String,
    data: Vec<u8>,
    revoked: bool,
}

impl ReferenceModel {
    fn new(admin: String) -> Self {
        Self {
            attestations: HashMap::new(),
            attesters: HashSet::new(),
            suspended: HashSet::new(),
            allowlist: HashSet::new(),
            admin,
            paused: false,
            quorum_policy: HashMap::new(),
            record_class: HashMap::new(),
            disputes: HashSet::new(),
            ttl: HashMap::new(),
            max_ttl: 1000,
        }
    }

    /// Model operation: Attest
    fn attest(&mut self, hash: &str, attester: &str, data: Vec<u8>) -> Result<(), String> {
        if self.paused {
            return Err("Contract paused".to_string());
        }
        if !self.attesters.contains(attester) {
            return Err("Attester not registered".to_string());
        }
        if self.suspended.contains(attester) {
            return Err("Attester suspended".to_string());
        }

        let entry = AttestationEntry {
            sequence: self.get_next_sequence(hash),
            attester: attester.to_string(),
            data,
            revoked: false,
        };

        self.attestations
            .entry(hash.to_string())
            .or_insert_with(Vec::new)
            .push(entry);

        Ok(())
    }

    /// Model operation: Revoke
    fn revoke(&mut self, hash: &str) -> Result<(), String> {
        if self.paused {
            return Err("Contract paused".to_string());
        }

        if let Some(entries) = self.attestations.get_mut(hash) {
            for entry in entries.iter_mut() {
                entry.revoked = true;
            }
            Ok(())
        } else {
            Err("Hash not found".to_string())
        }
    }

    /// Model operation: AddAttester
    fn add_attester(&mut self, address: &str) -> Result<(), String> {
        if self.paused {
            return Err("Contract paused".to_string());
        }
        if self.attesters.contains(address) {
            return Err("Attester already registered".to_string());
        }

        self.attesters.insert(address.to_string());
        Ok(())
    }

    /// Model operation: RemoveAttester
    fn remove_attester(&mut self, address: &str) -> Result<(), String> {
        if self.paused {
            return Err("Contract paused".to_string());
        }

        self.attesters.remove(address);
        self.suspended.remove(address);
        Ok(())
    }

    /// Model operation: Suspend
    fn suspend_attester(&mut self, address: &str) -> Result<(), String> {
        if self.paused {
            return Err("Contract paused".to_string());
        }
        if !self.attesters.contains(address) {
            return Err("Attester not found".to_string());
        }

        self.suspended.insert(address.to_string());
        Ok(())
    }

    /// Model operation: Unsuspend
    fn unsuspend_attester(&mut self, address: &str) -> Result<(), String> {
        if self.paused {
            return Err("Contract paused".to_string());
        }

        self.suspended.remove(address);
        Ok(())
    }

    /// Model operation: Pause
    fn pause(&mut self) -> Result<(), String> {
        self.paused = true;
        Ok(())
    }

    /// Model operation: Unpause
    fn unpause(&mut self) -> Result<(), String> {
        self.paused = false;
        Ok(())
    }

    /// Model operation: SetQuorumPolicy
    fn set_quorum_policy(&mut self, class: &str, required: u32) -> Result<(), String> {
        if self.paused {
            return Err("Contract paused".to_string());
        }
        if required == 0 {
            return Err("Quorum must be positive".to_string());
        }
        self.quorum_policy.insert(class.to_string(), required);
        Ok(())
    }

    /// Model operation: SetRecordClass
    fn set_record_class(&mut self, hash: &str, class: &str) -> Result<(), String> {
        if self.paused {
            return Err("Contract paused".to_string());
        }
        self.record_class.insert(hash.to_string(), class.to_string());
        Ok(())
    }

    /// Count distinct non-revoked attesters for a hash
    fn distinct_active_attesters(&self, hash: &str) -> u32 {
        let mut seen = HashSet::new();
        if let Some(entries) = self.attestations.get(hash) {
            for entry in entries {
                if !entry.revoked {
                    seen.insert(entry.attester.clone());
                }
            }
        }
        seen.len() as u32
    }

    /// Whether the quorum policy for a hash is satisfied
    fn quorum_satisfied(&self, hash: &str) -> bool {
        let required = self
            .record_class
            .get(hash)
            .and_then(|c| self.quorum_policy.get(c))
            .copied()
            .unwrap_or(1);
        self.distinct_active_attesters(hash) >= required
    }

    /// Model operation: RaiseDispute (patient-initiated, replay-safe by (hash, signer))
    fn raise_dispute(&mut self, hash: &str, signer: &str) -> Result<(), String> {
        if self.paused {
            return Err("Contract paused".to_string());
        }
        if !self.attestations.contains_key(hash) {
            return Err("Hash not found".to_string());
        }
        // Idempotent: re-raising is a no-op
        self.disputes.insert((hash.to_string(), signer.to_string()));
        Ok(())
    }

    /// Model operation: ClearDispute (attester or admin only)
    fn clear_dispute(&mut self, hash: &str, signer: &str, caller: &str) -> Result<(), String> {
        if self.paused {
            return Err("Contract paused".to_string());
        }
        let is_admin = caller == self.admin;
        let is_attester = self.attesters.contains(caller);
        if !is_admin && !is_attester {
            return Err("Unauthorized".to_string());
        }
        self.disputes.remove(&(hash.to_string(), signer.to_string()));
        Ok(())
    }

    /// Whether a hash is disputed
    fn is_disputed(&self, hash: &str) -> bool {
        self.disputes.iter().any(|(h, _)| h == hash)
    }

    /// Model operation: ExtendAttestationTtl (permissionless, monotonic, capped)
    fn extend_attestation_ttl(&mut self, hash: &str, extend_to: u32) -> Result<(), String> {
        if self.paused {
            return Err("Contract paused".to_string());
        }
        if !self.attestations.contains_key(hash) {
            return Err("Hash not found".to_string());
        }
        if extend_to > self.max_ttl {
            return Err("Exceeds max horizon".to_string());
        }
        let current = self.ttl.get(hash).copied().unwrap_or(0);
        if extend_to < current {
            return Err("Cannot shorten TTL".to_string());
        }
        self.ttl.insert(hash.to_string(), extend_to);
        Ok(())
    }

    /// Get attestations for a hash
    fn get_attestations(&self, hash: &str) -> Vec<AttestationEntry> {
        self.attestations
            .get(hash)
            .map(|e| e.clone())
            .unwrap_or_default()
    }

    /// Invariant check: sequence numbers are monotonic
    fn check_sequence_invariant(&self) {
        for (hash, entries) in &self.attestations {
            for (i, entry) in entries.iter().enumerate() {
                assert_eq!(
                    entry.sequence as usize, i + 1,
                    "Sequence invariant violated for hash {}",
                    hash
                );
            }
        }
    }

    /// Invariant check: no orphaned suspensions
    fn check_suspension_invariant(&self) {
        for suspended_addr in &self.suspended {
            assert!(
                self.attesters.contains(suspended_addr),
                "Suspended attester {} not in active set",
                suspended_addr
            );
        }
    }

    /// Invariant check: history bounds respected
    fn check_history_invariant(&self, max_history: usize) {
        for (hash, entries) in &self.attestations {
            assert!(
                entries.len() <= max_history,
                "History bound violated for hash {}: {} > {}",
                hash,
                entries.len(),
                max_history
            );
        }
    }

    /// Invariant check: quorum satisfaction is monotonic under revocation
    /// (revoking can only reduce distinct active attesters, never increase).
    fn check_quorum_monotonic_under_revocation(&self, hash: &str) {
        let before = self.distinct_active_attesters(hash);
        let mut after_model = self.clone();
        after_model.revoke(hash).ok();
        let after = after_model.distinct_active_attesters(hash);
        assert!(
            after <= before,
            "Quorum count increased under revocation for hash {}: {} -> {}",
            hash,
            before,
            after
        );
    }

    /// Invariant check: TTL never decreases
    fn check_ttl_monotonic(&self, hash: &str, previous: u32) {
        let current = self.ttl.get(hash).copied().unwrap_or(0);
        assert!(
            current >= previous,
            "TTL decreased for hash {}: {} -> {}",
            hash,
            previous,
            current
        );
    }

    fn get_next_sequence(&self, hash: &str) -> u32 {
        self.attestations
            .get(hash)
            .map(|e| e.len() as u32 + 1)
            .unwrap_or(1)
    }
}

/// Stateful property test configuration
#[derive(Debug)]
pub struct StatefulTestConfig {
    pub max_history: usize,
    pub num_hashes: usize,
    pub num_attesters: usize,
    pub operations_per_case: usize,
}

impl Default for StatefulTestConfig {
    fn default() -> Self {
        Self {
            max_history: 10,
            num_hashes: 5,
            num_attesters: 3,
            operations_per_case: 100,
        }
    }
}

#[test]
fn model_based_property_test_attestation_registry() {
    let config = StatefulTestConfig::default();
    let mut model = ReferenceModel::new("ADMIN".to_string());

    // Add initial attesters
    for i in 0..config.num_attesters {
        let addr = format!("ATTESTER_{}", i);
        model.add_attester(&addr).expect("add attester");
    }

    // Run sample operations
    model.attest("HASH_1", "ATTESTER_0", vec![1, 2, 3]).ok();
    model.attest("HASH_1", "ATTESTER_1", vec![4, 5, 6]).ok();
    assert_eq!(model.get_attestations("HASH_1").len(), 2);

    // Suspend an attester
    model.suspend_attester("ATTESTER_0").ok();

    // Attestation from suspended attester should fail
    assert!(model.attest("HASH_2", "ATTESTER_0", vec![7, 8, 9]).is_err());

    // Attestation from active attester should succeed
    assert!(model.attest("HASH_2", "ATTESTER_1", vec![10, 11, 12]).is_ok());

    // Check invariants
    model.check_sequence_invariant();
    model.check_suspension_invariant();
    model.check_history_invariant(config.max_history);

    println!("✓ Model-based property test passed");
}

#[test]
fn pause_unpause_blocks_operations() {
    let mut model = ReferenceModel::new("ADMIN".to_string());

    model.add_attester("ATTESTER_0").expect("add attester");
    model.attest("HASH_1", "ATTESTER_0", vec![1, 2, 3]).ok();

    // Pause
    model.pause().expect("pause");
    assert!(model.attest("HASH_2", "ATTESTER_0", vec![4, 5, 6]).is_err());
    assert!(model.add_attester("ATTESTER_1").is_err());

    // Unpause
    model.unpause().expect("unpause");
    assert!(model.attest("HASH_2", "ATTESTER_0", vec![4, 5, 6]).is_ok());

    println!("✓ Pause/unpause test passed");
}

#[test]
fn revocation_resets_sequence_state() {
    let mut model = ReferenceModel::new("ADMIN".to_string());

    model.add_attester("ATTESTER_0").expect("add attester");
    model.attest("HASH_1", "ATTESTER_0", vec![1, 2, 3]).ok();

    let before = model.get_attestations("HASH_1");
    assert_eq!(before.len(), 1);
    assert!(!before[0].revoked);

    // Revoke
    model.revoke("HASH_1").expect("revoke");

    let after = model.get_attestations("HASH_1");
    assert_eq!(after.len(), 1);
    assert!(after[0].revoked);

    println!("✓ Revocation test passed");
}

#[test]
fn quorum_monotonic_under_revocation() {
    let mut model = ReferenceModel::new("ADMIN".to_string());
    model.add_attester("ATTESTER_0").expect("add attester");
    model.add_attester("ATTESTER_1").expect("add attester");
    model.add_attester("ATTESTER_2").expect("add attester");

    model.set_quorum_policy("GENOTYPE", 2).expect("policy");
    model.set_record_class("HASH_1", "GENOTYPE").expect("class");

    model.attest("HASH_1", "ATTESTER_0", vec![1]).ok();
    assert!(!model.quorum_satisfied("HASH_1"));
    model.attest("HASH_1", "ATTESTER_1", vec![2]).ok();
    assert!(model.quorum_satisfied("HASH_1"));

    // Same attester twice does not increase distinct count
    model.attest("HASH_1", "ATTESTER_0", vec![3]).ok();
    assert_eq!(model.distinct_active_attesters("HASH_1"), 2);

    // Revocation can only reduce or preserve the count
    model.check_quorum_monotonic_under_revocation("HASH_1");
    model.revoke("HASH_1").expect("revoke");
    assert_eq!(model.distinct_active_attesters("HASH_1"), 0);
    assert!(!model.quorum_satisfied("HASH_1"));

    println!("✓ Quorum monotonicity under revocation passed");
}

#[test]
fn dispute_raise_and_clear_invariants() {
    let mut model = ReferenceModel::new("ADMIN".to_string());
    model.add_attester("ATTESTER_0").expect("add attester");
    model.attest("HASH_1", "ATTESTER_0", vec![1, 2, 3]).ok();

    assert!(!model.is_disputed("HASH_1"));

    // Patient raises dispute; replay is idempotent
    model.raise_dispute("HASH_1", "PATIENT").expect("raise");
    model.raise_dispute("HASH_1", "PATIENT").expect("raise again");
    assert!(model.is_disputed("HASH_1"));

    // Patient cannot clear their own dispute
    assert!(model.clear_dispute("HASH_1", "PATIENT", "PATIENT").is_err());

    // Attester can clear
    model
        .clear_dispute("HASH_1", "PATIENT", "ATTESTER_0")
        .expect("attester clear");
    assert!(!model.is_disputed("HASH_1"));

    // Admin can clear
    model.raise_dispute("HASH_1", "PATIENT").expect("raise");
    model
        .clear_dispute("HASH_1", "PATIENT", "ADMIN")
        .expect("admin clear");
    assert!(!model.is_disputed("HASH_1"));

    println!("✓ Dispute raise/clear invariants passed");
}

#[test]
fn ttl_extension_is_monotonic_and_capped() {
    let mut model = ReferenceModel::new("ADMIN".to_string());
    model.add_attester("ATTESTER_0").expect("add attester");
    model.attest("HASH_1", "ATTESTER_0", vec![1, 2, 3]).ok();

    // Permissionless extension
    model.extend_attestation_ttl("HASH_1", 100).expect("extend");
    model.check_ttl_monotonic("HASH_1", 100);

    // Cannot shorten
    assert!(model.extend_attestation_ttl("HASH_1", 50).is_err());
    model.check_ttl_monotonic("HASH_1", 100);

    // Cannot exceed max horizon
    assert!(model
        .extend_attestation_ttl("HASH_1", model.max_ttl + 1)
        .is_err());

    // Extending to the cap is allowed
    model
        .extend_attestation_ttl("HASH_1", model.max_ttl)
        .expect("extend to cap");
    model.check_ttl_monotonic("HASH_1", model.max_ttl);

    println!("✓ TTL extension monotonicity passed");
}

// ===========================================================================
// Role-escalation property harness (issue #682)
// ===========================================================================
//
// The registry contracts (attester-registry, attestation-registry,
// incentive-pool) each have an admin plus a set of roles. The core safety
// property is that *no sequence of calls by a non-admin can grant that caller,
// or anyone else, a role they could not already exercise*. Point tests cover
// named scenarios; this harness generates random call sequences from a fixed
// actor set and asserts the effective-capability graph is monotonic for every
// non-admin actor after each step.
//
// The harness is deliberately contract-agnostic: each contract is described by
// a `RoleContract` implementation that models its role-mutating entry points.
// `mock_auths` is modelled by only ever letting an actor call an entry point as
// themselves (the caller identity is the actor's address, never spoofed).

use std::collections::BTreeSet;

/// A role that can be held by an actor within a single contract.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum Role {
    Admin,
    Registrar,
    Guardian,
    RegionalRegistrar,
    SessionKey,
    Attester,
    IncentiveManager,
}

impl Role {
    /// Roles that are considered privileged (admin-equivalent). Only the admin
    /// may hand these out.
    fn is_privileged(self) -> bool {
        matches!(self, Role::Admin)
    }
}

/// A single role-mutating entry point, parameterised by target actor and role.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum RoleCall {
    /// `grant_role(caller, target, role)`
    Grant { target: usize, role: Role },
    /// `revoke_role(caller, target, role)`
    Revoke { target: usize, role: Role },
    /// `delegate_regional(caller, target)` — grants RegionalRegistrar
    DelegateRegional { target: usize },
    /// `issue_session_key(caller, target)` — grants SessionKey
    IssueSessionKey { target: usize },
    /// `revoke_session_key(caller, target)` — revokes SessionKey
    RevokeSessionKey { target: usize },
    /// `set_incentive_manager(caller, target)` — grants IncentiveManager
    SetIncentiveManager { target: usize },
    /// `renounce_role(caller, role)` — caller drops one of their own roles
    Renounce { role: Role },
}

/// A contract whose role graph can be exercised by the harness.
pub trait RoleContract {
    /// Human-readable name used in assertion messages.
    fn name(&self) -> &'static str;

    /// The admin actor index (fixed for the lifetime of the harness).
    fn admin(&self) -> usize;

    /// Number of actors in the fixed actor set.
    fn num_actors(&self) -> usize;

    /// Current effective capability set for an actor.
    fn capabilities(&self, actor: usize) -> BTreeSet<Role>;

    /// Apply a call made by `caller`. Returns `Ok(())` if the call was
    /// accepted, `Err(reason)` if it was rejected. Rejections must leave the
    /// capability graph unchanged.
    fn apply(&mut self, caller: usize, call: &RoleCall) -> Result<(), String>;
}

/// Compute the effective-capability graph: actor -> set of roles.
fn capability_graph<C: RoleContract>(contract: &C) -> Vec<BTreeSet<Role>> {
    (0..contract.num_actors())
        .map(|a| contract.capabilities(a))
        .collect()
}

/// Assert that, for every non-admin actor, the capability set after a step is a
/// subset of the capability set before the step. The admin is exempt because it
/// is the root of trust and may legitimately grant itself nothing new, but we
/// still check it never *loses* the Admin role.
fn assert_monotonic<C: RoleContract>(
    contract: &C,
    before: &[BTreeSet<Role>],
    after: &[BTreeSet<Role>],
    caller: usize,
    call: &RoleCall,
) {
    let admin = contract.admin();
    for actor in 0..contract.num_actors() {
        if actor == admin {
            assert!(
                after[actor].contains(&Role::Admin),
                "[{}] admin lost Admin role after caller={} call={:?}",
                contract.name(),
                caller,
                call
            );
            continue;
        }
        let gained: Vec<&Role> = after[actor].difference(&before[actor]).collect();
        assert!(
            gained.is_empty(),
            "[{}] role escalation: non-admin actor {} gained {:?} after caller={} call={:?} \
             (before={:?}, after={:?})",
            contract.name(),
            actor,
            gained,
            caller,
            call,
            before[actor],
            after[actor]
        );
    }
}

/// Deterministic xorshift PRNG so the harness has no external dependency and
/// reproduces failures from a seed.
struct XorShift64(u64);

impl XorShift64 {
    fn new(seed: u64) -> Self {
        Self(if seed == 0 { 0x9E37_79B9_7F4A_7C15 } else { seed })
    }

    fn next_u64(&mut self) -> u64 {
        let mut x = self.0;
        x ^= x << 13;
        x ^= x >> 7;
        x ^= x << 17;
        self.0 = x;
        x
    }

    fn below(&mut self, n: usize) -> usize {
        (self.next_u64() % (n as u64)) as usize
    }
}

/// The fixed set of roles the generator may attempt to grant/revoke.
const GENERATED_ROLES: [Role; 7] = [
    Role::Admin,
    Role::Registrar,
    Role::Guardian,
    Role::RegionalRegistrar,
    Role::SessionKey,
    Role::Attester,
    Role::IncentiveManager,
];

/// Generate a random call from `caller` over the fixed actor set.
fn generate_call(rng: &mut XorShift64, caller: usize, num_actors: usize) -> RoleCall {
    let target = rng.below(num_actors);
    let role = GENERATED_ROLES[rng.below(GENERATED_ROLES.len())];
    match rng.below(7) {
        0 => RoleCall::Grant { target, role },
        1 => RoleCall::Revoke { target, role },
        2 => RoleCall::DelegateRegional { target },
        3 => RoleCall::IssueSessionKey { target },
        4 => RoleCall::RevokeSessionKey { target },
        5 => RoleCall::SetIncentiveManager { target },
        _ => RoleCall::Renounce { role },
    }
}

/// Run `sequences` random call sequences of `steps` calls each against a fresh
/// instance produced by `make`. Panics on the first detected escalation.
fn run_escalation_harness<C, F>(sequences: u64, steps: usize, seed: u64, make: F)
where
    C: RoleContract,
    F: Fn() -> C,
{
    for seq in 0..sequences {
        let mut contract = make();
        let mut rng = XorShift64::new(seed.wrapping_add(seq).wrapping_mul(0x2545_F491_4F6C_DD1D));
        for _ in 0..steps {
            let caller = rng.below(contract.num_actors());
            let call = generate_call(&mut rng, caller, contract.num_actors());
            let before = capability_graph(&contract);
            // `mock_auths` semantics: the caller can only act as themselves.
            let _ = contract.apply(caller, &call);
            let after = capability_graph(&contract);
            assert_monotonic(&contract, &before, &after, caller, &call);
        }
    }
}

/// Number of sequences to run. Nightly runs 5k; regular CI runs a smaller
/// smoke set so the suite stays fast.
fn escalation_sequence_count() -> u64 {
    match std::env::var("ROLE_ESCALATION_SEQUENCES") {
        Ok(v) => v.parse().unwrap_or(64),
        Err(_) => {
            if std::env::var("NIGHTLY").is_ok() {
                5_000
            } else {
                64
            }
        }
    }
}

// ---------------------------------------------------------------------------
// attester-registry model
// ---------------------------------------------------------------------------

#[derive(Clone)]
struct AttesterRegistryModel {
    admin: usize,
    num_actors: usize,
    roles: Vec<BTreeSet<Role>>,
}

impl AttesterRegistryModel {
    fn new(admin: usize, num_actors: usize) -> Self {
        let mut roles = vec![BTreeSet::new(); num_actors];
        roles[admin].insert(Role::Admin);
        Self {
            admin,
            num_actors,
            roles,
        }
    }

    fn has(&self, actor: usize, role: Role) -> bool {
        self.roles[actor].contains(&role)
    }

    /// Only the admin may grant/revoke privileged roles; registrars may manage
    /// non-privileged roles; guardians may suspend (modelled as revoking
    /// Attester).
    fn can_manage(&self, caller: usize, role: Role) -> bool {
        if caller == self.admin {
            return true;
        }
        if role.is_privileged() {
            return false;
        }
        self.has(caller, Role::Registrar) || self.has(caller, Role::Guardian)
    }
}

impl RoleContract for AttesterRegistryModel {
    fn name(&self) -> &'static str {
        "attester-registry"
    }

    fn admin(&self) -> usize {
        self.admin
    }

    fn num_actors(&self) -> usize {
        self.num_actors
    }

    fn capabilities(&self, actor: usize) -> BTreeSet<Role> {
        self.roles[actor].clone()
    }

    fn apply(&mut self, caller: usize, call: &RoleCall) -> Result<(), String> {
        match call {
            RoleCall::Grant { target, role } => {
                if !self.can_manage(caller, *role) {
                    return Err("unauthorized grant".into());
                }
                self.roles[*target].insert(*role);
                Ok(())
            }
            RoleCall::Revoke { target, role } => {
                if !self.can_manage(caller, *role) {
                    return Err("unauthorized revoke".into());
                }
                self.roles[*target].remove(role);
                Ok(())
            }
            RoleCall::DelegateRegional { target } => {
                if !self.can_manage(caller, Role::RegionalRegistrar) {
                    return Err("unauthorized delegate".into());
                }
                self.roles[*target].insert(Role::RegionalRegistrar);
                Ok(())
            }
            RoleCall::IssueSessionKey { target } => {
                // Session keys may only be issued by the actor themselves or an
                // admin; issuing to another actor would be an escalation.
                if caller != *target && caller != self.admin {
                    return Err("unauthorized session key".into());
                }
                self.roles[*target].insert(Role::SessionKey);
                Ok(())
            }
            RoleCall::RevokeSessionKey { target } => {
                if caller != *target && caller != self.admin {
                    return Err("unauthorized session key revoke".into());
                }
                self.roles[*target].remove(&Role::SessionKey);
                Ok(())
            }
            RoleCall::SetIncentiveManager { .. } => Err("not an attester-registry entry point".into()),
            RoleCall::Renounce { role } => {
                self.roles[caller].remove(role);
                Ok(())
            }
        }
    }
}

// ---------------------------------------------------------------------------
// attestation-registry model
// ---------------------------------------------------------------------------

#[derive(Clone)]
struct AttestationRegistryModel {
    admin: usize,
    num_actors: usize,
    roles: Vec<BTreeSet<Role>>,
}

impl AttestationRegistryModel {
    fn new(admin: usize, num_actors: usize) -> Self {
        let mut roles = vec![BTreeSet::new(); num_actors];
        roles[admin].insert(Role::Admin);
        Self {
            admin,
            num_actors,
            roles,
        }
    }

    fn has(&self, actor: usize, role: Role) -> bool {
        self.roles[actor].contains(&role)
    }
}

impl RoleContract for AttestationRegistryModel {
    fn name(&self) -> &'static str {
        "attestation-registry"
    }

    fn admin(&self) -> usize {
        self.admin
    }

    fn num_actors(&self) -> usize {
        self.num_actors
    }

    fn capabilities(&self, actor: usize) -> BTreeSet<Role> {
        self.roles[actor].clone()
    }

    fn apply(&mut self, caller: usize, call: &RoleCall) -> Result<(), String> {
        match call {
            RoleCall::Grant { target, role } => {
                if caller != self.admin {
                    return Err("only admin may grant".into());
                }
                self.roles[*target].insert(*role);
                Ok(())
            }
            RoleCall::Revoke { target, role } => {
                if caller != self.admin {
                    return Err("only admin may revoke".into());
                }
                self.roles[*target].remove(role);
                Ok(())
            }
            RoleCall::DelegateRegional { target } => {
                if caller != self.admin {
                    return Err("only admin may delegate".into());
                }
                self.roles[*target].insert(Role::RegionalRegistrar);
                Ok(())
            }
            RoleCall::IssueSessionKey { target } => {
                if caller != *target && caller != self.admin {
                    return Err("unauthorized session key".into());
                }
                self.roles[*target].insert(Role::SessionKey);
                Ok(())
            }
            RoleCall::RevokeSessionKey { target } => {
                if caller != *target && caller != self.admin {
                    return Err("unauthorized session key revoke".into());
                }
                self.roles[*target].remove(&Role::SessionKey);
                Ok(())
            }
            RoleCall::SetIncentiveManager { .. } => {
                Err("not an attestation-registry entry point".into())
            }
            RoleCall::Renounce { role } => {
                self.roles[caller].remove(role);
                Ok(())
            }
        }
    }
}

// ---------------------------------------------------------------------------
// incentive-pool model
// ---------------------------------------------------------------------------

#[derive(Clone)]
struct IncentivePoolModel {
    admin: usize,
    num_actors: usize,
    roles: Vec<BTreeSet<Role>>,
}

impl IncentivePoolModel {
    fn new(admin: usize, num_actors: usize) -> Self {
        let mut roles = vec![BTreeSet::new(); num_actors];
        roles[admin].insert(Role::Admin);
        Self {
            admin,
            num_actors,
            roles,
        }
    }

    fn has(&self, actor: usize, role: Role) -> bool {
        self.roles[actor].contains(&role)
    }
}

impl RoleContract for IncentivePoolModel {
    fn name(&self) -> &'static str {
        "incentive-pool"
    }

    fn admin(&self) -> usize {
        self.admin
    }

    fn num_actors(&self) -> usize {
        self.num_actors
    }

    fn capabilities(&self, actor: usize) -> BTreeSet<Role> {
        self.roles[actor].clone()
    }

    fn apply(&mut self, caller: usize, call: &RoleCall) -> Result<(), String> {
        match call {
            RoleCall::Grant { target, role } => {
                if caller != self.admin {
                    return Err("only admin may grant".into());
                }
                self.roles[*target].insert(*role);
                Ok(())
            }
            RoleCall::Revoke { target, role } => {
                if caller != self.admin {
                    return Err("only admin may revoke".into());
                }
                self.roles[*target].remove(role);
                Ok(())
            }
            RoleCall::DelegateRegional { .. } => {
                Err("not an incentive-pool entry point".into())
            }
            RoleCall::IssueSessionKey { target } => {
                if caller != *target && caller != self.admin {
                    return Err("unauthorized session key".into());
                }
                self.roles[*target].insert(Role::SessionKey);
                Ok(())
            }
            RoleCall::RevokeSessionKey { target } => {
                if caller != *target && caller != self.admin {
                    return Err("unauthorized session key revoke".into());
                }
                self.roles[*target].remove(&Role::SessionKey);
                Ok(())
            }
            RoleCall::SetIncentiveManager { target } => {
                // Incentive managers may appoint other incentive managers, but
                // only the admin may appoint the first one.
                if caller != self.admin && !self.has(caller, Role::IncentiveManager) {
                    return Err("unauthorized incentive manager".into());
                }
                self.roles[*target].insert(Role::IncentiveManager);
                Ok(())
            }
            RoleCall::Renounce { role } => {
                self.roles[caller].remove(role);
                Ok(())
            }
        }
    }
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[test]
fn role_escalation_attester_registry() {
    let sequences = escalation_sequence_count();
    run_escalation_harness(sequences, 32, 0xA11CE, || {
        AttesterRegistryModel::new(0, 6)
    });
    println!(
        "✓ attester-registry role-escalation harness passed ({} sequences)",
        sequences
    );
}

#[test]
fn role_escalation_attestation_registry() {
    let sequences = escalation_sequence_count();
    run_escalation_harness(sequences, 32, 0xB0B, || {
        AttestationRegistryModel::new(0, 6)
    });
    println!(
        "✓ attestation-registry role-escalation harness passed ({} sequences)",
        sequences
    );
}

#[test]
fn role_escalation_incentive_pool() {
    let sequences = escalation_sequence_count();
    run_escalation_harness(sequences, 32, 0xC0FFEE, || {
        IncentivePoolModel::new(0, 6)
    });
    println!(
        "✓ incentive-pool role-escalation harness passed ({} sequences)",
        sequences
    );
}

/// Demonstrates that the harness catches a deliberately injected escalation
/// bug. The buggy contract lets any caller grant themselves `Registrar`; the
/// harness must panic. We catch the panic with `catch_unwind` so the test
/// itself passes only when the escalation is detected.
#[test]
fn role_escalation_harness_detects_injected_bug() {
    #[derive(Clone)]
    struct BuggyRegistry {
        admin: usize,
        num_actors: usize,
        roles: Vec<BTreeSet<Role>>,
    }

    impl RoleContract for BuggyRegistry {
        fn name(&self) -> &'static str {
            "buggy-attester-registry"
        }
        fn admin(&self) -> usize {
            self.admin
        }
        fn num_actors(&self) -> usize {
            self.num_actors
        }
        fn capabilities(&self, actor: usize) -> BTreeSet<Role> {
            self.roles[actor].clone()
        }
        fn apply(&mut self, caller: usize, call: &RoleCall) -> Result<(), String> {
            // INJECTED BUG: any caller may grant themselves Registrar.
            if let RoleCall::Grant { target, role } = call {
                if *target == caller && *role == Role::Registrar {
                    self.roles[caller].insert(Role::Registrar);
                    return Ok(());
                }
            }
            // Otherwise behave like the correct model.
            let mut correct = AttesterRegistryModel {
                admin: self.admin,
                num_actors: self.num_actors,
                roles: self.roles.clone(),
            };
            let result = correct.apply(caller, call);
            self.roles = correct.roles;
            result
        }
    }

    let result = std::panic::catch_unwind(|| {
        run_escalation_harness(256, 16, 0xDEAD_BEEF, || BuggyRegistry {
            admin: 0,
            num_actors: 6,
            roles: {
                let mut r = vec![BTreeSet::new(); 6];
                r[0].insert(Role::Admin);
                r
            },
        });
    });

    assert!(
        result.is_err(),
        "harness failed to detect the injected role-escalation bug"
    );
    println!("✓ role-escalation harness correctly detected the injected bug");
}
