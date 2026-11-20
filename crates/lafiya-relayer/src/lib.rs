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
    /// Policy for sponsored incentive-pool `claim` invocations (issue #699).
    pub claim: ClaimPolicy,
}

/// Policy for sponsored `claim` invocations against the incentive pool.
///
/// Claims are keyed separately from attestations: they have their own quota
/// and spend cap so a burst of claims cannot starve the attest flow (and vice
/// versa). Every claim is additionally validated against the configured pool
/// contract and the ownership of the work item being claimed.
#[derive(Debug, Clone)]
pub struct ClaimPolicy {
    /// The only pool contract a sponsored claim may target.
    pub pool_contract: String,
    /// Max sponsored claims per attester per window.
    pub quota_per_window: u32,
    /// Lifetime claim spend cap per attester (stroops).
    pub attester_spend_cap: u64,
    /// Lifetime claim spend cap for the whole relayer (stroops).
    pub global_spend_cap: u64,
    /// Max fee accepted for a single claim transaction (stroops).
    pub max_fee_per_tx: u64,
    /// Optional fee reimbursement deducted from the payout to fund the
    /// sponsor. Only enabled when the ADR approves it; default off.
    pub fee_reimbursement: Option<FeeReimbursement>,
}

/// A small, capped fee reimbursement taken out of a claim payout.
#[derive(Debug, Clone)]
pub struct FeeReimbursement {
    /// Max reimbursement per claim (stroops). The actual reimbursement is
    /// `min(fee, cap)`.
    pub cap: u64,
}

impl FeeReimbursement {
    /// Reimbursement owed to the sponsor for a claim that cost `fee` stroops.
    pub fn amount_for(&self, fee: u64) -> u64 {
        fee.min(self.cap)
    }
}

impl ClaimPolicy {
    /// A disabled claim policy: no pool configured, zero quotas/caps.
    pub fn disabled() -> Self {
        ClaimPolicy {
            pool_contract: String::new(),
            quota_per_window: 0,
            attester_spend_cap: 0,
            global_spend_cap: 0,
            max_fee_per_tx: 0,
            fee_reimbursement: None,
        }
    }
}

/// What a sponsored transaction is asking the relayer to pay for.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Action {
    /// An attestation submission (the original flow).
    Attest,
    /// A `claim` invocation on the incentive pool.
    Claim(ClaimInvocation),
}

/// The validated shape of a `claim` invocation the relayer is asked to sponsor.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ClaimInvocation {
    /// Contract the invocation targets; must equal `ClaimPolicy::pool_contract`.
    pub pool_contract: String,
    /// Work item being claimed.
    pub work_item: String,
    /// Attester that owns the work item; must equal the signer.
    pub work_item_owner: String,
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
    /// What the transaction does. Defaults to `Action::Attest` for the
    /// existing attest flow.
    pub action: Action,
}
=======

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
    /// A claim was submitted but the claim policy is disabled / unconfigured.
    ClaimNotConfigured,
    /// The claim invocation targets a contract other than the configured pool.
    WrongPoolContract,
    /// The claimed work item is not owned by the signing attester.
    WorkItemNotOwned,
    /// The per-attester claim quota for the window is exhausted.
    ClaimQuotaExceeded,
    /// The per-attester lifetime claim spend cap would be exceeded.
    ClaimAttesterSpendCapExceeded,
    /// The relayer-wide lifetime claim spend cap would be exceeded.
    ClaimGlobalSpendCapExceeded,
    /// The claim fee exceeds the claim-specific per-tx fee cap.
    ClaimFeeTooHigh,
}
=======

/// One row of the append-only spend ledger.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LedgerEntry {
    pub seq: u64,
    pub at: u64,
    pub idempotency_key: String,
    pub attester: String,
    pub tx_hash: String,
    pub fee: u64,
    /// What the sponsored transaction did (attest or claim).
    pub action: Action,
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

    /// Total stroops spent on claim transactions only.
    pub fn total_claim_spent(&self) -> u64 {
        self.ledger
            .iter()
            .filter(|e| matches!(e.action, Action::Claim(_)))
            .map(|e| e.fee)
            .sum()
    }
