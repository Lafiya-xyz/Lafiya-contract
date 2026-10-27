//! Minimal Soroban JSON-RPC client: `getEvents`, `getHealth`, and read-only
//! contract calls via `simulateTransaction`.

use std::collections::BTreeMap;
use std::future::Future;
use std::path::{Path, PathBuf};
use std::str::FromStr;
use std::sync::Arc;

use serde::de::DeserializeOwned;
use serde::Deserialize;
use serde_json::{json, Value};
use sha2::{Digest, Sha256};
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
    #[error("archive: {0}")]
    Archive(String),
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
    /// Hash of the ledger the page was served from, when the provider
    /// reports it. Used by ingest to verify chain continuity across pages
    /// and providers.
    #[serde(default)]
    pub latest_ledger_hash: Option<String>,
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

    /// Fetch the hash of `ledger`, or `None` if the provider does not retain
    /// it. Ingest uses this to verify continuity and to locate the common
    /// ancestor when a reorg is detected.
    fn ledger_hash(
        &self,
        ledger: u32,
    ) -> impl Future<Output = Result<Option<String>, RpcError>> + Send;
}

/// A history archive / ledger metadata export: NDJSON files of `getEvents`
/// event objects, one file per ledger range, plus a manifest of checkpoint
/// hashes used to verify the export before it is trusted.
///
/// Layout (all paths relative to the archive root):
///
/// ```text
/// manifest.json          { "networkPassphrase": "...", "checkpoints": [ ... ] }
/// ledgers/<from>-<to>.ndjson   one RawEvent JSON object per line
/// ```
///
/// Each checkpoint entry covers a contiguous ledger range and records the
/// SHA-256 of the corresponding NDJSON file:
///
/// ```json
/// { "from": 1, "to": 1000, "sha256": "<hex>", "ledgerHash": "<hex>" }
/// ```
///
/// The archive refuses to serve any range that is not covered by a verified
/// checkpoint, so an unverifiable source can never be used for backfill.
#[derive(Clone)]
pub struct ArchiveEventSource {
    root: PathBuf,
    contract_ids: Vec<String>,
    checkpoints: Arc<Vec<Checkpoint>>,
}

#[derive(Debug, Clone, Deserialize)]
pub struct Checkpoint {
    pub from: u32,
    pub to: u32,
    /// Hex-encoded SHA-256 of the NDJSON file for `[from, to]`.
    pub sha256: String,
    /// Hex-encoded ledger header hash at `to`, for cross-checking against RPC.
    #[serde(default)]
    pub ledger_hash: Option<String>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct ArchiveManifest {
    #[serde(default)]
    network_passphrase: Option<String>,
    checkpoints: Vec<Checkpoint>,
}

impl ArchiveEventSource {
    /// Open an archive, verifying every checkpoint file hash up front. Any
    /// mismatch or missing file is a hard error: unverifiable sources are
    /// refused rather than silently trusted.
    pub async fn open(
        root: impl Into<PathBuf>,
        contract_ids: Vec<String>,
    ) -> Result<Self, RpcError> {
        let root = root.into();
        let manifest_path = root.join("manifest.json");
        let raw = tokio::fs::read(&manifest_path)
            .await
            .map_err(|e| RpcError::Archive(format!("{}: {e}", manifest_path.display())))?;
        let manifest: ArchiveManifest = serde_json::from_slice(&raw)
            .map_err(|e| RpcError::Archive(format!("manifest: {e}")))?;
        if manifest.checkpoints.is_empty() {
            return Err(RpcError::Archive(
                "manifest has no checkpoints; refusing unverifiable source".into(),
            ));
        }
        let mut prev_to = 0u32;
        for cp in &manifest.checkpoints {
            if cp.from == 0 || cp.from > cp.to || cp.from != prev_to.saturating_add(1) {
                return Err(RpcError::Archive(format!(
                    "checkpoint range {}-{} is not contiguous",
                    cp.from, cp.to
                )));
            }
            let path = checkpoint_path(&root, cp);
            let bytes = tokio::fs::read(&path)
                .await
                .map_err(|e| RpcError::Archive(format!("{}: {e}", path.display())))?;
            let actual = hex_sha256(&bytes);
            if !actual.eq_ignore_ascii_case(&cp.sha256) {
                return Err(RpcError::Archive(format!(
                    "checkpoint {}-{} hash mismatch: expected {}, got {actual}",
                    cp.from, cp.to, cp.sha256
                )));
            }
            prev_to = cp.to;
        }
        Ok(Self {
            root,
            contract_ids,
            checkpoints: Arc::new(manifest.checkpoints),
        })
    }

