-- Ingestion checkpoint. `cursor` is the last RPC `getEvents` cursor; when it
-- is NULL, ingestion restarts from `ledger` via `startLedger` (set after a
-- backfill that is known to be complete through `ledger - 1`).
CREATE TABLE checkpoints (
    name       TEXT PRIMARY KEY,
    cursor     TEXT,
    ledger     BIGINT NOT NULL,
    updated_at TIMESTAMPTZ NOT NULL DEFAULT now()
);

-- Immutable audit copy of every ingested contract event. `id` is the RPC
-- event id, whose zero-padded form sorts in chain order.
CREATE TABLE raw_events (
    id               TEXT PRIMARY KEY,
    contract_id      TEXT NOT NULL,
    ledger           BIGINT NOT NULL,
    ledger_closed_at TIMESTAMPTZ NOT NULL,
    tx_hash          TEXT NOT NULL,
    name             TEXT,
    topics           TEXT[] NOT NULL,
    value            TEXT NOT NULL,
    decoded          JSONB NOT NULL,
    ingested_at      TIMESTAMPTZ NOT NULL DEFAULT now()
);
CREATE INDEX raw_events_ledger_idx ON raw_events (ledger);

CREATE FUNCTION raw_events_append_only() RETURNS trigger LANGUAGE plpgsql AS $$
BEGIN
    RAISE EXCEPTION 'raw_events is append-only';
END
$$;
CREATE TRIGGER raw_events_append_only
    BEFORE UPDATE OR DELETE ON raw_events
    FOR EACH ROW EXECUTE FUNCTION raw_events_append_only();

-- Attester lifecycle. The contract stores allowlist membership and the
-- suspension flag independently (`add_attester` does not clear a
-- suspension; `remove_attester` does), so both flags are tracked and the
-- status is derived from them.
CREATE TABLE attesters (
    address           TEXT PRIMARY KEY,
    allowlisted       BOOLEAN NOT NULL,
    suspended         BOOLEAN NOT NULL,
    status            TEXT GENERATED ALWAYS AS (
        CASE WHEN NOT allowlisted THEN 'removed'
             WHEN suspended THEN 'suspended'
             ELSE 'active' END
    ) STORED,
    region            TEXT,
    license_hash      TEXT,
    -- Set by `AttesterInfoUpdated` (which carries no data); cleared once the
    -- reconciler has read the new info from chain.
    info_stale        BOOLEAN NOT NULL DEFAULT false,
    first_seen_ledger BIGINT NOT NULL,
    updated_ledger    BIGINT NOT NULL,
    updated_event_id  TEXT NOT NULL
);
CREATE INDEX attesters_region_status_idx ON attesters (region, status);
CREATE INDEX attesters_status_idx ON attesters (status);

-- One row per `AttestationRecorded`. The current attestation for a record is
-- the latest row whose `revoked_event_id` is NULL.
CREATE TABLE attestations (
    event_id         TEXT PRIMARY KEY,
    record_hash      TEXT NOT NULL,
    attester         TEXT NOT NULL,
    attested_at      BIGINT NOT NULL,
    ledger           BIGINT NOT NULL,
    tx_hash          TEXT NOT NULL,
    revoked_event_id TEXT,
    -- Ledger at which the entry's TTL currently expires. Bumped by
    -- `extend_attestation_ttl`; never lowered (the contract rejects
    -- shortening), so the indexer can track the maximum observed horizon.
    expires_at_ledger BIGINT,
    -- Set by `AttestationTtlExtended`; the latest extension event for the
    -- record wins. NULL when no extension has been observed.
    ttl_extended_event_id TEXT
);
CREATE INDEX attestations_record_idx ON attestations (record_hash, event_id);
CREATE INDEX attestations_attester_idx ON attestations (attester, event_id);
-- Watchdog queries scan for entries approaching expiry.
CREATE INDEX attestations_expiry_idx ON attestations (expires_at_ledger)
    WHERE revoked_event_id IS NULL;

-- TTL extension events, one row per `AttestationTtlExtended`. Kept separate
-- from `attestations` so the full extension history is auditable.
CREATE TABLE attestation_ttl_extensions (
    event_id          TEXT PRIMARY KEY,
    record_hash       TEXT NOT NULL,
    caller            TEXT NOT NULL,
    previous_expiry   BIGINT,
    new_expiry        BIGINT NOT NULL,
    ledger            BIGINT NOT NULL,
    tx_hash           TEXT NOT NULL
);
CREATE INDEX attestation_ttl_extensions_record_idx
    ON attestation_ttl_extensions (record_hash, event_id);

-- Quorum policy per record class and the aggregated distinct-attester count
-- for each record. `attester_count` counts distinct attesters with a live
-- (non-revoked) attestation; the contract enforces the distinct-attester
-- rule, so a repeated attestation from the same attester is a no-op.
CREATE TABLE quorum_policies (
    record_class   TEXT PRIMARY KEY,
    required_count INTEGER NOT NULL CHECK (required_count >= 1),
    updated_ledger BIGINT NOT NULL,
    updated_event_id TEXT NOT NULL
);

CREATE TABLE record_quorum (
    record_hash    TEXT PRIMARY KEY,
    record_class   TEXT NOT NULL,
    attester_count INTEGER NOT NULL DEFAULT 0,
    -- Derived: attester_count >= policy.required_count at last update.
    satisfied      BOOLEAN NOT NULL DEFAULT false,
    updated_ledger BIGINT NOT NULL,
    updated_event_id TEXT NOT NULL
);
CREATE INDEX record_quorum_class_idx ON record_quorum (record_class, satisfied);