=======

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
        match &req.action {
            Action::Attest => self.check_attest_policy(&req)?,
            Action::Claim(inv) => self.check_claim_policy(&req, inv)?,
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
            action: req.action,
            outcome: outcome.clone(),
        });
        self.seen
            .insert(req.idempotency_key, (req.tx.hash.clone(), idx));
        self.by_hash.insert(req.tx.hash, idx);
        Ok(outcome)
    }

    /// Attest-flow policy: unchanged from the original relayer.
    fn check_attest_policy(&self, req: &SponsorRequest) -> Result<(), RelayError> {
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
        Ok(())
    }

    /// Claim-flow policy: separate quota and spend caps, plus validation that
    /// the invocation targets the configured pool and a work item owned by
    /// the signer.
    fn check_claim_policy(
        &self,
        req: &SponsorRequest,
        inv: &ClaimInvocation,
    ) -> Result<(), RelayError> {
        let cp = &self.cfg.claim;
        if cp.pool_contract.is_empty() {
            return Err(RelayError::ClaimNotConfigured);
        }
        if inv.pool_contract != cp.pool_contract {
            return Err(RelayError::WrongPoolContract);
        }
        if inv.work_item_owner != req.attester {
            return Err(RelayError::WorkItemNotOwned);
        }
        if req.fee > cp.max_fee_per_tx {
            return Err(RelayError::ClaimFeeTooHigh);
        }
        let mine = || {
            self.ledger
                .iter()
                .filter(|e| e.attester == req.attester && matches!(e.action, Action::Claim(_)))
        };
        let recent = mine()
            .filter(|e| e.at + self.cfg.window_secs > req.now)
            .count();
        if recent as u32 >= cp.quota_per_window {
            return Err(RelayError::ClaimQuotaExceeded);
        }
        if mine().map(|e| e.fee).sum::<u64>() + req.fee > cp.attester_spend_cap {
            return Err(RelayError::ClaimAttesterSpendCapExceeded);
        }
        if self.total_claim_spent() + req.fee > cp.global_spend_cap {
            return Err(RelayError::ClaimGlobalSpendCapExceeded);
        }
        Ok(())
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
            claim: ClaimPolicy::disabled(),
        }
    }

    const POOL: &str = "CPOOL";

    fn claim_cfg() -> RelayerConfig {
        RelayerConfig {
            claim: ClaimPolicy {
                pool_contract: POOL.into(),
                quota_per_window: 2,
                attester_spend_cap: 1_000,
                global_spend_cap: 5_000,
                max_fee_per_tx: 500,
                fee_reimbursement: None,
            },
            ..cfg()
        }
    }

    fn claim_req(key: &str, hash: &str, fee: u64, now: u64) -> SponsorRequest {
        SponsorRequest {
            action: Action::Claim(ClaimInvocation {
                pool_contract: POOL.into(),
                work_item: "WI-1".into(),
                work_item_owner: CHW.into(),
            }),
            ..req(key, hash, fee, now)
        }
    }

    fn req(key: &str, hash: &str, fee: u64, now: u64) -> SponsorRequest {
        SponsorRequest {
            idempotency_key: key.into(),
            attester: CHW.into(),
            tx: SignedTx::new(hash, "xdr"),
            fee,
            now,
            action: Action::Attest,
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
    fn sponsored_claim_is_accepted_and_lands_in_trustline() {
        // The CHW holds no XLM: the sponsor pays the claim fee and the USDC
        // payout lands in the CHW's trustline.
        let mut r = Relayer::new(claim_cfg(), accepting(1));
        assert_eq!(
            r.sponsor(claim_req("c1", "hc1", 100, 0)),
            Ok(Outcome::Accepted { ledger: 10 })
        );
        assert_eq!(r.total_claim_spent(), 100);
        assert_eq!(r.total_spent(), 100);
        assert!(matches!(r.ledger()[0].action, Action::Claim(_)));
    }

    #[test]
    fn claim_targeting_wrong_pool_is_rejected() {
        let mut r = Relayer::new(claim_cfg(), accepting(1));
        let mut bad = claim_req("c1", "hc1", 100, 0);
        if let Action::Claim(inv) = &mut bad.action {
            inv.pool_contract = "CEVIL".into();
        }
        assert_eq!(r.sponsor(bad), Err(RelayError::WrongPoolContract));
        assert_eq!(r.provider().submit_calls, 0);
    }

    #[test]
    fn claim_for_unowned_work_item_is_rejected() {
        let mut r = Relayer::new(claim_cfg(), accepting(1));
        let mut bad = claim_req("c1", "hc1", 100, 0);
        if let Action::Claim(inv) = &mut bad.action {
            inv.work_item_owner = "GOTHER".into();
        }
        assert_eq!(r.sponsor(bad), Err(RelayError::WorkItemNotOwned));
        assert_eq!(r.provider().submit_calls, 0);
    }

    #[test]
    fn claim_policy_is_disabled_by_default() {
        let mut r = Relayer::new(cfg(), accepting(1));
        assert_eq!(
            r.sponsor(claim_req("c1", "hc1", 100, 0)),
            Err(RelayError::ClaimNotConfigured)
        );
    }

    #[test]
    fn claim_quota_and_spend_caps_are_separate_from_attest() {
        let mut r = Relayer::new(claim_cfg(), accepting(5));
        // Attest quota is unaffected by claim usage.
        r.sponsor(req("a1", "ha1", 400, 0)).unwrap();
        r.sponsor(req("a2", "ha2", 400, 1)).unwrap();
        assert_eq!(
            r.sponsor(req("a3", "ha3", 100, 2)),
            Err(RelayError::QuotaExceeded)
        );
        // Claims have their own quota.
        r.sponsor(claim_req("c1", "hc1", 400, 0)).unwrap();
        r.sponsor(claim_req("c2", "hc2", 400, 1)).unwrap();
        assert_eq!(
            r.sponsor(claim_req("c3", "hc3", 100, 2)),
            Err(RelayError::ClaimQuotaExceeded)
        );
        // And their own lifetime cap.
        assert_eq!(
            r.sponsor(claim_req("c4", "hc4", 300, 4000)),
            Err(RelayError::ClaimAttesterSpendCapExceeded)
        );
        assert_eq!(
            r.sponsor(claim_req("c5", "hc5", 501, 4000)),
            Err(RelayError::ClaimFeeTooHigh)
        );
    }

    #[test]
    fn fee_reimbursement_is_capped() {
        let reimb = FeeReimbursement { cap: 50 };
        assert_eq!(reimb.amount_for(20), 20);
        assert_eq!(reimb.amount_for(50), 50);
        assert_eq!(reimb.amount_for(500), 50);
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
