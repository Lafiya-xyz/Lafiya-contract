//! Crash-consistent idempotency and spend-cap state (issue #746).
//!
//! The relayer must never double-spend across a crash or across replicas, so
//! every sponsored submission is recorded as a *reservation* before the
//! transaction is handed to the network, then transitioned to a final state
//! once the outcome is known. Reservations count against the caps from the
//! moment they are written, so a process that dies mid-submit still has its
//! fee accounted for.
//!
//! [`SpendStore`] is the storage contract; [`MemorySpendStore`] is the
//! reference implementation used by tests and single-process deployments.
//! A Postgres implementation is tracked in
//! `docs/issues/746-relayer-spend-store.md`.

use std::collections::HashMap;

/// Lifecycle of one reservation.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ReservationState {
    /// Written before submit; the network may or may not have seen the tx.
    Reserved,
    /// Handed to a provider; outcome unknown until reconciled by hash.
    Submitted,
    /// Final: included in a ledger and successful.
    Accepted { ledger: u32 },
    /// Final: included in a ledger but failed on-chain.
    Rejected { reason: String },
    /// Final: never reached the network (definite pre-send failure).
    Abandoned,
}

impl ReservationState {
    /// Whether the reservation still needs reconciliation by transaction hash.
    pub fn is_open(&self) -> bool {
        matches!(self, ReservationState::Reserved | ReservationState::Submitted)
    }
}

/// One persisted reservation row.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Reservation {
    pub idempotency_key: String,
    pub attester: String,
    pub tx_hash: String,
    pub fee: u64,
    pub at: u64,
    pub state: ReservationState,
}

/// Errors a store may return. Kept transport-agnostic so the Postgres
/// implementation can map driver errors onto the same variants.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum StoreError {
    /// The idempotency key already exists with a different transaction hash.
    KeyReused,
    /// The transaction hash is already recorded under another key.
    HashReused,
    /// The reservation does not exist.
    NotFound,
    /// The store could not be reached or the transaction failed.
    Backend(String),
}

/// Crash-consistent persistence for idempotency keys and spend accounting.
///
/// Implementations must make [`SpendStore::reserve`] atomic: the cap check
/// and the insert happen in one transaction so concurrent replicas sharing a
/// store cannot both pass the check and exceed a cap.
pub trait SpendStore {
    /// Atomically check the caps and record a `Reserved` entry.
    ///
    /// `attester_cap` / `global_cap` are the limits to enforce; `window_secs`
    /// and `quota` bound the per-attester request rate. Returns the stored
    /// reservation on success.
    #[allow(clippy::too_many_arguments)]
    fn reserve(
        &mut self,
        idempotency_key: &str,
        attester: &str,
        tx_hash: &str,
        fee: u64,
        at: u64,
        quota: u32,
        window_secs: u64,
        attester_cap: u64,
        global_cap: u64,
    ) -> Result<Reservation, StoreError>;

    /// Look up a reservation by idempotency key.
    fn get(&self, idempotency_key: &str) -> Option<Reservation>;

    /// Look up a reservation by transaction hash.
    fn get_by_hash(&self, tx_hash: &str) -> Option<Reservation>;

    /// Move a reservation to a new state. Rejects transitions out of a final
    /// state so a late reconciliation cannot rewrite history.
    fn transition(
        &mut self,
        idempotency_key: &str,
        state: ReservationState,
    ) -> Result<Reservation, StoreError>;

    /// All reservations still awaiting reconciliation, oldest first.
    fn open(&self) -> Vec<Reservation>;

    /// Total fee committed (reserved or final) for one attester.
    fn attester_spent(&self, attester: &str) -> u64;

    /// Total fee committed (reserved or final) across the relayer.
    fn total_spent(&self) -> u64;

    /// Number of reservations for `attester` inside the window ending at `now`.
    fn recent_count(&self, attester: &str, now: u64, window_secs: u64) -> u32;
}

/// In-memory [`SpendStore`]. Single-process only: it is crash-consistent
/// within a process (reservations are written before submit) but does not
/// coordinate replicas.
#[derive(Debug, Default)]
pub struct MemorySpendStore {
    rows: Vec<Reservation>,
    by_key: HashMap<String, usize>,
    by_hash: HashMap<String, usize>,
}

impl MemorySpendStore {
    pub fn new() -> Self {
        Self::default()
    }

    /// Number of persisted reservations (used by restart tests).
    pub fn len(&self) -> usize {
        self.rows.len()
    }

    pub fn is_empty(&self) -> bool {
        self.rows.is_empty()
    }
}

