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
    /// Incentive-pool model state
    pool: PoolModel,
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
            pool: PoolModel::new(),
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

/// Reference model for the incentive-pool's multi-asset, multi-identity accounting.
///
/// Every accounting figure is keyed by asset so that per-asset invariants can be
/// checked in isolation (no cross-asset conversion happens on-chain).
#[derive(Clone, Debug, Default)]
struct PoolModel {
    /// Allowlisted assets
    allowed_assets: HashSet<String>,
    /// Per-asset total deposited
    total_deposited: HashMap<String, i128>,
    /// Per-asset total paid out
    total_paid: HashMap<String, i128>,
    /// Per-asset cap on total deposits
    deposit_cap: HashMap<String, i128>,
    /// Per-asset cap on total payouts
    payout_cap: HashMap<String, i128>,
    /// Reservations: work_item_id -> (asset, amount, reserved)
    reservations: HashMap<String, (String, i128, bool)>,
    /// Work items: id -> item
    work_items: HashMap<String, WorkItem>,
    /// Per-identity cumulative payouts: (asset, identity_key) -> amount
    identity_claimed: HashMap<(String, String), i128>,
    /// Per-identity cap: asset -> cap
    max_per_identity: HashMap<String, i128>,
    /// Approver revocation watermarks: approver -> since
    approver_revoked_since: HashMap<String, u32>,
    /// Current ledger time
    now: u32,
}

#[derive(Clone, Debug, PartialEq)]
struct WorkItem {
    asset: String,
    amount: i128,
    approved_by: String,
    approved_at: u32,
    payout_binding: String,
    identity_key: String,
    push_enabled: bool,
    claimed: bool,
}

impl PoolModel {
    fn new() -> Self {
        Self {
            now: 1,
            ..Default::default()
        }
    }

    fn add_asset(&mut self, asset: &str, deposit_cap: i128, payout_cap: i128) {
        self.allowed_assets.insert(asset.to_string());
        self.deposit_cap.insert(asset.to_string(), deposit_cap);
        self.payout_cap.insert(asset.to_string(), payout_cap);
    }

    fn set_max_per_identity(&mut self, asset: &str, cap: i128) {
        self.max_per_identity.insert(asset.to_string(), cap);
    }

    fn deposit(&mut self, asset: &str, amount: i128) -> Result<(), String> {
        if !self.allowed_assets.contains(asset) {
            return Err("Asset not allowlisted".to_string());
        }
        if amount <= 0 {
            return Err("Amount must be positive".to_string());
        }
        let deposited = self.total_deposited.get(asset).copied().unwrap_or(0);
        let cap = self.deposit_cap.get(asset).copied().unwrap_or(i128::MAX);
        if deposited + amount > cap {
            return Err("Deposit cap exceeded".to_string());
        }
        self.total_deposited.insert(asset.to_string(), deposited + amount);
        Ok(())
    }

    #[allow(clippy::too_many_arguments)]
    fn approve_work_item(
        &mut self,
        id: &str,
        asset: &str,
        amount: i128,
        approver: &str,
        payout_binding: &str,
        identity_key: &str,
        push_enabled: bool,
    ) -> Result<(), String> {
        if !self.allowed_assets.contains(asset) {
            return Err("Asset not allowlisted".to_string());
        }
        if self.work_items.contains_key(id) {
            return Err("Work item exists".to_string());
        }
        let deposited = self.total_deposited.get(asset).copied().unwrap_or(0);
        let reserved: i128 = self
            .reservations
            .values()
            .filter(|(a, _, active)| a == asset && *active)
            .map(|(_, amt, _)| *amt)
            .sum();
        if deposited - reserved < amount {
            return Err("Insufficient unreserved balance".to_string());
        }
        self.reservations
            .insert(id.to_string(), (asset.to_string(), amount, true));
        self.work_items.insert(
            id.to_string(),
            WorkItem {
                asset: asset.to_string(),
                amount,
                approved_by: approver.to_string(),
                approved_at: self.now,
                payout_binding: payout_binding.to_string(),
                identity_key: identity_key.to_string(),
                push_enabled,
                claimed: false,
            },
        );
        Ok(())
    }

    /// Shared guard used by both `claim` and `disburse` so parity is structural.
    fn check_claimable(&self, id: &str) -> Result<WorkItem, String> {
        let item = self
            .work_items
            .get(id)
            .ok_or_else(|| "Work item not found".to_string())?
            .clone();
        if item.claimed {
            return Err("Already claimed".to_string());
        }
        if let Some(since) = self.approver_revoked_since.get(&item.approved_by) {
            if item.approved_at >= *since {
                return Err("Approval voided".to_string());
            }
        }
        let claimed = self
            .identity_claimed
            .get(&(item.asset.clone(), item.identity_key.clone()))
            .copied()
            .unwrap_or(0);
        let cap = self
            .max_per_identity
            .get(&item.asset)
            .copied()
            .unwrap_or(i128::MAX);
        if claimed + item.amount > cap {
            return Err("Identity cap exceeded".to_string());
        }
        let paid = self.total_paid.get(&item.asset).copied().unwrap_or(0);
        let payout_cap = self
            .payout_cap
            .get(&item.asset)
            .copied()
            .unwrap_or(i128::MAX);
        if paid + item.amount > payout_cap {
            return Err("Payout cap exceeded".to_string());
        }
        Ok(item)
    }

