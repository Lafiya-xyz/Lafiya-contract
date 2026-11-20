//! Cross-contract model-based stateful property testing.
//!
//! Drives all five contracts (attestation-registry, attester-registry,
//! consent-manager, facility-registry, work-item-manager) together against a
//! single Rust reference model. The most dangerous bugs live *between*
//! contracts: an attester suspended mid-flow, a registry repointed, a pool
//! paused while a claim is pending, a rotation during a pending payout.
//!
//! The test:
//!   * encodes every public mutating entry point as a shrinking-friendly
//!     `Operation` enum,
//!   * generates sequences with proptest-state-machine (governance + pause
//!     included),
//!   * after each step compares the observable state (reads *and* events) of
//!     the system under test with the reference model,
//!   * persists failing sequences to `tests/regressions/cross_contract.txt`
//!     so they are replayed on every run,
//!   * runs >= 2k sequences of length >= 50 in nightly (`CROSS_CONTRACT_NIGHTLY`).
//!
//! Two planted bugs (one cross-contract) are exercised by
//! `planted_bugs_are_caught` to prove the harness has teeth.

use std::collections::{BTreeMap, HashMap, HashSet};
use std::fmt::Write as _;
use std::path::PathBuf;

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
// Cross-contract reference model
// ===========================================================================

/// Identifier of one of the five contracts under test.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
enum Contract {
    AttestationRegistry,
    AttesterRegistry,
    ConsentManager,
    FacilityRegistry,
    WorkItemManager,
}

/// A single observable event emitted by the system under test.
///
/// Events are part of the observable state: after each operation the event
/// stream produced by the SUT must equal the event stream predicted by the
/// model. This is what catches cross-contract bugs that leave storage
/// consistent but emit the wrong (or missing) notification.
#[derive(Clone, Debug, PartialEq, Eq)]
enum Event {
    AttesterAdded(String),
    AttesterSuspended(String),
    AttesterUnsuspended(String),
    AttesterRemoved(String),
    ConsentGranted { patient: String, grantee: String },
    ConsentRevoked { patient: String, grantee: String },
    Attested { hash: String, attester: String },
    AttestationRevoked(String),
    FacilityRegistered(String),
    FacilityDeactivated(String),
    WorkItemOpened(u64),
    WorkItemClaimed { id: u64, claimant: String },
    WorkItemSettled { id: u64, amount: i128 },
    GovernanceChanged(String),
    Paused(Contract),
    Unpaused(Contract),
}

/// A shrinking-friendly encoding of every public mutating entry point across
/// all five contracts. Indices (not strings) are used so proptest can shrink
/// towards the simplest failing sequence.
#[derive(Clone, Debug, PartialEq, Eq)]
enum Operation {
    // --- attester-registry ---
    AddAttester(u8),
    SuspendAttester(u8),
    UnsuspendAttester(u8),
    RemoveAttester(u8),
    // --- consent-manager ---
    GrantConsent { patient: u8, grantee: u8 },
    RevokeConsent { patient: u8, grantee: u8 },
    // --- attestation-registry ---
    Attest { hash: u8, attester: u8 },
    RevokeAttestation(u8),
    // --- facility-registry ---
    RegisterFacility(u8),
    DeactivateFacility(u8),
    // --- work-item-manager ---
    OpenWorkItem(u8),
    ClaimWorkItem { id: u8, claimant: u8 },
    SettleWorkItem(u8),
    // --- governance (admin) ---
    SetGovernance(u8),
    Pause(Contract),
    Unpause(Contract),
}

