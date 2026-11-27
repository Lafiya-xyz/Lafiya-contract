-- Partition `raw_events` by ledger range so that closed ranges can be
-- exported to Parquet and detached (see docs/indexer.md, "Partitioning and
-- archival"). The table is recreated as a RANGE-partitioned table on
-- `ledger`; the append-only trigger is re-attached to the parent so it
-- applies to every partition.
--
-- Partition width is fixed at 100_000 ledgers (about 5.8 days at 5s/ledger).
-- Partition names are `raw_events_p<from>` where <from> is the first ledger
-- of the range, zero-padded to 12 digits so names sort in chain order.

ALTER TABLE raw_events RENAME TO raw_events_unpartitioned;

CREATE TABLE raw_events (
    id               TEXT NOT NULL,
    contract_id      TEXT NOT NULL,
    ledger           BIGINT NOT NULL,
    ledger_closed_at TIMESTAMPTZ NOT NULL,
    tx_hash          TEXT NOT NULL,
    name             TEXT,
    topics           TEXT[] NOT NULL,
    value            TEXT NOT NULL,
    decoded          JSONB NOT NULL,
    ingested_at      TIMESTAMPTZ NOT NULL DEFAULT now(),
    PRIMARY KEY (id, ledger)
) PARTITION BY RANGE (ledger);

-- Copy existing rows into the partitioned table. Partitions are created on
-- demand by `ensure_raw_events_partition` (called by the indexer before each
-- page) and by the lifecycle job below.
DO $$
DECLARE
    lo BIGINT;
    hi BIGINT;
BEGIN
    FOR lo, hi IN
        SELECT DISTINCT
            (ledger / 100000) * 100000,
            (ledger / 100000) * 100000 + 100000
        FROM raw_events_unpartitioned
        ORDER BY 1
    LOOP
        EXECUTE format(
            'CREATE TABLE IF NOT EXISTS %I PARTITION OF raw_events FOR VALUES FROM (%s) TO (%s)',
            'raw_events_p' || lpad(lo::text, 12, '0'), lo, hi
        );
    END LOOP;
END
$$;

INSERT INTO raw_events (id, contract_id, ledger, ledger_closed_at, tx_hash, name,
                        topics, value, decoded, ingested_at)
SELECT id, contract_id, ledger, ledger_closed_at, tx_hash, name,
       topics, value, decoded, ingested_at
FROM raw_events_unpartitioned;

DROP TABLE raw_events_unpartitioned;

CREATE INDEX raw_events_ledger_idx ON raw_events (ledger);

CREATE TRIGGER raw_events_append_only
    BEFORE UPDATE OR DELETE ON raw_events
    FOR EACH ROW EXECUTE FUNCTION raw_events_append_only();

-- Partition lifecycle bookkeeping. One row per managed partition.
CREATE TABLE raw_events_partitions (
    partition_name TEXT PRIMARY KEY,
    ledger_from    BIGINT NOT NULL,
    ledger_to      BIGINT NOT NULL,
    -- 'attached' -> 'exported' (Parquet written and verified) -> 'detached'
    state          TEXT NOT NULL DEFAULT 'attached'
        CHECK (state IN ('attached', 'exported', 'detached')),
    row_count      BIGINT,
    -- SHA-256 of the exported Parquet object, recorded at export time.
    parquet_sha256 TEXT,
    parquet_uri    TEXT,
    exported_at    TIMESTAMPTZ,
    detached_at    TIMESTAMPTZ,
    UNIQUE (ledger_from, ledger_to)
);

INSERT INTO raw_events_partitions (partition_name, ledger_from, ledger_to)
SELECT c.relname,
       substring(c.relname from 'raw_events_p([0-9]+)')::BIGINT,
       substring(c.relname from 'raw_events_p([0-9]+)')::BIGINT + 100000
FROM pg_class c
JOIN pg_inherits i ON i.inhrelid = c.oid
JOIN pg_class p ON p.oid = i.inhparent
WHERE p.relname = 'raw_events'
ON CONFLICT DO NOTHING;
