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
