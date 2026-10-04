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
  "expired",
  "verified",
] as const;

export type Verdict = (typeof VERDICTS)[number];

/** One-line meaning of each verdict, suitable for a UI. */
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
  expired: "The attestation is older than the maximum accepted age.",
  verified: "Attested by a currently trusted health worker.",
};

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
