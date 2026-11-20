//! Runtime interface negotiation against a deployed contract.
//!
//! Every Lafiya contract exposes `get_interface() -> InterfaceInfo`. The CLI
//! calls it once per session and refuses to operate, with a clear message,
//! when the contract is of the wrong kind or older than this CLI requires.
//! `contract_kind` is a weak identity check, not proof of authenticity.

use serde::Deserialize;

/// Arguments for the `restore` subcommand: re-fund (restore) an archived
/// attestation entry so proofs over it become readable again.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RestoreArgs {
    /// Hex-encoded record hash of the archived attestation.
    pub record_hash: String,
    /// Optional ledger horizon to extend the entry's TTL to. When `None` the
    /// contract applies its default restoration horizon.
    pub extend_to: Option<u32>,
}

impl RestoreArgs {
    /// Parse `restore <record_hash> [--extend-to <ledger>]`.
    pub fn parse(args: &[String]) -> Result<Self, String> {
        let mut record_hash: Option<String> = None;
        let mut extend_to: Option<u32> = None;
        let mut i = 0;
        while i < args.len() {
            let arg = &args[i];
            if arg == "--extend-to" {
                let value = args
                    .get(i + 1)
                    .ok_or_else(|| "--extend-to requires a ledger value".to_string())?;
                let parsed = value
                    .parse::<u32>()
                    .map_err(|e| format!("invalid --extend-to value {value:?}: {e}"))?;
                extend_to = Some(parsed);
                i += 2;
                continue;
            }
            if arg.starts_with("--") {
                return Err(format!("unknown flag {arg}"));
            }
            if record_hash.is_some() {
                return Err(format!("unexpected extra argument {arg}"));
            }
            record_hash = Some(arg.clone());
            i += 1;
        }
        let record_hash = record_hash.ok_or_else(|| "restore requires <record_hash>".to_string())?;
        Ok(Self {
            record_hash,
            extend_to,
        })
    }
}

/// Arguments for the `dispute` subcommand: a patient-signed flag over an
/// attestation commitment. The signature uses the QR-spec patient scheme with
/// domain separation; the CLI only transports it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DisputeArgs {
    /// Hex-encoded record hash being disputed.
    pub record_hash: String,
    /// Hex-encoded patient signer public key.
    pub signer: String,
    /// Hex-encoded signature over the domain-separated dispute commitment.
    pub signature: String,
}

impl DisputeArgs {
    /// Parse `dispute <record_hash> --signer <key> --signature <sig>`.
    pub fn parse(args: &[String]) -> Result<Self, String> {
        let mut record_hash: Option<String> = None;
        let mut signer: Option<String> = None;
        let mut signature: Option<String> = None;
        let mut i = 0;
        while i < args.len() {
            let arg = &args[i];
            let mut take_value = |name: &str| -> Result<String, String> {
                let value = args
                    .get(i + 1)
                    .ok_or_else(|| format!("{name} requires a value"))?;
                Ok(value.clone())
            };
            match arg.as_str() {
                "--signer" => {
                    signer = Some(take_value("--signer")?);
                    i += 2;
                }
                "--signature" => {
                    signature = Some(take_value("--signature")?);
                    i += 2;
                }
                other if other.starts_with("--") => {
                    return Err(format!("unknown flag {other}"));
                }
                other => {
                    if record_hash.is_some() {
                        return Err(format!("unexpected extra argument {other}"));
                    }
                    record_hash = Some(other.to_string());
                    i += 1;
                }
            }
        }
        let record_hash =
            record_hash.ok_or_else(|| "dispute requires <record_hash>".to_string())?;
        let signer = signer.ok_or_else(|| "dispute requires --signer <key>".to_string())?;
        let signature =
            signature.ok_or_else(|| "dispute requires --signature <sig>".to_string())?;
        Ok(Self {
            record_hash,
            signer,
            signature,
        })
    }
}

/// Decoded `InterfaceInfo` as printed (JSON) by `stellar contract invoke`.
#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
pub struct InterfaceInfo {
    pub contract_kind: String,
    pub interface_version: u32,
    pub features: Vec<String>,
    pub schema_version: u32,
    pub event_version: u32,
}

impl InterfaceInfo {
    #[allow(dead_code)]
    pub fn supports(&self, feature: &str) -> bool {
        self.features.iter().any(|f| f == feature)
    }
}

