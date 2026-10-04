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
    LedgerBounds, Limits, MuxedAccount, OperationBody, Preconditions, PreconditionsV2, ReadXdr,
    SorobanCredentials, TransactionEnvelope, Uint256, WriteXdr,
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
