//! Reference relayer core for sponsored attestations (issue #611).
//!
//! Implements the flow in `docs/relayer-sponsored-attestations.md`: the
//! attester signs only a Soroban authorization entry, the relayer places it in
//! a transaction whose source is the funded sponsor account, signs, and
//! submits. This crate is the policy layer around that submission -- who may
//! be sponsored, how much, and exactly-once handling -- independent of
//! transport. See `docs/security/relayer-threat-model.md` and
//! `docs/runbooks/relayer-operations.md`.

use lafiya_rpc_resilience::{RpcProvider, SignedTx, SubmitOutcome, TxState};
use std::collections::{HashMap, HashSet};

#[derive(Debug, Clone)]
pub struct RelayerConfig {
    /// Only these (allowlisted) attester addresses are ever sponsored.
    pub allowed_attesters: HashSet<String>,
    /// Max sponsored requests per attester per window.
    pub quota_per_window: u32,
    pub window_secs: u64,
    /// Lifetime spend cap per attester (stroops).
    pub attester_spend_cap: u64,
    /// Lifetime spend cap for the whole relayer (stroops).
    pub global_spend_cap: u64,
    /// Max fee accepted for a single transaction (stroops).
    pub max_fee_per_tx: u64,
    /// Status polls after submission before reporting `Pending`.
    pub max_polls: u32,
}

#[derive(Debug, Clone)]
pub struct SponsorRequest {
    /// Client-chosen key; retrying with the same key never double-spends.
    pub idempotency_key: String,
    pub attester: String,
    /// Sponsor-sourced transaction carrying the attester's signed auth entry.
    pub tx: SignedTx,
    /// Fee (stroops) the sponsor will pay for this transaction.
    pub fee: u64,
    /// Caller-supplied unix seconds (injected for deterministic tests).
    pub now: u64,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Outcome {
    Accepted {
        ledger: u32,
    },
    Rejected {
        reason: String,
    },
    /// Submitted but not yet final; poll the hash, never resubmit.
    Pending,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum RelayError {
    NotAllowlisted,
    QuotaExceeded,
    AttesterSpendCapExceeded,
    GlobalSpendCapExceeded,
    FeeTooHigh,
    /// The idempotency key was already used for a different transaction.
    KeyReused,
    /// The provider refused the submission before it reached the network.
    SubmitFailed(String),
}

/// One row of the append-only spend ledger.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LedgerEntry {
    pub seq: u64,
    pub at: u64,
    pub idempotency_key: String,
    pub attester: String,
    pub tx_hash: String,
    pub fee: u64,
    pub outcome: Outcome,
}

pub struct Relayer<P: RpcProvider> {
    cfg: RelayerConfig,
    provider: P,
    ledger: Vec<LedgerEntry>,
    /// idempotency key -> (tx hash, ledger index)
    seen: HashMap<String, (String, usize)>,
    /// tx hash -> ledger index; blocks replay of one envelope under new keys.
    by_hash: HashMap<String, usize>,
}

impl<P: RpcProvider> Relayer<P> {
    pub fn new(cfg: RelayerConfig, provider: P) -> Self {
        Relayer {
            cfg,
            provider,
            ledger: Vec::new(),
            seen: HashMap::new(),
            by_hash: HashMap::new(),
        }
    }

    /// The auditable record of every sponsored submission, in order.
    pub fn ledger(&self) -> &[LedgerEntry] {
        &self.ledger
    }

    pub fn provider(&self) -> &P {
        &self.provider
    }

    pub fn total_spent(&self) -> u64 {
        self.ledger.iter().map(|e| e.fee).sum()
    }

    pub fn sponsor(&mut self, req: SponsorRequest) -> Result<Outcome, RelayError> {
        // Idempotency / replay: a repeat returns the recorded result and
        // never submits or charges again.
        if let Some((hash, idx)) = self.seen.get(&req.idempotency_key) {
            return if *hash == req.tx.hash {
                Ok(self.ledger[*idx].outcome.clone())
            } else {
                Err(RelayError::KeyReused)
            };
        }
        if let Some(idx) = self.by_hash.get(&req.tx.hash) {
            return Ok(self.ledger[*idx].outcome.clone());
        }
        if !self.cfg.allowed_attesters.contains(&req.attester) {
            return Err(RelayError::NotAllowlisted);
        }
        if req.fee > self.cfg.max_fee_per_tx {
            return Err(RelayError::FeeTooHigh);
        }
        let mine = || self.ledger.iter().filter(|e| e.attester == req.attester);
        let recent = mine()
            .filter(|e| e.at + self.cfg.window_secs > req.now)
            .count();
        if recent as u32 >= self.cfg.quota_per_window {
            return Err(RelayError::QuotaExceeded);
        }
        if mine().map(|e| e.fee).sum::<u64>() + req.fee > self.cfg.attester_spend_cap {
            return Err(RelayError::AttesterSpendCapExceeded);
        }
        if self.total_spent() + req.fee > self.cfg.global_spend_cap {
            return Err(RelayError::GlobalSpendCapExceeded);
        }

        let outcome = match self.provider.submit(&req.tx) {
            SubmitOutcome::Definite(e) => return Err(RelayError::SubmitFailed(e.to_string())),
            SubmitOutcome::Ack(TxState::Rejected { reason }) => Outcome::Rejected { reason },
            SubmitOutcome::Ack(TxState::Accepted { ledger }) => Outcome::Accepted { ledger },
            // Ambiguous or pending: resolve by hash, never by resubmitting.
            _ => self.poll(&req.tx.hash),
        };
        // The fee is reserved from the first moment the network may have seen
        // the transaction, so a lost response cannot be used to drain funds.
        let idx = self.ledger.len();
        self.ledger.push(LedgerEntry {
            seq: idx as u64,
            at: req.now,
            idempotency_key: req.idempotency_key.clone(),
            attester: req.attester,
            tx_hash: req.tx.hash.clone(),
            fee: req.fee,
            outcome: outcome.clone(),
        });
        self.seen
            .insert(req.idempotency_key, (req.tx.hash.clone(), idx));
        self.by_hash.insert(req.tx.hash, idx);
        Ok(outcome)
    }

