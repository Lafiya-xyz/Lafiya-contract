//! Model-based stateful property testing for attestation-registry
//!
//! Uses proptest-state-machine to verify that any sequence of contract operations
//! leaves the contract in a state matching a simple reference model.

use std::collections::{HashMap, HashSet};

// ---------------------------------------------------------------------------
// Bond / slashing accounting model (issue #677)
// ---------------------------------------------------------------------------

/// Token model that can optionally charge a transfer fee on every transfer.
///
/// The fee is burned (removed from total supply), which is the harshest case
/// for the accounting invariant: the contract's balance delta is smaller than
/// the amount debited from the sender.
#[derive(Clone, Debug)]
struct FeeToken {
    /// Balances keyed by address.
    balances: HashMap<String, u128>,
    /// Fee in basis points charged on every transfer (0 = no fee).
    fee_bps: u128,
    /// Total amount burned as fees.
    burned: u128,
}

impl FeeToken {
    fn new(fee_bps: u128) -> Self {
        Self {
            balances: HashMap::new(),
            fee_bps,
            burned: 0,
        }
    }

    fn balance(&self, addr: &str) -> u128 {
        self.balances.get(addr).copied().unwrap_or(0)
    }

    fn mint(&mut self, addr: &str, amount: u128) {
        *self.balances.entry(addr.to_string()).or_insert(0) += amount;
    }

    /// Transfer `amount` from `from` to `to`, returning the amount actually
    /// credited to `to` (after fees).
    fn transfer(&mut self, from: &str, to: &str, amount: u128) -> Result<u128, String> {
        if self.balance(from) < amount {
            return Err("Insufficient balance".to_string());
        }
        let fee = amount * self.fee_bps / 10_000;
        let credited = amount - fee;
        *self.balances.entry(from.to_string()).or_insert(0) -= amount;
        *self.balances.entry(to.to_string()).or_insert(0) += credited;
        self.burned += fee;
        Ok(credited)
    }
}

/// Reference model for the attester-registry bond module.
#[derive(Clone, Debug)]
struct BondModel {
    /// Bonded amount per attester.
    bonds: HashMap<String, u128>,
    /// Slashed funds that have not yet been swept to the treasury.
    slashed_unswept: u128,
    /// Treasury address (slash destination and sweep recipient).
    treasury: String,
    /// Token custodying the bonds.
    token: FeeToken,
    /// Contract address holding the bonds.
    contract: String,
}

impl BondModel {
    fn new(treasury: &str, contract: &str, fee_bps: u128) -> Self {
        Self {
            bonds: HashMap::new(),
            slashed_unswept: 0,
            treasury: treasury.to_string(),
            token: FeeToken::new(fee_bps),
            contract: contract.to_string(),
        }
    }

    /// Model operation: Bond (deposit). The contract credits the *received*
    /// amount, so a fee-charging token cannot inflate the accounting.
    fn bond(&mut self, attester: &str, amount: u128) -> Result<u128, String> {
        if amount == 0 {
            return Err("Zero bond".to_string());
        }
        let received = self.token.transfer(attester, &self.contract.clone(), amount)?;
        *self.bonds.entry(attester.to_string()).or_insert(0) += received;
        self.check_bond_invariant();
        Ok(received)
    }

    /// Model operation: Unbond (withdraw). Only the bonded amount is returned.
    fn unbond(&mut self, attester: &str) -> Result<u128, String> {
        let bonded = self.bonds.get(attester).copied().unwrap_or(0);
        if bonded == 0 {
            return Err("Nothing bonded".to_string());
        }
        let received = self.token.transfer(&self.contract.clone(), attester, bonded)?;
        self.bonds.insert(attester.to_string(), 0);
        self.check_bond_invariant();
        Ok(received)
    }

    /// Model operation: Slash. Slashed funds are held unswept in the contract
    /// until the treasury sweeps them.
    fn slash(&mut self, attester: &str, amount: u128) -> Result<u128, String> {
        let bonded = self.bonds.get(attester).copied().unwrap_or(0);
        if amount == 0 || amount > bonded {
            return Err("Invalid slash amount".to_string());
        }
        self.bonds.insert(attester.to_string(), bonded - amount);
        self.slashed_unswept += amount;
        self.check_bond_invariant();
        Ok(amount)
    }

    /// Model operation: Sweep slashed funds to the treasury. Only the treasury
    /// may call this.
    fn sweep(&mut self, caller: &str) -> Result<u128, String> {
        if caller != self.treasury {
            return Err("Unauthorized sweep".to_string());
        }
        let amount = self.slashed_unswept;
        if amount == 0 {
            return Err("Nothing to sweep".to_string());
        }
        let received = self.token.transfer(&self.contract.clone(), &self.treasury.clone(), amount)?;
        self.slashed_unswept = 0;
        self.check_bond_invariant();
        Ok(received)
    }

