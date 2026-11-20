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

use soroban_sdk::{
    contract, contracterror, contractevent, contractimpl, contracttype, Address, BytesN, Env,
    String, Vec,
};

const SCHEMA_VERSION: u32 = 2;

/// Domain separator for portable facility identifiers (issue #689).
const FACILITY_ID_DOMAIN: &[u8] = b"lafiya.facility.v1";

/// Number of affiliation periods stored per page (issue #687). Pages are
/// fixed-size so no single storage entry grows without bound.
pub const AFFILIATION_PAGE_SIZE: u32 = 8;
/// Maximum number of pages retained per (facility, attester) pair.
pub const MAX_AFFILIATION_PAGES: u32 = 125;
/// Default page size returned by `get_affiliation_periods` when `limit` is 0.
pub const DEFAULT_PERIOD_LIMIT: u32 = 32;
/// Maximum page size accepted by `get_affiliation_periods`.
pub const MAX_PERIOD_LIMIT: u32 = 64;

/// Instance storage TTL policy, matching the other registries.
const INSTANCE_BUMP_AMOUNT: u32 = 1_555_200;
const INSTANCE_LIFETIME_THRESHOLD: u32 = 518_400;

/// Upper bound on enrolled facilities.
pub const MAX_FACILITIES: u64 = 10_000;
/// Upper bound on currently affiliated attesters per facility, bounding the
/// per-facility counter's state and the cost of an audit sweep.
pub const MAX_AFFILIATES_PER_FACILITY: u32 = 500;
/// Upper bound on retained affiliation periods per (facility, attester),
/// derived from the page layout.
pub const MAX_PERIODS: u32 = AFFILIATION_PAGE_SIZE * MAX_AFFILIATION_PAGES;

/// Portable facility identifier: `H(domain || jurisdiction ||
/// issuing_authority || facility_code)` (issue #689).
pub type FacilityId = BytesN<32>;

#[contracttype]
#[derive(Clone)]
enum DataKey {
    Admin,
    SchemaVersion,
    /// Highest facility id allocated (instance storage).
    FacilityCount,
    /// Monotonic mutation cursor (instance storage), per ADR-0017.
    MutationSeq,
    /// Local u64 alias -> facility record.
    Facility(u64),
    /// Portable id -> local u64 alias (issue #689).
    PortableId(FacilityId),
    /// External code hash -> local u64 alias (issue #689).
    ExternalCode(BytesN<32>),
    /// Fixed-size page of affiliation periods (issue #687).
    AffiliationPage(u64, Address, u32),
    /// Small header for a (facility, attester) pair (issue #687).
    AffiliationHeader(u64, Address),
    /// Number of attesters currently affiliated with a facility.
    AffiliateCount(u64),
}

/// Why a facility was deactivated (issue #688).
#[contracttype]
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum DeactivationReason {
    Closed = 0,
    LicenceLapsed = 1,
    Fraud = 2,
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
    /// Portable identifier (issue #689).
    pub facility_id: FacilityId,
    /// Hash of the external facility code used to derive `facility_id`.
    pub external_code_hash: BytesN<32>,
    /// Why the facility was last deactivated (issue #688).
    pub deactivation_reason: Option<DeactivationReason>,
}

/// One continuous period of affiliation. `until` is exclusive.
#[contracttype]
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct AffiliationPeriod {
    pub since: u64,
    pub until: Option<u64>,
}

/// Small, fixed-size header for a (facility, attester) pair (issue #687).
#[contracttype]
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct AffiliationHeader {
    /// Total number of periods recorded across all pages.
    pub total: u32,
    /// Number of pages allocated.
    pub pages: u32,
    /// Latest period, mirrored for O(1) `is_affiliated`.
    pub latest: Option<AffiliationPeriod>,
}

