//! Human-readable decoder for Soroban authorization entries (ADR-0007).
//!
//! `multisig-account` does not scope what a signer quorum approves, so every
//! signer must review the decoded authorization tree before signing. This
//! module turns a base64 `SorobanAuthorizationEntry` (or a whole transaction
//! envelope) into a labelled tree, flags risky shapes, and recomputes the
//! payload hash locally so a signer never has to trust a hash they were sent.

use serde::Serialize;
use sha2::{Digest, Sha256};
use std::collections::BTreeMap;
use stellar_xdr::{
    Hash, HashIdPreimage, HashIdPreimageSorobanAuthorization,
    HashIdPreimageSorobanAuthorizationWithAddress, Limits, ReadXdr, ScAddress, ScVal,
    SorobanAddressCredentials, SorobanAuthorizationEntry, SorobanAuthorizedFunction,
    SorobanAuthorizedInvocation, SorobanCredentials, TransactionEnvelope, WriteXdr,
};

/// Invocation trees deeper than this are flagged for extra scrutiny.
pub const MAX_QUIET_DEPTH: usize = 2;

/// Approximate ledger close time, used only for the human "in ~Nh" hint.
const SECONDS_PER_LEDGER: u64 = 5;

/// Functions that move assets on SEP-41 token contracts.
const TOKEN_FUNCTIONS: &[&str] = &[
    "transfer",
    "transfer_from",
    "approve",
    "burn",
    "burn_from",
    "mint",
    "clawback",
];

/// Parameter names of the Lafiya contract entry points, taken from the
/// contract specs in `contracts/*/src/lib.rs`, plus the SEP-41 token
/// interface. Used to label positional `ScVal` arguments.
fn param_names(function: &str) -> Option<&'static [&'static str]> {
    Some(match function {
        "initialize" => &["admin", "attester_registry"],
        "propose_admin" => &["new_admin"],
        "set_attester_registry" => &["new_registry"],
        "attest" => &["attester", "record_hash"],
        "revoke_attestation" | "get_attestation" | "get_attestation_history" => &["record_hash"],
        "add_attester"
        | "remove_attester"
        | "suspend_attester"
        | "reinstate_attester"
        | "is_attester"
        | "get_attester_info"
        | "get_attester_status" => &["attester"],
        "add_attester_with_info" | "update_attester_info" => &["attester", "info"],
        "add_attesters" | "remove_attesters" => &["attesters"],
        "set_max_attesters" => &["max_attesters"],
        "upgrade" => &["new_wasm_hash"],
        "transfer" => &["from", "to", "amount"],
        "transfer_from" => &["spender", "from", "to", "amount"],
        "approve" => &["from", "spender", "amount", "expiration_ledger"],
        "burn" => &["from", "amount"],
        "burn_from" => &["spender", "from", "amount"],
        "mint" => &["to", "amount"],
        "clawback" => &["from", "amount"],
        _ => return None,
    })
}

/// What the decoder knows about the environment the entry will be used in.
#[derive(Debug, Clone, Default)]
pub struct DecodeContext {
    pub network_name: String,
    pub network_passphrase: String,
    /// Known addresses (contract or account strkeys) mapped to a label.
    pub labels: BTreeMap<String, String>,
    /// Current ledger, if known, to turn the expiration ledger into a time hint.
    pub current_ledger: Option<u32>,
}

#[derive(Debug, Clone, Serialize, PartialEq)]
pub struct DecodedArg {
    pub name: String,
    #[serde(rename = "type")]
    pub ty: String,
    pub value: String,
}

#[derive(Debug, Clone, Serialize, PartialEq)]
pub struct DecodedInvocation {
    pub contract: String,
    pub contract_label: Option<String>,
    pub function: String,
    pub args: Vec<DecodedArg>,
    pub sub_invocations: Vec<DecodedInvocation>,
}

#[derive(Debug, Clone, Serialize, PartialEq)]
pub struct AssetMovement {
    pub token: String,
    pub function: String,
    pub args: Vec<DecodedArg>,
}

#[derive(Debug, Clone, Serialize, PartialEq)]
pub struct DecodedEntry {
    pub network: String,
    pub network_name: String,
    /// `None` when the entry authorizes with the transaction source account.
    pub authorizer: Option<String>,
    pub authorizer_label: Option<String>,
    pub nonce: Option<i64>,
    pub expiration_ledger: Option<u32>,
    /// Hex SHA-256 of the `HashIdPreimage` a signer signs, recomputed locally.
    pub payload_hash: Option<String>,
    pub invocation: DecodedInvocation,
    pub asset_movements: Vec<AssetMovement>,
    pub warnings: Vec<String>,
}