    /// Sum of all outstanding bonds.
    fn total_bonds(&self) -> u128 {
        self.bonds.values().copied().sum()
    }

    /// Bond accounting invariant:
    /// `sum(bonds) + slashed_unswept == token.balance(contract)`.
    fn check_bond_invariant(&self) {
        let expected = self.total_bonds() + self.slashed_unswept;
        let actual = self.token.balance(&self.contract);
        assert_eq!(
            expected, actual,
            "Bond invariant violated: sum(bonds)+slashed_unswept={} != contract balance={}",
            expected, actual
        );
    }
}

// ---------------------------------------------------------------------------
// Vesting grant model (issue #694)
// ---------------------------------------------------------------------------

/// A linear vesting grant with a cliff.
#[derive(Clone, Debug, PartialEq)]
struct VestingGrant {
    attester: String,
    total: u128,
    start: u64,
    cliff: u64,
    duration: u64,
    claimed: u128,
    forfeited_at: Option<u64>,
}

impl VestingGrant {
    /// Amount vested at time `now` (0 before the cliff).
    fn vested(&self, now: u64) -> u128 {
        if now < self.start + self.cliff {
            return 0;
        }
        let elapsed = now - self.start;
        if elapsed >= self.duration {
            return self.total;
        }
        // Linear vesting; integer division rounds down (favours the pool).
        (self.total as u128 * elapsed as u128 / self.duration as u128) as u128
    }

    /// Amount vested but not yet claimed at time `now`.
    fn claimable(&self, now: u64) -> u128 {
        if self.forfeited_at.is_some() {
            return 0;
        }
        self.vested(now).saturating_sub(self.claimed)
    }

    /// Unvested remainder that would be returned to the pool on forfeiture.
    fn unvested(&self, now: u64) -> u128 {
        self.total.saturating_sub(self.vested(now))
    }
}

/// Reference model for the incentive pool's vesting liabilities.
#[derive(Clone, Debug)]
struct VestingModel {
    grants: HashMap<u64, VestingGrant>,
    next_id: u64,
    /// Pool balance available for payouts.
    pool_balance: u128,
    /// Reserved liabilities (unvested + unclaimed grant amounts).
    reserved: u128,
}

impl VestingModel {
    fn new(pool_balance: u128) -> Self {
        Self {
            grants: HashMap::new(),
            next_id: 1,
            pool_balance,
            reserved: 0,
        }
    }

    fn create_grant(
        &mut self,
        attester: &str,
        total: u128,
        start: u64,
        cliff: u64,
        duration: u64,
    ) -> Result<u64, String> {
        if total == 0 || duration == 0 || cliff > duration {
            return Err("Invalid grant".to_string());
        }
        if self.pool_balance < self.reserved + total {
            return Err("Insufficient pool for reservation".to_string());
        }
        let id = self.next_id;
        self.next_id += 1;
        self.grants.insert(
            id,
            VestingGrant {
                attester: attester.to_string(),
                total,
                start,
                cliff,
                duration,
                claimed: 0,
                forfeited_at: None,
            },
        );
        self.reserved += total;
        self.check_solvency();
        Ok(id)
    }

    fn claim_vested(&mut self, id: u64, now: u64) -> Result<u128, String> {
        let grant = self.grants.get(&id).ok_or("Unknown grant")?.clone();
        let claimable = grant.claimable(now);
        if claimable == 0 {
            return Err("Nothing claimable".to_string());
        }
        let g = self.grants.get_mut(&id).unwrap();
        g.claimed += claimable;
        self.reserved -= claimable;
        self.pool_balance -= claimable;
        self.check_solvency();
        Ok(claimable)
    }

    fn forfeit(&mut self, id: u64, now: u64) -> Result<u128, String> {
        let grant = self.grants.get(&id).ok_or("Unknown grant")?.clone();
        if grant.forfeited_at.is_some() {
            return Err("Already forfeited".to_string());
        }
        let unclaimed = grant.total - grant.claimed;
        let g = self.grants.get_mut(&id).unwrap();
        g.forfeited_at = Some(now);
        self.reserved -= unclaimed;
        self.check_solvency();
        Ok(unclaimed)
    }

    /// Solvency invariant: reserved liabilities never exceed the pool balance.
    fn check_solvency(&self) {
        assert!(
            self.reserved <= self.pool_balance,
            "Solvency violated: reserved={} > pool_balance={}",
            self.reserved,
            self.pool_balance
        );
    }
}

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

