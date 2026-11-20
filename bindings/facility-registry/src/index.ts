// bindings/facility-registry/src/index.ts
//
// TypeScript bindings for the facility-registry Soroban contract.
//
// These bindings are hand-maintained and drift-checked in CI against the
// contract's public entry points (see scripts/conformance/check_bindings_drift.py
// and scripts/conformance/check_cli_coverage.py). Every public contract entry
// point must be represented here.

import { Address, nativeToScVal, scValToNative, xdr } from "@stellar/stellar-sdk";

/** A 32-byte portable facility identifier. */
export type FacilityId = Uint8Array;

/** Local, deployment-scoped alias for a facility. Kept for compatibility. */
export type FacilityAlias = bigint;

/** Reason codes for facility deactivation. */
export enum DeactivationReason {
  Closed = 0,
  LicenceLapsed = 1,
  Fraud = 2,
}

/** Human-readable labels for deactivation reason codes. */
export const DEACTIVATION_REASON_LABELS: Record<DeactivationReason, string> = {
  [DeactivationReason.Closed]: "closed",
  [DeactivationReason.LicenceLapsed]: "licence_lapsed",
  [DeactivationReason.Fraud]: "fraud",
};

/** Parse a deactivation reason from its numeric code or string label. */
export function parseDeactivationReason(
  value: DeactivationReason | number | string,
): DeactivationReason {
  if (typeof value === "number") {
    if (!(value in DEACTIVATION_REASON_LABELS)) {
      throw new Error(`unknown deactivation reason code: ${value}`);
    }
    return value as DeactivationReason;
  }
  if (typeof value === "string") {
    const normalized = value.trim().toLowerCase();
    for (const [code, label] of Object.entries(DEACTIVATION_REASON_LABELS)) {
      if (label === normalized) {
        return Number(code) as DeactivationReason;
      }
    }
    throw new Error(`unknown deactivation reason label: ${value}`);
  }
  throw new Error(`invalid deactivation reason: ${String(value)}`);
}

/** A facility record as returned by the contract. */
export interface Facility {
  /** Portable identifier (BytesN<32>). */
  id: FacilityId;
  /** Local, deployment-scoped alias (u64). */
  alias: FacilityAlias;
  /** Jurisdiction code the facility was enrolled under. */
  jurisdiction: string;
  /** Issuing authority that enrolled the facility. */
  issuingAuthority: string;
  /** External facility code (national registry / licence number). */
  facilityCode: string;
  /** Whether the facility is currently active. */
  active: boolean;
  /** Deactivation reason code, if deactivated. */
  deactivationReason: DeactivationReason | null;
  /** Ledger timestamp of deactivation, if deactivated. */
  deactivatedAt: bigint | null;
  /** Ledger timestamp of enrollment. */
  enrolledAt: bigint;
}

/** A single affiliation period between a facility and an attester. */
export interface AffiliationPeriod {
  /** Ledger timestamp at which the affiliation began. */
  start: bigint;
  /** Ledger timestamp at which the affiliation ended, or null if ongoing. */
  end: bigint | null;
}

/** A page of affiliation periods. */
export interface AffiliationPeriodPage {
  /** Periods in this page, ordered oldest-first. */
  periods: AffiliationPeriod[];
  /** Cursor to pass to fetch the next page, or null when exhausted. */
  nextCursor: bigint | null;
}

/** A facility affiliation record for an attester. */
export interface FacilityAffiliation {
  facilityId: FacilityId;
  attester: string;
  active: boolean;
}

/** Options for paginated reads. */
export interface PageOptions {
  /** Opaque cursor returned by a previous call. */
  cursor?: bigint | null;
  /** Maximum number of items to return. */
  limit?: number;
}

/** Default page size used when a caller does not specify a limit. */
export const DEFAULT_PAGE_LIMIT = 20;

/** Maximum page size accepted by the contract. */
export const MAX_PAGE_LIMIT = 100;

function toScValAddress(address: string): xdr.ScVal {
  return Address.fromString(address).toScVal();
}

function toScValFacilityId(id: FacilityId): xdr.ScVal {
  if (id.length !== 32) {
    throw new Error(`FacilityId must be 32 bytes, got ${id.length}`);
  }
  return nativeToScVal(id, { type: "bytes" });
}

function toScValAlias(alias: FacilityAlias): xdr.ScVal {
  return nativeToScVal(alias, { type: "u64" });
}

function toScValReason(reason: DeactivationReason): xdr.ScVal {
  return nativeToScVal(reason, { type: "u32" });
}

function toScValCursor(cursor: bigint | null | undefined): xdr.ScVal {
  return nativeToScVal(cursor ?? null, { type: "option" });
}