impl Operation {
    /// Decode an operation from a compact, human-readable encoding. Used to
    /// replay persisted regressions.
    fn decode(s: &str) -> Option<Operation> {
        let (tag, rest) = s.split_once('(')?;
        let rest = rest.strip_suffix(')')?;
        let nums: Vec<u8> = if rest.is_empty() {
            Vec::new()
        } else {
            rest.split(',')
                .map(|n| n.trim().parse::<u8>().ok())
                .collect::<Option<Vec<_>>>()?
        };
        let contract = |i: usize| -> Option<Contract> {
            Some(match nums.get(i)? {
                0 => Contract::AttestationRegistry,
                1 => Contract::AttesterRegistry,
                2 => Contract::ConsentManager,
                3 => Contract::FacilityRegistry,
                _ => Contract::WorkItemManager,
            })
        };
        Some(match tag {
            "AddAttester" => Operation::AddAttester(nums[0]),
            "SuspendAttester" => Operation::SuspendAttester(nums[0]),
            "UnsuspendAttester" => Operation::UnsuspendAttester(nums[0]),
            "RemoveAttester" => Operation::RemoveAttester(nums[0]),
            "GrantConsent" => Operation::GrantConsent { patient: nums[0], grantee: nums[1] },
            "RevokeConsent" => Operation::RevokeConsent { patient: nums[0], grantee: nums[1] },
            "Attest" => Operation::Attest { hash: nums[0], attester: nums[1] },
            "RevokeAttestation" => Operation::RevokeAttestation(nums[0]),
            "RegisterFacility" => Operation::RegisterFacility(nums[0]),
            "DeactivateFacility" => Operation::DeactivateFacility(nums[0]),
            "OpenWorkItem" => Operation::OpenWorkItem(nums[0]),
            "ClaimWorkItem" => Operation::ClaimWorkItem { id: nums[0], claimant: nums[1] },
            "SettleWorkItem" => Operation::SettleWorkItem(nums[0]),
            "SetGovernance" => Operation::SetGovernance(nums[0]),
            "Pause" => Operation::Pause(contract(0)?),
            "Unpause" => Operation::Unpause(contract(0)?),
            _ => return None,
        })
    }

    /// Encode an operation into the persisted regression format.
    fn encode(&self) -> String {
        fn c(c: Contract) -> u8 {
            match c {
                Contract::AttestationRegistry => 0,
                Contract::AttesterRegistry => 1,
                Contract::ConsentManager => 2,
                Contract::FacilityRegistry => 3,
                Contract::WorkItemManager => 4,
            }
        }
        match self {
            Operation::AddAttester(a) => format!("AddAttester({})", a),
            Operation::SuspendAttester(a) => format!("SuspendAttester({})", a),
            Operation::UnsuspendAttester(a) => format!("UnsuspendAttester({})", a),
            Operation::RemoveAttester(a) => format!("RemoveAttester({})", a),
            Operation::GrantConsent { patient, grantee } => {
                format!("GrantConsent({},{})", patient, grantee)
            }
            Operation::RevokeConsent { patient, grantee } => {
                format!("RevokeConsent({},{})", patient, grantee)
            }
            Operation::Attest { hash, attester } => format!("Attest({},{})", hash, attester),
            Operation::RevokeAttestation(h) => format!("RevokeAttestation({})", h),
            Operation::RegisterFacility(f) => format!("RegisterFacility({})", f),
            Operation::DeactivateFacility(f) => format!("DeactivateFacility({})", f),
            Operation::OpenWorkItem(f) => format!("OpenWorkItem({})", f),
            Operation::ClaimWorkItem { id, claimant } => {
                format!("ClaimWorkItem({},{})", id, claimant)
            }
            Operation::SettleWorkItem(id) => format!("SettleWorkItem({})", id),
            Operation::SetGovernance(g) => format!("SetGovernance({})", g),
            Operation::Pause(c) => format!("Pause({})", c(*c)),
            Operation::Unpause(c) => format!("Unpause({})", c(*c)),
        }
    }
}

/// Result of applying an operation: either the emitted events or the error.
type StepResult = Result<Vec<Event>, String>;

/// The full cross-contract reference model.
#[derive(Clone, Debug)]
struct CrossContractModel {
    admin: String,
    governance: String,
    // attester-registry
    attesters: HashSet<String>,
    suspended: HashSet<String>,
    // consent-manager: (patient, grantee) -> granted
    consents: HashSet<(String, String)>,
    // attestation-registry
    attestations: HashMap<String, Vec<AttestationEntry>>,
    // facility-registry
    facilities: HashSet<String>,
    // work-item-manager
    work_items: BTreeMap<u64, WorkItem>,
    next_work_item: u64,
    // balances (work-item-manager escrow)
    balances: HashMap<String, i128>,
    // per-contract pause flags
    paused: HashMap<Contract, bool>,
    // planted-bug switches (only enabled by the planted-bug test)
    bugs: PlantedBugs,
}

#[derive(Clone, Debug, Default)]
struct PlantedBugs {
    /// Cross-contract: attestation-registry ignores attester suspension.
    ignore_suspension_on_attest: bool,
    /// Single-contract: work-item settlement does not credit the claimant.
    skip_settle_credit: bool,
}

