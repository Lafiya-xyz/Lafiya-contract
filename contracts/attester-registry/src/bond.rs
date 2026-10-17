//! Optional attester bonds with quorum-gated slashing (issue #587).
//!
//! Bonds are held in escrow by this contract, accounted separately from any
//! pool treasury. A slash needs a quorum of distinct registrar/guardian
//! approvals, then survives a challenge window in which the attester may
//! appeal to the admin before anyone can execute it.
//! See `docs/adr/0018-attester-bond-and-slashing.md`.

use soroban_sdk::{contractevent, contractimpl, contracttype, token, Address, BytesN, Env, Vec};

use crate::{
    probation::require_registrar_or_guardian, AttesterRegistry, AttesterRegistryArgs,
    AttesterRegistryClient, DataKey, Error, StoredAttesterInfo, INSTANCE_BUMP_AMOUNT,
    INSTANCE_LIFETIME_THRESHOLD,
};

/// Maximum concurrently open slash cases per attester, bounding rotation work.
pub const MAX_OPEN_CASES: u32 = 5;

/// Admin-configured bond policy.
#[contracttype]
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct BondConfig {
    /// Token the bond is denominated in.
    pub token: Address,
    /// Bond an attester must hold to count as bonded.
    pub min_bond: i128,
    /// Seconds between quorum being reached and a slash becoming executable.
    pub challenge_window: u64,
    /// Distinct approvals (including the proposer) needed to start the window.
    pub quorum: u32,
    /// Cooldown between `request_bond_withdrawal` and `withdraw_bond`. Must
    /// exceed `challenge_window` so a slash can finish first.
    pub unbond_delay: u64,
    /// Where slashed funds are sent (remediation fund or pool).
    pub recipient: Address,
}

#[contracttype]
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct BondInfo {
    pub amount: i128,
    /// Portion reserved by open slash cases.
    pub locked: i128,
    pub withdraw_at: Option<u64>,
    pub open_cases: u32,
}

#[contracttype]
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum SlashStatus {
    /// Collecting approvals.
    Pending,
    /// Quorum reached; the challenge window is running.
    Challengeable,
    /// The attester appealed; awaiting the admin's ruling.
    Appealed,
    /// The admin rejected the appeal; executable immediately.
    Upheld,
    Executed,
    Cancelled,
}

#[contracttype]
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct SlashCase {
    pub attester: Address,
    pub amount: i128,
    pub evidence_hash: BytesN<32>,
    pub approvals: Vec<Address>,
    pub status: SlashStatus,
    pub window_ends_at: u64,
}

#[contractevent]
#[derive(Clone, Debug)]
pub struct BondConfigured {
    pub min_bond: i128,
    pub challenge_window: u64,
    pub quorum: u32,
    pub unbond_delay: u64,
}

#[contractevent]
#[derive(Clone, Debug)]
pub struct BondPosted {
    #[topic]
    pub attester: Address,
    pub amount: i128,
    pub total: i128,
}

#[contractevent]
#[derive(Clone, Debug)]
pub struct BondWithdrawalRequested {
    #[topic]
    pub attester: Address,
    pub withdraw_at: u64,
}

#[contractevent]
#[derive(Clone, Debug)]
pub struct BondWithdrawn {
    #[topic]
    pub attester: Address,
    pub amount: i128,
}

#[contractevent]
#[derive(Clone, Debug)]
pub struct SlashProposed {
    #[topic]
    pub case_id: u64,
    #[topic]
    pub attester: Address,
    pub amount: i128,
    pub evidence_hash: BytesN<32>,
}

#[contractevent]
#[derive(Clone, Debug)]
pub struct SlashApproved {
    #[topic]
    pub case_id: u64,
    pub approver: Address,
    /// Set when this approval reached quorum and opened the challenge window.
    pub window_ends_at: Option<u64>,
}