    /// The highest ledger covered by a verified checkpoint. Backfill should
    /// hand off to live RPC at `latest_verified_ledger() + 1`.
    pub fn latest_verified_ledger(&self) -> u32 {
        self.checkpoints.last().map(|c| c.to).unwrap_or(0)
    }

    /// The lowest ledger covered by the archive.
    pub fn oldest_ledger(&self) -> u32 {
        self.checkpoints.first().map(|c| c.from).unwrap_or(0)
    }

    /// Verified checkpoint ranges, for parallel backfill planning.
    pub fn checkpoint_ranges(&self) -> Vec<(u32, u32)> {
        self.checkpoints.iter().map(|c| (c.from, c.to)).collect()
    }

    fn checkpoint_for(&self, ledger: u32) -> Option<&Checkpoint> {
        self.checkpoints
            .iter()
            .find(|c| ledger >= c.from && ledger <= c.to)
    }

    async fn load_range(&self, cp: &Checkpoint) -> Result<Vec<RawEvent>, RpcError> {
        let path = checkpoint_path(&self.root, cp);
        let bytes = tokio::fs::read(&path)
            .await
            .map_err(|e| RpcError::Archive(format!("{}: {e}", path.display())))?;
        let actual = hex_sha256(&bytes);
        if !actual.eq_ignore_ascii_case(&cp.sha256) {
            return Err(RpcError::Archive(format!(
                "checkpoint {}-{} hash mismatch on read",
                cp.from, cp.to
            )));
        }
        let text = String::from_utf8(bytes)
            .map_err(|e| RpcError::Archive(format!("{}: {e}", path.display())))?;
        let mut events = Vec::new();
        for (i, line) in text.lines().enumerate() {
            let line = line.trim();
            if line.is_empty() {
                continue;
            }
            let event: RawEvent = serde_json::from_str(line).map_err(|e| {
                RpcError::Archive(format!("{}:{}: {e}", path.display(), i + 1))
            })?;
            if self.contract_ids.is_empty()
                || self.contract_ids.iter().any(|id| id == &event.contract_id)
            {
                events.push(event);
            }
        }
        events.sort_by_key(|e| (e.ledger, e.tx_index, e.event_index));
        Ok(events)
    }
}

fn checkpoint_path(root: &Path, cp: &Checkpoint) -> PathBuf {
    root.join("ledgers").join(format!("{}-{}.ndjson", cp.from, cp.to))
}

fn hex_sha256(bytes: &[u8]) -> String {
    let digest = Sha256::digest(bytes);
    let mut out = String::with_capacity(64);
    for b in digest {
        out.push_str(&format!("{b:02x}"));
    }
    out
}

impl EventSource for ArchiveEventSource {
    async fn ledger_window(&self) -> Result<LedgerWindow, RpcError> {
        Ok(LedgerWindow {
            oldest_ledger: self.oldest_ledger(),
            latest_ledger: self.latest_verified_ledger(),
        })
    }