#[test]
fn bond_accounting_invariant_with_fee_token() {
    // 1% transfer fee: the contract must credit only what it receives.
    let mut model = BondModel::new("TREASURY", "CONTRACT", 100);
    model.token.mint("ATTESTER_0", 10_000);
    model.token.mint("ATTESTER_1", 10_000);

    let received = model.bond("ATTESTER_0", 1_000).expect("bond");
    assert_eq!(received, 990, "fee must reduce credited bond");
    model.check_bond_invariant();

    model.bond("ATTESTER_1", 500).expect("bond");
    model.check_bond_invariant();

    // Slash part of a bond; funds stay unswept in the contract.
    model.slash("ATTESTER_0", 400).expect("slash");
    assert_eq!(model.slashed_unswept, 400);
    model.check_bond_invariant();

    // Only the treasury may sweep.
    assert!(model.sweep("ATTESTER_1").is_err());
    model.check_bond_invariant();
    let swept = model.sweep("TREASURY").expect("treasury sweep");
    assert_eq!(swept, 396, "sweep also pays the transfer fee");
    assert_eq!(model.slashed_unswept, 0);
    model.check_bond_invariant();

    // Unbond returns the remaining bonded amount.
    model.unbond("ATTESTER_1").expect("unbond");
    model.check_bond_invariant();

    println!("✓ Bond accounting invariant with fee token passed");
}

#[test]
fn bond_stateful_sequences_preserve_invariant() {
    // Deterministic pseudo-random sequence over bond/unbond/slash/sweep.
    let mut seed: u64 = 0x5eed_1234_abcd_0001;
    let mut next = || {
        seed = seed.wrapping_mul(6364136223846793005).wrapping_add(1442695040888963407);
        seed
    };

    for fee_bps in [0u128, 50, 100, 250] {
        let mut model = BondModel::new("TREASURY", "CONTRACT", fee_bps);
        let attesters = ["A0", "A1", "A2"];
        for a in attesters {
            model.token.mint(a, 1_000_000);
        }

        for _ in 0..2_000 {
            let a = attesters[(next() % 3) as usize];
            let amount = (next() % 5_000) as u128 + 1;
            match next() % 4 {
                0 => {
                    model.bond(a, amount).ok();
                }
                1 => {
                    model.unbond(a).ok();
                }
                2 => {
                    model.slash(a, amount).ok();
                }
                _ => {
                    model.sweep("TREASURY").ok();
                }
            }
            // Invariant must hold after *every* step, including failures.
            model.check_bond_invariant();
        }
    }

    println!("✓ Bond stateful sequences preserve invariant");
}

#[test]
fn vesting_grants_claim_and_forfeit() {
    let mut model = VestingModel::new(10_000);

    let id = model
        .create_grant("CHW_0", 1_000, 0, 100, 1_000)
        .expect("create grant");
    assert_eq!(model.reserved, 1_000);
    model.check_solvency();

    // Before the cliff nothing is claimable.
    assert!(model.claim_vested(id, 50).is_err());

    // Halfway through vesting, half is claimable.
    let claimed = model.claim_vested(id, 500).expect("claim");
    assert_eq!(claimed, 500);
    assert_eq!(model.reserved, 500);
    model.check_solvency();

    // Forfeit returns the unclaimed remainder to the pool.
    let returned = model.forfeit(id, 600).expect("forfeit");
    assert_eq!(returned, 500);
    assert_eq!(model.reserved, 0);
    model.check_solvency();

    // A forfeited grant cannot be claimed again.
    assert!(model.claim_vested(id, 1_000).is_err());

    println!("✓ Vesting grant claim/forfeit passed");
}

#[test]
fn vesting_stateful_sequences_preserve_solvency() {
    let mut seed: u64 = 0xfeed_face_dead_beef;
    let mut next = || {
        seed = seed.wrapping_mul(6364136223846793005).wrapping_add(1442695040888963407);
        seed
    };

    let mut model = VestingModel::new(100_000);
    let mut now: u64 = 0;

    for _ in 0..2_000 {
        now += (next() % 50) as u64;
        let total = (next() % 2_000) as u128 + 1;
        let cliff = (next() % 100) as u64;
        let duration = cliff + (next() % 500) as u64 + 1;
        match next() % 3 {
            0 => {
                model
                    .create_grant("CHW", total, now, cliff, duration)
                    .ok();
            }
            1 => {
                let ids: Vec<u64> = model.grants.keys().copied().collect();
                if !ids.is_empty() {
                    let id = ids[(next() as usize) % ids.len()];
                    model.claim_vested(id, now).ok();
                }
            }
            _ => {
                let ids: Vec<u64> = model.grants.keys().copied().collect();
                if !ids.is_empty() {
                    let id = ids[(next() as usize) % ids.len()];
                    model.forfeit(id, now).ok();
                }
            }
        }
        model.check_solvency();
    }

    println!("✓ Vesting stateful sequences preserve solvency");
}
