/** Contract spec type node, as in `stellar contract info interface --output json`. */
export type SpecType = string | { [kind: string]: any };

/** One field of a `#[contractevent]`, in declaration order. */
export interface FieldSpec {
  /** Name in the contract (`record_hash`). */
  name: string;
  /** Key on the decoded payload (`recordHash`). */
  key: string;
  type: SpecType;
  /** `topic`: one topic after the prefix topics. `data`: a key of the data map. */
  location: "topic" | "data";
}

/** One payload version of a contract event. */
export interface EventSpec {
  prefixTopics: string[];
  dataFormat: string;
  fields: FieldSpec[];
  /**
   * Ordering scheme for this event.
   *
   * `ledger`: the event carries no shared sequence number. Ordering is derived
   * from the ledger sequence, transaction index and event index reported by
   * RPC (`(ledger, txIndex, eventIndex)`), so hot paths never write a shared
   * instance key. Gap detection must use that tuple instead of a sequence.
   *
   * `sequence`: legacy scheme where the contract increments a global
   * `EventSeq` in instance storage. Kept only for events that still emit it.
   */
  ordering?: "ledger" | "sequence";
}

/** Events emitted by the incentive-pool contract, keyed by event name. */
export const incentivePoolEvents: Record<string, EventSpec> = {
  PoolInitialized: {
    prefixTopics: ["PoolInitialized"],
    dataFormat: "map",
    ordering: "ledger",
    fields: [
      { name: "admin", key: "admin", type: "address", location: "data" },
      { name: "token", key: "token", type: "address", location: "data" },
      { name: "reward_rate", key: "rewardRate", type: "i128", location: "data" },
    ],
  },
  Deposit: {
    prefixTopics: ["Deposit"],
    dataFormat: "map",
    ordering: "ledger",
    fields: [
      { name: "depositor", key: "depositor", type: "address", location: "topic" },
      { name: "amount", key: "amount", type: "i128", location: "data" },
      { name: "shares", key: "shares", type: "i128", location: "data" },
    ],
  },
  Withdraw: {
    prefixTopics: ["Withdraw"],
    dataFormat: "map",
    ordering: "ledger",
    fields: [
      { name: "withdrawer", key: "withdrawer", type: "address", location: "topic" },
      { name: "amount", key: "amount", type: "i128", location: "data" },
      { name: "shares", key: "shares", type: "i128", location: "data" },
    ],
  },
  RewardClaimed: {
    prefixTopics: ["RewardClaimed"],
    dataFormat: "map",
    ordering: "ledger",
    fields: [
      { name: "claimant", key: "claimant", type: "address", location: "topic" },
      { name: "amount", key: "amount", type: "i128", location: "data" },
    ],
  },
  RewardRateUpdated: {
    prefixTopics: ["RewardRateUpdated"],
    dataFormat: "map",
    ordering: "ledger",
    fields: [
      { name: "old_rate", key: "oldRate", type: "i128", location: "data" },
      { name: "new_rate", key: "newRate", type: "i128", location: "data" },
    ],
  },
  PoolPaused: {
    prefixTopics: ["PoolPaused"],
    dataFormat: "map",
    ordering: "ledger",
    fields: [
      { name: "admin", key: "admin", type: "address", location: "topic" },
    ],
  },
  PoolResumed: {
    prefixTopics: ["PoolResumed"],
    dataFormat: "map",
    ordering: "ledger",
    fields: [
      { name: "admin", key: "admin", type: "address", location: "topic" },
    ],
  },
};

/**
 * Events emitted by the attester-registry contract, keyed by event name.
 *
 * Hot-path events (attest, co-attest, add_attester) use `ordering: "ledger"`:
 * they carry no shared sequence number, so they never write a shared instance
 * key and can execute in parallel. Indexers order them by
 * `(ledger, txIndex, eventIndex)` and detect gaps by comparing consecutive
 * tuples rather than a monotonic `EventSeq`.
 */
export const attesterRegistryEvents: Record<string, EventSpec> = {
  AttestationAdded: {
    prefixTopics: ["AttestationAdded"],
    dataFormat: "map",
    ordering: "ledger",
    fields: [
      { name: "attester", key: "attester", type: "address", location: "topic" },
      { name: "subject", key: "subject", type: "address", location: "topic" },
      { name: "record_hash", key: "recordHash", type: "bytes", location: "data" },
    ],
  },
  CoAttestationAdded: {
    prefixTopics: ["CoAttestationAdded"],
    dataFormat: "map",
    ordering: "ledger",
    fields: [
      { name: "attester", key: "attester", type: "address", location: "topic" },
      { name: "record_hash", key: "recordHash", type: "bytes", location: "data" },
    ],
  },
  AttesterAdded: {
    prefixTopics: ["AttesterAdded"],
    dataFormat: "map",
    ordering: "ledger",
    fields: [
      { name: "attester", key: "attester", type: "address", location: "topic" },
    ],
  },
};

/**
 * Events emitted by the attestation-registry contract, keyed by event name.
 *
 * Status-list events are epoch-versioned: segments are published for an
 * `epoch`, and a root is activated only for an epoch whose segments are all
 * present and consistent. Verifiers read `root(epoch)` first, then segments
 * for that epoch. Revocation is monotonic across epochs; clearing a bit
 * requires the explicit `CredentialUnrevoked` event.
 */
export const attestationRegistryEvents: Record<string, EventSpec> = {
  AttestationAdded: {
    prefixTopics: ["AttestationAdded"],
    dataFormat: "map",
    ordering: "ledger",
    fields: [
      { name: "attester", key: "attester", type: "address", location: "topic" },
      { name: "subject", key: "subject", type: "address", location: "topic" },
      { name: "record_hash", key: "recordHash", type: "bytes", location: "data" },
    ],
  },
  CoAttestationAdded: {
    prefixTopics: ["CoAttestationAdded"],
    dataFormat: "map",
    ordering: "ledger",
    fields: [
      { name: "attester", key: "attester", type: "address", location: "topic" },
      { name: "record_hash", key: "recordHash", type: "bytes", location: "data" },
    ],
  },
  StatusListSegmentPublished: {
    prefixTopics: ["StatusListSegmentPublished"],
    dataFormat: "map",
    ordering: "ledger",
    fields: [
      { name: "epoch", key: "epoch", type: "u64", location: "topic" },
      { name: "index", key: "index", type: "u32", location: "topic" },
      { name: "segment_hash", key: "segmentHash", type: "bytes", location: "data" },
    ],
  },
  StatusListRootSet: {
    prefixTopics: ["StatusListRootSet"],
    dataFormat: "map",
    ordering: "ledger",
    fields: [
      { name: "epoch", key: "epoch", type: "u64", location: "topic" },
      { name: "root", key: "root", type: "bytes", location: "data" },
    ],
  },
  CredentialRevoked: {
    prefixTopics: ["CredentialRevoked"],
    dataFormat: "map",
    ordering: "ledger",
    fields: [
      { name: "epoch", key: "epoch", type: "u64", location: "topic" },
      { name: "record_hash", key: "recordHash", type: "bytes", location: "data" },
    ],
  },
  CredentialUnrevoked: {
    prefixTopics: ["CredentialUnrevoked"],
    dataFormat: "map",
    ordering: "ledger",
    fields: [
      { name: "epoch", key: "epoch", type: "u64", location: "topic" },
      { name: "record_hash", key: "recordHash", type: "bytes", location: "data" },
    ],
  },
};