    fn poll(&mut self, hash: &str) -> Outcome {
        for _ in 0..self.cfg.max_polls {
            match self.provider.get_transaction(hash) {
                Ok(TxState::Accepted { ledger }) => return Outcome::Accepted { ledger },
                Ok(TxState::Rejected { reason }) => return Outcome::Rejected { reason },
                _ => {}
            }
        }
        Outcome::Pending
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use lafiya_rpc_resilience::mock::ScriptedProvider;

    const CHW: &str = "GCHW";

    fn cfg() -> RelayerConfig {
        RelayerConfig {
            allowed_attesters: [CHW.to_string()].into(),
            quota_per_window: 2,
            window_secs: 3600,
            attester_spend_cap: 1_000,
            global_spend_cap: 5_000,
            max_fee_per_tx: 500,
            max_polls: 3,
        }
    }

    fn req(key: &str, hash: &str, fee: u64, now: u64) -> SponsorRequest {
        SponsorRequest {
            idempotency_key: key.into(),
            attester: CHW.into(),
            tx: SignedTx::new(hash, "xdr"),
            fee,
            now,
        }
    }

    fn accepting(n: usize) -> ScriptedProvider {
        (0..n).fold(ScriptedProvider::new("sandbox"), |p, i| {
            p.then_submit(SubmitOutcome::Ack(TxState::Accepted {
                ledger: 10 + i as u32,
            }))
        })
    }

    #[test]
    fn unfunded_attester_completes_attestation_via_sponsor() {
        // The attester holds no XLM: only a sponsor-sourced envelope is ever
        // submitted, and the relayer pays the fee.
        let mut r = Relayer::new(cfg(), accepting(1));
        assert_eq!(
            r.sponsor(req("k1", "h1", 100, 0)),
            Ok(Outcome::Accepted { ledger: 10 })
        );
        assert_eq!(r.total_spent(), 100);
        assert_eq!(r.ledger()[0].attester, CHW);
    }

    #[test]
    fn idempotent_retry_does_not_resubmit_or_recharge() {
        let mut r = Relayer::new(cfg(), accepting(1));
        r.sponsor(req("k1", "h1", 100, 0)).unwrap();
        assert_eq!(
            r.sponsor(req("k1", "h1", 100, 1)),
            Ok(Outcome::Accepted { ledger: 10 })
        );
        assert_eq!(r.provider().submit_calls, 1);
        assert_eq!(r.total_spent(), 100);
        assert_eq!(
            r.sponsor(req("k1", "other", 100, 2)),
            Err(RelayError::KeyReused)
        );
    }

    #[test]
    fn replayed_envelope_under_new_key_is_not_resubmitted() {
        let mut r = Relayer::new(cfg(), accepting(2));
        r.sponsor(req("k1", "h1", 100, 0)).unwrap();
        r.sponsor(req("k2", "h1", 100, 1)).unwrap();
        assert_eq!(r.provider().submit_calls, 1);
        assert_eq!(r.ledger().len(), 1);
    }

    #[test]
    fn fee_drain_controls() {
        let mut r = Relayer::new(cfg(), accepting(5));
        let mut stranger = req("s", "hs", 100, 0);
        stranger.attester = "GOTHER".into();
        assert_eq!(r.sponsor(stranger), Err(RelayError::NotAllowlisted));
        assert_eq!(
            r.sponsor(req("a", "ha", 501, 0)),
            Err(RelayError::FeeTooHigh)
        );
        r.sponsor(req("b", "hb", 400, 0)).unwrap();
        r.sponsor(req("c", "hc", 400, 1)).unwrap();
        assert_eq!(
            r.sponsor(req("d", "hd", 100, 2)),
            Err(RelayError::QuotaExceeded)
        );
        // after the window the quota resets, but the lifetime cap still binds
        assert_eq!(
            r.sponsor(req("e", "he", 300, 4000)),
            Err(RelayError::AttesterSpendCapExceeded)
        );
        assert_eq!(r.provider().submit_calls, 2);
    }

    #[test]
    fn ambiguous_submit_is_resolved_by_polling_not_resubmission() {
        let p = ScriptedProvider::new("flaky")
            .then_submit(SubmitOutcome::Ambiguous(
                lafiya_rpc_resilience::RpcError::Timeout,
            ))
            .then_poll(Ok(TxState::Accepted { ledger: 42 }));
        let mut r = Relayer::new(cfg(), p);
        assert_eq!(
            r.sponsor(req("k", "h", 100, 0)),
            Ok(Outcome::Accepted { ledger: 42 })
        );
        assert_eq!(r.provider().submit_calls, 1);
    }
}