#[contractevent]
#[derive(Clone, Debug)]
pub struct SlashAppealed {
    #[topic]
    pub case_id: u64,
    #[topic]
    pub attester: Address,
}

#[contractevent]
#[derive(Clone, Debug)]
pub struct SlashAppealResolved {
    #[topic]
    pub case_id: u64,
    pub upheld: bool,
}

#[contractevent]
#[derive(Clone, Debug)]
pub struct SlashExecuted {
    #[topic]
    pub case_id: u64,
    #[topic]
    pub attester: Address,
    pub amount: i128,
}

#[contractevent]
#[derive(Clone, Debug)]
pub struct SlashCancelled {
    #[topic]
    pub case_id: u64,
}

fn config(env: &Env) -> Result<BondConfig, Error> {
    env.storage()
        .instance()
        .get(&DataKey::BondConfig)
        .ok_or(Error::BondNotConfigured)
}

fn bond(env: &Env, attester: &Address) -> Option<BondInfo> {
    env.storage()
        .persistent()
        .get(&DataKey::Bond(attester.clone()))
}

fn save_bond(env: &Env, attester: &Address, info: &BondInfo) {
    let key = DataKey::Bond(attester.clone());
    env.storage().persistent().set(&key, info);
    env.storage()
        .persistent()
        .extend_ttl(&key, INSTANCE_LIFETIME_THRESHOLD, INSTANCE_BUMP_AMOUNT);
}

fn case(env: &Env, case_id: u64) -> Result<SlashCase, Error> {
    env.storage()
        .persistent()
        .get(&DataKey::SlashCase(case_id))
        .ok_or(Error::SlashCaseNotFound)
}

fn save_case(env: &Env, case_id: u64, value: &SlashCase) {
    let key = DataKey::SlashCase(case_id);
    env.storage().persistent().set(&key, value);
    env.storage()
        .persistent()
        .extend_ttl(&key, INSTANCE_LIFETIME_THRESHOLD, INSTANCE_BUMP_AMOUNT);
}

fn open_cases(env: &Env, attester: &Address) -> Vec<u64> {
    env.storage()
        .persistent()
        .get(&DataKey::AttesterCases(attester.clone()))
        .unwrap_or_else(|| Vec::new(env))
}

fn save_open_cases(env: &Env, attester: &Address, ids: &Vec<u64>) {
    let key = DataKey::AttesterCases(attester.clone());
    if ids.is_empty() {
        env.storage().persistent().remove(&key);
    } else {
        env.storage().persistent().set(&key, ids);
        env.storage().persistent().extend_ttl(
            &key,
            INSTANCE_LIFETIME_THRESHOLD,
            INSTANCE_BUMP_AMOUNT,
        );
    }
}

/// Release the case's lock on the attester's bond and drop it from the open list.
fn close_case(env: &Env, case_id: u64, value: &SlashCase, slashed: bool) {
    let mut ids = open_cases(env, &value.attester);
    if let Some(index) = ids.first_index_of(case_id) {
        ids.remove(index);
    }
    save_open_cases(env, &value.attester, &ids);
    if let Some(mut info) = bond(env, &value.attester) {
        info.locked -= value.amount;
        info.open_cases = info.open_cases.saturating_sub(1);
        if slashed {
            info.amount -= value.amount;
        }
        save_bond(env, &value.attester, &info);
    }
}

/// Move a bond and its open slash cases to the successor key on rotation,
/// so a rotation can neither shed a pending slash nor strand the bond.
pub(crate) fn rotate(env: &Env, previous: &Address, new: &Address) {
    let Some(old) = bond(env, previous) else {
        return;
    };
    let mut merged = bond(env, new).unwrap_or(BondInfo {
        amount: 0,
        locked: 0,
        withdraw_at: None,
        open_cases: 0,
    });
    merged.amount += old.amount;
    merged.locked += old.locked;
    merged.open_cases += old.open_cases;
    merged.withdraw_at = old.withdraw_at.or(merged.withdraw_at);
    save_bond(env, new, &merged);
    env.storage()
        .persistent()
        .remove(&DataKey::Bond(previous.clone()));

    let mut ids = open_cases(env, new);
    for id in open_cases(env, previous).iter() {
        if let Ok(mut moved) = case(env, id) {
            moved.attester = new.clone();
            save_case(env, id, &moved);
        }
        ids.push_back(id);
    }
    save_open_cases(env, new, &ids);
    save_open_cases(env, previous, &Vec::new(env));
}

