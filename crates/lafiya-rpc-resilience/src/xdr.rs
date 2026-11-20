//! Ledger bounds for real Stellar transaction envelopes (issue #407).
//!
//! [`apply_ledger_bounds`] takes a base64 `TransactionEnvelope` (as produced
//! by `stellar contract invoke --build-only` followed by `stellar tx
//! simulate`) and returns it with:
//!
//! - `PreconditionsV2.ledgerBounds.maxLedger` set, preserving any existing
//!   time bounds, so that after `maxLedger` closes the transaction can never
//!   be included; and
//! - every address-credential Soroban auth entry's
//!   `signature_expiration_ledger` set to the same ledger, so a signed
//!   authorization can't outlive the transaction it was built for.
//!
//! It also returns the [`TxBounds`] that
//! [`FailoverClient::submit_with_bounds`](crate::FailoverClient::submit_with_bounds)
//! needs to resolve the outcome deterministically.

use crate::TxBounds;
use std::fmt;
use stellar_xdr::{
    ExtensionPoint, LedgerBounds, LedgerEntry, LedgerKey, Limits, MuxedAccount, Operation,
    OperationBody, Preconditions, PreconditionsV2, ReadXdr, RestoreFootprintOp, SorobanCredentials,
    SorobanTransactionData, SorobanTransactionDataExt, Transaction, TransactionEnvelope,
    TransactionExt, TransactionV1Envelope, Uint256, VecM, WriteXdr,
};

/// Why an envelope could not be bounded.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum BoundsError {
    /// The input is not a valid base64 `TransactionEnvelope`.
    Decode(String),
    /// Only V1 (`ENVELOPE_TYPE_TX`) envelopes are built by the CLI; V0 and
    /// fee-bump envelopes are refused rather than silently left unbounded.
    UnsupportedEnvelope,
    /// The envelope already carries signatures, which changing its
    /// preconditions would invalidate. Bound it before signing.
    AlreadySigned,
    /// Re-encoding failed.
    Encode(String),
    /// The restore preamble could not be parsed or was internally inconsistent.
    RestorePreamble(String),
}

impl fmt::Display for BoundsError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            BoundsError::Decode(e) => write!(f, "invalid transaction envelope: {e}"),
            BoundsError::UnsupportedEnvelope => {
                write!(f, "only V1 transaction envelopes can be bounded")
            }
            BoundsError::AlreadySigned => {
                write!(
                    f,
                    "envelope is already signed; apply ledger bounds before signing"
                )
            }
            BoundsError::Encode(e) => write!(f, "could not encode envelope: {e}"),
            BoundsError::RestorePreamble(e) => write!(f, "invalid restore preamble: {e}"),
        }
    }
}

impl std::error::Error for BoundsError {}

/// Set `maxLedger` (and matching auth-entry signature expiry) on an
/// unsigned envelope. Returns the new base64 envelope and its bounds.
pub fn apply_ledger_bounds(
    envelope_b64: &str,
    max_ledger: u32,
) -> Result<(String, TxBounds), BoundsError> {
    let mut envelope = TransactionEnvelope::from_xdr_base64(envelope_b64.trim(), Limits::none())
        .map_err(|e| BoundsError::Decode(e.to_string()))?;

    let TransactionEnvelope::Tx(v1) = &mut envelope else {
        return Err(BoundsError::UnsupportedEnvelope);
    };
    if !v1.signatures.is_empty() {
        return Err(BoundsError::AlreadySigned);
    }
    let tx = &mut v1.tx;

    let mut v2 = match &tx.cond {
        Preconditions::None => PreconditionsV2::default(),
        Preconditions::Time(time_bounds) => PreconditionsV2 {
            time_bounds: Some(time_bounds.clone()),
            ..PreconditionsV2::default()
        },
        Preconditions::V2(existing) => existing.clone(),
    };
    let min_ledger = v2.ledger_bounds.as_ref().map_or(0, |b| b.min_ledger);
    v2.ledger_bounds = Some(LedgerBounds {
        min_ledger,
        max_ledger,
    });
    tx.cond = Preconditions::V2(v2);

    for op in tx.operations.iter_mut() {
        let OperationBody::InvokeHostFunction(invoke) = &mut op.body else {
            continue;
        };
        for entry in invoke.auth.iter_mut() {
            match &mut entry.credentials {
                SorobanCredentials::Address(creds) | SorobanCredentials::AddressV2(creds) => {
                    creds.signature_expiration_ledger = max_ledger;
                }
                SorobanCredentials::AddressWithDelegates(creds) => {
                    creds.address_credentials.signature_expiration_ledger = max_ledger;
                }
                SorobanCredentials::SourceAccount => {}
            }
        }
    }

    let source_account = match &tx.source_account {
        MuxedAccount::Ed25519(key) => MuxedAccount::Ed25519(key.clone()),
        MuxedAccount::MuxedEd25519(muxed) => MuxedAccount::Ed25519(Uint256(muxed.ed25519.0)),
    }
    .to_string();
    let bounds = TxBounds {
        max_ledger,
        source_account,
        sequence: tx.seq_num.0,
    };

    let encoded = envelope
        .to_xdr_base64(Limits::none())
        .map_err(|e| BoundsError::Encode(e.to_string()))?;
    Ok((encoded, bounds))
}