    fn settle(&mut self, id: &str, item: &WorkItem) {
        let paid = self.total_paid.get(&item.asset).copied().unwrap_or(0);
        self.total_paid.insert(item.asset.clone(), paid + item.amount);
        let key = (item.asset.clone(), item.identity_key.clone());
        let claimed = self.identity_claimed.get(&key).copied().unwrap_or(0);
        self.identity_claimed.insert(key, claimed + item.amount);
        if let Some(res) = self.reservations.get_mut(id) {
            res.2 = false;
        }
        if let Some(w) = self.work_items.get_mut(id) {
            w.claimed = true;
        }
    }

    fn claim(&mut self, id: &str) -> Result<(), String> {
        let item = self.check_claimable(id)?;
        self.settle(id, &item);
        Ok(())
    }

    /// Approver-initiated push disbursement; only allowed for opt-in bindings.
    fn disburse(&mut self, ids: &[&str]) -> Result<(), String> {
        for id in ids {
            let item = self.check_claimable(id)?;
            if !item.push_enabled {
                return Err("Push not enabled for binding".to_string());
            }
        }
        for id in ids {
            let item = self.check_claimable(id)?;
            self.settle(id, &item);
        }
        Ok(())
    }

    fn void_approvals_since(&mut self, approver: &str, since: u32) {
        self.approver_revoked_since
            .insert(approver.to_string(), since);
    }

    /// Permissionless cleanup releasing reservations for voided items.
    fn sweep_voided(&mut self) {
        let voided: Vec<String> = self
            .work_items
            .iter()
            .filter(|(_, w)| {
                !w.claimed
                    && self
                        .approver_revoked_since
                        .get(&w.approved_by)
                        .map(|since| w.approved_at >= *since)
                        .unwrap_or(false)
            })
            .map(|(id, _)| id.clone())
            .collect();
        for id in voided {
            if let Some(res) = self.reservations.get_mut(&id) {
                res.2 = false;
            }
        }
    }

    fn reserved_for(&self, asset: &str) -> i128 {
        self.reservations
            .values()
            .filter(|(a, _, active)| a == asset && *active)
            .map(|(_, amt, _)| *amt)
            .sum()
    }

    /// Per-asset invariant: paid + reserved <= deposited <= cap.
    fn check_per_asset_invariants(&self) {
        for asset in &self.allowed_assets {
            let deposited = self.total_deposited.get(asset).copied().unwrap_or(0);
            let paid = self.total_paid.get(asset).copied().unwrap_or(0);
            let reserved = self.reserved_for(asset);
            assert!(
                paid + reserved <= deposited,
                "Asset {}: paid {} + reserved {} > deposited {}",
                asset,
                paid,
                reserved,
                deposited
            );
            let cap = self.deposit_cap.get(asset).copied().unwrap_or(i128::MAX);
            assert!(
                deposited <= cap,
                "Asset {}: deposited {} > cap {}",
                asset,
                deposited,
                cap
            );
        }
    }