#[contractimpl]
impl AttesterRegistry {
    /// Configure the bond policy. Requires the admin.
    pub fn set_bond_config(
        env: Env,
        token: Address,
        min_bond: i128,
        challenge_window: u64,
        quorum: u32,
        unbond_delay: u64,
        recipient: Address,
    ) -> Result<(), Error> {
        Self::admin(&env)?.require_auth();
        if min_bond <= 0 || quorum == 0 || unbond_delay <= challenge_window {
            return Err(Error::InvalidBondConfig);
        }
        env.storage().instance().set(
            &DataKey::BondConfig,
            &BondConfig {
                token,
                min_bond,
                challenge_window,
                quorum,
                unbond_delay,
                recipient,
            },
        );
        env.storage()
            .instance()
            .extend_ttl(INSTANCE_LIFETIME_THRESHOLD, INSTANCE_BUMP_AMOUNT);
        BondConfigured {
            min_bond,
            challenge_window,
            quorum,
            unbond_delay,
        }
        .publish(&env);
        Ok(())
    }

    pub fn get_bond_config(env: Env) -> Option<BondConfig> {
        env.storage().instance().get(&DataKey::BondConfig)
    }

    pub fn get_bond(env: Env, attester: Address) -> Option<BondInfo> {
        bond(&env, &attester)
    }

    /// Whether `attester`'s unlocked bond meets the configured minimum.
    pub fn is_bonded(env: Env, attester: Address) -> bool {
        let (Ok(cfg), Some(info)) = (config(&env), bond(&env, &attester)) else {
            return false;
        };
        info.amount - info.locked >= cfg.min_bond
    }

    pub fn get_slash_case(env: Env, case_id: u64) -> Option<SlashCase> {
        case(&env, case_id).ok()
    }

    /// Deposit `amount` into the caller's bond. The caller must be an
    /// enrolled attester. Topping up cancels a pending withdrawal request.
    pub fn post_bond(env: Env, attester: Address, amount: i128) -> Result<(), Error> {
        attester.require_auth();
        Self::require_not_paused(&env)?;
        let cfg = config(&env)?;
        if amount <= 0 {
            return Err(Error::InvalidBondAmount);
        }
        env.storage()
            .persistent()
            .get::<_, StoredAttesterInfo>(&DataKey::Attester(attester.clone()))
            .filter(|info| !info.removed)
            .ok_or(Error::AttesterNotFound)?;
        token::Client::new(&env, &cfg.token).transfer(
            &attester,
            env.current_contract_address(),
            &amount,
        );
        let mut info = bond(&env, &attester).unwrap_or(BondInfo {
            amount: 0,
            locked: 0,
            withdraw_at: None,
            open_cases: 0,
        });
        info.amount += amount;
        info.withdraw_at = None;
        save_bond(&env, &attester, &info);
        BondPosted {
            attester,
            amount,
            total: info.amount,
        }
        .publish(&env);
        Ok(())
    }

    /// Start the unbonding cooldown. The bond stays slashable until withdrawn.
    pub fn request_bond_withdrawal(env: Env, attester: Address) -> Result<(), Error> {
        attester.require_auth();
        let cfg = config(&env)?;
        let mut info = bond(&env, &attester).ok_or(Error::BondNotFound)?;
        let withdraw_at = env.ledger().timestamp().saturating_add(cfg.unbond_delay);
        info.withdraw_at = Some(withdraw_at);
        save_bond(&env, &attester, &info);
        BondWithdrawalRequested {
            attester,
            withdraw_at,
        }
        .publish(&env);
        Ok(())
    }