/// The `restorePreamble` reported by `simulateTransaction` when a simulation
/// touches archived persistent entries.
///
/// When present, the real transaction cannot succeed until the listed
/// entries are restored, so clients must submit a `RestoreFootprintOp` (or
/// rely on protocol auto-restore) first.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RestorePreamble {
    /// The `SorobanTransactionData` (base64 XDR) the restore transaction must
    /// carry, including the footprint of the archived entries and the
    /// resource fee the network quoted for the restore.
    pub transaction_data_b64: String,
    /// The minimum resource fee (in stroops) the network quoted for the
    /// restore transaction.
    pub min_resource_fee: u64,
}

impl RestorePreamble {
    /// Parse a `restorePreamble` object from a JSON simulation result.
    ///
    /// Accepts the raw JSON value as returned by `simulateTransaction`:
    /// `{ "transactionData": "<base64>", "minResourceFee": "<stroops>" }`.
    /// Returns `Ok(None)` when the field is absent or `null`.
    pub fn from_json(value: &serde_json::Value) -> Result<Option<Self>, BoundsError> {
        let Some(obj) = value.get("restorePreamble") else {
            return Ok(None);
        };
        if obj.is_null() {
            return Ok(None);
        }
        let transaction_data_b64 = obj
            .get("transactionData")
            .and_then(|v| v.as_str())
            .ok_or_else(|| {
                BoundsError::RestorePreamble("missing `transactionData`".to_string())
            })?
            .to_string();
        let min_resource_fee = match obj.get("minResourceFee") {
            None | Some(serde_json::Value::Null) => 0,
            Some(serde_json::Value::String(s)) => s.trim().parse::<u64>().map_err(|e| {
                BoundsError::RestorePreamble(format!("invalid `minResourceFee`: {e}"))
            })?,
            Some(serde_json::Value::Number(n)) => n.as_u64().ok_or_else(|| {
                BoundsError::RestorePreamble("`minResourceFee` is not a u64".to_string())
            })?,
            Some(_) => {
                return Err(BoundsError::RestorePreamble(
                    "`minResourceFee` must be a string or number".to_string(),
                ))
            }
        };
        Ok(Some(Self {
            transaction_data_b64,
            min_resource_fee,
        }))
    }

    /// Decode the quoted `SorobanTransactionData`.
    pub fn transaction_data(&self) -> Result<SorobanTransactionData, BoundsError> {
        SorobanTransactionData::from_xdr_base64(&self.transaction_data_b64, Limits::none())
            .map_err(|e| BoundsError::RestorePreamble(format!("invalid transaction data: {e}")))
    }

    /// The archived ledger keys the restore transaction must cover.
    pub fn archived_keys(&self) -> Result<Vec<LedgerKey>, BoundsError> {
        let data = self.transaction_data()?;
        let read_only = data.resources.footprint.read_only.to_vec();
        let read_write = data.resources.footprint.read_write.to_vec();
        Ok(read_only.into_iter().chain(read_write).collect())
    }
}