function toScValLimit(limit: number | undefined): xdr.ScVal {
  const value = limit ?? DEFAULT_PAGE_LIMIT;
  if (!Number.isInteger(value) || value <= 0) {
    throw new Error(`limit must be a positive integer, got ${value}`);
  }
  if (value > MAX_PAGE_LIMIT) {
    throw new Error(`limit must be <= ${MAX_PAGE_LIMIT}, got ${value}`);
  }
  return nativeToScVal(value, { type: "u32" });
}

/** Minimal contract client surface these bindings depend on. */
export interface ContractClient {
  call(method: string, ...args: xdr.ScVal[]): Promise<xdr.ScVal>;
}

/** Decode a native facility record. */
function decodeFacility(raw: any): Facility {
  const reasonRaw = raw.deactivation_reason ?? raw.deactivationReason ?? null;
  const deactivatedAtRaw = raw.deactivated_at ?? raw.deactivatedAt ?? null;
  return {
    id: Uint8Array.from(raw.id),
    alias: BigInt(raw.alias),
    jurisdiction: String(raw.jurisdiction),
    issuingAuthority: String(raw.issuing_authority ?? raw.issuingAuthority),
    facilityCode: String(raw.facility_code ?? raw.facilityCode),
    active: Boolean(raw.active),
    deactivationReason:
      reasonRaw === null || reasonRaw === undefined
        ? null
        : (Number(reasonRaw) as DeactivationReason),
    deactivatedAt:
      deactivatedAtRaw === null || deactivatedAtRaw === undefined
        ? null
        : BigInt(deactivatedAtRaw),
    enrolledAt: BigInt(raw.enrolled_at ?? raw.enrolledAt),
  };
}

/** Decode a native affiliation period. */
function decodeAffiliationPeriod(raw: any): AffiliationPeriod {
  const endRaw = raw.end ?? null;
  return {
    start: BigInt(raw.start),
    end: endRaw === null || endRaw === undefined ? null : BigInt(endRaw),
  };
}

/** Decode a native affiliation period page. */
function decodeAffiliationPeriodPage(raw: any): AffiliationPeriodPage {
  const periodsRaw = raw.periods ?? [];
  const nextRaw = raw.next_cursor ?? raw.nextCursor ?? null;
  return {
    periods: periodsRaw.map(decodeAffiliationPeriod),
    nextCursor: nextRaw === null || nextRaw === undefined ? null : BigInt(nextRaw),
  };
}

/**
 * Typed bindings for the facility-registry contract.
 *
 * All methods map 1:1 to a public contract entry point. Read methods return
 * decoded native values; write methods return the raw result of the call.
 */
export class FacilityRegistryClient {
  constructor(private readonly client: ContractClient) {}

  // ---------------------------------------------------------------------
  // Enrollment and lifecycle
  // ---------------------------------------------------------------------

  /** Enroll a new facility. Returns the assigned portable FacilityId. */
  async enrollFacility(params: {
    admin: string;
    jurisdiction: string;
    issuingAuthority: string;
    facilityCode: string;
  }): Promise<FacilityId> {
    const result = await this.client.call(
      "enroll_facility",
      toScValAddress(params.admin),
      nativeToScVal(params.jurisdiction, { type: "string" }),
      nativeToScVal(params.issuingAuthority, { type: "string" }),
      nativeToScVal(params.facilityCode, { type: "string" }),
    );
    return Uint8Array.from(scValToNative(result));
  }

  /** Deactivate a facility with a reason code. */
  async deactivateFacility(params: {
    admin: string;
    facilityId: FacilityId;
    reason: DeactivationReason | number | string;
  }): Promise<void> {
    await this.client.call(
      "deactivate_facility",
      toScValAddress(params.admin),
      toScValFacilityId(params.facilityId),
      toScValReason(parseDeactivationReason(params.reason)),
    );
  }

  /** Reactivate a previously deactivated facility. */
  async reactivateFacility(params: {
    admin: string;
    facilityId: FacilityId;
  }): Promise<void> {
    await this.client.call(
      "reactivate_facility",
      toScValAddress(params.admin),
      toScValFacilityId(params.facilityId),
    );
  }

  // ---------------------------------------------------------------------
  // Affiliation management
  // ---------------------------------------------------------------------

  /** Affiliate an attester with a facility. */
  async affiliate(params: {
    admin: string;
    facilityId: FacilityId;
    attester: string;
  }): Promise<void> {
    await this.client.call(
      "affiliate",
      toScValAddress(params.admin),
      toScValFacilityId(params.facilityId),
      toScValAddress(params.attester),
    );
  }

  /** Disaffiliate an attester from a facility. */
  async disaffiliate(params: {
    admin: string;
    facilityId: FacilityId;
    attester: string;
  }): Promise<void> {
    await this.client.call(
      "disaffiliate",
      toScValAddress(params.admin),
      toScValFacilityId(params.facilityId),
      toScValAddress(params.attester),
    );
  }

  // ---------------------------------------------------------------------
  // Reads
  // ---------------------------------------------------------------------