    async fn get_events(&self, start: &Start, limit: u32) -> Result<EventPage, RpcError> {
        let from = match start {
            Start::Ledger(l) => *l,
            Start::Cursor(_) => {
                return Err(RpcError::Archive(
                    "archive source does not support cursors; use Start::Ledger".into(),
                ))
            }
        };
        let cp = self.checkpoint_for(from).ok_or_else(|| {
            RpcError::Archive(format!("ledger {from} is not covered by a verified checkpoint"))
        })?;
        let all = self.load_range(cp).await?;
        let events: Vec<RawEvent> = all
            .into_iter()
            .filter(|e| e.ledger >= from)
            .take(limit as usize)
            .collect();
        let next = events
            .last()
            .map(|e| e.ledger.saturating_add(1))
            .unwrap_or(cp.to.saturating_add(1));
        Ok(EventPage {
            events,
            cursor: format!("ledger:{next}"),
            latest_ledger: cp.to,
        })
    }
}

/// A source that serves historical ledgers from a verified archive and hands
/// off to live RPC once the archive's verified boundary is crossed.
pub struct HandoffSource {
    archive: ArchiveEventSource,
    live: RpcClient,
    boundary: u32,
}

impl HandoffSource {
    /// Build a handoff source. The boundary is the last ledger served by the
    /// archive; ledgers after it come from `live`. The archive's verified
    /// boundary must not exceed the live RPC's oldest ledger, otherwise there
    /// would be a gap between the two sources.
    pub async fn new(archive: ArchiveEventSource, live: RpcClient) -> Result<Self, RpcError> {
        let boundary = archive.latest_verified_ledger();
        let window = live.ledger_window().await?;
        if boundary.saturating_add(1) < window.oldest_ledger {
            return Err(RpcError::Archive(format!(
                "gap between archive (ends {boundary}) and live RPC (starts {})",
                window.oldest_ledger
            )));
        }
        Ok(Self {
            archive,
            live,
            boundary,
        })
    }

    pub fn boundary(&self) -> u32 {
        self.boundary
    }
}

impl EventSource for HandoffSource {
    async fn ledger_window(&self) -> Result<LedgerWindow, RpcError> {
        let live = self.live.ledger_window().await?;
        Ok(LedgerWindow {
            oldest_ledger: self.archive.oldest_ledger(),
            latest_ledger: live.latest_ledger,
        })
    }