/// Build an unsigned `RestoreFootprintOp` transaction envelope (base64 XDR)
/// from a simulation's [`RestorePreamble`].
///
/// The returned envelope has no signatures and no ledger bounds; callers
/// should apply [`apply_ledger_bounds`] before signing. `fee` is the total
/// transaction fee in stroops (typically `min_resource_fee` plus a base fee
/// and inclusion margin).
pub fn build_restore_footprint_tx(
    preamble: &RestorePreamble,
    source_account: MuxedAccount,
    seq_num: i64,
    fee: u32,
) -> Result<String, BoundsError> {
    let data = preamble.transaction_data()?;
    let op = Operation {
        source_account: None,
        body: OperationBody::RestoreFootprint(RestoreFootprintOp {
            ext: ExtensionPoint::V0,
        }),
    };
    let tx = Transaction {
        source_account,
        fee,
        seq_num: stellar_xdr::SequenceNumber(seq_num),
        cond: Preconditions::None,
        memo: stellar_xdr::Memo::None,
        operations: vec![op]
            .try_into()
            .map_err(|e: stellar_xdr::xdr::Error| BoundsError::RestorePreamble(e.to_string()))?,
        ext: TransactionExt::V1(data),
    };
    TransactionEnvelope::Tx(TransactionV1Envelope {
        tx,
        signatures: VecM::default(),
    })
    .to_xdr_base64(Limits::none())
    .map_err(|e| BoundsError::Encode(e.to_string()))
}

/// Build an unsigned auto-restore transaction envelope (base64 XDR).
///
/// On protocols that support automatic restoration, the real transaction can
/// carry the restore footprint itself: the same `SorobanTransactionData` the
/// simulation quoted is attached to the original operations, and the network
/// restores the archived entries before executing them. `envelope_b64` must be
/// an unsigned V1 envelope; its existing `SorobanTransactionData` (if any) is
/// replaced by the preamble's.
pub fn apply_auto_restore(
    envelope_b64: &str,
    preamble: &RestorePreamble,
) -> Result<String, BoundsError> {
    let mut envelope = TransactionEnvelope::from_xdr_base64(envelope_b64.trim(), Limits::none())
        .map_err(|e| BoundsError::Decode(e.to_string()))?;
    let TransactionEnvelope::Tx(v1) = &mut envelope else {
        return Err(BoundsError::UnsupportedEnvelope);
    };
    if !v1.signatures.is_empty() {
        return Err(BoundsError::AlreadySigned);
    }
    let data = preamble.transaction_data()?;
    v1.tx.ext = TransactionExt::V1(data);
    envelope
        .to_xdr_base64(Limits::none())
        .map_err(|e| BoundsError::Encode(e.to_string()))
}

/// Read the `SorobanTransactionData` attached to an envelope, if any.
pub fn soroban_data_of(
    envelope_b64: &str,
) -> Result<Option<SorobanTransactionData>, BoundsError> {
    let envelope = TransactionEnvelope::from_xdr_base64(envelope_b64.trim(), Limits::none())
        .map_err(|e| BoundsError::Decode(e.to_string()))?;
    let TransactionEnvelope::Tx(v1) = envelope else {
        return Err(BoundsError::UnsupportedEnvelope);
    };
    Ok(match v1.tx.ext {
        TransactionExt::V0 => None,
        TransactionExt::V1(data) => Some(data),
    })
}

