/**
 * Every verdict the SDK can return. The set is finite and closed: a UI can
 * switch over it exhaustively. Only `"verified"` means the card may be
 * shown as trusted. Order is precedence: when several checks fail, the
 * earliest one here is the verdict (all failures are still in `reasons`).
 */
export const VERDICTS = [
  "invalid_payload",
  "untrusted_contract",
  "commitment_mismatch",
  "unavailable",
  "revoked",
  "not_found",
  "superseded",
  "attester_removed",
  "attester_suspended",
  "facility_untrusted",
  "expired",
  "disputed",
  "quorum_not_met",
  "verified",
] as const;

export type Verdict = (typeof VERDICTS)[number];

/**
 * One-line technical meaning of each verdict. For responder-facing,
 * localized, plain-language text use `localizeVerdict` (./i18n.ts).
 */
export const VERDICT_DESCRIPTIONS: Record<Verdict, string> = {
  invalid_payload: "The QR code is not a valid Lafiya card payload.",
  untrusted_contract:
    "The card points to a network or contract that is not in the trusted list.",
  commitment_mismatch: "The disclosed record does not match the card's record hash.",
  unavailable: "The attestation could not be checked (network or RPC failure).",
  revoked: "The attestation for this record was revoked by the registry admin.",
  not_found: "No attestation exists for this record.",
  superseded: "A newer attestation exists for this record than the one on the card.",
  attester_removed: "The attesting health worker is no longer on the allowlist.",
  attester_suspended: "The attesting health worker is currently suspended.",
  facility_untrusted:
    "The attesting facility was deactivated for fraud; attestations made before that are untrusted.",
  expired: "The attestation is older than the maximum accepted age.",
  disputed:
    "The patient has flagged this attestation as disputed; treat the data as contested.",
  quorum_not_met:
    "The record requires more independent attesters than have co-attested so far.",
  verified: "Attested by a currently trusted health worker.",
};

/**
 * Why a facility was deactivated. Mirrors the on-chain `deactivation_reason`
 * code. `fraud` is the only reason that invalidates attestations made before
 * deactivation; `closed` and `licence_lapsed` leave them valid.
 */
export const FACILITY_DEACTIVATION_REASONS = [
  "closed",
  "licence_lapsed",
  "fraud",
] as const;

export type FacilityDeactivationReason =
  (typeof FACILITY_DEACTIVATION_REASONS)[number];

/**
 * Facility status as observed at verification time. `deactivatedAt` is the
 * ledger timestamp of deactivation (seconds); `reason` is absent while active.
 */
export interface FacilityStatus {
  active: boolean;
  reason?: FacilityDeactivationReason;
  deactivatedAt?: number;
}

/**
 * Whether an attestation made at `attestedAt` is invalidated by the facility's
 * deactivation. Only `fraud` invalidates, and only attestations made at or
 * before the deactivation timestamp. Mirrors the Rust model.
 */
export function facilityInvalidatesAttestation(
  status: FacilityStatus | undefined,
  attestedAt: number,
): boolean {
  if (!status || status.active) return false;
  if (status.reason !== "fraud") return false;
  if (status.deactivatedAt === undefined) return false;
  return attestedAt <= status.deactivatedAt;
}

/** A single problem (or, for `verified`, the success) behind a verdict. */
export interface Reason {
  code: Verdict;
  message: string;
}

/** Pick the highest-precedence verdict among `reasons` (`verified` if none). */
export function verdictOf(reasons: readonly Reason[]): Verdict {
  let best: Verdict = "verified";
  for (const r of reasons) {
    if (VERDICTS.indexOf(r.code) < VERDICTS.indexOf(best)) best = r.code;
  }
  return best;
}
