//! Minimal Soroban JSON-RPC client: `getEvents`, `getHealth`, and read-only
//! contract calls via `simulateTransaction`.

use std::future::Future;
use std::str::FromStr;

use serde::de::DeserializeOwned;
use serde::Deserialize;
use serde_json::{json, Value};
use stellar_xdr::{
    HostFunction, InvokeContractArgs, InvokeHostFunctionOp, Limits, Memo, MuxedAccount, Operation,
    OperationBody, Preconditions, ScAddress, ScSymbol, ScVal, SequenceNumber, Transaction,
    TransactionEnvelope, TransactionExt, TransactionV1Envelope, Uint256, WriteXdr,
};

use crate::decode::{parse_scval, RawEvent};

#[derive(Debug, thiserror::Error)]
pub enum RpcError {
    #[error("rpc transport: {0}")]
    Transport(#[from] reqwest::Error),
    #[error("rpc error {code}: {message}")]
    Rpc { code: i64, message: String },
    #[error("rpc response: {0}")]
    Response(String),
}

impl RpcError {
    /// Whether RPC rejected the request because the requested ledger or
    /// cursor is outside its retention window.
    pub fn is_out_of_retention(&self) -> bool {
        matches!(self, Self::Rpc { message, .. }
            if message.contains("oldest ledger") || message.contains("ledger range"))
    }
}

/// Where a `getEvents` page starts.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Start {
    Ledger(u32),
    Cursor(String),
}

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct EventPage {
    pub events: Vec<RawEvent>,
    pub cursor: String,
    pub latest_ledger: u32,
}

#[derive(Debug, Clone, Copy, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct LedgerWindow {
    pub oldest_ledger: u32,
    pub latest_ledger: u32,
}

/// Source of contract events; implemented by [`RpcClient`] and by test mocks.
pub trait EventSource {
    fn ledger_window(&self) -> impl Future<Output = Result<LedgerWindow, RpcError>> + Send;
    fn get_events(
        &self,
        start: &Start,
        limit: u32,
    ) -> impl Future<Output = Result<EventPage, RpcError>> + Send;
}

#[derive(Clone)]
pub struct RpcClient {
    http: reqwest::Client,
    url: String,
    contract_ids: Vec<String>,
}

impl RpcClient {
    pub fn new(url: impl Into<String>, contract_ids: Vec<String>) -> Self {
        Self {
            http: reqwest::Client::builder()
                .timeout(std::time::Duration::from_secs(30))
                .build()
                .expect("static reqwest config"),
            url: url.into(),
            contract_ids,
        }
    }

    async fn call<T: DeserializeOwned>(&self, method: &str, params: Value) -> Result<T, RpcError> {
        let mut body = json!({ "jsonrpc": "2.0", "id": 1, "method": method });
        if !params.is_null() {
            body["params"] = params;
        }
        let resp: Value = self
            .http
            .post(&self.url)
            .json(&body)
            .send()
            .await?
            .error_for_status()?
            .json()
            .await?;
        if let Some(err) = resp.get("error") {
            return Err(RpcError::Rpc {
                code: err.get("code").and_then(Value::as_i64).unwrap_or_default(),
                message: err
                    .get("message")
                    .and_then(Value::as_str)
                    .unwrap_or_default()
                    .into(),
            });
        }
        serde_json::from_value(resp.get("result").cloned().unwrap_or(Value::Null))
            .map_err(|e| RpcError::Response(format!("{method}: {e}")))
    }

    /// Call a read-only contract function by simulating it from the null
    /// account (as the JS SDK does for read calls) and return its result.
    pub async fn read_contract(
        &self,
        contract_id: &str,
        function: &str,
        args: Vec<ScVal>,
    ) -> Result<ScVal, RpcError> {
        let bad = |e: String| RpcError::Response(e);
        let op = InvokeHostFunctionOp {
            host_function: HostFunction::InvokeContract(InvokeContractArgs {
                contract_address: ScAddress::from_str(contract_id)
                    .map_err(|e| bad(format!("contract id {contract_id}: {e}")))?,
                function_name: ScSymbol(
                    function
                        .try_into()
                        .map_err(|_| bad("function name".into()))?,
                ),
                args: args.try_into().map_err(|_| bad("too many args".into()))?,
            }),
            auth: Default::default(),
        };
        let tx = Transaction {
            source_account: MuxedAccount::Ed25519(Uint256([0; 32])),
            fee: 100,
            seq_num: SequenceNumber(0),
            cond: Preconditions::None,
            memo: Memo::None,
            operations: vec![Operation {
                source_account: None,
                body: OperationBody::InvokeHostFunction(op),
            }]
            .try_into()
            .expect("one operation"),
            ext: TransactionExt::V0,
        };
        let envelope = TransactionEnvelope::Tx(TransactionV1Envelope {
            tx,
            signatures: Default::default(),
        })
        .to_xdr_base64(Limits::none())
        .map_err(|e| bad(e.to_string()))?;

        #[derive(Deserialize)]
        struct SimResult {
            xdr: String,
        }
        #[derive(Deserialize)]
        struct Sim {
            error: Option<String>,
            results: Option<Vec<SimResult>>,
        }
        let sim: Sim = self
            .call("simulateTransaction", json!({ "transaction": envelope }))
            .await?;
        if let Some(error) = sim.error {
            return Err(bad(format!("{function} simulation failed: {error}")));
        }
        let result = sim
            .results
            .and_then(|r| r.into_iter().next())
            .ok_or_else(|| bad(format!("{function}: simulation returned no result")))?;
        parse_scval(&result.xdr).map_err(bad)
    }
}

impl EventSource for RpcClient {
    async fn ledger_window(&self) -> Result<LedgerWindow, RpcError> {
        self.call("getHealth", Value::Null).await
    }

    async fn get_events(&self, start: &Start, limit: u32) -> Result<EventPage, RpcError> {
        let filters = json!([{ "type": "contract", "contractIds": self.contract_ids }]);
        let params = match start {
            Start::Ledger(l) => json!({
                "startLedger": l,
                "filters": filters,
                "pagination": { "limit": limit },
            }),
            Start::Cursor(c) => json!({
                "filters": filters,
                "pagination": { "cursor": c, "limit": limit },
            }),
        };
        self.call("getEvents", params).await
    }
}
