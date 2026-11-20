//! Soroban contract modelling licensed health facilities and the attesters
//! affiliated with them (issue #592, ADR-0019).
//!
//! The admin enrolls facilities and governs attester affiliations.
//! `attestation-registry` consults `is_affiliated` when an attestation names a
//! facility; verifiers use `was_affiliated_at` and `get_facility` to judge an
//! existing attestation later. Enumeration follows ADR-0017: no on-chain
//! directory, only a monotonic mutation cursor plus events.
#![no_std]
#![deny(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use lafiya_contract_common::time::{Ledgers, Seconds};
use soroban_sdk::{
    contract, contracterror, contractevent, contractimpl, contracttype, Address, BytesN, Env,
    String, Vec,
};

const SCHEMA_VERSION: u32 = 1;

/// Declared ledger close-time assumption used for every seconds/ledgers
/// conversion in this contract. Soroban targets ~5s closes; we assume 5s and
/// apply a 2x safety factor so a protocol upgrade that halves the close time
/// still leaves the chosen TTL covering the validity window.
const ASSUMED_CLOSE_TIME: Seconds = Seconds::new(5);

/// Instance storage TTL policy, matching the other registries. Expressed in
/// seconds and converted to ledgers through the declared close-time
/// assumption so the policy survives close-time changes.
const INSTANCE_BUMP_SECONDS: Seconds = Seconds::new(90 * 24 * 60 * 60);
const INSTANCE_LIFETIME_SECONDS: Seconds = Seconds::new(30 * 24 * 60 * 60);

/// Ledger forms of the TTL policy, derived from the seconds constants above.
const INSTANCE_BUMP_AMOUNT: Ledgers = INSTANCE_BUMP_SECONDS.to_ledgers(ASSUMED_CLOSE_TIME);
const INSTANCE_LIFETIME_THRESHOLD: Ledgers =
    INSTANCE_LIFETIME_SECONDS.to_ledgers(ASSUMED_CLOSE_TIME);

/// Minimum number of ledgers that must have elapsed since the last `touch()`
/// before another `touch()` extends the TTLs. Caps griefing: a caller cannot
/// force repeated extensions (which they pay for) more often than this.
const TOUCH_MIN_INTERVAL_LEDGERS: Ledgers = Ledgers::new(1_000);
=======

/// Upper bound on enrolled facilities.
pub const MAX_FACILITIES: u64 = 10_000;
/// Upper bound on currently affiliated attesters per facility, bounding the
/// per-facility counter's state and the cost of an audit sweep.
pub const MAX_AFFILIATES_PER_FACILITY: u32 = 500;
/// Upper bound on retained affiliation periods per (facility, attester).
pub const MAX_PERIODS: u32 = 8;

#[contracttype]
#[derive(Clone)]
enum DataKey {
    Admin,
    SchemaVersion,
    /// Highest facility id allocated (instance storage).
    FacilityCount,
    /// Monotonic mutation cursor (instance storage), per ADR-0017.
    MutationSeq,
    Facility(u64),
    /// Affiliation periods for a (facility, attester) pair, oldest first.
    Affiliation(u64, Address),
    /// Number of attesters currently affiliated with a facility.
    AffiliateCount(u64),
    /// Ledger sequence of the last successful `touch()` (instance storage).
    LastTouch,
}

/// A licensed facility.
#[contracttype]
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct FacilityInfo {
    /// Hash of the facility's off-chain license document.
    pub license_hash: BytesN<32>,
    /// ISO 3166-2 region the facility operates in, if recorded.
    pub region: Option<String>,
    pub enrolled_at: u64,
    /// A deactivated facility (for example a revoked license) has no valid
    /// affiliations.
    pub active: bool,
    pub deactivated_at: Option<u64>,
}

/// One continuous period of affiliation. `until` is exclusive.
#[contracttype]
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct AffiliationPeriod {
    pub since: u64,
    pub until: Option<u64>,
}

#[contracterror]
#[derive(Copy, Clone, Debug, Eq, PartialEq, PartialOrd, Ord)]
#[repr(u32)]
pub enum Error {
    /// The contract has no admin configured.
    NotInitialized = 1,
    /// No facility exists with this id.
    FacilityNotFound = 2,
    /// The facility is deactivated.
    FacilityInactive = 3,
    /// The facility is already in the requested activation state.
    FacilityStateUnchanged = 4,
    /// The facility cap `MAX_FACILITIES` is reached.
    FacilityLimitReached = 5,
    /// The attester is already affiliated with the facility.
    AlreadyAffiliated = 6,
    /// The attester is not currently affiliated with the facility.
    NotAffiliated = 7,
    /// The facility already has `MAX_AFFILIATES_PER_FACILITY` attesters.
    AffiliateLimitReached = 8,
    /// The pair already has `MAX_PERIODS` retained affiliation periods.
    PeriodLimitReached = 9,
}