/// What this CLI build needs from a contract of a given kind.
#[derive(Debug, Clone, Copy)]
pub struct Requirement {
    pub contract_kind: &'static str,
    pub min_interface_version: u32,
}

pub const ATTESTER_REGISTRY: Requirement = Requirement {
    contract_kind: "lafiya_attester_registry",
    min_interface_version: 1,
};

pub const ATTESTATION_REGISTRY: Requirement = Requirement {
    contract_kind: "lafiya_attestation_registry",
    min_interface_version: 1,
};

pub const INCENTIVE_POOL: Requirement = Requirement {
    contract_kind: "lafiya_incentive_pool",
    min_interface_version: 1,
};

pub const FACILITY_REGISTRY: Requirement = Requirement {
    contract_kind: "lafiya_facility_registry",
    min_interface_version: 1,
};

/// Deactivation reason codes accepted by `facility deactivate --reason`.
///
/// Mirrors the contract's `deactivation_reason` codes. `fraud` additionally
/// marks attestations made at the facility as `facility_untrusted`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DeactivationReason {
    Closed,
    LicenceLapsed,
    Fraud,
}

impl DeactivationReason {
    /// Wire code stored and emitted by the contract.
    pub fn code(self) -> &'static str {
        match self {
            DeactivationReason::Closed => "closed",
            DeactivationReason::LicenceLapsed => "licence_lapsed",
            DeactivationReason::Fraud => "fraud",
        }
    }

    /// Parse a wire code (also accepts the hyphenated spelling).
    pub fn parse(value: &str) -> Result<Self, String> {
        match value {
            "closed" => Ok(DeactivationReason::Closed),
            "licence_lapsed" | "licence-lapsed" => Ok(DeactivationReason::LicenceLapsed),
            "fraud" => Ok(DeactivationReason::Fraud),
            other => Err(format!(
                "invalid --reason {other:?}; expected one of closed, licence_lapsed, fraud"
            )),
        }
    }
}

/// Arguments for `facility enroll`: register a facility under a portable ID.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FacilityEnrollArgs {
    /// Hex-encoded 32-byte portable `FacilityId`.
    pub facility_id: String,
    /// Jurisdiction code used in the canonical ID derivation.
    pub jurisdiction: String,
    /// Issuing authority code used in the canonical ID derivation.
    pub issuing_authority: String,
    /// External facility code (e.g. national registry code or licence number).
    pub facility_code: String,
    /// Hex-encoded operator/admin public key authorizing enrollment.
    pub operator: String,
}

impl FacilityEnrollArgs {
    /// Parse `facility enroll <facility_id> --jurisdiction <j> --issuing-authority <a>
    /// --facility-code <c> --operator <key>`.
    pub fn parse(args: &[String]) -> Result<Self, String> {
        let mut facility_id: Option<String> = None;
        let mut jurisdiction: Option<String> = None;
        let mut issuing_authority: Option<String> = None;
        let mut facility_code: Option<String> = None;
        let mut operator: Option<String> = None;
        let mut i = 0;
        while i < args.len() {
            let arg = &args[i];
            let mut take_value = |name: &str| -> Result<String, String> {
                args.get(i + 1)
                    .cloned()
                    .ok_or_else(|| format!("{name} requires a value"))
            };
            match arg.as_str() {
                "--jurisdiction" => {
                    jurisdiction = Some(take_value("--jurisdiction")?);
                    i += 2;
                }
                "--issuing-authority" => {
                    issuing_authority = Some(take_value("--issuing-authority")?);
                    i += 2;
                }
                "--facility-code" => {
                    facility_code = Some(take_value("--facility-code")?);
                    i += 2;
                }
                "--operator" => {
                    operator = Some(take_value("--operator")?);
                    i += 2;
                }
                other if other.starts_with("--") => {
                    return Err(format!("unknown flag {other}"));
                }
                other => {
                    if facility_id.is_some() {
                        return Err(format!("unexpected extra argument {other}"));
                    }
                    facility_id = Some(other.to_string());
                    i += 1;
                }
            }
        }
        let facility_id =
            facility_id.ok_or_else(|| "facility enroll requires <facility_id>".to_string())?;
        let jurisdiction = jurisdiction
            .ok_or_else(|| "facility enroll requires --jurisdiction <j>".to_string())?;
        let issuing_authority = issuing_authority
            .ok_or_else(|| "facility enroll requires --issuing-authority <a>".to_string())?;
        let facility_code = facility_code
            .ok_or_else(|| "facility enroll requires --facility-code <c>".to_string())?;
        let operator =
            operator.ok_or_else(|| "facility enroll requires --operator <key>".to_string())?;
        Ok(Self {
            facility_id,
            jurisdiction,
            issuing_authority,
            facility_code,
            operator,
        })
    }
}