    /// Withdraw the whole bond once the cooldown has elapsed and no slash
    /// case is open.
    pub fn withdraw_bond(env: Env, attester: Address) -> Result<(), Error> {
        attester.require_auth();
        let cfg = config(&env)?;
        let info = bond(&env, &attester).ok_or(Error::BondNotFound)?;
        match info.withdraw_at {
            Some(at) if env.ledger().timestamp() >= at => {}
            _ => return Err(Error::BondWithdrawalNotReady),
        }
        if info.open_cases > 0 {
            return Err(Error::SlashCasesOpen);
        }
        env.storage()
            .persistent()
            .remove(&DataKey::Bond(attester.clone()));
        token::Client::new(&env, &cfg.token).transfer(
            &env.current_contract_address(),
            &attester,
            &info.amount,
        );
        BondWithdrawn {
            attester,
            amount: info.amount,
        }
        .publish(&env);
        Ok(())
    }

    /// Open a slash case against `attester`'s bond and lock `amount`. The
    /// proposer (registrar or guardian) counts as the first approval. Returns
    /// the case id.
    pub fn propose_slash(
        env: Env,
        proposer: Address,
        attester: Address,
        amount: i128,
        evidence_hash: BytesN<32>,
    ) -> Result<u64, Error> {
        require_registrar_or_guardian(&env, &proposer)?;
        Self::require_not_paused(&env)?;
        let cfg = config(&env)?;
        let mut info = bond(&env, &attester).ok_or(Error::BondNotFound)?;
        if amount <= 0 {
            return Err(Error::InvalidBondAmount);
        }
        if info.amount - info.locked < amount {
            return Err(Error::InsufficientBond);
        }
        if info.open_cases >= MAX_OPEN_CASES {
            return Err(Error::TooManyOpenCases);
        }
        let case_id: u64 = env
            .storage()
            .instance()
            .get(&DataKey::SlashCaseCount)
            .unwrap_or(0)
            + 1;
        env.storage()
            .instance()
            .set(&DataKey::SlashCaseCount, &case_id);
        env.storage()
            .instance()
            .extend_ttl(INSTANCE_LIFETIME_THRESHOLD, INSTANCE_BUMP_AMOUNT);

        info.locked += amount;
        info.open_cases += 1;
        save_bond(&env, &attester, &info);
        let mut ids = open_cases(&env, &attester);
        ids.push_back(case_id);
        save_open_cases(&env, &attester, &ids);

        let mut approvals = Vec::new(&env);
        approvals.push_back(proposer.clone());
        let mut value = SlashCase {
            attester: attester.clone(),
            amount,
            evidence_hash: evidence_hash.clone(),
            approvals,
            status: SlashStatus::Pending,
            window_ends_at: 0,
        };
        let window_ends_at = (cfg.quorum <= 1).then(|| {
            value.status = SlashStatus::Challengeable;
            value.window_ends_at = env
                .ledger()
                .timestamp()
                .saturating_add(cfg.challenge_window);
            value.window_ends_at
        });
        save_case(&env, case_id, &value);
        SlashProposed {
            case_id,
            attester,
            amount,
            evidence_hash,
        }
        .publish(&env);
        SlashApproved {
            case_id,
            approver: proposer,
            window_ends_at,
        }
        .publish(&env);
        Ok(case_id)
    }