/// Read `maxLedger` back out of an envelope, if it has one.
pub fn max_ledger_of(envelope_b64: &str) -> Result<Option<u32>, BoundsError> {
    let envelope = TransactionEnvelope::from_xdr_base64(envelope_b64.trim(), Limits::none())
        .map_err(|e| BoundsError::Decode(e.to_string()))?;
    let cond = match &envelope {
        TransactionEnvelope::Tx(v1) => &v1.tx.cond,
        _ => return Err(BoundsError::UnsupportedEnvelope),
    };
    Ok(match cond {
        Preconditions::V2(v2) => v2.ledger_bounds.as_ref().map(|b| b.max_ledger),
        _ => None,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use stellar_xdr::{
        ContractId, Hash, HostFunction, InvokeContractArgs, InvokeHostFunctionOp, Memo, Operation,
        ScAddress, ScSymbol, ScVal, SequenceNumber, SorobanAddressCredentials,
        SorobanAuthorizationEntry, SorobanAuthorizedFunction, SorobanAuthorizedInvocation,
        TimeBounds, TimePoint, Transaction, TransactionExt, TransactionV1Envelope, VecM,
    };

    fn envelope(cond: Preconditions, auth: Vec<SorobanAuthorizationEntry>) -> String {
        let call = InvokeContractArgs {
            contract_address: ScAddress::Contract(ContractId(Hash([1; 32]))),
            function_name: ScSymbol("add_attester".try_into().unwrap()),
            args: VecM::default(),
        };
        let op = Operation {
            source_account: None,
            body: OperationBody::InvokeHostFunction(InvokeHostFunctionOp {
                host_function: HostFunction::InvokeContract(call),
                auth: auth.try_into().unwrap(),
            }),
        };
        TransactionEnvelope::Tx(TransactionV1Envelope {
            tx: Transaction {
                source_account: MuxedAccount::Ed25519(Uint256([7; 32])),
                fee: 100,
                seq_num: SequenceNumber(4242),
                cond,
                memo: Memo::None,
                operations: vec![op].try_into().unwrap(),
                ext: TransactionExt::V0,
            },
            signatures: VecM::default(),
        })
        .to_xdr_base64(Limits::none())
        .unwrap()
    }

    fn address_auth() -> SorobanAuthorizationEntry {
        SorobanAuthorizationEntry {
            credentials: SorobanCredentials::Address(SorobanAddressCredentials {
                address: ScAddress::Contract(ContractId(Hash([2; 32]))),
                nonce: 1,
                signature_expiration_ledger: 0,
                signature: ScVal::Void,
            }),
            root_invocation: SorobanAuthorizedInvocation {
                function: SorobanAuthorizedFunction::ContractFn(InvokeContractArgs {
                    contract_address: ScAddress::Contract(ContractId(Hash([1; 32]))),
                    function_name: ScSymbol("add_attester".try_into().unwrap()),
                    args: VecM::default(),
                }),
                sub_invocations: VecM::default(),
            },
        }
    }

    fn decode(b64: &str) -> Transaction {
        match TransactionEnvelope::from_xdr_base64(b64, Limits::none()).unwrap() {
            TransactionEnvelope::Tx(v1) => v1.tx,
            _ => unreachable!(),
        }
    }

    #[test]
    fn sets_max_ledger_and_auth_expiry() {
        let input = envelope(Preconditions::None, vec![address_auth()]);
        assert_eq!(max_ledger_of(&input).unwrap(), None);

        let (out, bounds) = apply_ledger_bounds(&input, 1_060).unwrap();

        let tx = decode(&out);
        let Preconditions::V2(v2) = &tx.cond else {
            panic!("expected PreconditionsV2, got {:?}", tx.cond)
        };
        assert_eq!(
            v2.ledger_bounds,
            Some(LedgerBounds {
                min_ledger: 0,
                max_ledger: 1_060
            })
        );
        let OperationBody::InvokeHostFunction(invoke) = &tx.operations[0].body else {
            unreachable!()
        };
        let SorobanCredentials::Address(creds) = &invoke.auth[0].credentials else {
            unreachable!()
        };
        assert_eq!(creds.signature_expiration_ledger, 1_060);

        assert_eq!(max_ledger_of(&out).unwrap(), Some(1_060));
        assert_eq!(bounds.max_ledger, 1_060);
        assert_eq!(bounds.sequence, 4242);
        assert!(bounds.source_account.starts_with('G'));
    }

    #[test]
    fn preserves_existing_time_bounds() {
        let time = TimeBounds {
            min_time: TimePoint(0),
            max_time: TimePoint(1_700_000_000),
        };
        let input = envelope(Preconditions::Time(time.clone()), vec![]);
        let (out, _) = apply_ledger_bounds(&input, 500).unwrap();
        let Preconditions::V2(v2) = decode(&out).cond else {
            panic!("expected PreconditionsV2")
        };
        assert_eq!(v2.time_bounds, Some(time));
    }

    fn sample_preamble() -> RestorePreamble {
        let key = LedgerKey::ContractData(stellar_xdr::LedgerKeyContractData {
            contract: ScAddress::Contract(ContractId(Hash([1; 32]))),
            key: ScVal::LedgerKeyContractInstance,
            durability: stellar_xdr::ContractDataDurability::Persistent,
        });
        let data = SorobanTransactionData {
            ext: SorobanTransactionDataExt::V0,
            resources: stellar_xdr::SorobanResources {
                footprint: stellar_xdr::LedgerFootprint {
                    read_only: vec![key].try_into().unwrap(),
                    read_write: VecM::default(),
                },
                instructions: 0,
                disk_read_bytes: 0,
                write_bytes: 0,
            },
            resource_fee: 12_345,
        };
        RestorePreamble {
            transaction_data_b64: data.to_xdr_base64(Limits::none()).unwrap(),
            min_resource_fee: 12_345,
        }
    }

    #[test]
    fn parses_restore_preamble_from_simulation_json() {
        let preamble = sample_preamble();
        let json = serde_json::json!({
            "restorePreamble": {
                "transactionData": preamble.transaction_data_b64,
                "minResourceFee": "12345",
            }
        });
        let parsed = RestorePreamble::from_json(&json).unwrap().unwrap();
        assert_eq!(parsed, preamble);
        assert_eq!(parsed.archived_keys().unwrap().len(), 1);

        assert_eq!(
            RestorePreamble::from_json(&serde_json::json!({})).unwrap(),
            None
        );
        assert_eq!(
            RestorePreamble::from_json(&serde_json::json!({ "restorePreamble": null })).unwrap(),
            None
        );
        assert!(matches!(
            RestorePreamble::from_json(&serde_json::json!({ "restorePreamble": {} })),
            Err(BoundsError::RestorePreamble(_))
        ));
    }

    #[test]
    fn builds_restore_footprint_transaction() {
        let preamble = sample_preamble();
        let b64 = build_restore_footprint_tx(
            &preamble,
            MuxedAccount::Ed25519(Uint256([7; 32])),
            99,
            20_000,
        )
        .unwrap();
        let env = TransactionEnvelope::from_xdr_base64(&b64, Limits::none()).unwrap();
        let TransactionEnvelope::Tx(v1) = env else {
            unreachable!()
        };
        assert!(v1.signatures.is_empty());
        assert_eq!(v1.tx.seq_num.0, 99);
        assert_eq!(v1.tx.fee, 20_000);
        assert!(matches!(
            v1.tx.operations[0].body,
            OperationBody::RestoreFootprint(_)
        ));
        let TransactionExt::V1(data) = &v1.tx.ext else {
            panic!("expected SorobanTransactionData")
        };
        assert_eq!(data.resource_fee, 12_345);

        // Bounds can be applied to the restore transaction like any other.
        let (bounded, bounds) = apply_ledger_bounds(&b64, 777).unwrap();
        assert_eq!(bounds.max_ledger, 777);
        assert_eq!(max_ledger_of(&bounded).unwrap(), Some(777));
    }

    #[test]
    fn applies_auto_restore_to_unsigned_envelope() {
        let preamble = sample_preamble();
        let input = envelope(Preconditions::None, vec![]);
        assert!(soroban_data_of(&input).unwrap().is_none());

        let out = apply_auto_restore(&input, &preamble).unwrap();
        let data = soroban_data_of(&out).unwrap().unwrap();
        assert_eq!(data.resource_fee, 12_345);
        assert_eq!(data.resources.footprint.read_only.len(), 1);

        let mut env = TransactionEnvelope::from_xdr_base64(&input, Limits::none()).unwrap();
        if let TransactionEnvelope::Tx(v1) = &mut env {
            v1.signatures = vec![Default::default()].try_into().unwrap();
        }
        let signed = env.to_xdr_base64(Limits::none()).unwrap();
        assert_eq!(
            apply_auto_restore(&signed, &preamble),
            Err(BoundsError::AlreadySigned)
        );
    }

    #[test]
    fn refuses_signed_envelopes_and_garbage() {
        assert!(matches!(
            apply_ledger_bounds("not-xdr", 1),
            Err(BoundsError::Decode(_))
        ));

        let mut env = TransactionEnvelope::from_xdr_base64(
            envelope(Preconditions::None, vec![]),
            Limits::none(),
        )
        .unwrap();
        if let TransactionEnvelope::Tx(v1) = &mut env {
            v1.signatures = vec![Default::default()].try_into().unwrap();
        }
        let signed = env.to_xdr_base64(Limits::none()).unwrap();
        assert_eq!(
            apply_ledger_bounds(&signed, 1),
            Err(BoundsError::AlreadySigned)
        );
    }
}