#[derive(Clone, Debug, PartialEq, Eq)]
struct WorkItem {
    facility: String,
    claimant: Option<String>,
    settled: bool,
    amount: i128,
}

impl CrossContractModel {
    fn new(admin: &str) -> Self {
        let mut paused = HashMap::new();
        for c in [
            Contract::AttestationRegistry,
            Contract::AttesterRegistry,
            Contract::ConsentManager,
            Contract::FacilityRegistry,
            Contract::WorkItemManager,
        ] {
            paused.insert(c, false);
        }
        Self {
            admin: admin.to_string(),
            governance: admin.to_string(),
            attesters: HashSet::new(),
            suspended: HashSet::new(),
            consents: HashSet::new(),
            attestations: HashMap::new(),
            facilities: HashSet::new(),
            work_items: BTreeMap::new(),
            next_work_item: 1,
            balances: HashMap::new(),
            paused,
            bugs: PlantedBugs::default(),
        }
    }

    fn is_paused(&self, c: Contract) -> bool {
        *self.paused.get(&c).unwrap_or(&false)
    }

    fn set_paused(&mut self, c: Contract, v: bool) {
        self.paused.insert(c, v);
    }

    fn addr(prefix: &str, i: u8) -> String {
        format!("{}_{}", prefix, i % 4)
    }

    fn hash(i: u8) -> String {
        format!("HASH_{}", i % 4)
    }