/// Decode a base64 XDR `SorobanAuthorizationEntry` or `TransactionEnvelope`
/// into one decoded view per authorization entry.
pub fn decode_input(input: &str, ctx: &DecodeContext) -> anyhow::Result<Vec<DecodedEntry>> {
    let input = input.trim();
    if let Ok(entry) = SorobanAuthorizationEntry::from_xdr_base64(input, Limits::none()) {
        return Ok(vec![decode_entry(&entry, ctx)?]);
    }
    let envelope = TransactionEnvelope::from_xdr_base64(input, Limits::none()).map_err(|e| {
        anyhow::anyhow!(
            "input is neither a SorobanAuthorizationEntry nor a TransactionEnvelope ({e})"
        )
    })?;
    let entries: Vec<_> = envelope
        .auths()
        .map(|entry| decode_entry(entry, ctx))
        .collect::<anyhow::Result<_>>()?;
    if entries.is_empty() {
        anyhow::bail!("transaction contains no Soroban authorization entries");
    }
    Ok(entries)
}

/// Network ID as defined by the protocol: SHA-256 of the passphrase.
pub fn network_id(passphrase: &str) -> [u8; 32] {
    Sha256::digest(passphrase.as_bytes()).into()
}

/// Recompute the payload hash a signer of `entry` signs on `passphrase`.
/// Returns `None` for source-account credentials, which sign the transaction.
pub fn payload_hash(
    entry: &SorobanAuthorizationEntry,
    passphrase: &str,
) -> anyhow::Result<Option<[u8; 32]>> {
    let network_id = Hash(network_id(passphrase));
    let invocation = entry.root_invocation.clone();
    let preimage = match &entry.credentials {
        SorobanCredentials::SourceAccount => return Ok(None),
        SorobanCredentials::Address(c) => {
            HashIdPreimage::SorobanAuthorization(HashIdPreimageSorobanAuthorization {
                network_id,
                nonce: c.nonce,
                signature_expiration_ledger: c.signature_expiration_ledger,
                invocation,
            })
        }
        SorobanCredentials::AddressV2(c) => with_address(network_id, c, invocation),
        SorobanCredentials::AddressWithDelegates(c) => {
            with_address(network_id, &c.address_credentials, invocation)
        }
    };
    let bytes = preimage.to_xdr(Limits::none())?;
    Ok(Some(Sha256::digest(&bytes).into()))
}

fn with_address(
    network_id: Hash,
    c: &SorobanAddressCredentials,
    invocation: SorobanAuthorizedInvocation,
) -> HashIdPreimage {
    HashIdPreimage::SorobanAuthorizationWithAddress(HashIdPreimageSorobanAuthorizationWithAddress {
        network_id,
        nonce: c.nonce,
        signature_expiration_ledger: c.signature_expiration_ledger,
        address: c.address.clone(),
        invocation,
    })
}

fn address_credentials(creds: &SorobanCredentials) -> Option<&SorobanAddressCredentials> {
    match creds {
        SorobanCredentials::SourceAccount => None,
        SorobanCredentials::Address(c) | SorobanCredentials::AddressV2(c) => Some(c),
        SorobanCredentials::AddressWithDelegates(c) => Some(&c.address_credentials),
    }
}

pub fn decode_entry(
    entry: &SorobanAuthorizationEntry,
    ctx: &DecodeContext,
) -> anyhow::Result<DecodedEntry> {
    let creds = address_credentials(&entry.credentials);
    let authorizer = creds.map(|c| c.address.to_string());
    let mut warnings = Vec::new();
    let mut movements = Vec::new();
    let invocation = decode_invocation(
        &entry.root_invocation,
        ctx,
        1,
        &mut warnings,
        &mut movements,
    );
    Ok(DecodedEntry {
        network: ctx.network_passphrase.clone(),
        network_name: ctx.network_name.clone(),
        authorizer_label: authorizer.as_ref().and_then(|a| ctx.labels.get(a).cloned()),
        authorizer,
        nonce: creds.map(|c| c.nonce),
        expiration_ledger: creds.map(|c| c.signature_expiration_ledger),
        payload_hash: payload_hash(entry, &ctx.network_passphrase)?.map(hex::encode),
        invocation,
        asset_movements: movements,
        warnings,
    })
}