-- Patient-initiated dispute overlay. Keyed by record hash and the patient
-- signer; disputes never delete the underlying attestation. `cleared_*`
-- columns are set when an attester or admin clears the dispute.
CREATE TABLE disputes (
    record_hash       TEXT NOT NULL,
    signer            TEXT NOT NULL,
    raised_event_id   TEXT NOT NULL,
    raised_ledger     BIGINT NOT NULL,
    raised_tx_hash    TEXT NOT NULL,
    reason_hash       TEXT,
    cleared_event_id  TEXT,
    cleared_ledger    BIGINT,
    cleared_by        TEXT,
    PRIMARY KEY (record_hash, signer)
);
CREATE INDEX disputes_open_idx ON disputes (record_hash)
    WHERE cleared_event_id IS NULL;

-- Status-list segments (W3C Bitstring Status List style). Each segment is a
-- versioned bitstring with a signed root; `root` is the commitment verifiers
-- validate inclusion against.
CREATE TABLE status_list_segments (
    segment_id     BIGINT PRIMARY KEY,
    version        BIGINT NOT NULL,
    root           TEXT NOT NULL,
    bit_length     INTEGER NOT NULL CHECK (bit_length > 0),
    -- Encoded bitstring payload (gzip+base64) for verifier sync.
    payload        TEXT NOT NULL,
    published_ledger BIGINT NOT NULL,
    published_event_id TEXT NOT NULL
);
CREATE INDEX status_list_segments_version_idx
    ON status_list_segments (segment_id, version);

-- Signed roots over the full status-list state, so verifiers can validate a
-- cached segment set against a single signed commitment.
CREATE TABLE status_list_roots (
    event_id       TEXT PRIMARY KEY,
    root           TEXT NOT NULL,
    segment_count  INTEGER NOT NULL,
    ledger         BIGINT NOT NULL,
    tx_hash        TEXT NOT NULL
);
CREATE INDEX status_list_roots_ledger_idx ON status_list_roots (ledger);

CREATE TABLE revocations (
    event_id    TEXT PRIMARY KEY,
    record_hash TEXT NOT NULL,
    ledger      BIGINT NOT NULL,
    tx_hash     TEXT NOT NULL
);
CREATE INDEX revocations_record_idx ON revocations (record_hash, event_id);
CREATE INDEX revocations_ledger_idx ON revocations (ledger, event_id);

-- Administration events (admin transfer, pause/unpause, upgrade, registry
-- repointing, initialization) with their decoded topic fields.
CREATE TABLE admin_events (
    event_id    TEXT PRIMARY KEY,
    contract_id TEXT NOT NULL,
    name        TEXT NOT NULL,
    fields      JSONB NOT NULL,
    ledger      BIGINT NOT NULL,
    tx_hash     TEXT NOT NULL
);

-- Incentive-pool work items. One row per `WorkItemCreated`; the mutable
-- columns are maintained from the pool's lifecycle events (approval,
-- claim, withdrawal, cancellation).
CREATE TABLE work_items (
    work_item_id      TEXT PRIMARY KEY,
    pool_id           TEXT NOT NULL,
    attester          TEXT NOT NULL,
    amount            NUMERIC(38, 0) NOT NULL,
    status            TEXT NOT NULL,
    created_ledger    BIGINT NOT NULL,
    created_event_id  TEXT NOT NULL,
    approved_ledger   BIGINT,
    approved_event_id TEXT,
    claimed_ledger    BIGINT,
    claimed_event_id  TEXT,
    updated_ledger    BIGINT NOT NULL,
    updated_event_id  TEXT NOT NULL
);
CREATE INDEX work_items_pool_status_idx ON work_items (pool_id, status);
CREATE INDEX work_items_attester_idx ON work_items (attester, work_item_id);

-- One row per pool payout (claim) with the ledger it was observed at.
CREATE TABLE payouts (
    event_id       TEXT PRIMARY KEY,
    pool_id        TEXT NOT NULL,
    work_item_id   TEXT NOT NULL,
    attester       TEXT NOT NULL,
    amount         NUMERIC(38, 0) NOT NULL,
    ledger         BIGINT NOT NULL,
    tx_hash        TEXT NOT NULL,
    ledger_closed_at TIMESTAMPTZ NOT NULL
);
CREATE INDEX payouts_pool_idx ON payouts (pool_id, event_id);
CREATE INDEX payouts_attester_idx ON payouts (attester, event_id);
CREATE INDEX payouts_work_item_idx ON payouts (work_item_id);

-- Pool administration events (pause/unpause, funding, approvals and
-- withdrawals) with their decoded topic fields.
CREATE TABLE pool_events (
    event_id    TEXT PRIMARY KEY,
    contract_id TEXT NOT NULL,
    name        TEXT NOT NULL,
    fields      JSONB NOT NULL,
    ledger      BIGINT NOT NULL,
    tx_hash     TEXT NOT NULL
);
CREATE INDEX pool_events_name_ledger_idx ON pool_events (name, ledger);

-- Ledger hash tracking for reorg detection. Every ingested ledger records
-- the hash reported by the provider so continuity can be verified on each
-- page and derived state rolled back to a common ancestor on divergence.
CREATE TABLE ledger_hashes (
    ledger      BIGINT PRIMARY KEY,
    ledger_hash TEXT NOT NULL,
    ingested_at TIMESTAMPTZ NOT NULL DEFAULT now()
);

ALTER TABLE raw_events ADD COLUMN ledger_hash TEXT;
ALTER TABLE checkpoints ADD COLUMN ledger_hash TEXT;
ALTER TABLE checkpoints ADD COLUMN finalized_ledger BIGINT;