#[contractevent]
#[derive(Clone, Debug)]
pub struct FacilityEnrolled {
    #[topic]
    pub facility_id: u64,
    pub license_hash: BytesN<32>,
    pub mutation_seq: u64,
}

#[contractevent]
#[derive(Clone, Debug)]
pub struct FacilityDeactivated {
    #[topic]
    pub facility_id: u64,
    pub mutation_seq: u64,
}

#[contractevent]
#[derive(Clone, Debug)]
pub struct FacilityReactivated {
    #[topic]
    pub facility_id: u64,
    pub mutation_seq: u64,
}

#[contractevent]
#[derive(Clone, Debug)]
pub struct AttesterAffiliated {
    #[topic]
    pub facility_id: u64,
    #[topic]
    pub attester: Address,
    pub mutation_seq: u64,
}

#[contractevent]
#[derive(Clone, Debug)]
pub struct AttesterDisaffiliated {
    #[topic]
    pub facility_id: u64,
    #[topic]
    pub attester: Address,
    pub mutation_seq: u64,
}

soroban_sdk::contractmeta!(
    key = "source_repo",
    val = "github:Lafiya-xyz/Lafiya-contract"
);
soroban_sdk::contractmeta!(key = "home_domain", val = "lafiya-xyz.github.io");
soroban_sdk::contractmeta!(key = "crate_name", val = env!("CARGO_PKG_NAME"));
soroban_sdk::contractmeta!(key = "crate_version", val = env!("CARGO_PKG_VERSION"));
soroban_sdk::contractmeta!(key = "source_rev", val = env!("LAFIYA_GIT_COMMIT"));

/// The facility registry contract.
#[contract]
pub struct FacilityRegistry;

#[contractimpl]
impl FacilityRegistry {
    /// Configure the admin at deployment.
    pub fn __constructor(env: Env, admin: Address) {
        env.storage().instance().set(&DataKey::Admin, &admin);
        env.storage()
            .instance()
            .set(&DataKey::SchemaVersion, &SCHEMA_VERSION);
        env.storage().instance().extend_ttl(
            INSTANCE_LIFETIME_THRESHOLD.get(),
            INSTANCE_BUMP_AMOUNT.get(),
        );
    }

    pub fn get_admin(env: Env) -> Result<Address, Error> {
        Self::admin(&env)
    }

    pub fn get_schema_version(env: Env) -> u32 {
        env.storage()
            .instance()
            .get(&DataKey::SchemaVersion)
            .unwrap_or(SCHEMA_VERSION)
    }

    /// Enroll a facility and return its id. Requires the admin.
    pub fn enroll_facility(
        env: Env,
        license_hash: BytesN<32>,
        region: Option<String>,
    ) -> Result<u64, Error> {
        Self::admin(&env)?.require_auth();
        let id: u64 = env
            .storage()
            .instance()
            .get(&DataKey::FacilityCount)
            .unwrap_or(0)
            + 1;
        if id > MAX_FACILITIES {
            return Err(Error::FacilityLimitReached);
        }
        env.storage().instance().set(&DataKey::FacilityCount, &id);
        Self::save_facility(
            &env,
            id,
            &FacilityInfo {
                license_hash: license_hash.clone(),
                region,
                enrolled_at: env.ledger().timestamp(),
                active: true,
                deactivated_at: None,
            },
        );
        let mutation_seq = Self::bump_seq(&env);
        FacilityEnrolled {
            facility_id: id,
            license_hash,
            mutation_seq,
        }
        .publish(&env);
        Ok(id)
    }

    /// Deactivate a facility. Its attesters stop counting as affiliated
    /// immediately; recorded affiliation periods are kept for audit.
    /// Requires the admin.
    pub fn deactivate_facility(env: Env, facility_id: u64) -> Result<(), Error> {
        Self::admin(&env)?.require_auth();
        let mut info = Self::facility(&env, facility_id)?;
        if !info.active {
            return Err(Error::FacilityStateUnchanged);
        }
        info.active = false;
        info.deactivated_at = Some(env.ledger().timestamp());
        Self::save_facility(&env, facility_id, &info);
        let mutation_seq = Self::bump_seq(&env);
        FacilityDeactivated {
            facility_id,
            mutation_seq,
        }
        .publish(&env);
        Ok(())
    }

