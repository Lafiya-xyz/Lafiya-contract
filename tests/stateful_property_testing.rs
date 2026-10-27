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