fn decode_invocation(
    inv: &SorobanAuthorizedInvocation,
    ctx: &DecodeContext,
    depth: usize,
    warnings: &mut Vec<String>,
    movements: &mut Vec<AssetMovement>,
) -> DecodedInvocation {
    let (contract, function, args) = match &inv.function {
        SorobanAuthorizedFunction::ContractFn(call) => {
            let function = call.function_name.to_string();
            let names = param_names(&function);
            let args = call
                .args
                .iter()
                .enumerate()
                .map(|(i, v)| DecodedArg {
                    name: names
                        .and_then(|n| n.get(i))
                        .map_or_else(|| format!("arg{i}"), |n| n.to_string()),
                    ty: type_name(v).to_string(),
                    value: render_val(v, ctx),
                })
                .collect::<Vec<_>>();
            (call.contract_address.to_string(), function, args)
        }
        SorobanAuthorizedFunction::CreateContractHostFn(_)
        | SorobanAuthorizedFunction::CreateContractV2HostFn(_) => {
            warnings.push("authorizes deploying a new contract (create_contract)".into());
            ("<host>".into(), "create_contract".into(), Vec::new())
        }
    };

    let contract_label = ctx.labels.get(&contract).cloned();
    if contract_label.is_none() && contract != "<host>" {
        warnings.push(format!(
            "UNKNOWN contract {contract} (not in config or --label)"
        ));
    }
    if TOKEN_FUNCTIONS.contains(&function.as_str()) {
        warnings.push(format!("TOKEN MOVEMENT: {function} on {contract}"));
        movements.push(AssetMovement {
            token: contract.clone(),
            function: function.clone(),
            args: args.clone(),
        });
    }
    if depth == MAX_QUIET_DEPTH + 1 {
        warnings.push(format!(
            "DEEP invocation tree: nesting exceeds {MAX_QUIET_DEPTH} levels"
        ));
    }

    let sub_invocations = inv
        .sub_invocations
        .iter()
        .map(|s| decode_invocation(s, ctx, depth + 1, warnings, movements))
        .collect();
    DecodedInvocation {
        contract,
        contract_label,
        function,
        args,
        sub_invocations,
    }
}

fn type_name(v: &ScVal) -> &'static str {
    match v {
        ScVal::Bool(_) => "bool",
        ScVal::Void => "void",
        ScVal::Error(_) => "error",
        ScVal::U32(_) => "u32",
        ScVal::I32(_) => "i32",
        ScVal::U64(_) => "u64",
        ScVal::I64(_) => "i64",
        ScVal::Timepoint(_) => "timepoint",
        ScVal::Duration(_) => "duration",
        ScVal::U128(_) => "u128",
        ScVal::I128(_) => "i128",
        ScVal::U256(_) => "u256",
        ScVal::I256(_) => "i256",
        ScVal::Bytes(b) if b.len() == 32 => "BytesN<32>",
        ScVal::Bytes(_) => "Bytes",
        ScVal::String(_) => "String",
        ScVal::Symbol(_) => "Symbol",
        ScVal::Vec(_) => "Vec",
        ScVal::Map(_) => "Map",
        ScVal::Address(_) => "Address",
        _ => "other",
    }
}

fn render_address(a: &ScAddress, ctx: &DecodeContext) -> String {
    let s = a.to_string();
    match ctx.labels.get(&s) {
        Some(label) => format!("{s} ({label})"),
        None => s,
    }
}

fn render_val(v: &ScVal, ctx: &DecodeContext) -> String {
    match v {
        ScVal::Bool(b) => b.to_string(),
        ScVal::Void => "()".into(),
        ScVal::U32(n) => n.to_string(),
        ScVal::I32(n) => n.to_string(),
        ScVal::U64(n) => n.to_string(),
        ScVal::I64(n) => n.to_string(),
        ScVal::U128(p) => (((p.hi as u128) << 64) | p.lo as u128).to_string(),
        ScVal::I128(p) => (((p.hi as i128) << 64) | p.lo as i128).to_string(),
        ScVal::Bytes(b) => hex::encode(b.as_slice()),
        ScVal::String(s) => format!("{:?}", s.to_string()),
        ScVal::Symbol(s) => s.to_string(),
        ScVal::Address(a) => render_address(a, ctx),
        ScVal::Vec(Some(items)) => format!(
            "[{}]",
            items
                .iter()
                .map(|i| render_val(i, ctx))
                .collect::<Vec<_>>()
                .join(", ")
        ),
        ScVal::Map(Some(entries)) => format!(
            "{{{}}}",
            entries
                .iter()
                .map(|e| format!("{}: {}", render_val(&e.key, ctx), render_val(&e.val, ctx)))
                .collect::<Vec<_>>()
                .join(", ")
        ),
        other => format!("{other:?}"),
    }
}