    /// Reactivate a deactivated facility. Open affiliations resume.
    /// Requires the admin.
    pub fn reactivate_facility(env: Env, facility_id: u64) -> Result<(), Error> {
        Self::admin(&env)?.require_auth();
        let mut info = Self::facility(&env, facility_id)?;
        if info.active {
            return Err(Error::FacilityStateUnchanged);
        }
        info.active = true;
        info.deactivated_at = None;
        Self::save_facility(&env, facility_id, &info);
        let mutation_seq = Self::bump_seq(&env);
        FacilityReactivated {
            facility_id,
            mutation_seq,
        }
        .publish(&env);
        Ok(())
    }

    /// Affiliate `attester` with an active facility. Requires the admin.
    pub fn affiliate_attester(env: Env, facility_id: u64, attester: Address) -> Result<(), Error> {
        Self::admin(&env)?.require_auth();
        if !Self::facility(&env, facility_id)?.active {
            return Err(Error::FacilityInactive);
        }
        let mut periods = Self::periods(&env, facility_id, &attester);
        if periods.last().is_some_and(|last| last.until.is_none()) {
            return Err(Error::AlreadyAffiliated);
        }
        if periods.len() >= MAX_PERIODS {
            return Err(Error::PeriodLimitReached);
        }
        let count = Self::affiliate_count(&env, facility_id);
        if count >= MAX_AFFILIATES_PER_FACILITY {
            return Err(Error::AffiliateLimitReached);
        }
        periods.push_back(AffiliationPeriod {
            since: env.ledger().timestamp(),
            until: None,
        });
        Self::save_periods(&env, facility_id, &attester, &periods);
        Self::save_count(&env, facility_id, count + 1);
        let mutation_seq = Self::bump_seq(&env);
        AttesterAffiliated {
            facility_id,
            attester,
            mutation_seq,
        }
        .publish(&env);
        Ok(())
    }

    /// End `attester`'s open affiliation. Attestations already made for the
    /// facility keep their stored scope; `was_affiliated_at` still answers
    /// for the earlier period. Requires the admin.
    pub fn disaffiliate_attester(
        env: Env,
        facility_id: u64,
        attester: Address,
    ) -> Result<(), Error> {
        Self::admin(&env)?.require_auth();
        Self::facility(&env, facility_id)?;
        let mut periods = Self::periods(&env, facility_id, &attester);
        let last = periods.last().filter(|last| last.until.is_none());
        let Some(mut last) = last else {
            return Err(Error::NotAffiliated);
        };
        last.until = Some(env.ledger().timestamp());
        periods.set(periods.len() - 1, last);
        Self::save_periods(&env, facility_id, &attester, &periods);
        let count = Self::affiliate_count(&env, facility_id);
        Self::save_count(&env, facility_id, count.saturating_sub(1));
        let mutation_seq = Self::bump_seq(&env);
        AttesterDisaffiliated {
            facility_id,
            attester,
            mutation_seq,
        }
        .publish(&env);
        Ok(())
    }

    pub fn get_facility(env: Env, facility_id: u64) -> Option<FacilityInfo> {
        Self::facility(&env, facility_id).ok()
    }

    pub fn get_facility_count(env: Env) -> u64 {
        env.storage()
            .instance()
            .get(&DataKey::FacilityCount)
            .unwrap_or(0)
    }

    pub fn is_facility_active(env: Env, facility_id: u64) -> bool {
        Self::facility(&env, facility_id).is_ok_and(|info| info.active)
    }

    /// Number of attesters currently affiliated with the facility.
    pub fn get_affiliate_count(env: Env, facility_id: u64) -> u32 {
        Self::affiliate_count(&env, facility_id)
    }

    /// Whether `attester` is affiliated with `facility_id` right now and the
    /// facility is active.
    pub fn is_affiliated(env: Env, facility_id: u64, attester: Address) -> bool {
        Self::is_facility_active(env.clone(), facility_id)
            && Self::periods(&env, facility_id, &attester)
                .last()
                .is_some_and(|last| last.until.is_none())
    }

    /// Whether `attester` was affiliated with `facility_id` at `timestamp`,
    /// ignoring the facility's present activation state. Verifiers combine
    /// this with `get_facility` to judge a stored attestation.
    pub fn was_affiliated_at(
        env: Env,
        facility_id: u64,
        attester: Address,
        timestamp: u64,
    ) -> bool {
        Self::periods(&env, facility_id, &attester)
            .iter()
            .any(|p| p.since <= timestamp && p.until.is_none_or(|until| timestamp < until))
    }

    /// Retained affiliation periods for the pair, oldest first.
    pub fn get_affiliation_periods(
        env: Env,
        facility_id: u64,
        attester: Address,
    ) -> Vec<AffiliationPeriod> {
        Self::periods(&env, facility_id, &attester)
    }