  /** Fetch a facility by its portable identifier. */
  async getFacility(facilityId: FacilityId): Promise<Facility | null> {
    const result = await this.client.call(
      "get_facility",
      toScValFacilityId(facilityId),
    );
    const native = scValToNative(result);
    return native === null || native === undefined ? null : decodeFacility(native);
  }

  /** Fetch a facility by its local u64 alias (compatibility path). */
  async getFacilityByAlias(alias: FacilityAlias): Promise<Facility | null> {
    const result = await this.client.call(
      "get_facility_by_alias",
      toScValAlias(alias),
    );
    const native = scValToNative(result);
    return native === null || native === undefined ? null : decodeFacility(native);
  }

  /**
   * Look up a facility by the hash of its external code.
   *
   * The hash is `H(domain || jurisdiction || issuing_authority || facility_code)`
   * as defined in docs/specs/facility-id.md.
   */
  async getFacilityByExternalCodeHash(codeHash: FacilityId): Promise<Facility | null> {
    const result = await this.client.call(
      "get_facility_by_external_code_hash",
      toScValFacilityId(codeHash),
    );
    const native = scValToNative(result);
    return native === null || native === undefined ? null : decodeFacility(native);
  }

  /** Total number of facilities enrolled (drives the u64 alias). */
  async getFacilityCount(): Promise<bigint> {
    const result = await this.client.call("get_facility_count");
    return BigInt(scValToNative(result));
  }

  /** Whether an attester is currently affiliated with a facility. */
  async isAffiliated(params: {
    facilityId: FacilityId;
    attester: string;
  }): Promise<boolean> {
    const result = await this.client.call(
      "is_affiliated",
      toScValFacilityId(params.facilityId),
      toScValAddress(params.attester),
    );
    return Boolean(scValToNative(result));
  }

  /** Whether an attester was affiliated with a facility at a given timestamp. */
  async wasAffiliatedAt(params: {
    facilityId: FacilityId;
    attester: string;
    timestamp: bigint;
  }): Promise<boolean> {
    const result = await this.client.call(
      "was_affiliated_at",
      toScValFacilityId(params.facilityId),
      toScValAddress(params.attester),
      nativeToScVal(params.timestamp, { type: "u64" }),
    );
    return Boolean(scValToNative(result));
  }

  /**
   * Paginated affiliation-period history for a (facility, attester) pair.
   *
   * Periods are stored in fixed-size pages; this method returns one page at a
   * time. Pass the returned `nextCursor` to fetch the next page.
   */
  async getAffiliationPeriods(params: {
    facilityId: FacilityId;
    attester: string;
    cursor?: bigint | null;
    limit?: number;
  }): Promise<AffiliationPeriodPage> {
    const result = await this.client.call(
      "get_affiliation_periods",
      toScValFacilityId(params.facilityId),
      toScValAddress(params.attester),
      toScValCursor(params.cursor),
      toScValLimit(params.limit),
    );
    return decodeAffiliationPeriodPage(scValToNative(result));
  }

  /**
   * Paginated list of facilities an attester is (or was) affiliated with.
   */
  async listAffiliations(params: {
    attester: string;
    cursor?: bigint | null;
    limit?: number;
  }): Promise<{ affiliations: FacilityAffiliation[]; nextCursor: bigint | null }> {
    const result = await this.client.call(
      "list_affiliations",
      toScValAddress(params.attester),
      toScValCursor(params.cursor),
      toScValLimit(params.limit),
    );
    const native = scValToNative(result);
    const itemsRaw = native.affiliations ?? [];
    const nextRaw = native.next_cursor ?? native.nextCursor ?? null;
    return {
      affiliations: itemsRaw.map((item: any) => ({
        facilityId: Uint8Array.from(item.facility_id ?? item.facilityId),
        attester: String(item.attester),
        active: Boolean(item.active),
      })),
      nextCursor: nextRaw === null || nextRaw === undefined ? null : BigInt(nextRaw),
    };
  }

  // ---------------------------------------------------------------------
  // Verification model
  // ---------------------------------------------------------------------

  /**
   * Resolve the facility-status component of the verification model for an
   * attestation made at `facilityId` at `attestationTimestamp`.
   *
   * Returns `facility_untrusted` when the facility was deactivated for fraud
   * before the attestation timestamp; otherwise `valid`.
   */
  async facilityStatusAt(params: {
    facilityId: FacilityId;
    attestationTimestamp: bigint;
  }): Promise<"valid" | "facility_inactive" | "facility_untrusted"> {
    const facility = await this.getFacility(params.facilityId);
    if (facility === null) {
      return "facility_untrusted";
    }
    if (facility.active) {
      return "valid";
    }
    if (
      facility.deactivationReason === DeactivationReason.Fraud &&
      facility.deactivatedAt !== null &&
      params.attestationTimestamp >= facility.deactivatedAt
    ) {
      return "facility_untrusted";
    }
    return "facility_inactive";
  }
}

export default FacilityRegistryClient;