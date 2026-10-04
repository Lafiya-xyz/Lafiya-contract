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
    revoked_event_id TEXT
);
CREATE INDEX attestations_record_idx ON attestations (record_hash, event_id);
CREATE INDEX attestations_attester_idx ON attestations (attester, event_id);

CREATE TABLE revocations (
    event_id    TEXT PRIMARY KEY,
    record_hash TEXT NOT NULL,
    ledger      BIGINT NOT NULL,
    tx_hash     TEXT NOT NULL
);
CREATE INDEX revocations_record_idx ON revocations (record_hash, event_id);

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