/// Arguments for `facility deactivate`: reason code plus effective timestamp.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FacilityDeactivateArgs {
    /// Hex-encoded 32-byte portable `FacilityId`.
    pub facility_id: String,
    /// Deactivation reason code.
    pub reason: DeactivationReason,
    /// Ledger timestamp (seconds) at which the deactivation takes effect.
    pub effective_at: u64,
    /// Hex-encoded operator/admin public key authorizing the change.
    pub operator: String,
}

impl FacilityDeactivateArgs {
    /// Parse `facility deactivate <facility_id> --reason <code> --effective-at <ts>
    /// --operator <key>`.
    pub fn parse(args: &[String]) -> Result<Self, String> {
        let mut facility_id: Option<String> = None;
        let mut reason: Option<DeactivationReason> = None;
        let mut effective_at: Option<u64> = None;
        let mut operator: Option<String> = None;
        let mut i = 0;
        while i < args.len() {
            let arg = &args[i];
            let mut take_value = |name: &str| -> Result<String, String> {
                args.get(i + 1)
                    .cloned()
                    .ok_or_else(|| format!("{name} requires a value"))
            };
            match arg.as_str() {
                "--reason" => {
                    reason = Some(DeactivationReason::parse(&take_value("--reason")?)?);
                    i += 2;
                }
                "--effective-at" => {
                    let value = take_value("--effective-at")?;
                    let parsed = value
                        .parse::<u64>()
                        .map_err(|e| format!("invalid --effective-at value {value:?}: {e}"))?;
                    effective_at = Some(parsed);
                    i += 2;
                }
                "--operator" => {
                    operator = Some(take_value("--operator")?);
                    i += 2;
                }
                other if other.starts_with("--") => {
                    return Err(format!("unknown flag {other}"));
                }
                other => {
                    if facility_id.is_some() {
                        return Err(format!("unexpected extra argument {other}"));
                    }
                    facility_id = Some(other.to_string());
                    i += 1;
                }
            }
        }
        let facility_id =
            facility_id.ok_or_else(|| "facility deactivate requires <facility_id>".to_string())?;
        let reason =
            reason.ok_or_else(|| "facility deactivate requires --reason <code>".to_string())?;
        let effective_at = effective_at
            .ok_or_else(|| "facility deactivate requires --effective-at <ts>".to_string())?;
        let operator =
            operator.ok_or_else(|| "facility deactivate requires --operator <key>".to_string())?;
        Ok(Self {
            facility_id,
            reason,
            effective_at,
            operator,
        })
    }
}

/// Arguments for `facility reactivate`: clear a prior deactivation.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FacilityReactivateArgs {
    /// Hex-encoded 32-byte portable `FacilityId`.
    pub facility_id: String,
    /// Hex-encoded operator/admin public key authorizing the change.
    pub operator: String,
}

impl FacilityReactivateArgs {
    /// Parse `facility reactivate <facility_id> --operator <key>`.
    pub fn parse(args: &[String]) -> Result<Self, String> {
        let mut facility_id: Option<String> = None;
        let mut operator: Option<String> = None;
        let mut i = 0;
        while i < args.len() {
            let arg = &args[i];
            match arg.as_str() {
                "--operator" => {
                    operator = Some(
                        args.get(i + 1)
                            .cloned()
                            .ok_or_else(|| "--operator requires a value".to_string())?,
                    );
                    i += 2;
                }
                other if other.starts_with("--") => {
                    return Err(format!("unknown flag {other}"));
                }
                other => {
                    if facility_id.is_some() {
                        return Err(format!("unexpected extra argument {other}"));
                    }
                    facility_id = Some(other.to_string());
                    i += 1;
                }
            }
        }
        let facility_id =
            facility_id.ok_or_else(|| "facility reactivate requires <facility_id>".to_string())?;
        let operator =
            operator.ok_or_else(|| "facility reactivate requires --operator <key>".to_string())?;
        Ok(Self {
            facility_id,
            operator,
        })
    }
}