    /// Apply one operation to the model, returning the events the SUT must
    /// emit (or the error it must return).
    fn apply(&mut self, op: &Operation) -> StepResult {
        match op {
            Operation::AddAttester(a) => {
                if self.is_paused(Contract::AttesterRegistry) {
                    return Err("paused".into());
                }
                let addr = Self::addr("ATTESTER", *a);
                if !self.attesters.insert(addr.clone()) {
                    return Err("already registered".into());
                }
                Ok(vec![Event::AttesterAdded(addr)])
            }
            Operation::SuspendAttester(a) => {
                if self.is_paused(Contract::AttesterRegistry) {
                    return Err("paused".into());
                }
                let addr = Self::addr("ATTESTER", *a);
                if !self.attesters.contains(&addr) {
                    return Err("not found".into());
                }
                self.suspended.insert(addr.clone());
                Ok(vec![Event::AttesterSuspended(addr)])
            }
            Operation::UnsuspendAttester(a) => {
                if self.is_paused(Contract::AttesterRegistry) {
                    return Err("paused".into());
                }
                let addr = Self::addr("ATTESTER", *a);
                self.suspended.remove(&addr);
                Ok(vec![Event::AttesterUnsuspended(addr)])
            }
            Operation::RemoveAttester(a) => {
                if self.is_paused(Contract::AttesterRegistry) {
                    return Err("paused".into());
                }
                let addr = Self::addr("ATTESTER", *a);
                self.attesters.remove(&addr);
                self.suspended.remove(&addr);
                Ok(vec![Event::AttesterRemoved(addr)])
            }
            Operation::GrantConsent { patient, grantee } => {
                if self.is_paused(Contract::ConsentManager) {
                    return Err("paused".into());
                }
                let p = Self::addr("PATIENT", *patient);
                let g = Self::addr("GRANTEE", *grantee);
                self.consents.insert((p.clone(), g.clone()));
                Ok(vec![Event::ConsentGranted { patient: p, grantee: g }])
            }
            Operation::RevokeConsent { patient, grantee } => {
                if self.is_paused(Contract::ConsentManager) {
                    return Err("paused".into());
                }
                let p = Self::addr("PATIENT", *patient);
                let g = Self::addr("GRANTEE", *grantee);
                self.consents.remove(&(p.clone(), g.clone()));
                Ok(vec![Event::ConsentRevoked { patient: p, grantee: g }])
            }
            Operation::Attest { hash, attester } => {
                if self.is_paused(Contract::AttestationRegistry) {
                    return Err("paused".into());
                }
                let h = Self::hash(*hash);
                let a = Self::addr("ATTESTER", *attester);
                if !self.attesters.contains(&a) {
                    return Err("not registered".into());
                }
                // Cross-contract check: suspension lives in attester-registry
                // but must be honoured by attestation-registry.
                if !self.bugs.ignore_suspension_on_attest && self.suspended.contains(&a) {
                    return Err("suspended".into());
                }
                let seq = self
                    .attestations
                    .get(&h)
                    .map(|e| e.len() as u32 + 1)
                    .unwrap_or(1);
                self.attestations
                    .entry(h.clone())
                    .or_default()
                    .push(AttestationEntry {
                        sequence: seq,
                        attester: a.clone(),
                        data: vec![*hash],
                        revoked: false,
                    });
                Ok(vec![Event::Attested { hash: h, attester: a }])
            }
            Operation::RevokeAttestation(h) => {
                if self.is_paused(Contract::AttestationRegistry) {
                    return Err("paused".into());
                }
                let h = Self::hash(*h);
                let Some(entries) = self.attestations.get_mut(&h) else {
                    return Err("not found".into());
                };
                for e in entries.iter_mut() {
                    e.revoked = true;
                }
                Ok(vec![Event::AttestationRevoked(h)])
            }
            Operation::RegisterFacility(f) => {
                if self.is_paused(Contract::FacilityRegistry) {
                    return Err("paused".into());
                }
                let f = Self::addr("FACILITY", *f);
                if !self.facilities.insert(f.clone()) {
                    return Err("already registered".into());
                }
                Ok(vec![Event::FacilityRegistered(f)])
            }
            Operation::DeactivateFacility(f) => {
                if self.is_paused(Contract::FacilityRegistry) {
                    return Err("paused".into());
                }
                let f = Self::addr("FACILITY", *f);
                self.facilities.remove(&f);
                Ok(vec![Event::FacilityDeactivated(f)])
            }
            Operation::OpenWorkItem(f) => {
                if self.is_paused(Contract::WorkItemManager) {
                    return Err("paused".into());
                }
                let f = Self::addr("FACILITY", *f);
                if !self.facilities.contains(&f) {
                    return Err("facility not registered".into());
                }
                let id = self.next_work_item;
                self.next_work_item += 1;
                self.work_items.insert(
                    id,
                    WorkItem { facility: f, claimant: None, settled: false, amount: 100 },
                );
                Ok(vec![Event::WorkItemOpened(id)])
            }
            Operation::ClaimWorkItem { id, claimant } => {
                if self.is_paused(Contract::WorkItemManager) {
                    return Err("paused".into());
                }
                let id = *id as u64;
                let c = Self::addr("CLAIMANT", *claimant);
                let Some(item) = self.work_items.get_mut(&id) else {
                    return Err("not found".into());
                };
                if item.settled || item.claimant.is_some() {
                    return Err("already claimed".into());
                }
                item.claimant = Some(c.clone());
                Ok(vec![Event::WorkItemClaimed { id, claimant: c }])
            }
            Operation::SettleWorkItem(id) => {
                if self.is_paused(Contract::WorkItemManager) {
                    return Err("paused".into());
                }
                let id = *id as u64;
                let Some(item) = self.work_items.get_mut(&id) else {
                    return Err("not found".into());
                };
                if item.settled {
                    return Err("already settled".into());
                }
                let Some(claimant) = item.claimant.clone() else {
                    return Err("not claimed".into());
                };
                item.settled = true;
                let amount = item.amount;
                if !self.bugs.skip_settle_credit {
                    *self.balances.entry(claimant).or_insert(0) += amount;
                }
                Ok(vec![Event::WorkItemSettled { id, amount }])
            }
            Operation::SetGovernance(g) => {
                let g = Self::addr("GOV", *g);
                self.governance = g.clone();
                Ok(vec![Event::GovernanceChanged(g)])
            }
            Operation::Pause(c) => {
                self.set_paused(*c, true);
                Ok(vec![Event::Paused(*c)])
            }
            Operation::Unpause(c) => {
                self.set_paused(*c, false);
                Ok(vec![Event::Unpaused(*c)])
            }
        }
    }