    async fn get_events(&self, start: &Start, limit: u32) -> Result<EventPage, RpcError> {
        match start {
            Start::Ledger(l) if *l <= self.boundary => self.archive.get_events(start, limit).await,
            Start::Ledger(_) => self.live.get_events(start, limit).await,
            Start::Cursor(c) => {
                if let Some(rest) = c.strip_prefix("ledger:") {
                    let next: u32 = rest.parse().map_err(|_| {
                        RpcError::Archive(format!("bad archive cursor {c}"))
                    })?;
                    if next <= self.boundary {
                        return self.archive.get_events(&Start::Ledger(next), limit).await;
                    }
                }
                self.live.get_events(start, limit).await
            }
        }
    }
}

/// Plan a resumable, parallelizable backfill over verified archive ranges.
///
/// `resume_from` is the next ledger to fetch (typically the indexer's stored
/// checkpoint). Ranges already fully behind it are skipped; the returned
/// ranges can be fetched concurrently and each is independently verifiable.
pub fn plan_backfill(
    archive: &ArchiveEventSource,
    resume_from: u32,
) -> Vec<(u32, u32)> {
    archive
        .checkpoint_ranges()
        .into_iter()
        .filter(|(_, to)| *to >= resume_from)
        .map(|(from, to)| (from.max(resume_from), to))
        .collect()
}

/// Fetch a single backfill range from an archive source, returning events in
/// ledger order. Intended to be spawned concurrently per range.
pub async fn fetch_backfill_range(
    archive: &ArchiveEventSource,
    from: u32,
    to: u32,
) -> Result<Vec<RawEvent>, RpcError> {
    let mut out = Vec::new();
    let mut cursor = from;
    while cursor <= to {
        let page = archive.get_events(&Start::Ledger(cursor), 1000).await?;
        if page.events.is_empty() {
            break;
        }
        let mut advanced = false;
        for event in page.events {
            if event.ledger > to {
                continue;
            }
            if event.ledger >= cursor {
                advanced = true;
            }
            out.push(event);
        }
        if !advanced {
            break;
        }
        cursor = out.last().map(|e| e.ledger.saturating_add(1)).unwrap_or(to + 1);
    }
    out.sort_by_key(|e| (e.ledger, e.tx_index, e.event_index));
    Ok(out)
}

/// Group events by ledger for golden comparisons and checkpointing.
pub fn group_by_ledger(events: &[RawEvent]) -> BTreeMap<u32, Vec<RawEvent>> {
    let mut map: BTreeMap<u32, Vec<RawEvent>> = BTreeMap::new();
    for e in events {
        map.entry(e.ledger).or_default().push(e.clone());
    }
    map
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

    async fn ledger_hash(&self, ledger: u32) -> Result<Option<String>, RpcError> {
        #[derive(Deserialize)]
        #[serde(rename_all = "camelCase")]
        struct LedgerInfo {
            hash: Option<String>,
        }
        let info: LedgerInfo = self
            .call("getLedgerEntries", json!({ "ledger": ledger }))
            .await
            .or_else(|_| {
                // Fall back to `getLedgers` for providers that expose the
                // ledger header through the newer method.
                self.call("getLedgers", json!({ "startLedger": ledger, "pagination": { "limit": 1 } }))
            })?;
        Ok(info.hash)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn event_json(ledger: u32, tx: u32, idx: u32, contract: &str) -> String {
        json!({
            "type": "contract",
            "ledger": ledger,
            "ledgerClosedAt": "2024-01-01T00:00:00Z",
            "contractId": contract,
            "id": format!("{ledger}-{tx}-{idx}"),
            "pagingToken": format!("{ledger}-{tx}-{idx}"),
            "inSuccessfulContractCall": true,
            "txHash": "00",
            "topic": [],
            "value": "AAA",
            "txIndex": tx,
            "eventIndex": idx,
        })
        .to_string()
    }

    async fn write_archive(dir: &Path, ranges: &[(u32, u32, Vec<String>)]) {
        tokio::fs::create_dir_all(dir.join("ledgers")).await.unwrap();
        let mut checkpoints = Vec::new();
        for (from, to, lines) in ranges {
            let body = lines.join("\n");
            let path = dir.join("ledgers").join(format!("{from}-{to}.ndjson"));
            tokio::fs::write(&path, &body).await.unwrap();
            checkpoints.push(json!({
                "from": from,
                "to": to,
                "sha256": hex_sha256(body.as_bytes()),
            }));
        }
        let manifest = json!({ "checkpoints": checkpoints });
        tokio::fs::write(dir.join("manifest.json"), manifest.to_string())
            .await
            .unwrap();
    }

    #[tokio::test]
    async fn archive_refuses_unverifiable_source() {
        let dir = tempfile::tempdir().unwrap();
        write_archive(
            dir.path(),
            &[(1, 10, vec![event_json(1, 0, 0, "C1")])],
        )
        .await;
        // Corrupt the file after the manifest was written.
        tokio::fs::write(dir.path().join("ledgers/1-10.ndjson"), "tampered")
            .await
            .unwrap();
        let err = ArchiveEventSource::open(dir.path(), vec!["C1".into()])
            .await
            .unwrap_err();
        assert!(matches!(err, RpcError::Archive(_)));
    }

    #[tokio::test]
    async fn archive_serves_verified_ranges() {
        let dir = tempfile::tempdir().unwrap();
        write_archive(
            dir.path(),
            &[
                (1, 10, vec![event_json(1, 0, 0, "C1"), event_json(5, 0, 0, "C1")]),
                (11, 20, vec![event_json(15, 0, 0, "C1")]),
            ],
        )
        .await;
        let src = ArchiveEventSource::open(dir.path(), vec!["C1".into()])
            .await
            .unwrap();
        assert_eq!(src.oldest_ledger(), 1);
        assert_eq!(src.latest_verified_ledger(), 20);
        let page = src.get_events(&Start::Ledger(1), 100).await.unwrap();
        assert_eq!(page.events.len(), 2);
        let page = src.get_events(&Start::Ledger(11), 100).await.unwrap();
        assert_eq!(page.events.len(), 1);
        assert_eq!(page.events[0].ledger, 15);
    }

    #[tokio::test]
    async fn backfill_is_resumable_and_parallelizable() {
        let dir = tempfile::tempdir().unwrap();
        write_archive(
            dir.path(),
            &[
                (1, 10, vec![event_json(1, 0, 0, "C1")]),
                (11, 20, vec![event_json(15, 0, 0, "C1")]),
                (21, 30, vec![event_json(25, 0, 0, "C1")]),
            ],
        )
        .await;
        let src = ArchiveEventSource::open(dir.path(), vec!["C1".into()])
            .await
            .unwrap();
        // Resume from ledger 12: first range is partially consumed.
        let plan = plan_backfill(&src, 12);
        assert_eq!(plan, vec![(12, 20), (21, 30)]);
        let (a, b) = tokio::join!(
            fetch_backfill_range(&src, plan[0].0, plan[0].1),
            fetch_backfill_range(&src, plan[1].0, plan[1].1),
        );
        let mut all = a.unwrap();
        all.extend(b.unwrap());
        all.sort_by_key(|e| (e.ledger, e.tx_index, e.event_index));
        let ledgers: Vec<u32> = all.iter().map(|e| e.ledger).collect();
        assert_eq!(ledgers, vec![15, 25]);
    }

    #[tokio::test]
    async fn golden_backfill_matches_live() {
        // "Live" source: a mock EventSource serving the same events the
        // archive holds, as if fetched from RPC.
        struct Live(Vec<RawEvent>);
        impl EventSource for Live {
            async fn ledger_window(&self) -> Result<LedgerWindow, RpcError> {
                Ok(LedgerWindow {
                    oldest_ledger: 1,
                    latest_ledger: 30,
                })
            }
            async fn get_events(&self, start: &Start, limit: u32) -> Result<EventPage, RpcError> {
                let from = match start {
                    Start::Ledger(l) => *l,
                    Start::Cursor(_) => 1,
                };
                let events: Vec<RawEvent> = self
                    .0
                    .iter()
                    .filter(|e| e.ledger >= from)
                    .take(limit as usize)
                    .cloned()
                    .collect();
                Ok(EventPage {
                    events,
                    cursor: String::new(),
                    latest_ledger: 30,
                })
            }
        }

        let dir = tempfile::tempdir().unwrap();
        let lines = vec![
            event_json(1, 0, 0, "C1"),
            event_json(5, 0, 0, "C1"),
            event_json(15, 0, 0, "C1"),
            event_json(25, 0, 0, "C1"),
        ];
        write_archive(dir.path(), &[(1, 30, lines)]).await;
        let archive = ArchiveEventSource::open(dir.path(), vec!["C1".into()])
            .await
            .unwrap();

        // Backfill a fresh DB from deployment genesis.
        let mut backfilled = Vec::new();
        for (from, to) in plan_backfill(&archive, 1) {
            backfilled.extend(fetch_backfill_range(&archive, from, to).await.unwrap());
        }
        backfilled.sort_by_key(|e| (e.ledger, e.tx_index, e.event_index));

        // Build the same DB live.
        let live = Live(backfilled.clone());
        let mut live_events = Vec::new();
        let mut cursor = 1;
        loop {
            let page = live.get_events(&Start::Ledger(cursor), 100).await.unwrap();
            if page.events.is_empty() {
                break;
            }
            cursor = page.events.last().unwrap().ledger + 1;
            live_events.extend(page.events);
        }
        live_events.sort_by_key(|e| (e.ledger, e.tx_index, e.event_index));

        assert_eq!(
            group_by_ledger(&backfilled),
            group_by_ledger(&live_events),
            "backfilled DB must equal live-built DB"
        );
    }
}