/// Arguments for `facility affiliate` / `facility disaffiliate`: link an
/// attester to a facility.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FacilityAffiliationArgs {
    /// Hex-encoded 32-byte portable `FacilityId`.
    pub facility_id: String,
    /// Hex-encoded attester public key being affiliated/disaffiliated.
    pub attester: String,
    /// Hex-encoded operator/admin public key authorizing the change.
    pub operator: String,
}

impl FacilityAffiliationArgs {
    /// Parse `<facility_id> --attester <key> --operator <key>`.
    pub fn parse(args: &[String], command: &str) -> Result<Self, String> {
        let mut facility_id: Option<String> = None;
        let mut attester: Option<String> = None;
        let mut operator: Option<String> = None;
        let mut i = 0;
        while i < args.len() {
            let arg = &args[i];
            let mut take_value = |name: &str| -> Result<String, String> {
                args.get(i + 1)
                    .cloned()
                    .ok_or_else(|| format!("{name} requires a value"))
            };
            match arg.as_str() {
                "--attester" => {
                    attester = Some(take_value("--attester")?);
                    i += 2;
                }
                "--operator" => {
                    operator = Some(take_value("--operator")?);
                    i += 2;
                }
                other if other.starts_with("--") => {
                    return Err(format!("unknown flag {other}"));
                }
                other => {
                    if facility_id.is_some() {
                        return Err(format!("unexpected extra argument {other}"));
                    }
                    facility_id = Some(other.to_string());
                    i += 1;
                }
            }
        }
        let facility_id = facility_id
            .ok_or_else(|| format!("facility {command} requires <facility_id>"))?;
        let attester = attester
            .ok_or_else(|| format!("facility {command} requires --attester <key>"))?;
        let operator = operator
            .ok_or_else(|| format!("facility {command} requires --operator <key>"))?;
        Ok(Self {
            facility_id,
            attester,
            operator,
        })
    }
}

/// Arguments for `facility show`: read a facility record.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FacilityShowArgs {
    /// Hex-encoded 32-byte portable `FacilityId`.
    pub facility_id: String,
}

impl FacilityShowArgs {
    /// Parse `facility show <facility_id>`.
    pub fn parse(args: &[String]) -> Result<Self, String> {
        let mut facility_id: Option<String> = None;
        for arg in args {
            if arg.starts_with("--") {
                return Err(format!("unknown flag {arg}"));
            }
            if facility_id.is_some() {
                return Err(format!("unexpected extra argument {arg}"));
            }
            facility_id = Some(arg.clone());
        }
        let facility_id =
            facility_id.ok_or_else(|| "facility show requires <facility_id>".to_string())?;
        Ok(Self { facility_id })
    }
}

/// Arguments for `facility list-affiliations`: list attesters linked to a
/// facility, with optional pagination.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FacilityListAffiliationsArgs {
    /// Hex-encoded 32-byte portable `FacilityId`.
    pub facility_id: String,
    /// Optional page cursor (opaque, contract-defined).
    pub cursor: Option<String>,
    /// Optional page size.
    pub limit: Option<u32>,
}

impl FacilityListAffiliationsArgs {
    /// Parse `facility list-affiliations <facility_id> [--cursor <c>] [--limit <n>]`.
    pub fn parse(args: &[String]) -> Result<Self, String> {
        let mut facility_id: Option<String> = None;
        let mut cursor: Option<String> = None;
        let mut limit: Option<u32> = None;
        let mut i = 0;
        while i < args.len() {
            let arg = &args[i];
            let mut take_value = |name: &str| -> Result<String, String> {
                args.get(i + 1)
                    .cloned()
                    .ok_or_else(|| format!("{name} requires a value"))
            };
            match arg.as_str() {
                "--cursor" => {
                    cursor = Some(take_value("--cursor")?);
                    i += 2;
                }
                "--limit" => {
                    let value = take_value("--limit")?;
                    let parsed = value
                        .parse::<u32>()
                        .map_err(|e| format!("invalid --limit value {value:?}: {e}"))?;
                    limit = Some(parsed);
                    i += 2;
                }
                other if other.starts_with("--") => {
                    return Err(format!("unknown flag {other}"));
                }
                other => {
                    if facility_id.is_some() {
                        return Err(format!("unexpected extra argument {other}"));
                    }
                    facility_id = Some(other.to_string());
                    i += 1;
                }
            }
        }
        let facility_id = facility_id
            .ok_or_else(|| "facility list-affiliations requires <facility_id>".to_string())?;
        Ok(Self {
            facility_id,
            cursor,
            limit,
        })
    }
}