    /// Add a distinct registrar/guardian approval. The approval that reaches
    /// quorum starts the challenge window.
    pub fn approve_slash(env: Env, approver: Address, case_id: u64) -> Result<(), Error> {
        require_registrar_or_guardian(&env, &approver)?;
        let cfg = config(&env)?;
        let mut value = case(&env, case_id)?;
        if value.status != SlashStatus::Pending {
            return Err(Error::SlashCaseWrongState);
        }
        if value.approvals.contains(&approver) {
            return Err(Error::AlreadyApproved);
        }
        value.approvals.push_back(approver.clone());
        let window_ends_at = (value.approvals.len() >= cfg.quorum).then(|| {
            value.status = SlashStatus::Challengeable;
            value.window_ends_at = env
                .ledger()
                .timestamp()
                .saturating_add(cfg.challenge_window);
            value.window_ends_at
        });
        save_case(&env, case_id, &value);
        SlashApproved {
            case_id,
            approver,
            window_ends_at,
        }
        .publish(&env);
        Ok(())
    }

    /// The accused attester appeals during the challenge window, freezing
    /// the case until the admin rules.
    pub fn appeal_slash(env: Env, case_id: u64) -> Result<(), Error> {
        let mut value = case(&env, case_id)?;
        value.attester.require_auth();
        if value.status != SlashStatus::Challengeable {
            return Err(Error::SlashCaseWrongState);
        }
        if env.ledger().timestamp() >= value.window_ends_at {
            return Err(Error::ChallengeWindowClosed);
        }
        value.status = SlashStatus::Appealed;
        save_case(&env, case_id, &value);
        SlashAppealed {
            case_id,
            attester: value.attester,
        }
        .publish(&env);
        Ok(())
    }

    /// Rule on an appeal. Requires the admin. Upholding the slash makes it
    /// executable immediately; rejecting it cancels the case and unlocks the
    /// bond.
    pub fn resolve_appeal(env: Env, case_id: u64, uphold: bool) -> Result<(), Error> {
        Self::admin(&env)?.require_auth();
        let mut value = case(&env, case_id)?;
        if value.status != SlashStatus::Appealed {
            return Err(Error::SlashCaseWrongState);
        }
        if uphold {
            value.status = SlashStatus::Upheld;
            save_case(&env, case_id, &value);
        } else {
            value.status = SlashStatus::Cancelled;
            save_case(&env, case_id, &value);
            close_case(&env, case_id, &value, false);
        }
        SlashAppealResolved {
            case_id,
            upheld: uphold,
        }
        .publish(&env);
        Ok(())
    }

    /// Execute a slash after the challenge window closes without an appeal,
    /// or after an appeal was rejected. Callable by anyone.
    pub fn execute_slash(env: Env, case_id: u64) -> Result<(), Error> {
        let cfg = config(&env)?;
        let mut value = case(&env, case_id)?;
        match value.status {
            SlashStatus::Upheld => {}
            SlashStatus::Challengeable => {
                if env.ledger().timestamp() < value.window_ends_at {
                    return Err(Error::ChallengeWindowOpen);
                }
            }
            _ => return Err(Error::SlashCaseWrongState),
        }
        value.status = SlashStatus::Executed;
        save_case(&env, case_id, &value);
        close_case(&env, case_id, &value, true);
        token::Client::new(&env, &cfg.token).transfer(
            &env.current_contract_address(),
            &cfg.recipient,
            &value.amount,
        );
        SlashExecuted {
            case_id,
            attester: value.attester,
            amount: value.amount,
        }
        .publish(&env);
        Ok(())
    }

    /// Cancel an unexecuted slash case and unlock its amount. Requires the admin.
    pub fn cancel_slash(env: Env, case_id: u64) -> Result<(), Error> {
        Self::admin(&env)?.require_auth();
        let mut value = case(&env, case_id)?;
        if matches!(value.status, SlashStatus::Executed | SlashStatus::Cancelled) {
            return Err(Error::SlashCaseWrongState);
        }
        value.status = SlashStatus::Cancelled;
        save_case(&env, case_id, &value);
        close_case(&env, case_id, &value, false);
        SlashCancelled { case_id }.publish(&env);
        Ok(())
    }
}
