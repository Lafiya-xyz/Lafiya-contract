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
}

/** Events emitted by the incentive-pool contract, keyed by event name. */
export const incentivePoolEvents: Record<string, EventSpec> = {
  PoolInitialized: {
    prefixTopics: ["PoolInitialized"],
    dataFormat: "map",
    fields: [
      { name: "admin", key: "admin", type: "address", location: "data" },
      { name: "token", key: "token", type: "address", location: "data" },
      { name: "reward_rate", key: "rewardRate", type: "i128", location: "data" },
    ],
  },
  Deposit: {
    prefixTopics: ["Deposit"],
    dataFormat: "map",
    fields: [
      { name: "depositor", key: "depositor", type: "address", location: "topic" },
      { name: "amount", key: "amount", type: "i128", location: "data" },
      { name: "shares", key: "shares", type: "i128", location: "data" },
    ],
  },
  Withdraw: {
    prefixTopics: ["Withdraw"],
    dataFormat: "map",
    fields: [
      { name: "withdrawer", key: "withdrawer", type: "address", location: "topic" },
      { name: "amount", key: "amount", type: "i128", location: "data" },
      { name: "shares", key: "shares", type: "i128", location: "data" },
    ],
  },
  RewardClaimed: {
    prefixTopics: ["RewardClaimed"],
    dataFormat: "map",
    fields: [
      { name: "claimant", key: "claimant", type: "address", location: "topic" },
      { name: "amount", key: "amount", type: "i128", location: "data" },
    ],
  },
  RewardRateUpdated: {
    prefixTopics: ["RewardRateUpdated"],
    dataFormat: "map",
    fields: [
      { name: "old_rate", key: "oldRate", type: "i128", location: "data" },
      { name: "new_rate", key: "newRate", type: "i128", location: "data" },
    ],
  },
  PoolPaused: {
    prefixTopics: ["PoolPaused"],
    dataFormat: "map",
    fields: [
      { name: "admin", key: "admin", type: "address", location: "topic" },
    ],
  },
  PoolResumed: {
    prefixTopics: ["PoolResumed"],
    dataFormat: "map",
    fields: [
      { name: "admin", key: "admin", type: "address", location: "topic" },
    ],
  },
};