/// Simulation preview for a state-changing facility command.
///
/// The CLI runs the invocation through `stellar contract invoke --simulate`
/// first and prints this shape; `--execute` then submits for real.
#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
pub struct FacilitySimulationPreview {
    /// Whether the simulated invocation would succeed.
    pub success: bool,
    /// Human-readable error when `success` is false.
    #[serde(default)]
    pub error: Option<String>,
    /// Estimated resource fee in stroops, when reported.
    #[serde(default)]
    pub fee_stroops: Option<u64>,
    /// Decoded return value of the simulated call, when any.
    #[serde(default)]
    pub result: Option<serde_json::Value>,
}

/// JSON output shape for `facility show`.
#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
pub struct FacilityRecord {
    /// Hex-encoded 32-byte portable `FacilityId`.
    pub facility_id: String,
    /// Legacy sequential u64 alias, retained for compatibility.
    #[serde(default)]
    pub legacy_id: Option<u64>,
    pub jurisdiction: String,
    pub issuing_authority: String,
    pub facility_code: String,
    pub operator: String,
    pub active: bool,
    /// Reason code when inactive (`closed`, `licence_lapsed`, `fraud`).
    #[serde(default)]
    pub deactivation_reason: Option<String>,
    /// Ledger timestamp of the deactivation, when inactive.
    #[serde(default)]
    pub deactivated_at: Option<u64>,
}

/// JSON output shape for `facility list-affiliations`.
#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
pub struct FacilityAffiliations {
    /// Hex-encoded attester public keys currently affiliated.
    pub attesters: Vec<String>,
    /// Opaque cursor for the next page, when more results exist.
    #[serde(default)]
    pub next_cursor: Option<String>,
}

/// Check the raw `get_interface` output against `req`.
///
/// `raw` is `None` when the call failed, which means the contract predates
/// `get_interface` (interface v0).
pub fn negotiate(
    contract_id: &str,
    raw: Option<&str>,
    req: Requirement,
) -> Result<InterfaceInfo, String> {
    let Some(raw) = raw else {
        return Err(format!(
            "contract {contract_id} does not implement get_interface (interface v0); \
             this CLI requires {} >= v{}",
            req.contract_kind, req.min_interface_version
        ));
    };
    let info: InterfaceInfo = serde_json::from_str(raw.trim()).map_err(|e| {
        format!("contract {contract_id} returned an unreadable get_interface result: {e}")
    })?;
    if info.contract_kind != req.contract_kind {
        return Err(format!(
            "contract {contract_id} is a {}, expected {}",
            info.contract_kind, req.contract_kind
        ));
    }
    if info.interface_version < req.min_interface_version {
        return Err(format!(
            "contract {contract_id} is interface v{}; this CLI requires >= v{}",
            info.interface_version, req.min_interface_version
        ));
    }
    Ok(info)
}

#[cfg(test)]
mod tests {
    use super::*;