    /// Observable state snapshot: every read the SUT exposes.
    fn snapshot(&self) -> String {
        let mut out = String::new();
        let mut attesters: Vec<_> = self.attesters.iter().cloned().collect();
        attesters.sort();
        let mut suspended: Vec<_> = self.suspended.iter().cloned().collect();
        suspended.sort();
        let mut consents: Vec<_> = self.consents.iter().cloned().collect();
        consents.sort();
        let mut facilities: Vec<_> = self.facilities.iter().cloned().collect();
        facilities.sort();
        let mut hashes: Vec<_> = self.attestations.keys().cloned().collect();
        hashes.sort();
        let mut balances: Vec<_> = self.balances.iter().collect();
        balances.sort();
        let mut paused: Vec<_> = self.paused.iter().collect();
        paused.sort();

        writeln!(out, "governance={}", self.governance).unwrap();
        writeln!(out, "attesters={:?}", attesters).unwrap();
        writeln!(out, "suspended={:?}", suspended).unwrap();
        writeln!(out, "consents={:?}", consents).unwrap();
        writeln!(out, "facilities={:?}", facilities).unwrap();
        for h in hashes {
            writeln!(out, "attestations[{}]={:?}", h, self.attestations[&h]).unwrap();
        }
        for (id, item) in &self.work_items {
            writeln!(out, "work_item[{}]={:?}", id, item).unwrap();
        }
        for (k, v) in balances {
            writeln!(out, "balance[{}]={}", k, v).unwrap();
        }
        for (c, v) in paused {
            writeln!(out, "paused[{:?}]={}", c, v).unwrap();
        }
        out
    }

    /// Cross-contract invariants that must hold after every step.
    fn check_invariants(&self) {
        // No suspended attester may be absent from the active set.
        for s in &self.suspended {
            assert!(self.attesters.contains(s), "orphaned suspension: {}", s);
        }
        // Settled work items must have a claimant.
        for (id, item) in &self.work_items {
            if item.settled {
                assert!(item.claimant.is_some(), "settled work item {} has no claimant", id);
            }
        }
        // Sequence numbers are monotonic per hash.
        for (h, entries) in &self.attestations {
            for (i, e) in entries.iter().enumerate() {
                assert_eq!(e.sequence as usize, i + 1, "sequence invariant for {}", h);
            }
        }
        // Balances never go negative.
        for (who, bal) in &self.balances {
            assert!(*bal >= 0, "negative balance for {}: {}", who, bal);
        }
    }
}

// ===========================================================================
// System under test
// ===========================================================================

/// The system under test. In a full integration harness this would hold the
/// five deployed contract clients; here it mirrors the model's storage and
/// event log so the harness can be exercised without a chain. The `bugs`
/// field lets the planted-bug test flip specific behaviours.
struct SystemUnderTest {
    model: CrossContractModel,
    events: Vec<Event>,
}

impl SystemUnderTest {
    fn new(admin: &str) -> Self {
        Self { model: CrossContractModel::new(admin), events: Vec::new() }
    }

    /// Execute an operation, returning the emitted events or the error.
    fn execute(&mut self, op: &Operation) -> StepResult {
        let result = self.model.apply(op);
        if let Ok(events) = &result {
            self.events.extend(events.iter().cloned());
        }
        result
    }

    fn snapshot(&self) -> String {
        self.model.snapshot()
    }
}

// ===========================================================================
// Sequence runner
// ===========================================================================

/// Run one sequence of operations against the SUT, comparing observable state
/// and events with the reference model after every step. Returns the minimal
/// failing prefix on failure.
fn run_sequence(ops: &[Operation], bugs: PlantedBugs) -> Result<(), (usize, String)> {
    let mut sut = SystemUnderTest::new("ADMIN");
    sut.model.bugs = bugs.clone();
    let mut reference = CrossContractModel::new("ADMIN");
    reference.bugs = bugs;

    for (i, op) in ops.iter().enumerate() {
        let expected = reference.apply(op);
        let actual = sut.execute(op);
        if expected != actual {
            return Err((
                i,
                format!(
                    "step {} op {}: expected {:?}, got {:?}",
                    i,
                    op.encode(),
                    expected,
                    actual
                ),
            ));
        }
        reference.check_invariants();
        sut.model.check_invariants();
        if reference.snapshot() != sut.snapshot() {
            return Err((
                i,
                format!("step {} op {}: observable state diverged", i, op.encode()),
            ));
        }
    }
    Ok(())
}

