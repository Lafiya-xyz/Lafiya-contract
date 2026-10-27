//! Rust implementation of the verification-result model
//! (`docs/specs/verification-model.md`).
//!
//! [`evaluate`] maps the normalized inputs of `verification-model.json` to a
//! verdict, the precedence rule that produced it, and the offline qualifier.
//! It must agree with `scripts/verification_model.py` and the TypeScript SDK
//! (`packages/verifier-sdk/src/model.ts`) on every input:
//! `scripts/differential_verification.py` runs all three over generated
//! scenarios and fails on any difference.

use serde::{Deserialize, Serialize};

/// `model_version` of the truth table this crate implements.
pub const MODEL_VERSION: &str = "1.0.0";

/// Verdicts in precedence order (spec section 2).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum Verdict {
    WrongNetwork,
    Indeterminate,
    NotFound,
    Revoked,
    AttesterCompromised,
    AttesterRemoved,
    Expired,
    AttesterSuspended,
    Superseded,
    Verified,
}

/// The precedence rule (spec section 3) that produced a verdict.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Reason {
    NetworkMismatch,
    AttestationReadFailed,
    AttestationMissing,
    Revoked,
    AttesterStatusReadFailed,
    CompromiseWindow,
    AttesterRemoved,
    AgeExceedsPolicy,
    AttesterSuspended,
    Superseded,
    None,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Qualifier {
    AsOfBundle,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ReadStatus {
    Ok,
    Failed,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum AttesterStatus {
    Active,
    Suspended,
    Removed,
}

/// `default_input` of `verification-model.json`. Unknown fields are
/// rejected so a new model input cannot be silently ignored.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Input {
    pub network_matches: bool,
    pub attestation_read: ReadStatus,
    pub attester_status_read: ReadStatus,
    pub attestation_found: bool,
    pub revoked: bool,
    pub attester_status: AttesterStatus,
    pub attested_in_compromise_window: bool,
    pub age_exceeds_policy: bool,
    pub superseded: bool,
    pub offline: bool,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Outcome {
    pub verdict: Verdict,
    pub reason: Reason,
    pub qualifier: Option<Qualifier>,
}

/// Evaluate the model for one fully-populated input.
pub fn evaluate(i: &Input) -> Outcome {
    let qualifier = i.offline.then_some(Qualifier::AsOfBundle);
    let (verdict, reason) = if !i.network_matches {
        (Verdict::WrongNetwork, Reason::NetworkMismatch)
    } else if i.attestation_read != ReadStatus::Ok {
        (Verdict::Indeterminate, Reason::AttestationReadFailed)
    } else if !i.attestation_found {
        (Verdict::NotFound, Reason::AttestationMissing)
    } else if i.revoked {
        (Verdict::Revoked, Reason::Revoked)
    // Everything below depends on attester state; never guess it.
    } else if i.attester_status_read != ReadStatus::Ok {
        (Verdict::Indeterminate, Reason::AttesterStatusReadFailed)
    } else if i.attested_in_compromise_window {
        (Verdict::AttesterCompromised, Reason::CompromiseWindow)
    } else if i.attester_status == AttesterStatus::Removed {
        (Verdict::AttesterRemoved, Reason::AttesterRemoved)
    } else if i.age_exceeds_policy {
        (Verdict::Expired, Reason::AgeExceedsPolicy)
    } else if i.attester_status == AttesterStatus::Suspended {
        (Verdict::AttesterSuspended, Reason::AttesterSuspended)
    } else if i.superseded {
        (Verdict::Superseded, Reason::Superseded)
    } else {
        (Verdict::Verified, Reason::None)
    };
    Outcome {
        verdict,
        reason,
        qualifier,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::{Map, Value};

    const TABLE: &str = include_str!("../../../docs/specs/verification-model.json");

    #[test]
    fn truth_table() {
        let table: Value = serde_json::from_str(TABLE).unwrap();
        assert_eq!(table["model_version"], MODEL_VERSION);
        let defaults = table["default_input"].as_object().unwrap();
        for case in table["cases"].as_array().unwrap() {
            let mut input: Map<String, Value> = defaults.clone();
            for (k, v) in case["input"].as_object().unwrap() {
                input.insert(k.clone(), v.clone());
            }
            let input: Input = serde_json::from_value(Value::Object(input)).unwrap();
            let got = serde_json::to_value(evaluate(&input)).unwrap();
            assert_eq!(got["verdict"], case["verdict"], "case {}", case["id"]);
            assert_eq!(
                got["qualifier"],
                case.get("qualifier").cloned().unwrap_or(Value::Null),
                "case {}",
                case["id"]
            );
        }
    }
}