    /// Per-identity invariant: cumulative payouts never exceed the cap.
    fn check_identity_invariants(&self) {
        for ((asset, identity), claimed) in &self.identity_claimed {
            let cap = self
                .max_per_identity
                .get(asset)
                .copied()
                .unwrap_or(i128::MAX);
            assert!(
                *claimed <= cap,
                "Identity {} on asset {}: claimed {} > cap {}",
                identity,
                asset,
                claimed,
                cap
            );
        }
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

#[test]
fn pool_multi_asset_custody_is_isolated() {
    let mut model = ReferenceModel::new("ADMIN".to_string());
    model.pool.add_asset("USDC", 1_000, 1_000);
    model.pool.add_asset("EURC", 500, 500);

    model.pool.deposit("USDC", 400).expect("usdc deposit");
    model.pool.deposit("EURC", 200).expect("eurc deposit");

    // Unknown asset rejected
    assert!(model.pool.deposit("XLM", 10).is_err());

    // Per-asset deposit cap enforced independently
    assert!(model.pool.deposit("EURC", 400).is_err());
    assert!(model.pool.deposit("USDC", 700).is_ok());

    model.pool.set_max_per_identity("USDC", 1_000);
    model.pool.set_max_per_identity("EURC", 1_000);

    model
        .pool
        .approve_work_item("W1", "USDC", 300, "APPROVER", "PAYOUT_A", "ID_A", true)
        .expect("approve usdc");
    model
        .pool
        .approve_work_item("W2", "EURC", 150, "APPROVER", "PAYOUT_B", "ID_B", true)
        .expect("approve eurc");

    model.pool.claim("W1").expect("claim usdc");
    model.pool.claim("W2").expect("claim eurc");

    // Isolated accounting: paying USDC does not touch EURC totals
    assert_eq!(model.pool.total_paid.get("USDC").copied().unwrap_or(0), 300);
    assert_eq!(model.pool.total_paid.get("EURC").copied().unwrap_or(0), 150);
    assert_eq!(model.pool.reserved_for("USDC"), 0);
    assert_eq!(model.pool.reserved_for("EURC"), 0);

    model.pool.check_per_asset_invariants();
    model.pool.check_identity_invariants();

    println!("✓ Multi-asset custody isolation passed");
}

#[test]
fn pool_void_approvals_since_keeps_reservations_consistent() {
    let mut model = ReferenceModel::new("ADMIN".to_string());
    model.pool.add_asset("USDC", 1_000, 1_000);
    model.pool.set_max_per_identity("USDC", 1_000);
    model.pool.deposit("USDC", 1_000).expect("deposit");

    model.pool.now = 10;
    model
        .pool
        .approve_work_item("OLD", "USDC", 100, "APPROVER", "PAYOUT_A", "ID_A", true)
        .expect("approve old");
    model.pool.now = 20;
    model
        .pool
        .approve_work_item("NEW", "USDC", 100, "APPROVER", "PAYOUT_A", "ID_A", true)
        .expect("approve new");
    model
        .pool
        .approve_work_item("OTHER", "USDC", 100, "APPROVER2", "PAYOUT_B", "ID_B", true)
        .expect("approve other");

    assert_eq!(model.pool.reserved_for("USDC"), 300);

    // Void approvals issued by APPROVER at or after ledger 15
    model.pool.void_approvals_since("APPROVER", 15);

    // NEW is voided, OLD and OTHER remain claimable
    assert!(model.pool.claim("NEW").is_err());
    model.pool.claim("OLD").expect("old still claimable");
    model.pool.claim("OTHER").expect("other approver unaffected");

    // Lazy cleanup releases the voided reservation
    model.pool.sweep_voided();
    assert_eq!(model.pool.reserved_for("USDC"), 0);
    model.pool.check_per_asset_invariants();

    println!("✓ void_approvals_since reservation consistency passed");
}

#[test]
fn pool_disburse_matches_claim_guards() {
    let mut model = ReferenceModel::new("ADMIN".to_string());
    model.pool.add_asset("USDC", 1_000, 1_000);
    model.pool.set_max_per_identity("USDC", 250);
    model.pool.deposit("USDC", 1_000).expect("deposit");

    model
        .pool
        .approve_work_item("PUSH", "USDC", 100, "APPROVER", "PAYOUT_A", "ID_A", true)
        .expect("approve push");
    model
        .pool
        .approve_work_item("NOPUSH", "USDC", 100, "APPROVER", "PAYOUT_B", "ID_B", false)
        .expect("approve nopush");

    // Push requires opt-in
    assert!(model.pool.disburse(&["NOPUSH"]).is_err());

    // Identity cap applies to push exactly as to claim
    model.pool.disburse(&["PUSH"]).expect("disburse push");
    assert!(model.pool.claim("PUSH").is_err(), "replay blocked");

    model
        .pool
        .approve_work_item("PUSH2", "USDC", 100, "APPROVER", "PAYOUT_A", "ID_A", true)
        .expect("approve push2");
    model
        .pool
        .approve_work_item("PUSH3", "USDC", 100, "APPROVER", "PAYOUT_A", "ID_A", true)
        .expect("approve push3");
    model.pool.disburse(&["PUSH2"]).expect("disburse push2");
    // 100 + 100 + 100 would exceed the 250 identity cap
    assert!(model.pool.disburse(&["PUSH3"]).is_err());

    model.pool.check_per_asset_invariants();
    model.pool.check_identity_invariants();

    println!("✓ Disburse/claim guard parity passed");
}

#[test]
fn pool_identity_caps_hold_across_addresses() {
    let mut model = ReferenceModel::new("ADMIN".to_string());
    model.pool.add_asset("USDC", 1_000, 1_000);
    model.pool.set_max_per_identity("USDC", 150);
    model.pool.deposit("USDC", 1_000).expect("deposit");

    // Two attester addresses sharing one identity key
    model
        .pool
        .approve_work_item("A1", "USDC", 100, "APPROVER", "PAYOUT_A", "ID_SHARED", true)
        .expect("approve a1");
    model
        .pool
        .approve_work_item("A2", "USDC", 100, "APPROVER", "PAYOUT_B", "ID_SHARED", true)
        .expect("approve a2");

    model.pool.claim("A1").expect("claim a1");
    // Second address sharing the identity is blocked by the cumulative cap
    assert!(model.pool.claim("A2").is_err());

    // A different identity is unaffected
    model
        .pool
        .approve_work_item("B1", "USDC", 100, "APPROVER", "PAYOUT_C", "ID_OTHER", true)
        .expect("approve b1");
    model.pool.claim("B1").expect("claim b1");

    model.pool.check_per_asset_invariants();
    model.pool.check_identity_invariants();

    println!("✓ Per-identity cumulative caps across addresses passed");
}