/// One page of affiliation periods returned by `get_affiliation_periods`.
#[contracttype]
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct AffiliationPageResult {
    pub periods: Vec<AffiliationPeriod>,
    /// Cursor to pass for the next page, or `None` when exhausted.
    pub next_cursor: Option<u32>,
    pub total: u32,
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
    /// The portable facility id is already registered.
    FacilityIdTaken = 10,
    /// The external code hash is already registered.
    ExternalCodeTaken = 11,
    /// No facility exists with this portable id.
    PortableIdNotFound = 12,
    /// No facility exists with this external code hash.
    ExternalCodeNotFound = 13,
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
    pub reason: DeactivationReason,
    pub deactivated_at: u64,
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
        env.storage()
            .instance()
            .extend_ttl(INSTANCE_LIFETIME_THRESHOLD, INSTANCE_BUMP_AMOUNT);
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

    /// Enroll a facility and return its local u64 alias. Requires the admin.
    ///
    /// The portable id is derived from `jurisdiction`, `issuing_authority`
    /// and `facility_code` (issue #689). The external code hash is also
    /// indexed for lookup.
    pub fn enroll_facility(
        env: Env,
        license_hash: BytesN<32>,
        region: Option<String>,
        jurisdiction: String,
        issuing_authority: String,
        facility_code: String,
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
        let portable = Self::derive_facility_id(
            &env,
            &jurisdiction,
            &issuing_authority,
            &facility_code,
        );
        let external_hash = Self::hash_external_code(&env, &jurisdiction, &facility_code);
        if env
            .storage()
            .persistent()
            .has(&DataKey::PortableId(portable.clone()))
        {
            return Err(Error::FacilityIdTaken);
        }
        if env
            .storage()
            .persistent()
            .has(&DataKey::ExternalCode(external_hash.clone()))
        {
            return Err(Error::ExternalCodeTaken);
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
                facility_id: portable.clone(),
                external_code_hash: external_hash.clone(),
                deactivation_reason: None,
            },
        );
        Self::save_portable_index(&env, &portable, id);
        Self::save_external_index(&env, &external_hash, id);
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
    pub fn deactivate_facility(
        env: Env,
        facility_id: u64,
        reason: DeactivationReason,
    ) -> Result<(), Error> {
        Self::admin(&env)?.require_auth();
        let mut info = Self::facility(&env, facility_id)?;
        if !info.active {
            return Err(Error::FacilityStateUnchanged);
        }
        let now = env.ledger().timestamp();
        info.active = false;
        info.deactivated_at = Some(now);
        info.deactivation_reason = Some(reason);
        Self::save_facility(&env, facility_id, &info);
        let mutation_seq = Self::bump_seq(&env);
        FacilityDeactivated {
            facility_id,
            reason,
            deactivated_at: now,
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
        info.deactivation_reason = None;
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
        let mut header = Self::header(&env, facility_id, &attester);
        if header
            .latest
            .as_ref()
            .is_some_and(|last| last.until.is_none())
        {
            return Err(Error::AlreadyAffiliated);
        }
        if header.total >= MAX_PERIODS {
            return Err(Error::PeriodLimitReached);
        }
        let count = Self::affiliate_count(&env, facility_id);
        if count >= MAX_AFFILIATES_PER_FACILITY {
            return Err(Error::AffiliateLimitReached);
        }
        let period = AffiliationPeriod {
            since: env.ledger().timestamp(),
            until: None,
        };
        Self::append_period(&env, facility_id, &attester, &mut header, period);
        Self::save_header(&env, facility_id, &attester, &header);
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
        let mut header = Self::header(&env, facility_id, &attester);
        let Some(mut last) = header.latest.clone().filter(|last| last.until.is_none()) else {
            return Err(Error::NotAffiliated);
        };
        last.until = Some(env.ledger().timestamp());
        Self::overwrite_last_period(&env, facility_id, &attester, &header, last.clone());
        header.latest = Some(last);
        Self::save_header(&env, facility_id, &attester, &header);
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
    ///
    /// Implemented as a binary search over page boundaries (issue #687), so
    /// the read cost is O(log n) storage entries.
    pub fn was_affiliated_at(
        env: Env,
        facility_id: u64,
        attester: Address,
        timestamp: u64,
    ) -> bool {
        let header = Self::header(&env, facility_id, &attester);
        if header.total == 0 {
            return false;
        }
        // Find the last page whose first period starts at or before
        // `timestamp`; the answer, if any, lives in that page or the one
        // before it.
        let mut lo: u32 = 0;
        let mut hi: u32 = header.pages;
        while lo < hi {
            let mid = lo + (hi - lo) / 2;
            let page = Self::page(&env, facility_id, &attester, mid);
            let first_since = page.first().map(|p| p.since).unwrap_or(u64::MAX);
            if first_since <= timestamp {
                lo = mid + 1;
            } else {
                hi = mid;
            }
        }
        let candidate = lo.saturating_sub(1);
        let page = Self::page(&env, facility_id, &attester, candidate);
        page.iter()
            .any(|p| p.since <= timestamp && p.until.is_none_or(|until| timestamp < until))
    }

    /// Retained affiliation periods for the pair, oldest first, paginated
    /// (issue #687). `cursor` is the index of the first period to return;
    /// `limit` is clamped to `MAX_PERIOD_LIMIT` (0 means
    /// `DEFAULT_PERIOD_LIMIT`).
    pub fn get_affiliation_periods(
        env: Env,
        facility_id: u64,
        attester: Address,
        cursor: u32,
        limit: u32,
    ) -> AffiliationPageResult {
        let header = Self::header(&env, facility_id, &attester);
        let effective_limit = if limit == 0 {
            DEFAULT_PERIOD_LIMIT
        } else {
            limit.min(MAX_PERIOD_LIMIT)
        };
        let mut periods = Vec::new(&env);
        if cursor >= header.total {
            return AffiliationPageResult {
                periods,
                next_cursor: None,
                total: header.total,
            };
        }
        let end = cursor.saturating_add(effective_limit).min(header.total);
        let mut idx = cursor;
        while idx < end {
            let page_no = idx / AFFILIATION_PAGE_SIZE;
            let offset = idx % AFFILIATION_PAGE_SIZE;
            let page = Self::page(&env, facility_id, &attester, page_no);
            if let Some(period) = page.get(offset) {
                periods.push_back(period);
            }
            idx += 1;
        }
        let next_cursor = if end < header.total { Some(end) } else { None };
        AffiliationPageResult {
            periods,
            next_cursor,
            total: header.total,
        }
    }

    /// Look up a facility by its portable id (issue #689).
    pub fn get_facility_by_portable_id(env: Env, facility_id: FacilityId) -> Option<FacilityInfo> {
        let local: u64 = env
            .storage()
            .persistent()
            .get(&DataKey::PortableId(facility_id))?;
        Self::facility(&env, local).ok()
    }

    /// Look up a facility by the hash of its external code (issue #689).
    pub fn get_facility_by_external_code(
        env: Env,
        external_code_hash: BytesN<32>,
    ) -> Option<FacilityInfo> {
        let local: u64 = env
            .storage()
            .persistent()
            .get(&DataKey::ExternalCode(external_code_hash))?;
        Self::facility(&env, local).ok()
    }

    /// Derive the canonical portable id for an external facility code
    /// (issue #689). Exposed so bindings and CLI can compute it off-chain.
    pub fn derive_facility_id(
        env: Env,
        jurisdiction: String,
        issuing_authority: String,
        facility_code: String,
    ) -> FacilityId {
        Self::derive_facility_id_inner(&env, &jurisdiction, &issuing_authority, &facility_code)
    }

    /// Hash of the external facility code, used for lookup (issue #689).
    pub fn hash_external_code(
        env: Env,
        jurisdiction: String,
        facility_code: String,
    ) -> BytesN<32> {
        Self::hash_external_code_inner(&env, &jurisdiction, &facility_code)
    }

    /// Monotonic cursor incremented by every mutation, letting an indexer
    /// prove it observed every event (ADR-0017).
    pub fn get_mutation_seq(env: Env) -> u64 {
        env.storage()
            .instance()
            .get(&DataKey::MutationSeq)
            .unwrap_or(0)
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
            INSTANCE_LIFETIME_THRESHOLD,
            INSTANCE_BUMP_AMOUNT,
        );
    }

    fn header(env: &Env, facility_id: u64, attester: &Address) -> AffiliationHeader {
        env.storage()
            .persistent()
            .get(&DataKey::AffiliationHeader(facility_id, attester.clone()))
            .unwrap_or(AffiliationHeader {
                total: 0,
                pages: 0,
                latest: None,
            })
    }

    fn save_header(
        env: &Env,
        facility_id: u64,
        attester: &Address,
        header: &AffiliationHeader,
    ) {
        let key = DataKey::AffiliationHeader(facility_id, attester.clone());
        env.storage().persistent().set(&key, header);
        env.storage().persistent().extend_ttl(
            &key,
            INSTANCE_LIFETIME_THRESHOLD,
            INSTANCE_BUMP_AMOUNT,
        );
    }

    fn page(env: &Env, facility_id: u64, attester: &Address, page_no: u32) -> Vec<AffiliationPeriod> {
        env.storage()
            .persistent()
            .get(&DataKey::AffiliationPage(
                facility_id,
                attester.clone(),
                page_no,
            ))
            .unwrap_or_else(|| Vec::new(env))
    }

    fn save_page(
        env: &Env,
        facility_id: u64,
        attester: &Address,
        page_no: u32,
        page: &Vec<AffiliationPeriod>,
    ) {
        let key = DataKey::AffiliationPage(facility_id, attester.clone(), page_no);
        env.storage().persistent().set(&key, page);
        env.storage().persistent().extend_ttl(
            &key,
            INSTANCE_LIFETIME_THRESHOLD,
            INSTANCE_BUMP_AMOUNT,
        );
    }

    /// Append a period to the pair's pages, rolling over to a new page when
    /// the current one is full, and update the header.
    fn append_period(
        env: &Env,
        facility_id: u64,
        attester: &Address,
        header: &mut AffiliationHeader,
        period: AffiliationPeriod,
    ) {
        let page_no = if header.total == 0 {
            0
        } else {
            (header.total - 1) / AFFILIATION_PAGE_SIZE
        };
        let mut page = Self::page(env, facility_id, attester, page_no);
        if page.len() >= AFFILIATION_PAGE_SIZE {
            let next_page = page_no + 1;
            let mut fresh = Vec::new(env);
            fresh.push_back(period.clone());
            Self::save_page(env, facility_id, attester, next_page, &fresh);
            header.pages = next_page + 1;
        } else {
            page.push_back(period.clone());
            Self::save_page(env, facility_id, attester, page_no, &page);
            if header.pages <= page_no {
                header.pages = page_no + 1;
            }
        }
        header.total += 1;
        header.latest = Some(period);
    }

    /// Overwrite the most recently appended period (used by disaffiliate).
    fn overwrite_last_period(
        env: &Env,
        facility_id: u64,
        attester: &Address,
        header: &AffiliationHeader,
        period: AffiliationPeriod,
    ) {
        if header.total == 0 {
            return;
        }
        let idx = header.total - 1;
        let page_no = idx / AFFILIATION_PAGE_SIZE;
        let offset = idx % AFFILIATION_PAGE_SIZE;
        let mut page = Self::page(env, facility_id, attester, page_no);
        page.set(offset, period);
        Self::save_page(env, facility_id, attester, page_no, &page);
    }

    fn save_portable_index(env: &Env, portable: &FacilityId, local: u64) {
        let key = DataKey::PortableId(portable.clone());
        env.storage().persistent().set(&key, &local);
        env.storage().persistent().extend_ttl(
            &key,
            INSTANCE_LIFETIME_THRESHOLD,
            INSTANCE_BUMP_AMOUNT,
        );
    }

    fn save_external_index(env: &Env, external_hash: &BytesN<32>, local: u64) {
        let key = DataKey::ExternalCode(external_hash.clone());
        env.storage().persistent().set(&key, &local);
        env.storage().persistent().extend_ttl(
            &key,
            INSTANCE_LIFETIME_THRESHOLD,
            INSTANCE_BUMP_AMOUNT,
        );
    }

    /// `H(domain || jurisdiction || issuing_authority || facility_code)`.
    fn derive_facility_id_inner(
        env: &Env,
        jurisdiction: &String,
        issuing_authority: &String,
        facility_code: &String,
    ) -> FacilityId {
        let mut buf = Vec::new(env);
        for byte in FACILITY_ID_DOMAIN {
            buf.push_back(*byte);
        }
        buf.push_back(0);
        Self::push_string(env, &mut buf, jurisdiction);
        buf.push_back(0);
        Self::push_string(env, &mut buf, issuing_authority);
        buf.push_back(0);
        Self::push_string(env, &mut buf, facility_code);
        env.crypto().sha256(&buf).to_bytes()
    }

    /// `H(domain || jurisdiction || facility_code)`.
    fn hash_external_code_inner(
        env: &Env,
        jurisdiction: &String,
        facility_code: &String,
    ) -> BytesN<32> {
        let mut buf = Vec::new(env);
        for byte in FACILITY_ID_DOMAIN {
            buf.push_back(*byte);
        }
        buf.push_back(1);
        Self::push_string(env, &mut buf, jurisdiction);
        buf.push_back(0);
        Self::push_string(env, &mut buf, facility_code);
        env.crypto().sha256(&buf).to_bytes()
    }

    fn push_string(env: &Env, buf: &mut Vec<u8>, value: &String) {
        let bytes = value.to_bytes();
        let mut i = 0;
        while i < bytes.len() {
            buf.push_back(bytes.get(i).unwrap_or(0));
            i += 1;
        }
        let _ = env;
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
            INSTANCE_LIFETIME_THRESHOLD,
            INSTANCE_BUMP_AMOUNT,
        );
    }

    /// Increment and return the mutation cursor, refreshing instance TTL.
    fn bump_seq(env: &Env) -> u64 {
        let next = Self::get_mutation_seq(env.clone()) + 1;
        env.storage().instance().set(&DataKey::MutationSeq, &next);
        env.storage()
            .instance()
            .extend_ttl(INSTANCE_LIFETIME_THRESHOLD, INSTANCE_BUMP_AMOUNT);
        next
    }
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
mod test;