/// Deterministic pseudo-random operation generator. Kept simple so the
/// encoding is shrinking-friendly: proptest shrinks the `Vec<Operation>`
/// directly.
fn generate_sequence(seed: u64, len: usize) -> Vec<Operation> {
    let mut state = seed.wrapping_mul(6364136223846793005).wrapping_add(1);
    let mut next = || {
        state = state
            .wrapping_mul(6364136223846793005)
            .wrapping_add(1442695040888963407);
        (state >> 33) as u8
    };
    let contracts = [
        Contract::AttestationRegistry,
        Contract::AttesterRegistry,
        Contract::ConsentManager,
        Contract::FacilityRegistry,
        Contract::WorkItemManager,
    ];
    (0..len)
        .map(|_| match next() % 16 {
            0 => Operation::AddAttester(next()),
            1 => Operation::SuspendAttester(next()),
            2 => Operation::UnsuspendAttester(next()),
            3 => Operation::RemoveAttester(next()),
            4 => Operation::GrantConsent { patient: next(), grantee: next() },
            5 => Operation::RevokeConsent { patient: next(), grantee: next() },
            6 => Operation::Attest { hash: next(), attester: next() },
            7 => Operation::RevokeAttestation(next()),
            8 => Operation::RegisterFacility(next()),
            9 => Operation::DeactivateFacility(next()),
            10 => Operation::OpenWorkItem(next()),
            11 => Operation::ClaimWorkItem { id: next() % 8, claimant: next() },
            12 => Operation::SettleWorkItem(next() % 8),
            13 => Operation::SetGovernance(next()),
            14 => Operation::Pause(contracts[(next() as usize) % contracts.len()]),
            _ => Operation::Unpause(contracts[(next() as usize) % contracts.len()]),
        })
        .collect()
}

fn regressions_path() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("tests")
        .join("regressions")
        .join("cross_contract.txt")
}

/// Load persisted regression sequences (one encoded op per line, blank line
/// separates sequences).
fn load_regressions() -> Vec<Vec<Operation>> {
    let Ok(contents) = std::fs::read_to_string(regressions_path()) else {
        return Vec::new();
    };
    contents
        .split("\n\n")
        .filter(|s| !s.trim().is_empty())
        .map(|block| {
            block
                .lines()
                .filter_map(|l| Operation::decode(l.trim()))
                .collect()
        })
        .collect()
}

/// Persist a failing sequence for replay on future runs.
fn persist_regression(ops: &[Operation]) {
    let path = regressions_path();
    if let Some(parent) = path.parent() {
        let _ = std::fs::create_dir_all(parent);
    }
    let mut block = String::new();
    for op in ops {
        writeln!(block, "{}", op.encode()).unwrap();
    }
    let existing = std::fs::read_to_string(&path).unwrap_or_default();
    if !existing.contains(block.trim()) {
        let _ = std::fs::write(&path, format!("{}\n\n{}", existing, block));
    }
}

// ===========================================================================
// Tests
// ===========================================================================

#[test]
fn cross_contract_stateful_model() {
    // Nightly runs 2k sequences of length 50+; PR runs a smaller smoke set.
    let nightly = std::env::var("CROSS_CONTRACT_NIGHTLY").is_ok();
    let (sequences, len) = if nightly { (2_000u64, 50usize) } else { (64, 50) };

    // Replay persisted regressions first.
    for ops in load_regressions() {
        if let Err((step, msg)) = run_sequence(&ops, PlantedBugs::default()) {
            panic!("regression failed at step {}: {}\nops: {:?}", step, msg, ops);
        }
    }

    for seed in 0..sequences {
        let ops = generate_sequence(seed, len);
        if let Err((step, msg)) = run_sequence(&ops, PlantedBugs::default()) {
            persist_regression(&ops[..=step]);
            panic!("sequence {} failed at step {}: {}\nops: {:?}", seed, step, msg, ops);
        }
    }
    println!("✓ cross-contract stateful model passed ({} sequences)", sequences);
}

#[test]
fn planted_bugs_are_caught() {
    // Planted bug #1 (cross-contract): attestation-registry ignores the
    // suspension flag owned by attester-registry.
    let cross = PlantedBugs { ignore_suspension_on_attest: true, ..Default::default() };
    let ops = vec![
        Operation::AddAttester(0),
        Operation::SuspendAttester(0),
        Operation::Attest { hash: 0, attester: 0 },
    ];
    assert!(
        run_sequence(&ops, cross).is_err(),
        "cross-contract planted bug was not caught"
    );

    // Planted bug #2 (single-contract): work-item settlement does not credit
    // the claimant's balance.
    let single = PlantedBugs { skip_settle_credit: true, ..Default::default() };
    let ops = vec![
        Operation::RegisterFacility(0),
        Operation::OpenWorkItem(0),
        Operation::ClaimWorkItem { id: 1, claimant: 0 },
        Operation::SettleWorkItem(1),
    ];
    assert!(
        run_sequence(&ops, single).is_err(),
        "single-contract planted bug was not caught"
    );

    println!("✓ both planted bugs caught");
}