/// Render decoded entries in the reviewer-facing text format.
pub fn render_text(entries: &[DecodedEntry], ctx: &DecodeContext) -> String {
    let mut out = String::new();
    for (i, e) in entries.iter().enumerate() {
        if entries.len() > 1 {
            out.push_str(&format!(
                "=== Authorization entry {} of {}\n",
                i + 1,
                entries.len()
            ));
        }
        out.push_str(&format!(
            "Network:     {}  (config '{}')\n",
            e.network, e.network_name
        ));
        let authorizer = match (&e.authorizer, &e.authorizer_label) {
            (Some(a), Some(l)) => format!("{a} (known: {l})"),
            (Some(a), None) => format!("{a} (UNKNOWN)"),
            (None, _) => "transaction source account".into(),
        };
        out.push_str(&format!("Authorizer:  {authorizer}\n"));
        if let Some(nonce) = e.nonce {
            out.push_str(&format!("Nonce:       {nonce}\n"));
        }
        if let Some(exp) = e.expiration_ledger {
            let hint = match ctx.current_ledger {
                Some(cur) if exp >= cur => {
                    let secs = u64::from(exp - cur) * SECONDS_PER_LEDGER;
                    format!("  (~{}h{}m from now)", secs / 3600, (secs % 3600) / 60)
                }
                Some(_) => "  (EXPIRED)".into(),
                None => String::new(),
            };
            out.push_str(&format!("Expires:     ledger {exp}{hint}\n"));
        }
        out.push_str("Invocation:\n");
        render_invocation(&e.invocation, "  ", &mut out);
        if e.asset_movements.is_empty() {
            out.push_str("Asset movements: none\n");
        } else {
            out.push_str("Asset movements:\n");
            for m in &e.asset_movements {
                out.push_str(&format!(
                    "  {} {}({})\n",
                    m.token,
                    m.function,
                    args_line(&m.args)
                ));
            }
        }
        match &e.payload_hash {
            Some(h) => out.push_str(&format!("Payload hash:  {h}  (recomputed locally)\n")),
            None => out
                .push_str("Payload hash:  n/a (source-account credentials sign the transaction)\n"),
        }
        for w in &e.warnings {
            out.push_str(&format!("WARNING: {w}\n"));
        }
    }
    out
}

fn args_line(args: &[DecodedArg]) -> String {
    args.iter()
        .map(|a| format!("{}: {} = {}", a.name, a.ty, a.value))
        .collect::<Vec<_>>()
        .join(", ")
}

