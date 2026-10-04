//! Minimal Soroban RPC client: `getEvents`, `getLatestLedger`, and the
//! contract-instance wasm hash via `getLedgerEntries`.

use crate::Event;
use serde_json::{json, Value};
use stellar_xdr::{
    ContractDataDurability, ContractExecutable, LedgerEntryData, LedgerKey, LedgerKeyContractData,
    Limits, ReadXdr, ScAddress, ScVal, WriteXdr,
};

pub struct Rpc {
    url: String,
}

impl Rpc {
    pub fn new(url: impl Into<String>) -> Rpc {
        Rpc { url: url.into() }
    }

    fn call(&self, method: &str, params: Value) -> anyhow::Result<Value> {
        let mut resp = ureq::post(&self.url).send_json(json!({
            "jsonrpc": "2.0", "id": 1, "method": method, "params": params,
        }))?;
        let body: Value = resp.body_mut().read_json()?;
        if let Some(err) = body.get("error") {
            anyhow::bail!("{method}: {err}");
        }
        Ok(body["result"].clone())
    }

    pub fn latest_ledger(&self) -> anyhow::Result<u32> {
        let r = self.call("getLatestLedger", json!({}))?;
        r["sequence"]
            .as_u64()
            .map(|s| s as u32)
            .ok_or_else(|| anyhow::anyhow!("getLatestLedger: missing sequence"))
    }

    /// Events for `contracts` in `[start, end)` (end = latest when `None`),
    /// following pagination. Returns the events and the latest ledger.
    pub fn events(
        &self,
        contracts: &[String],
        start: u32,
        end: Option<u32>,
    ) -> anyhow::Result<(Vec<Event>, u32)> {
        let mut out = Vec::new();
        let mut cursor: Option<String> = None;
        loop {
            let mut params = json!({
                "filters": [{ "type": "contract", "contractIds": contracts }],
                "pagination": { "limit": 100 },
            });
            match &cursor {
                Some(c) => params["pagination"]["cursor"] = json!(c),
                None => {
                    params["startLedger"] = json!(start);
                    if let Some(end) = end {
                        params["endLedger"] = json!(end);
                    }
                }
            }
            let r = self.call("getEvents", params)?;
            let page = r["events"].as_array().cloned().unwrap_or_default();
            for raw in &page {
                if let Some(e) = decode_event(raw) {
                    out.push(e);
                }
            }
            let latest = r["latestLedger"].as_u64().unwrap_or(0) as u32;
            if page.len() < 100 {
                return Ok((out, latest));
            }
            cursor = r["cursor"].as_str().map(str::to_string);
            if cursor.is_none() {
                return Ok((out, latest));
            }
        }
    }

    /// The wasm hash a contract instance currently runs.
    pub fn contract_wasm_hash(&self, contract: &str) -> anyhow::Result<String> {
        let address: ScAddress = contract.parse()?;
        let key = LedgerKey::ContractData(LedgerKeyContractData {
            contract: address,
            key: ScVal::LedgerKeyContractInstance,
            durability: ContractDataDurability::Persistent,
        });
        let r = self.call(
            "getLedgerEntries",
            json!({ "keys": [key.to_xdr_base64(Limits::none())?] }),
        )?;
        let xdr = r["entries"][0]["xdr"]
            .as_str()
            .ok_or_else(|| anyhow::anyhow!("contract instance {contract} not found"))?;
        match LedgerEntryData::from_xdr_base64(xdr, Limits::none())? {
            LedgerEntryData::ContractData(d) => match d.val {
                ScVal::ContractInstance(i) => match i.executable {
                    ContractExecutable::Wasm(h) => Ok(hex::encode(h.0)),
                    ContractExecutable::StellarAsset => Ok("stellar-asset".into()),
                },
                _ => anyhow::bail!("unexpected instance value for {contract}"),
            },
            _ => anyhow::bail!("unexpected ledger entry for {contract}"),
        }
    }
}

/// Render an `ScVal` compactly for alert text.
pub fn render(v: &ScVal) -> String {
    match v {
        ScVal::Symbol(s) => s.to_string(),
        ScVal::Address(a) => a.to_string(),
        ScVal::Bytes(b) => hex::encode(b.as_slice()),
        ScVal::String(s) => s.to_string(),
        ScVal::U32(n) => n.to_string(),
        ScVal::I32(n) => n.to_string(),
        ScVal::U64(n) => n.to_string(),
        ScVal::I64(n) => n.to_string(),
        ScVal::Bool(b) => b.to_string(),
        ScVal::Void => String::new(),
        ScVal::Vec(Some(items)) => items.iter().map(render).collect::<Vec<_>>().join(", "),
        other => format!("{other:?}"),
    }
}

/// Decode one `getEvents` item. Returns `None` for events without a symbol
/// name topic (not emitted by Lafiya contracts).
pub fn decode_event(raw: &Value) -> Option<Event> {
    let topics: Vec<ScVal> = raw["topic"]
        .as_array()?
        .iter()
        .filter_map(|t| ScVal::from_xdr_base64(t.as_str()?, Limits::none()).ok())
        .collect();
    let name = match topics.first()? {
        ScVal::Symbol(s) => s.to_string(),
        _ => return None,
    };
    let data = raw["value"]
        .as_str()
        .and_then(|v| ScVal::from_xdr_base64(v, Limits::none()).ok())
        .map(|v| render(&v))
        .unwrap_or_default();
    Some(Event {
        ledger: raw["ledger"].as_u64()? as u32,
        contract: raw["contractId"].as_str()?.to_string(),
        name,
        topics: topics[1..].iter().map(render).collect(),
        data,
        tx_hash: raw["txHash"].as_str().map(str::to_string),
        closed_at: raw["ledgerClosedAt"].as_str().map(str::to_string),
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use stellar_xdr::{ScBytes, ScSymbol};

    #[test]
    fn decodes_upgraded_event() {
        let name = ScVal::Symbol(ScSymbol("upgraded".try_into().unwrap()));
        let hash = ScVal::Bytes(ScBytes(vec![0xab; 32].try_into().unwrap()));
        let raw = json!({
            "ledger": 42,
            "contractId": "CDLZFC3SYJYDZT7K67VZ75HPJVIEUVNIXF47ZG2FB2RMQQVU2HHGCYSC",
            "topic": [name.to_xdr_base64(Limits::none()).unwrap(), hash.to_xdr_base64(Limits::none()).unwrap()],
            "value": ScVal::Void.to_xdr_base64(Limits::none()).unwrap(),
            "txHash": "ff".repeat(32),
        });
        let e = decode_event(&raw).unwrap();
        assert_eq!(e.name, "upgraded");
        assert_eq!(e.topics, vec!["ab".repeat(32)]);
        assert_eq!(e.ledger, 42);
    }
}