    const NEW: &str = r#"{"contract_kind":"lafiya_attester_registry","interface_version":1,
        "features":["pause","roles"],"schema_version":2,"event_version":1}"#;

    #[test]
    fn accepts_current_contract_and_reports_features() {
        let info = negotiate("CDX", Some(NEW), ATTESTER_REGISTRY).unwrap();
        assert_eq!(info.interface_version, 1);
        assert!(info.supports("pause"));
        assert!(!info.supports("attest_many"));
    }

    #[test]
    fn rejects_old_contract_without_get_interface() {
        let err = negotiate("CDX", None, ATTESTER_REGISTRY).unwrap_err();
        assert!(err.contains("interface v0"), "{err}");
    }

    #[test]
    fn rejects_older_interface_version() {
        let req = Requirement {
            min_interface_version: 3,
            ..ATTESTER_REGISTRY
        };
        let err = negotiate("CDX", Some(NEW), req).unwrap_err();
        assert_eq!(err, "contract CDX is interface v1; this CLI requires >= v3");
    }

    #[test]
    fn rejects_wrong_contract_kind() {
        let err = negotiate("CDX", Some(NEW), ATTESTATION_REGISTRY).unwrap_err();
        assert!(
            err.contains("expected lafiya_attestation_registry"),
            "{err}"
        );
    }

    #[test]
    fn rejects_garbage_output() {
        let err = negotiate("CDX", Some("not json"), ATTESTER_REGISTRY).unwrap_err();
        assert!(err.contains("unreadable"), "{err}");
    }

    fn argv(items: &[&str]) -> Vec<String> {
        items.iter().map(|s| s.to_string()).collect()
    }

    #[test]
    fn parses_restore_with_and_without_horizon() {
        let bare = RestoreArgs::parse(&argv(&["abcd"])).unwrap();
        assert_eq!(bare.record_hash, "abcd");
        assert_eq!(bare.extend_to, None);

        let with = RestoreArgs::parse(&argv(&["abcd", "--extend-to", "1000"])).unwrap();
        assert_eq!(with.record_hash, "abcd");
        assert_eq!(with.extend_to, Some(1000));
    }

    #[test]
    fn restore_rejects_missing_hash_and_bad_horizon() {
        assert!(RestoreArgs::parse(&argv(&[])).unwrap_err().contains("record_hash"));
        assert!(RestoreArgs::parse(&argv(&["abcd", "--extend-to", "x"]))
            .unwrap_err()
            .contains("invalid --extend-to"));
        assert!(RestoreArgs::parse(&argv(&["abcd", "--bogus"]))
            .unwrap_err()
            .contains("unknown flag"));
    }

    #[test]
    fn parses_dispute_with_signer_and_signature() {
        let args = DisputeArgs::parse(&argv(&[
            "abcd",
            "--signer",
            "key",
            "--signature",
            "sig",
        ]))
        .unwrap();
        assert_eq!(args.record_hash, "abcd");
        assert_eq!(args.signer, "key");
        assert_eq!(args.signature, "sig");
    }

    #[test]
    fn dispute_requires_signer_and_signature() {
        assert!(DisputeArgs::parse(&argv(&["abcd"]))
            .unwrap_err()
            .contains("--signer"));
        assert!(DisputeArgs::parse(&argv(&["abcd", "--signer", "key"]))
            .unwrap_err()
            .contains("--signature"));
    }

    #[test]
    fn accepts_facility_registry_contract() {
        let raw = r#"{"contract_kind":"lafiya_facility_registry","interface_version":1,
            "features":["portable_ids"],"schema_version":1,"event_version":1}"#;
        let info = negotiate("CDX", Some(raw), FACILITY_REGISTRY).unwrap();
        assert!(info.supports("portable_ids"));
    }

    #[test]
    fn deactivation_reason_round_trips() {
        for reason in [
            DeactivationReason::Closed,
            DeactivationReason::LicenceLapsed,
            DeactivationReason::Fraud,
        ] {
            assert_eq!(DeactivationReason::parse(reason.code()).unwrap(), reason);
        }
        assert_eq!(
            DeactivationReason::parse("licence-lapsed").unwrap(),
            DeactivationReason::LicenceLapsed
        );
        assert!(DeactivationReason::parse("bogus")
            .unwrap_err()
            .contains("invalid --reason"));
    }

    #[test]
    fn parses_facility_enroll() {
        let args = FacilityEnrollArgs::parse(&argv(&[
            "ab",
            "--jurisdiction",
            "NG",
            "--issuing-authority",
            "FMOH",
            "--facility-code",
            "F-001",
            "--operator",
            "key",
        ]))
        .unwrap();
        assert_eq!(args.facility_id, "ab");
        assert_eq!(args.jurisdiction, "NG");
        assert_eq!(args.issuing_authority, "FMOH");
        assert_eq!(args.facility_code, "F-001");
        assert_eq!(args.operator, "key");
    }

    #[test]
    fn facility_enroll_requires_all_fields() {
        assert!(FacilityEnrollArgs::parse(&argv(&["ab"]))
            .unwrap_err()
            .contains("--jurisdiction"));
        assert!(FacilityEnrollArgs::parse(&argv(&["ab", "--jurisdiction", "NG"]))
            .unwrap_err()
            .contains("--issuing-authority"));
    }

    #[test]
    fn parses_facility_deactivate() {
        let args = FacilityDeactivateArgs::parse(&argv(&[
            "ab",
            "--reason",
            "fraud",
            "--effective-at",
            "1700",
            "--operator",
            "key",
        ]))
        .unwrap();
        assert_eq!(args.facility_id, "ab");
        assert_eq!(args.reason, DeactivationReason::Fraud);
        assert_eq!(args.effective_at, 1700);
        assert_eq!(args.operator, "key");
    }

    #[test]
    fn facility_deactivate_rejects_bad_reason_and_timestamp() {
        assert!(FacilityDeactivateArgs::parse(&argv(&[
            "ab",
            "--reason",
            "bogus",
            "--effective-at",
            "1",
            "--operator",
            "k",
        ]))
        .unwrap_err()
        .contains("invalid --reason"));
        assert!(FacilityDeactivateArgs::parse(&argv(&[
            "ab",
            "--reason",
            "closed",
            "--effective-at",
            "x",
            "--operator",
            "k",
        ]))
        .unwrap_err()
        .contains("invalid --effective-at"));
    }

    #[test]
    fn parses_facility_reactivate() {
        let args =
            FacilityReactivateArgs::parse(&argv(&["ab", "--operator", "key"])).unwrap();
        assert_eq!(args.facility_id, "ab");
        assert_eq!(args.operator, "key");
        assert!(FacilityReactivateArgs::parse(&argv(&["ab"]))
            .unwrap_err()
            .contains("--operator"));
    }

    #[test]
    fn parses_facility_affiliation() {
        let args = FacilityAffiliationArgs::parse(
            &argv(&["ab", "--attester", "att", "--operator", "key"]),
            "affiliate",
        )
        .unwrap();
        assert_eq!(args.facility_id, "ab");
        assert_eq!(args.attester, "att");
        assert_eq!(args.operator, "key");
        assert!(FacilityAffiliationArgs::parse(&argv(&["ab"]), "disaffiliate")
            .unwrap_err()
            .contains("--attester"));
    }

    #[test]
    fn parses_facility_show_and_list_affiliations() {
        let show = FacilityShowArgs::parse(&argv(&["ab"])).unwrap();
        assert_eq!(show.facility_id, "ab");
        assert!(FacilityShowArgs::parse(&argv(&[])).unwrap_err().contains("facility_id"));

        let list = FacilityListAffiliationsArgs::parse(&argv(&[
            "ab", "--cursor", "c1", "--limit", "10",
        ]))
        .unwrap();
        assert_eq!(list.facility_id, "ab");
        assert_eq!(list.cursor.as_deref(), Some("c1"));
        assert_eq!(list.limit, Some(10));
        assert!(FacilityListAffiliationsArgs::parse(&argv(&["ab", "--limit", "x"]))
            .unwrap_err()
            .contains("invalid --limit"));
    }

    #[test]
    fn decodes_facility_record_and_affiliations() {
        let record: FacilityRecord = serde_json::from_str(
            r#"{"facility_id":"ab","legacy_id":7,"jurisdiction":"NG",
                "issuing_authority":"FMOH","facility_code":"F-001","operator":"key",
                "active":false,"deactivation_reason":"fraud","deactivated_at":1700}"#,
        )
        .unwrap();
        assert_eq!(record.legacy_id, Some(7));
        assert_eq!(record.deactivation_reason.as_deref(), Some("fraud"));
        assert_eq!(record.deactivated_at, Some(1700));

        let affiliations: FacilityAffiliations =
            serde_json::from_str(r#"{"attesters":["a","b"],"next_cursor":"c2"}"#).unwrap();
        assert_eq!(affiliations.attesters.len(), 2);
        assert_eq!(affiliations.next_cursor.as_deref(), Some("c2"));
    }

    #[test]
    fn decodes_simulation_preview() {
        let preview: FacilitySimulationPreview =
            serde_json::from_str(r#"{"success":true,"fee_stroops":100}"#).unwrap();
        assert!(preview.success);
        assert_eq!(preview.fee_stroops, Some(100));
        assert_eq!(preview.result, None);
    }
}