fn render_invocation(inv: &DecodedInvocation, indent: &str, out: &mut String) {
    let label = inv
        .contract_label
        .as_deref()
        .map_or_else(|| "UNKNOWN".to_string(), |l| format!("known: {l}"));
    out.push_str(&format!("{indent}└─ {} ({label})\n", inv.contract));
    out.push_str(&format!(
        "{indent}     {}({})\n",
        inv.function,
        args_line(&inv.args)
    ));
    if inv.sub_invocations.is_empty() {
        out.push_str(&format!("{indent}     Sub-invocations: none\n"));
    } else {
        let deeper = format!("{indent}     ");
        for sub in &inv.sub_invocations {
            render_invocation(sub, &deeper, out);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use stellar_xdr::{
        ContractId, InvokeContractArgs, ScBytes, ScSymbol, SorobanAddressCredentials, VecM,
    };

    const TESTNET: &str = "Test SDF Network ; September 2015";

    fn contract(byte: u8) -> ScAddress {
        ScAddress::Contract(ContractId(Hash([byte; 32])))
    }

    fn call(
        c: ScAddress,
        f: &str,
        args: Vec<ScVal>,
        subs: Vec<SorobanAuthorizedInvocation>,
    ) -> SorobanAuthorizedInvocation {
        SorobanAuthorizedInvocation {
            function: SorobanAuthorizedFunction::ContractFn(InvokeContractArgs {
                contract_address: c,
                function_name: ScSymbol(f.try_into().unwrap()),
                args: VecM::try_from(args).unwrap(),
            }),
            sub_invocations: VecM::try_from(subs).unwrap(),
        }
    }

    fn entry(inv: SorobanAuthorizedInvocation) -> SorobanAuthorizationEntry {
        SorobanAuthorizationEntry {
            credentials: SorobanCredentials::Address(SorobanAddressCredentials {
                address: contract(0xAA),
                nonce: 42,
                signature_expiration_ledger: 1_234_567,
                signature: ScVal::Void,
            }),
            root_invocation: inv,
        }
    }

    fn ctx() -> DecodeContext {
        let mut labels = BTreeMap::new();
        labels.insert(contract(0xAA).to_string(), "registry-admin multisig".into());
        labels.insert(contract(0x01).to_string(), "attester-registry".into());
        DecodeContext {
            network_name: "testnet".into(),
            network_passphrase: TESTNET.into(),
            labels,
            current_ledger: Some(1_230_247),
        }
    }

    fn golden(name: &str, e: SorobanAuthorizationEntry) {
        let xdr = e.to_xdr_base64(Limits::none()).unwrap();
        let decoded = decode_input(&xdr, &ctx()).unwrap();
        let actual = render_text(&decoded, &ctx());
        let path = format!(
            "{}/tests/golden/auth/{name}.txt",
            env!("CARGO_MANIFEST_DIR")
        );
        if std::env::var_os("UPDATE_GOLDEN").is_some() {
            std::fs::write(&path, &actual).unwrap();
        }
        let expected = std::fs::read_to_string(&path).unwrap();
        assert_eq!(
            actual, expected,
            "golden mismatch for {name}; rerun with UPDATE_GOLDEN=1"
        );
    }

    #[test]
    fn golden_simple_admin_call() {
        golden(
            "admin_call",
            entry(call(
                contract(0x01),
                "add_attester",
                vec![ScVal::Address(contract(0x05))],
                vec![],
            )),
        );
    }

    #[test]
    fn golden_upgrade() {
        let hash = ScVal::Bytes(ScBytes([0x9f; 32].to_vec().try_into().unwrap()));
        golden(
            "upgrade",
            entry(call(contract(0x01), "upgrade", vec![hash], vec![])),
        );
    }

    #[test]
    fn golden_nested_invocation() {
        let leaf = call(contract(0x01), "pause", vec![], vec![]);
        let mid = call(contract(0x02), "execute", vec![], vec![leaf]);
        let root = call(contract(0x03), "proxy", vec![], vec![mid]);
        golden("nested", entry(root));
    }

    #[test]
    fn golden_token_transfer_warns() {
        let amount = ScVal::I128(stellar_xdr::Int128Parts {
            hi: 0,
            lo: 5_000_000,
        });
        let inv = call(
            contract(0x07),
            "transfer",
            vec![
                ScVal::Address(contract(0xAA)),
                ScVal::Address(contract(0x08)),
                amount,
            ],
            vec![],
        );
        let decoded = decode_entry(&entry(inv.clone()), &ctx()).unwrap();
        assert_eq!(decoded.asset_movements.len(), 1);
        assert!(decoded
            .warnings
            .iter()
            .any(|w| w.starts_with("TOKEN MOVEMENT")));
        golden("token_transfer", entry(inv));
    }

    /// Fixtures produced by stellar-cli 28.0.0:
    /// `stellar xdr encode --type HashIdPreimage < preimage.json | base64 -d | sha256sum`
    /// See tests/golden/auth/README.md for the exact commands.
    #[test]
    fn payload_hash_matches_stellar_cli_fixture() {
        let fixture: serde_json::Value = serde_json::from_str(include_str!(
            "../tests/golden/auth/payload_hash_fixture.json"
        ))
        .unwrap();
        let e = SorobanAuthorizationEntry::from_xdr_base64(
            fixture["entry_xdr"].as_str().unwrap(),
            Limits::none(),
        )
        .unwrap();
        let hash = payload_hash(&e, fixture["network_passphrase"].as_str().unwrap())
            .unwrap()
            .unwrap();
        assert_eq!(hex::encode(hash), fixture["payload_hash"].as_str().unwrap());
    }

    #[test]
    fn source_account_credentials_have_no_payload_hash() {
        let mut e = entry(call(contract(0x01), "pause", vec![], vec![]));
        e.credentials = SorobanCredentials::SourceAccount;
        assert_eq!(payload_hash(&e, TESTNET).unwrap(), None);
    }

    #[test]
    fn json_output_is_machine_readable() {
        let e = entry(call(
            contract(0x01),
            "add_attester",
            vec![ScVal::Address(contract(0x05))],
            vec![],
        ));
        let decoded = decode_entry(&e, &ctx()).unwrap();
        let json = serde_json::to_value(&decoded).unwrap();
        assert_eq!(json["invocation"]["function"], "add_attester");
        assert_eq!(json["invocation"]["args"][0]["name"], "attester");
        assert_eq!(json["invocation"]["args"][0]["type"], "Address");
    }

    #[test]
    fn rejects_garbage_input() {
        assert!(decode_input("not-xdr", &ctx()).is_err());
    }
}
