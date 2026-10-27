/**
 * The normative verification model (docs/specs/verification-model.md),
 * evaluated over its normalized inputs. It must agree with
 * scripts/verification_model.py and the Rust `lafiya-verification-model`
 * crate on every input; scripts/differential_verification.py checks that.
 */
export const MODEL_VERSION = "1.0.0";

export const MODEL_VERDICTS = [
  "WrongNetwork",
  "Indeterminate",
  "NotFound",
  "Revoked",
  "AttesterCompromised",
  "AttesterRemoved",
  "Expired",
  "AttesterSuspended",
  "Superseded",
  "Verified",
] as const;

export type ModelVerdict = (typeof MODEL_VERDICTS)[number];

/** The precedence rule that produced a verdict (spec section 3). */
export type ModelReason =
  | "network_mismatch"
  | "attestation_read_failed"
  | "attestation_missing"
  | "revoked"
  | "attester_status_read_failed"
  | "compromise_window"
  | "attester_removed"
  | "age_exceeds_policy"
  | "attester_suspended"
  | "superseded"
  | "none";

/** `default_input` of verification-model.json. */
export interface ModelInput {
  network_matches: boolean;
  attestation_read: "ok" | "failed";
  attester_status_read: "ok" | "failed";
  attestation_found: boolean;
  revoked: boolean;
  attester_status: "active" | "suspended" | "removed";
  attested_in_compromise_window: boolean;
  age_exceeds_policy: boolean;
  superseded: boolean;
  offline: boolean;
}

export interface ModelResult {
  verdict: ModelVerdict;
  reason: ModelReason;
  qualifier: "as_of_bundle" | null;
}

export function evaluateModel(i: ModelInput): ModelResult {
  const qualifier = i.offline ? "as_of_bundle" : null;
  const r = (verdict: ModelVerdict, reason: ModelReason): ModelResult => ({
    verdict,
    reason,
    qualifier,
  });
  if (!i.network_matches) return r("WrongNetwork", "network_mismatch");
  if (i.attestation_read !== "ok") return r("Indeterminate", "attestation_read_failed");
  if (!i.attestation_found) return r("NotFound", "attestation_missing");
  if (i.revoked) return r("Revoked", "revoked");
  // Everything below depends on attester state; never guess it.
  if (i.attester_status_read !== "ok") return r("Indeterminate", "attester_status_read_failed");
  if (i.attested_in_compromise_window) return r("AttesterCompromised", "compromise_window");
  if (i.attester_status === "removed") return r("AttesterRemoved", "attester_removed");
  if (i.age_exceeds_policy) return r("Expired", "age_exceeds_policy");
  if (i.attester_status === "suspended") return r("AttesterSuspended", "attester_suspended");
  if (i.superseded) return r("Superseded", "superseded");
  return r("Verified", "none");
}