    /// Monotonic cursor incremented by every mutation, letting an indexer
    /// prove it observed every event (ADR-0017).
    pub fn get_mutation_seq(env: Env) -> u64 {
        env.storage()
            .instance()
            .get(&DataKey::MutationSeq)
            .unwrap_or(0)
    }

    /// Permissionless TTL keep-alive (issue #717).
    ///
    /// Extends the instance and code TTLs so a low-traffic deployment does
    /// not archive. Works regardless of any paused state (this contract has
    /// none today, but the entry point is deliberately unconditional) and
    /// requires no auth: the caller pays the extension fees.
    ///
    /// Returns `true` when the TTLs were extended, `false` when the call was
    /// throttled because `TOUCH_MIN_INTERVAL_LEDGERS` has not elapsed since
    /// the previous `touch()`. Throttling bounds griefing: an attacker cannot
    /// force repeated paid extensions more often than the interval.
    pub fn touch(env: Env) -> bool {
        let now = env.ledger().sequence();
        let last: u32 = env
            .storage()
            .instance()
            .get(&DataKey::LastTouch)
            .unwrap_or(0);
        if now.saturating_sub(last) < TOUCH_MIN_INTERVAL_LEDGERS.get() {
            return false;
        }
        env.storage().instance().set(&DataKey::LastTouch, &now);
        env.storage().instance().extend_ttl(
            INSTANCE_LIFETIME_THRESHOLD.get(),
            INSTANCE_BUMP_AMOUNT.get(),
        );
        env.storage()
            .instance()
            .extend_ttl(INSTANCE_LIFETIME_THRESHOLD.get(), INSTANCE_BUMP_AMOUNT.get());
        env.deployer().extend_current_contract_instance_and_code_ttl(
            INSTANCE_LIFETIME_THRESHOLD.get(),
            INSTANCE_BUMP_AMOUNT.get(),
        );
        true
    }
}

impl FacilityRegistry {
    fn admin(env: &Env) -> Result<Address, Error> {
        env.storage()
            .instance()
            .get(&DataKey::Admin)
            .ok_or(Error::NotInitialized)
    }

    fn facility(env: &Env, facility_id: u64) -> Result<FacilityInfo, Error> {
        env.storage()
            .persistent()
            .get(&DataKey::Facility(facility_id))
            .ok_or(Error::FacilityNotFound)
    }

    fn save_facility(env: &Env, facility_id: u64, info: &FacilityInfo) {
        let key = DataKey::Facility(facility_id);
        env.storage().persistent().set(&key, info);
        env.storage().persistent().extend_ttl(
            &key,
            INSTANCE_LIFETIME_THRESHOLD.get(),
            INSTANCE_BUMP_AMOUNT.get(),
        );
    }

    fn periods(env: &Env, facility_id: u64, attester: &Address) -> Vec<AffiliationPeriod> {
        env.storage()
            .persistent()
            .get(&DataKey::Affiliation(facility_id, attester.clone()))
            .unwrap_or_else(|| Vec::new(env))
    }

    fn save_periods(
        env: &Env,
        facility_id: u64,
        attester: &Address,
        periods: &Vec<AffiliationPeriod>,
    ) {
        let key = DataKey::Affiliation(facility_id, attester.clone());
        env.storage().persistent().set(&key, periods);
        env.storage().persistent().extend_ttl(
            &key,
            INSTANCE_LIFETIME_THRESHOLD.get(),
            INSTANCE_BUMP_AMOUNT.get(),
        );
    }

    fn affiliate_count(env: &Env, facility_id: u64) -> u32 {
        env.storage()
            .persistent()
            .get(&DataKey::AffiliateCount(facility_id))
            .unwrap_or(0)
    }

    fn save_count(env: &Env, facility_id: u64, count: u32) {
        let key = DataKey::AffiliateCount(facility_id);
        env.storage().persistent().set(&key, &count);
        env.storage().persistent().extend_ttl(
            &key,
            INSTANCE_LIFETIME_THRESHOLD.get(),
            INSTANCE_BUMP_AMOUNT.get(),
        );
    }

    /// Increment and return the mutation cursor, refreshing instance TTL.
    fn bump_seq(env: &Env) -> u64 {
        let next = Self::get_mutation_seq(env.clone()) + 1;
        env.storage().instance().set(&DataKey::MutationSeq, &next);
        env.storage().instance().extend_ttl(
            INSTANCE_LIFETIME_THRESHOLD.get(),
            INSTANCE_BUMP_AMOUNT.get(),
        );
        next
    }
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
mod test;