impl SpendStore for MemorySpendStore {
    fn reserve(
        &mut self,
        idempotency_key: &str,
        attester: &str,
        tx_hash: &str,
        fee: u64,
        at: u64,
        quota: u32,
        window_secs: u64,
        attester_cap: u64,
        global_cap: u64,
    ) -> Result<Reservation, StoreError> {
        if let Some(idx) = self.by_key.get(idempotency_key) {
            let existing = &self.rows[*idx];
            return if existing.tx_hash == tx_hash {
                Ok(existing.clone())
            } else {
                Err(StoreError::KeyReused)
            };
        }
        if self.by_hash.contains_key(tx_hash) {
            return Err(StoreError::HashReused);
        }
        if self.recent_count(attester, at, window_secs) >= quota {
            return Err(StoreError::Backend("quota".into()));
        }
        if self.attester_spent(attester) + fee > attester_cap {
            return Err(StoreError::Backend("attester cap".into()));
        }
        if self.total_spent() + fee > global_cap {
            return Err(StoreError::Backend("global cap".into()));
        }
        let row = Reservation {
            idempotency_key: idempotency_key.to_string(),
            attester: attester.to_string(),
            tx_hash: tx_hash.to_string(),
            fee,
            at,
            state: ReservationState::Reserved,
        };
        let idx = self.rows.len();
        self.rows.push(row.clone());
        self.by_key.insert(idempotency_key.to_string(), idx);
        self.by_hash.insert(tx_hash.to_string(), idx);
        Ok(row)
    }

    fn get(&self, idempotency_key: &str) -> Option<Reservation> {
        self.by_key.get(idempotency_key).map(|i| self.rows[*i].clone())
    }

    fn get_by_hash(&self, tx_hash: &str) -> Option<Reservation> {
        self.by_hash.get(tx_hash).map(|i| self.rows[*i].clone())
    }

    fn transition(
        &mut self,
        idempotency_key: &str,
        state: ReservationState,
    ) -> Result<Reservation, StoreError> {
        let idx = *self
            .by_key
            .get(idempotency_key)
            .ok_or(StoreError::NotFound)?;
        let row = &mut self.rows[idx];
        if !row.state.is_open() {
            return Err(StoreError::Backend("already final".into()));
        }
        row.state = state;
        Ok(row.clone())
    }

    fn open(&self) -> Vec<Reservation> {
        let mut open: Vec<Reservation> = self
            .rows
            .iter()
            .filter(|r| r.state.is_open())
            .cloned()
            .collect();
        open.sort_by_key(|r| r.at);
        open
    }

    fn attester_spent(&self, attester: &str) -> u64 {
        self.rows
            .iter()
            .filter(|r| r.attester == attester)
            .map(|r| r.fee)
            .sum()
    }

    fn total_spent(&self) -> u64 {
        self.rows.iter().map(|r| r.fee).sum()
    }

    fn recent_count(&self, attester: &str, now: u64, window_secs: u64) -> u32 {
        self.rows
            .iter()
            .filter(|r| r.attester == attester && r.at + window_secs > now)
            .count() as u32
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn reserve(s: &mut MemorySpendStore, key: &str, hash: &str, fee: u64) -> Result<Reservation, StoreError> {
        s.reserve(key, "GCHW", hash, fee, 0, 10, 3600, 1_000, 5_000)
    }

    #[test]
    fn reservation_is_visible_before_submit() {
        let mut s = MemorySpendStore::new();
        reserve(&mut s, "k1", "h1", 100).unwrap();
        assert_eq!(s.total_spent(), 100);
        assert_eq!(s.get("k1").unwrap().state, ReservationState::Reserved);
        assert_eq!(s.open().len(), 1);
    }

    #[test]
    fn same_key_same_hash_is_idempotent() {
        let mut s = MemorySpendStore::new();
        reserve(&mut s, "k1", "h1", 100).unwrap();
        reserve(&mut s, "k1", "h1", 100).unwrap();
        assert_eq!(s.len(), 1);
        assert_eq!(s.total_spent(), 100);
    }

    #[test]
    fn same_key_different_hash_is_rejected() {
        let mut s = MemorySpendStore::new();
        reserve(&mut s, "k1", "h1", 100).unwrap();
        assert_eq!(reserve(&mut s, "k1", "h2", 100), Err(StoreError::KeyReused));
    }

    #[test]
    fn caps_are_enforced_against_reservations() {
        let mut s = MemorySpendStore::new();
        reserve(&mut s, "k1", "h1", 600).unwrap();
        assert!(reserve(&mut s, "k2", "h2", 500).is_err());
        assert_eq!(s.total_spent(), 600);
    }

    #[test]
    fn final_state_cannot_be_rewritten() {
        let mut s = MemorySpendStore::new();
        reserve(&mut s, "k1", "h1", 100).unwrap();
        s.transition("k1", ReservationState::Accepted { ledger: 7 }).unwrap();
        assert!(s.transition("k1", ReservationState::Abandoned).is_err());
        assert!(s.open().is_empty());
    }
}
