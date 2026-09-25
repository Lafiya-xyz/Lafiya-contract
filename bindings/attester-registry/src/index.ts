import { Buffer } from "buffer";
import { Address } from "@stellar/stellar-sdk";
import {
  AssembledTransaction,
  Client as ContractClient,
  ClientOptions as ContractClientOptions,
  MethodOptions,
  Result,
  Spec as ContractSpec,
} from "@stellar/stellar-sdk/contract";
import type {
  u32,
  i32,
  u64,
  i64,
  u128,
  i128,
  u256,
  i256,
  Option,
  Timepoint,
  Duration,
} from "@stellar/stellar-sdk/contract";
export * from "@stellar/stellar-sdk";
export * as contract from "@stellar/stellar-sdk/contract";
export * as rpc from "@stellar/stellar-sdk/rpc";

if (typeof window !== "undefined") {
  //@ts-ignore Buffer exists
  window.Buffer = window.Buffer || Buffer;
}




/**
 * Errors returned by the attester registry's public entry points.
 */
export const Errors = {
  /**
   * `initialize` has not been called yet; call
   * `initialize(admin: Address)` before using the contract.
   */
  1: {message:"NotInitialized"},
  /**
   * `initialize` was called more than once.
   */
  2: {message:"AlreadyInitialized"},
  /**
   * `accept_admin` was called with no pending admin transfer. Admin transfer is a
   * two-step flow: the current admin must first call `propose_admin` to nominate a
   * successor, then the nominated address must call `accept_admin` to complete the
   * transfer. This error is returned when `accept_admin` is called before a
   * corresponding `propose_admin` call has set a pending admin.
   */
  3: {message:"NoPendingTransfer"},
  /**
   * The requested operation is blocked while the contract is paused.
   */
  4: {message:"ContractPaused"},
  /**
   * The allowlist is at its configured maximum size. Raise the cap via `set_max_attesters`, or free a slot via `remove_attester`.
   */
  5: {message:"AllowlistFull"},
  /**
   * `migrate()` was called while the stored schema version is already
   * `>= SCHEMA_VERSION`. Only call `migrate()` after `upgrade()` to a
   * build that bumps `SCHEMA_VERSION`; this error is a safe no-op signal
   * that there is nothing pending, not a failure to react to.
   */
  6: {message:"MigrationNotRequired"},
  /**
   * The referenced attester is not currently allowlisted (never added,
   * or since removed).
   */
  7: {message:"AttesterNotFound"},
  /**
   * The supplied batch exceeds `BATCH_LIMIT` addresses.
   */
  8: {message:"BatchTooLarge"}
}






/**
 * Metadata associated with an allowlisted attester.
 */
export interface AttesterInfo {
  /**
 * Hash of the attester's off-chain license/credential document, if any.
 */
license_hash: Option<Buffer>;
  /**
 * The geographic region the attester is authorized to attest for, if any.
 */
region: Option<string>;
}



/**
 * An allowlisted attester's metadata together with its current suspension
 * state, as returned by `get_attester_status`.
 */
export interface AttesterStatus {
  /**
 * The attester's stored metadata.
 */
info: AttesterInfo;
  /**
 * Whether the attester is currently suspended.
 */
suspended: boolean;
}






export interface Client {
  /**
   * Construct and simulate a pause transaction. Returns an `AssembledTransaction` object which will have a `result` field containing the result of the simulation. If this transaction changes contract state, you will need to call `signAndSend()` on the returned object.
   * Pause the contract, blocking `add_attester`, `add_attester_with_info`,
   * `update_attester_info`, `remove_attester`, `suspend_attester`, and
   * `reinstate_attester` until `unpause` is called. Requires the admin's
   * authorization.
   */
  pause: (options?: MethodOptions) => Promise<AssembledTransaction<Result<void>>>

  /**
   * Construct and simulate a migrate transaction. Returns an `AssembledTransaction` object which will have a `result` field containing the result of the simulation. If this transaction changes contract state, you will need to call `signAndSend()` on the returned object.
   * Run any pending storage migration, then record the new schema
   * version. Requires the admin's authorization.
   * 
   * Call this after `upgrade()` only when the new build bumps
   * `SCHEMA_VERSION` (a storage-schema-changing release) — including the
   * first upgrade of a legacy (pre-versioning, schema version `0`)
   * instance, which must be migrated to version 1. When no migration is
   * pending (`SchemaVersion >= SCHEMA_VERSION`) this returns
   * `Error::MigrationNotRequired` so the call can't accidentally re-run.
   */
  migrate: (options?: MethodOptions) => Promise<AssembledTransaction<Result<void>>>

  /**
   * Construct and simulate a unpause transaction. Returns an `AssembledTransaction` object which will have a `result` field containing the result of the simulation. If this transaction changes contract state, you will need to call `signAndSend()` on the returned object.
   * Resume normal operation after a `pause`. Requires the admin's authorization.
   */
  unpause: (options?: MethodOptions) => Promise<AssembledTransaction<Result<void>>>

  /**
   * Construct and simulate a upgrade transaction. Returns an `AssembledTransaction` object which will have a `result` field containing the result of the simulation. If this transaction changes contract state, you will need to call `signAndSend()` on the returned object.
   * Upgrade the contract's Wasm code to a new version.
   * Requires the admin's authorization.
   * 
   * Runbook:
   * 1. Build the new Wasm binary (e.g. `cargo build --workspace --release --target wasm32v1-none`).
   * 2. Upload/install the new Wasm on-chain to obtain its 32-byte hash (`new_wasm_hash`).
   * 3. The admin calls this `upgrade` function passing the `new_wasm_hash`.
   * 
   * For any accompanying state/data migrations, see the storage-versioning guidelines
   * (e.g. implementing migration scripts or handling lazy migrations on reading old schema versions).
   */
  upgrade: ({new_wasm_hash}: {new_wasm_hash: Buffer}, options?: MethodOptions) => Promise<AssembledTransaction<Result<void>>>

  /**
   * Construct and simulate a get_admin transaction. Returns an `AssembledTransaction` object which will have a `result` field containing the result of the simulation. If this transaction changes contract state, you will need to call `signAndSend()` on the returned object.
   * Return the current admin address.
   */
  get_admin: (options?: MethodOptions) => Promise<AssembledTransaction<Result<string>>>

  /**
   * Construct and simulate a is_paused transaction. Returns an `AssembledTransaction` object which will have a `result` field containing the result of the simulation. If this transaction changes contract state, you will need to call `signAndSend()` on the returned object.
   * Whether the contract is currently paused.
   */
  is_paused: (options?: MethodOptions) => Promise<AssembledTransaction<boolean>>

  /**
   * Construct and simulate a initialize transaction. Returns an `AssembledTransaction` object which will have a `result` field containing the result of the simulation. If this transaction changes contract state, you will need to call `signAndSend()` on the returned object.
   * Set the admin address authorized to manage the allowlist. Can only
   * be called once; the caller must authorize as the given `admin`.
   */
  initialize: ({admin}: {admin: string}, options?: MethodOptions) => Promise<AssembledTransaction<Result<void>>>

  /**
   * Construct and simulate a is_attester transaction. Returns an `AssembledTransaction` object which will have a `result` field containing the result of the simulation. If this transaction changes contract state, you will need to call `signAndSend()` on the returned object.
   * Whether `attester` is currently allowlisted (and not suspended). Callable by anyone,
   * including other contracts (e.g. `attestation-registry`).
   */
  is_attester: ({attester}: {attester: string}, options?: MethodOptions) => Promise<AssembledTransaction<boolean>>

  /**
   * Construct and simulate a accept_admin transaction. Returns an `AssembledTransaction` object which will have a `result` field containing the result of the simulation. If this transaction changes contract state, you will need to call `signAndSend()` on the returned object.
   * Accept the proposed admin transfer. The caller must authorize as the pending admin.
   */
  accept_admin: (options?: MethodOptions) => Promise<AssembledTransaction<Result<void>>>

  /**
   * Construct and simulate a add_attester transaction. Returns an `AssembledTransaction` object which will have a `result` field containing the result of the simulation. If this transaction changes contract state, you will need to call `signAndSend()` on the returned object.
   * Add `attester` to the allowlist. Requires the admin's authorization.
   * Fails with `Error::AllowlistFull` if the allowlist is at capacity and
   * `attester` is not already present (see `set_max_attesters`).
   */
  add_attester: ({attester}: {attester: string}, options?: MethodOptions) => Promise<AssembledTransaction<Result<void>>>

  /**
   * Construct and simulate a add_attesters transaction. Returns an `AssembledTransaction` object which will have a `result` field containing the result of the simulation. If this transaction changes contract state, you will need to call `signAndSend()` on the returned object.
   * Add multiple attesters to the allowlist in a single transaction.
   * 
   * Requires the admin's authorization. Blocked while the contract is paused.
   * Returns `Error::BatchTooLarge` if `attesters.len() > BATCH_LIMIT`.
   * Returns `Error::AllowlistFull` if adding the new (non-duplicate)
   * addresses would exceed the configured `max_attesters` cap. Addresses
   * that are already allowlisted are silently skipped (idempotent), so the
   * call never fails due to duplicates in the batch and no duplicate events
   * are emitted. Exactly one `AttesterAdded` event is emitted per newly
   * added address.
   */
  add_attesters: ({attesters}: {attesters: Array<string>}, options?: MethodOptions) => Promise<AssembledTransaction<Result<void>>>

  /**
   * Construct and simulate a propose_admin transaction. Returns an `AssembledTransaction` object which will have a `result` field containing the result of the simulation. If this transaction changes contract state, you will need to call `signAndSend()` on the returned object.
   * Propose a new admin address. The caller must authorize as the current admin.
   * Calling this a second time before `accept_admin` overwrites any pending proposal — the most recent call wins.
   */
  propose_admin: ({new_admin}: {new_admin: string}, options?: MethodOptions) => Promise<AssembledTransaction<Result<void>>>

  /**
   * Construct and simulate a remove_attester transaction. Returns an `AssembledTransaction` object which will have a `result` field containing the result of the simulation. If this transaction changes contract state, you will need to call `signAndSend()` on the returned object.
   * Remove `attester` from the allowlist. Requires the admin's
   * authorization. A no-op if the attester was never allowlisted.
   */
  remove_attester: ({attester}: {attester: string}, options?: MethodOptions) => Promise<AssembledTransaction<Result<void>>>

  /**
   * Construct and simulate a remove_attesters transaction. Returns an `AssembledTransaction` object which will have a `result` field containing the result of the simulation. If this transaction changes contract state, you will need to call `signAndSend()` on the returned object.
   * Remove multiple attesters from the allowlist in a single transaction.
   * 
   * Requires the admin's authorization. Blocked while the contract is paused.
   * Returns `Error::BatchTooLarge` if `attesters.len() > BATCH_LIMIT`.
   * Addresses that are not currently allowlisted are silently skipped
   * (idempotent), so the call never fails if an address was already removed
   * and no spurious events are emitted. Exactly one `AttesterRemoved` event
   * is emitted per address that was actually removed.
   */
  remove_attesters: ({attesters}: {attesters: Array<string>}, options?: MethodOptions) => Promise<AssembledTransaction<Result<void>>>

  /**
   * Construct and simulate a suspend_attester transaction. Returns an `AssembledTransaction` object which will have a `result` field containing the result of the simulation. If this transaction changes contract state, you will need to call `signAndSend()` on the returned object.
   * Suspend an allowlisted attester. Requires the admin's authorization.
   * 
   * **Note:** this function does **not** check whether `attester` was ever
   * added via `add_attester`. If called on an address that is not in the
   * allowlist, it silently sets the `Suspended` storage key and emits
   * `AttesterSuspended` for that address — a no-op from an access-control
   * perspective because `is_attester` also checks for an `Attester` storage
   * entry, so the phantom suspension has no effect on allowlist queries.
   * This diverges from `update_attester_info`, which returns
   * `Error::AttesterNotFound` for unknown addresses. The inconsistency is
   * known and documented here rather than silently changed; a follow-up
   * issue should decide whether to align both functions.
   */
  suspend_attester: ({attester}: {attester: string}, options?: MethodOptions) => Promise<AssembledTransaction<Result<void>>>

  /**
   * Construct and simulate a get_attester_info transaction. Returns an `AssembledTransaction` object which will have a `result` field containing the result of the simulation. If this transaction changes contract state, you will need to call `signAndSend()` on the returned object.
   * Get the optional metadata associated with `attester` if they are allowlisted.
   */
  get_attester_info: ({attester}: {attester: string}, options?: MethodOptions) => Promise<AssembledTransaction<Option<AttesterInfo>>>

  /**
   * Construct and simulate a get_max_attesters transaction. Returns an `AssembledTransaction` object which will have a `result` field containing the result of the simulation. If this transaction changes contract state, you will need to call `signAndSend()` on the returned object.
   * The current soft cap on the number of allowlisted attesters.
   */
  get_max_attesters: (options?: MethodOptions) => Promise<AssembledTransaction<u32>>

  /**
   * Construct and simulate a set_max_attesters transaction. Returns an `AssembledTransaction` object which will have a `result` field containing the result of the simulation. If this transaction changes contract state, you will need to call `signAndSend()` on the returned object.
   * Set the soft cap on the number of allowlisted attesters. Requires the
   * admin's authorization. Does not evict existing attesters if lowered
   * below the current count; it only blocks further `add_attester` calls.
   */
  set_max_attesters: ({max_attesters}: {max_attesters: u32}, options?: MethodOptions) => Promise<AssembledTransaction<Result<void>>>

  /**
   * Construct and simulate a get_attester_count transaction. Returns an `AssembledTransaction` object which will have a `result` field containing the result of the simulation. If this transaction changes contract state, you will need to call `signAndSend()` on the returned object.
   * The current number of allowlisted attesters.
   */
  get_attester_count: (options?: MethodOptions) => Promise<AssembledTransaction<u32>>

  /**
   * Construct and simulate a get_schema_version transaction. Returns an `AssembledTransaction` object which will have a `result` field containing the result of the simulation. If this transaction changes contract state, you will need to call `signAndSend()` on the returned object.
   * Query the current storage schema version of the contract.
   */
  get_schema_version: (options?: MethodOptions) => Promise<AssembledTransaction<u32>>

  /**
   * Construct and simulate a reinstate_attester transaction. Returns an `AssembledTransaction` object which will have a `result` field containing the result of the simulation. If this transaction changes contract state, you will need to call `signAndSend()` on the returned object.
   * Reinstate a suspended attester. Requires the admin's authorization.
   */
  reinstate_attester: ({attester}: {attester: string}, options?: MethodOptions) => Promise<AssembledTransaction<Result<void>>>

  /**
   * Construct and simulate a get_attester_status transaction. Returns an `AssembledTransaction` object which will have a `result` field containing the result of the simulation. If this transaction changes contract state, you will need to call `signAndSend()` on the returned object.
   * Get `attester`'s metadata together with its current suspension state
   * in a single call. Returns `None` if `attester` is not currently
   * allowlisted (never added, or since removed).
   */
  get_attester_status: ({attester}: {attester: string}, options?: MethodOptions) => Promise<AssembledTransaction<Option<AttesterStatus>>>

  /**
   * Construct and simulate a update_attester_info transaction. Returns an `AssembledTransaction` object which will have a `result` field containing the result of the simulation. If this transaction changes contract state, you will need to call `signAndSend()` on the returned object.
   * Update the metadata of an already-allowlisted `attester`. Requires
   * the admin's authorization. Unlike `add_attester_with_info`, this
   * never enrolls a new attester: it fails with `Error::AttesterNotFound`
   * if `attester` is not currently allowlisted (never added, or since
   * removed), and always emits `AttesterInfoUpdated` rather than
   * `AttesterAdded`, so profile changes are distinguishable from
   * enrollment.
   */
  update_attester_info: ({attester, license_hash, region}: {attester: string, license_hash: Option<Buffer>, region: Option<string>}, options?: MethodOptions) => Promise<AssembledTransaction<Result<void>>>

  /**
   * Construct and simulate a add_attester_with_info transaction. Returns an `AssembledTransaction` object which will have a `result` field containing the result of the simulation. If this transaction changes contract state, you will need to call `signAndSend()` on the returned object.
   * Add `attester` with optional metadata to the allowlist. Requires the admin's authorization.
   * Fails with `Error::AllowlistFull` if the allowlist is at capacity and
   * `attester` is not already present (see `set_max_attesters`).
   */
  add_attester_with_info: ({attester, license_hash, region}: {attester: string, license_hash: Option<Buffer>, region: Option<string>}, options?: MethodOptions) => Promise<AssembledTransaction<Result<void>>>

}
export class Client extends ContractClient {
  static async deploy<T = Client>(
    /** Options for initializing a Client as well as for calling a method, with extras specific to deploying. */
    options: MethodOptions &
      Omit<ContractClientOptions, "contractId"> & {
        /** The hash of the Wasm blob, which must already be installed on-chain. */
        wasmHash: Buffer | string;
        /** Salt used to generate the contract's ID. Passed through to {@link Operation.createCustomContract}. Default: random. */
        salt?: Buffer | Uint8Array;
        /** The format used to decode `wasmHash`, if it's provided as a string. */
        format?: "hex" | "base64";
      }
  ): Promise<AssembledTransaction<T>> {
    return ContractClient.deploy(null, options)
  }
  constructor(public readonly options: ContractClientOptions) {
    super(
      new ContractSpec([ "AAAABAAAAD9FcnJvcnMgcmV0dXJuZWQgYnkgdGhlIGF0dGVzdGVyIHJlZ2lzdHJ5J3MgcHVibGljIGVudHJ5IHBvaW50cy4AAAAAAAAAAAVFcnJvcgAAAAAAAAgAAABiYGluaXRpYWxpemVgIGhhcyBub3QgYmVlbiBjYWxsZWQgeWV0OyBjYWxsCmBpbml0aWFsaXplKGFkbWluOiBBZGRyZXNzKWAgYmVmb3JlIHVzaW5nIHRoZSBjb250cmFjdC4AAAAAAA5Ob3RJbml0aWFsaXplZAAAAAAAAQAAACdgaW5pdGlhbGl6ZWAgd2FzIGNhbGxlZCBtb3JlIHRoYW4gb25jZS4AAAAAEkFscmVhZHlJbml0aWFsaXplZAAAAAAAAgAAAW9gYWNjZXB0X2FkbWluYCB3YXMgY2FsbGVkIHdpdGggbm8gcGVuZGluZyBhZG1pbiB0cmFuc2Zlci4gQWRtaW4gdHJhbnNmZXIgaXMgYQp0d28tc3RlcCBmbG93OiB0aGUgY3VycmVudCBhZG1pbiBtdXN0IGZpcnN0IGNhbGwgYHByb3Bvc2VfYWRtaW5gIHRvIG5vbWluYXRlIGEKc3VjY2Vzc29yLCB0aGVuIHRoZSBub21pbmF0ZWQgYWRkcmVzcyBtdXN0IGNhbGwgYGFjY2VwdF9hZG1pbmAgdG8gY29tcGxldGUgdGhlCnRyYW5zZmVyLiBUaGlzIGVycm9yIGlzIHJldHVybmVkIHdoZW4gYGFjY2VwdF9hZG1pbmAgaXMgY2FsbGVkIGJlZm9yZSBhCmNvcnJlc3BvbmRpbmcgYHByb3Bvc2VfYWRtaW5gIGNhbGwgaGFzIHNldCBhIHBlbmRpbmcgYWRtaW4uAAAAABFOb1BlbmRpbmdUcmFuc2ZlcgAAAAAAAAMAAABAVGhlIHJlcXVlc3RlZCBvcGVyYXRpb24gaXMgYmxvY2tlZCB3aGlsZSB0aGUgY29udHJhY3QgaXMgcGF1c2VkLgAAAA5Db250cmFjdFBhdXNlZAAAAAAABAAAAH1UaGUgYWxsb3dsaXN0IGlzIGF0IGl0cyBjb25maWd1cmVkIG1heGltdW0gc2l6ZS4gUmFpc2UgdGhlIGNhcCB2aWEgYHNldF9tYXhfYXR0ZXN0ZXJzYCwgb3IgZnJlZSBhIHNsb3QgdmlhIGByZW1vdmVfYXR0ZXN0ZXJgLgAAAAAAAA1BbGxvd2xpc3RGdWxsAAAAAAAABQAAAQJgbWlncmF0ZSgpYCB3YXMgY2FsbGVkIHdoaWxlIHRoZSBzdG9yZWQgc2NoZW1hIHZlcnNpb24gaXMgYWxyZWFkeQpgPj0gU0NIRU1BX1ZFUlNJT05gLiBPbmx5IGNhbGwgYG1pZ3JhdGUoKWAgYWZ0ZXIgYHVwZ3JhZGUoKWAgdG8gYQpidWlsZCB0aGF0IGJ1bXBzIGBTQ0hFTUFfVkVSU0lPTmA7IHRoaXMgZXJyb3IgaXMgYSBzYWZlIG5vLW9wIHNpZ25hbAp0aGF0IHRoZXJlIGlzIG5vdGhpbmcgcGVuZGluZywgbm90IGEgZmFpbHVyZSB0byByZWFjdCB0by4AAAAAABRNaWdyYXRpb25Ob3RSZXF1aXJlZAAAAAYAAABVVGhlIHJlZmVyZW5jZWQgYXR0ZXN0ZXIgaXMgbm90IGN1cnJlbnRseSBhbGxvd2xpc3RlZCAobmV2ZXIgYWRkZWQsCm9yIHNpbmNlIHJlbW92ZWQpLgAAAAAAABBBdHRlc3Rlck5vdEZvdW5kAAAABwAAADNUaGUgc3VwcGxpZWQgYmF0Y2ggZXhjZWVkcyBgQkFUQ0hfTElNSVRgIGFkZHJlc3Nlcy4AAAAADUJhdGNoVG9vTGFyZ2UAAAAAAAAI",
        "AAAABQAAADJFbWl0dGVkIHdoZW4gc3RhdGUtY2hhbmdpbmcgb3BlcmF0aW9ucyBhcmUgcGF1c2VkLgAAAAAAAAAAAAZQYXVzZWQAAAAAAAEAAAAGcGF1c2VkAAAAAAABAAAAAAAAAAJieQAAAAAAEwAAAAEAAAAC",
        "AAAABQAAADRFbWl0dGVkIHdoZW4gc3RhdGUtY2hhbmdpbmcgb3BlcmF0aW9ucyBhcmUgdW5wYXVzZWQuAAAAAAAAAAhVbnBhdXNlZAAAAAEAAAAIdW5wYXVzZWQAAAABAAAAAAAAAAJieQAAAAAAEwAAAAEAAAAC",
        "AAAABQAAADJFbWl0dGVkIHdoZW4gdGhlIGNvbnRyYWN0IGlzIHVwZ3JhZGVkIHRvIG5ldyB3YXNtLgAAAAAAAAAAAAhVcGdyYWRlZAAAAAEAAAAIdXBncmFkZWQAAAABAAAAAAAAAA1uZXdfd2FzbV9oYXNoAAAAAAAD7gAAACAAAAABAAAAAg==",
        "AAAABQAAAC9FbWl0dGVkIG9uY2UsIHdoZW4gdGhlIGNvbnRyYWN0IGlzIGluaXRpYWxpemVkLgAAAAAAAAAAC0luaXRpYWxpemVkAAAAAAEAAAALaW5pdGlhbGl6ZWQAAAAAAQAAAAAAAAAFYWRtaW4AAAAAAAATAAAAAQAAAAI=",
        "AAAAAQAAADFNZXRhZGF0YSBhc3NvY2lhdGVkIHdpdGggYW4gYWxsb3dsaXN0ZWQgYXR0ZXN0ZXIuAAAAAAAAAAAAAAxBdHRlc3RlckluZm8AAAACAAAARUhhc2ggb2YgdGhlIGF0dGVzdGVyJ3Mgb2ZmLWNoYWluIGxpY2Vuc2UvY3JlZGVudGlhbCBkb2N1bWVudCwgaWYgYW55LgAAAAAAAAxsaWNlbnNlX2hhc2gAAAPoAAAD7gAAACAAAABHVGhlIGdlb2dyYXBoaWMgcmVnaW9uIHRoZSBhdHRlc3RlciBpcyBhdXRob3JpemVkIHRvIGF0dGVzdCBmb3IsIGlmIGFueS4AAAAABnJlZ2lvbgAAAAAD6AAAABE=",
        "AAAABQAAADNFbWl0dGVkIHdoZW4gYW4gYXR0ZXN0ZXIgaXMgYWRkZWQgdG8gdGhlIGFsbG93bGlzdC4AAAAAAAAAAA1BdHRlc3RlckFkZGVkAAAAAAAAAQAAAA5hdHRlc3Rlcl9hZGRlZAAAAAAAAQAAAAAAAAAIYXR0ZXN0ZXIAAAATAAAAAQAAAAI=",
        "AAAAAQAAAHRBbiBhbGxvd2xpc3RlZCBhdHRlc3RlcidzIG1ldGFkYXRhIHRvZ2V0aGVyIHdpdGggaXRzIGN1cnJlbnQgc3VzcGVuc2lvbgpzdGF0ZSwgYXMgcmV0dXJuZWQgYnkgYGdldF9hdHRlc3Rlcl9zdGF0dXNgLgAAAAAAAAAOQXR0ZXN0ZXJTdGF0dXMAAAAAAAIAAAAfVGhlIGF0dGVzdGVyJ3Mgc3RvcmVkIG1ldGFkYXRhLgAAAAAEaW5mbwAAB9AAAAAMQXR0ZXN0ZXJJbmZvAAAALFdoZXRoZXIgdGhlIGF0dGVzdGVyIGlzIGN1cnJlbnRseSBzdXNwZW5kZWQuAAAACXN1c3BlbmRlZAAAAAAAAAE=",
        "AAAABQAAADdFbWl0dGVkIHdoZW4gYW4gYXR0ZXN0ZXIgaXMgcmVtb3ZlZCBmcm9tIHRoZSBhbGxvd2xpc3QuAAAAAAAAAAAPQXR0ZXN0ZXJSZW1vdmVkAAAAAAEAAAAQYXR0ZXN0ZXJfcmVtb3ZlZAAAAAEAAAAAAAAACGF0dGVzdGVyAAAAEwAAAAEAAAAC",
        "AAAABQAAAERFbWl0dGVkIHdoZW4gYWRtaW4gb3duZXJzaGlwIGZpbmlzaGVzIHRyYW5zZmVycmluZyB0byBhIG5ldyBhZGRyZXNzLgAAAAAAAAAQQWRtaW5UcmFuc2ZlcnJlZAAAAAEAAAARYWRtaW5fdHJhbnNmZXJyZWQAAAAAAAACAAAAAAAAAA5wcmV2aW91c19hZG1pbgAAAAAAEwAAAAEAAAAAAAAACW5ld19hZG1pbgAAAAAAABMAAAABAAAAAg==",
        "AAAABQAAACZFbWl0dGVkIHdoZW4gYW4gYXR0ZXN0ZXIgaXMgc3VzcGVuZGVkLgAAAAAAAAAAABFBdHRlc3RlclN1c3BlbmRlZAAAAAAAAAEAAAASYXR0ZXN0ZXJfc3VzcGVuZGVkAAAAAAABAAAAAAAAAAhhdHRlc3RlcgAAABMAAAABAAAAAg==",
        "AAAABQAAADBFbWl0dGVkIHdoZW4gYSBzdXNwZW5kZWQgYXR0ZXN0ZXIgaXMgcmVpbnN0YXRlZC4AAAAAAAAAEkF0dGVzdGVyUmVpbnN0YXRlZAAAAAAAAQAAABNhdHRlc3Rlcl9yZWluc3RhdGVkAAAAAAEAAAAAAAAACGF0dGVzdGVyAAAAEwAAAAEAAAAC",
        "AAAABQAAALFFbWl0dGVkIHdoZW4gYW4gYWxyZWFkeS1hbGxvd2xpc3RlZCBhdHRlc3RlcidzIG1ldGFkYXRhIGlzIHVwZGF0ZWQgdmlhCmB1cGRhdGVfYXR0ZXN0ZXJfaW5mb2AuIERpc3Rpbmd1aXNoYWJsZSBmcm9tIGBBdHRlc3RlckFkZGVkYCwgd2hpY2ggaXMKb25seSBlbWl0dGVkIG9uIGluaXRpYWwgZW5yb2xsbWVudC4AAAAAAAAAAAAAE0F0dGVzdGVySW5mb1VwZGF0ZWQAAAAAAQAAABVhdHRlc3Rlcl9pbmZvX3VwZGF0ZWQAAAAAAAABAAAAAAAAAAhhdHRlc3RlcgAAABMAAAABAAAAAg==",
        "AAAAAAAAAN1QYXVzZSB0aGUgY29udHJhY3QsIGJsb2NraW5nIGBhZGRfYXR0ZXN0ZXJgLCBgYWRkX2F0dGVzdGVyX3dpdGhfaW5mb2AsCmB1cGRhdGVfYXR0ZXN0ZXJfaW5mb2AsIGByZW1vdmVfYXR0ZXN0ZXJgLCBgc3VzcGVuZF9hdHRlc3RlcmAsIGFuZApgcmVpbnN0YXRlX2F0dGVzdGVyYCB1bnRpbCBgdW5wYXVzZWAgaXMgY2FsbGVkLiBSZXF1aXJlcyB0aGUgYWRtaW4ncwphdXRob3JpemF0aW9uLgAAAAAAAAVwYXVzZQAAAAAAAAAAAAABAAAD6QAAAAIAAAAD",
        "AAAAAAAAAe1SdW4gYW55IHBlbmRpbmcgc3RvcmFnZSBtaWdyYXRpb24sIHRoZW4gcmVjb3JkIHRoZSBuZXcgc2NoZW1hCnZlcnNpb24uIFJlcXVpcmVzIHRoZSBhZG1pbidzIGF1dGhvcml6YXRpb24uCgpDYWxsIHRoaXMgYWZ0ZXIgYHVwZ3JhZGUoKWAgb25seSB3aGVuIHRoZSBuZXcgYnVpbGQgYnVtcHMKYFNDSEVNQV9WRVJTSU9OYCAoYSBzdG9yYWdlLXNjaGVtYS1jaGFuZ2luZyByZWxlYXNlKSDigJQgaW5jbHVkaW5nIHRoZQpmaXJzdCB1cGdyYWRlIG9mIGEgbGVnYWN5IChwcmUtdmVyc2lvbmluZywgc2NoZW1hIHZlcnNpb24gYDBgKQppbnN0YW5jZSwgd2hpY2ggbXVzdCBiZSBtaWdyYXRlZCB0byB2ZXJzaW9uIDEuIFdoZW4gbm8gbWlncmF0aW9uIGlzCnBlbmRpbmcgKGBTY2hlbWFWZXJzaW9uID49IFNDSEVNQV9WRVJTSU9OYCkgdGhpcyByZXR1cm5zCmBFcnJvcjo6TWlncmF0aW9uTm90UmVxdWlyZWRgIHNvIHRoZSBjYWxsIGNhbid0IGFjY2lkZW50YWxseSByZS1ydW4uAAAAAAAAB21pZ3JhdGUAAAAAAAAAAAEAAAPpAAAAAgAAAAM=",
        "AAAAAAAAAExSZXN1bWUgbm9ybWFsIG9wZXJhdGlvbiBhZnRlciBhIGBwYXVzZWAuIFJlcXVpcmVzIHRoZSBhZG1pbidzIGF1dGhvcml6YXRpb24uAAAAB3VucGF1c2UAAAAAAAAAAAEAAAPpAAAAAgAAAAM=",
        "AAAAAAAAAhNVcGdyYWRlIHRoZSBjb250cmFjdCdzIFdhc20gY29kZSB0byBhIG5ldyB2ZXJzaW9uLgpSZXF1aXJlcyB0aGUgYWRtaW4ncyBhdXRob3JpemF0aW9uLgoKUnVuYm9vazoKMS4gQnVpbGQgdGhlIG5ldyBXYXNtIGJpbmFyeSAoZS5nLiBgY2FyZ28gYnVpbGQgLS13b3Jrc3BhY2UgLS1yZWxlYXNlIC0tdGFyZ2V0IHdhc20zMnYxLW5vbmVgKS4KMi4gVXBsb2FkL2luc3RhbGwgdGhlIG5ldyBXYXNtIG9uLWNoYWluIHRvIG9idGFpbiBpdHMgMzItYnl0ZSBoYXNoIChgbmV3X3dhc21faGFzaGApLgozLiBUaGUgYWRtaW4gY2FsbHMgdGhpcyBgdXBncmFkZWAgZnVuY3Rpb24gcGFzc2luZyB0aGUgYG5ld193YXNtX2hhc2hgLgoKRm9yIGFueSBhY2NvbXBhbnlpbmcgc3RhdGUvZGF0YSBtaWdyYXRpb25zLCBzZWUgdGhlIHN0b3JhZ2UtdmVyc2lvbmluZyBndWlkZWxpbmVzCihlLmcuIGltcGxlbWVudGluZyBtaWdyYXRpb24gc2NyaXB0cyBvciBoYW5kbGluZyBsYXp5IG1pZ3JhdGlvbnMgb24gcmVhZGluZyBvbGQgc2NoZW1hIHZlcnNpb25zKS4AAAAAB3VwZ3JhZGUAAAAAAQAAAAAAAAANbmV3X3dhc21faGFzaAAAAAAAA+4AAAAgAAAAAQAAA+kAAAACAAAAAw==",
        "AAAAAAAAACFSZXR1cm4gdGhlIGN1cnJlbnQgYWRtaW4gYWRkcmVzcy4AAAAAAAAJZ2V0X2FkbWluAAAAAAAAAAAAAAEAAAPpAAAAEwAAAAM=",
        "AAAAAAAAAClXaGV0aGVyIHRoZSBjb250cmFjdCBpcyBjdXJyZW50bHkgcGF1c2VkLgAAAAAAAAlpc19wYXVzZWQAAAAAAAAAAAAAAQAAAAE=",
        "AAAAAAAAAIJTZXQgdGhlIGFkbWluIGFkZHJlc3MgYXV0aG9yaXplZCB0byBtYW5hZ2UgdGhlIGFsbG93bGlzdC4gQ2FuIG9ubHkKYmUgY2FsbGVkIG9uY2U7IHRoZSBjYWxsZXIgbXVzdCBhdXRob3JpemUgYXMgdGhlIGdpdmVuIGBhZG1pbmAuAAAAAAAKaW5pdGlhbGl6ZQAAAAAAAQAAAAAAAAAFYWRtaW4AAAAAAAATAAAAAQAAA+kAAAACAAAAAw==",
        "AAAAAAAAAI1XaGV0aGVyIGBhdHRlc3RlcmAgaXMgY3VycmVudGx5IGFsbG93bGlzdGVkIChhbmQgbm90IHN1c3BlbmRlZCkuIENhbGxhYmxlIGJ5IGFueW9uZSwKaW5jbHVkaW5nIG90aGVyIGNvbnRyYWN0cyAoZS5nLiBgYXR0ZXN0YXRpb24tcmVnaXN0cnlgKS4AAAAAAAALaXNfYXR0ZXN0ZXIAAAAAAQAAAAAAAAAIYXR0ZXN0ZXIAAAATAAAAAQAAAAE=",
        "AAAAAAAAAFNBY2NlcHQgdGhlIHByb3Bvc2VkIGFkbWluIHRyYW5zZmVyLiBUaGUgY2FsbGVyIG11c3QgYXV0aG9yaXplIGFzIHRoZSBwZW5kaW5nIGFkbWluLgAAAAAMYWNjZXB0X2FkbWluAAAAAAAAAAEAAAPpAAAAAgAAAAM=",
        "AAAAAAAAAMdBZGQgYGF0dGVzdGVyYCB0byB0aGUgYWxsb3dsaXN0LiBSZXF1aXJlcyB0aGUgYWRtaW4ncyBhdXRob3JpemF0aW9uLgpGYWlscyB3aXRoIGBFcnJvcjo6QWxsb3dsaXN0RnVsbGAgaWYgdGhlIGFsbG93bGlzdCBpcyBhdCBjYXBhY2l0eSBhbmQKYGF0dGVzdGVyYCBpcyBub3QgYWxyZWFkeSBwcmVzZW50IChzZWUgYHNldF9tYXhfYXR0ZXN0ZXJzYCkuAAAAAAxhZGRfYXR0ZXN0ZXIAAAABAAAAAAAAAAhhdHRlc3RlcgAAABMAAAABAAAD6QAAAAIAAAAD",
        "AAAAAAAAAjZBZGQgbXVsdGlwbGUgYXR0ZXN0ZXJzIHRvIHRoZSBhbGxvd2xpc3QgaW4gYSBzaW5nbGUgdHJhbnNhY3Rpb24uCgpSZXF1aXJlcyB0aGUgYWRtaW4ncyBhdXRob3JpemF0aW9uLiBCbG9ja2VkIHdoaWxlIHRoZSBjb250cmFjdCBpcyBwYXVzZWQuClJldHVybnMgYEVycm9yOjpCYXRjaFRvb0xhcmdlYCBpZiBgYXR0ZXN0ZXJzLmxlbigpID4gQkFUQ0hfTElNSVRgLgpSZXR1cm5zIGBFcnJvcjo6QWxsb3dsaXN0RnVsbGAgaWYgYWRkaW5nIHRoZSBuZXcgKG5vbi1kdXBsaWNhdGUpCmFkZHJlc3NlcyB3b3VsZCBleGNlZWQgdGhlIGNvbmZpZ3VyZWQgYG1heF9hdHRlc3RlcnNgIGNhcC4gQWRkcmVzc2VzCnRoYXQgYXJlIGFscmVhZHkgYWxsb3dsaXN0ZWQgYXJlIHNpbGVudGx5IHNraXBwZWQgKGlkZW1wb3RlbnQpLCBzbyB0aGUKY2FsbCBuZXZlciBmYWlscyBkdWUgdG8gZHVwbGljYXRlcyBpbiB0aGUgYmF0Y2ggYW5kIG5vIGR1cGxpY2F0ZSBldmVudHMKYXJlIGVtaXR0ZWQuIEV4YWN0bHkgb25lIGBBdHRlc3RlckFkZGVkYCBldmVudCBpcyBlbWl0dGVkIHBlciBuZXdseQphZGRlZCBhZGRyZXNzLgAAAAAADWFkZF9hdHRlc3RlcnMAAAAAAAABAAAAAAAAAAlhdHRlc3RlcnMAAAAAAAPqAAAAEwAAAAEAAAPpAAAAAgAAAAM=",
        "AAAAAAAAALxQcm9wb3NlIGEgbmV3IGFkbWluIGFkZHJlc3MuIFRoZSBjYWxsZXIgbXVzdCBhdXRob3JpemUgYXMgdGhlIGN1cnJlbnQgYWRtaW4uCkNhbGxpbmcgdGhpcyBhIHNlY29uZCB0aW1lIGJlZm9yZSBgYWNjZXB0X2FkbWluYCBvdmVyd3JpdGVzIGFueSBwZW5kaW5nIHByb3Bvc2FsIOKAlCB0aGUgbW9zdCByZWNlbnQgY2FsbCB3aW5zLgAAAA1wcm9wb3NlX2FkbWluAAAAAAAAAQAAAAAAAAAJbmV3X2FkbWluAAAAAAAAEwAAAAEAAAPpAAAAAgAAAAM=",
        "AAAAAAAAAHhSZW1vdmUgYGF0dGVzdGVyYCBmcm9tIHRoZSBhbGxvd2xpc3QuIFJlcXVpcmVzIHRoZSBhZG1pbidzCmF1dGhvcml6YXRpb24uIEEgbm8tb3AgaWYgdGhlIGF0dGVzdGVyIHdhcyBuZXZlciBhbGxvd2xpc3RlZC4AAAAPcmVtb3ZlX2F0dGVzdGVyAAAAAAEAAAAAAAAACGF0dGVzdGVyAAAAEwAAAAEAAAPpAAAAAgAAAAM=",
        "AAAAAAAAAddSZW1vdmUgbXVsdGlwbGUgYXR0ZXN0ZXJzIGZyb20gdGhlIGFsbG93bGlzdCBpbiBhIHNpbmdsZSB0cmFuc2FjdGlvbi4KClJlcXVpcmVzIHRoZSBhZG1pbidzIGF1dGhvcml6YXRpb24uIEJsb2NrZWQgd2hpbGUgdGhlIGNvbnRyYWN0IGlzIHBhdXNlZC4KUmV0dXJucyBgRXJyb3I6OkJhdGNoVG9vTGFyZ2VgIGlmIGBhdHRlc3RlcnMubGVuKCkgPiBCQVRDSF9MSU1JVGAuCkFkZHJlc3NlcyB0aGF0IGFyZSBub3QgY3VycmVudGx5IGFsbG93bGlzdGVkIGFyZSBzaWxlbnRseSBza2lwcGVkCihpZGVtcG90ZW50KSwgc28gdGhlIGNhbGwgbmV2ZXIgZmFpbHMgaWYgYW4gYWRkcmVzcyB3YXMgYWxyZWFkeSByZW1vdmVkCmFuZCBubyBzcHVyaW91cyBldmVudHMgYXJlIGVtaXR0ZWQuIEV4YWN0bHkgb25lIGBBdHRlc3RlclJlbW92ZWRgIGV2ZW50CmlzIGVtaXR0ZWQgcGVyIGFkZHJlc3MgdGhhdCB3YXMgYWN0dWFsbHkgcmVtb3ZlZC4AAAAAEHJlbW92ZV9hdHRlc3RlcnMAAAABAAAAAAAAAAlhdHRlc3RlcnMAAAAAAAPqAAAAEwAAAAEAAAPpAAAAAgAAAAM=",
        "AAAAAAAAAuBTdXNwZW5kIGFuIGFsbG93bGlzdGVkIGF0dGVzdGVyLiBSZXF1aXJlcyB0aGUgYWRtaW4ncyBhdXRob3JpemF0aW9uLgoKKipOb3RlOioqIHRoaXMgZnVuY3Rpb24gZG9lcyAqKm5vdCoqIGNoZWNrIHdoZXRoZXIgYGF0dGVzdGVyYCB3YXMgZXZlcgphZGRlZCB2aWEgYGFkZF9hdHRlc3RlcmAuIElmIGNhbGxlZCBvbiBhbiBhZGRyZXNzIHRoYXQgaXMgbm90IGluIHRoZQphbGxvd2xpc3QsIGl0IHNpbGVudGx5IHNldHMgdGhlIGBTdXNwZW5kZWRgIHN0b3JhZ2Uga2V5IGFuZCBlbWl0cwpgQXR0ZXN0ZXJTdXNwZW5kZWRgIGZvciB0aGF0IGFkZHJlc3Mg4oCUIGEgbm8tb3AgZnJvbSBhbiBhY2Nlc3MtY29udHJvbApwZXJzcGVjdGl2ZSBiZWNhdXNlIGBpc19hdHRlc3RlcmAgYWxzbyBjaGVja3MgZm9yIGFuIGBBdHRlc3RlcmAgc3RvcmFnZQplbnRyeSwgc28gdGhlIHBoYW50b20gc3VzcGVuc2lvbiBoYXMgbm8gZWZmZWN0IG9uIGFsbG93bGlzdCBxdWVyaWVzLgpUaGlzIGRpdmVyZ2VzIGZyb20gYHVwZGF0ZV9hdHRlc3Rlcl9pbmZvYCwgd2hpY2ggcmV0dXJucwpgRXJyb3I6OkF0dGVzdGVyTm90Rm91bmRgIGZvciB1bmtub3duIGFkZHJlc3Nlcy4gVGhlIGluY29uc2lzdGVuY3kgaXMKa25vd24gYW5kIGRvY3VtZW50ZWQgaGVyZSByYXRoZXIgdGhhbiBzaWxlbnRseSBjaGFuZ2VkOyBhIGZvbGxvdy11cAppc3N1ZSBzaG91bGQgZGVjaWRlIHdoZXRoZXIgdG8gYWxpZ24gYm90aCBmdW5jdGlvbnMuAAAAEHN1c3BlbmRfYXR0ZXN0ZXIAAAABAAAAAAAAAAhhdHRlc3RlcgAAABMAAAABAAAD6QAAAAIAAAAD",
        "AAAAAAAAAE1HZXQgdGhlIG9wdGlvbmFsIG1ldGFkYXRhIGFzc29jaWF0ZWQgd2l0aCBgYXR0ZXN0ZXJgIGlmIHRoZXkgYXJlIGFsbG93bGlzdGVkLgAAAAAAABFnZXRfYXR0ZXN0ZXJfaW5mbwAAAAAAAAEAAAAAAAAACGF0dGVzdGVyAAAAEwAAAAEAAAPoAAAH0AAAAAxBdHRlc3RlckluZm8=",
        "AAAAAAAAADxUaGUgY3VycmVudCBzb2Z0IGNhcCBvbiB0aGUgbnVtYmVyIG9mIGFsbG93bGlzdGVkIGF0dGVzdGVycy4AAAARZ2V0X21heF9hdHRlc3RlcnMAAAAAAAAAAAAAAQAAAAQ=",
        "AAAAAAAAAM9TZXQgdGhlIHNvZnQgY2FwIG9uIHRoZSBudW1iZXIgb2YgYWxsb3dsaXN0ZWQgYXR0ZXN0ZXJzLiBSZXF1aXJlcyB0aGUKYWRtaW4ncyBhdXRob3JpemF0aW9uLiBEb2VzIG5vdCBldmljdCBleGlzdGluZyBhdHRlc3RlcnMgaWYgbG93ZXJlZApiZWxvdyB0aGUgY3VycmVudCBjb3VudDsgaXQgb25seSBibG9ja3MgZnVydGhlciBgYWRkX2F0dGVzdGVyYCBjYWxscy4AAAAAEXNldF9tYXhfYXR0ZXN0ZXJzAAAAAAAAAQAAAAAAAAANbWF4X2F0dGVzdGVycwAAAAAAAAQAAAABAAAD6QAAAAIAAAAD",
        "AAAAAAAAACxUaGUgY3VycmVudCBudW1iZXIgb2YgYWxsb3dsaXN0ZWQgYXR0ZXN0ZXJzLgAAABJnZXRfYXR0ZXN0ZXJfY291bnQAAAAAAAAAAAABAAAABA==",
        "AAAAAAAAADlRdWVyeSB0aGUgY3VycmVudCBzdG9yYWdlIHNjaGVtYSB2ZXJzaW9uIG9mIHRoZSBjb250cmFjdC4AAAAAAAASZ2V0X3NjaGVtYV92ZXJzaW9uAAAAAAAAAAAAAQAAAAQ=",
        "AAAAAAAAAENSZWluc3RhdGUgYSBzdXNwZW5kZWQgYXR0ZXN0ZXIuIFJlcXVpcmVzIHRoZSBhZG1pbidzIGF1dGhvcml6YXRpb24uAAAAABJyZWluc3RhdGVfYXR0ZXN0ZXIAAAAAAAEAAAAAAAAACGF0dGVzdGVyAAAAEwAAAAEAAAPpAAAAAgAAAAM=",
        "AAAAAAAAALFHZXQgYGF0dGVzdGVyYCdzIG1ldGFkYXRhIHRvZ2V0aGVyIHdpdGggaXRzIGN1cnJlbnQgc3VzcGVuc2lvbiBzdGF0ZQppbiBhIHNpbmdsZSBjYWxsLiBSZXR1cm5zIGBOb25lYCBpZiBgYXR0ZXN0ZXJgIGlzIG5vdCBjdXJyZW50bHkKYWxsb3dsaXN0ZWQgKG5ldmVyIGFkZGVkLCBvciBzaW5jZSByZW1vdmVkKS4AAAAAAAATZ2V0X2F0dGVzdGVyX3N0YXR1cwAAAAABAAAAAAAAAAhhdHRlc3RlcgAAABMAAAABAAAD6AAAB9AAAAAOQXR0ZXN0ZXJTdGF0dXMAAA==",
        "AAAAAAAAAZFVcGRhdGUgdGhlIG1ldGFkYXRhIG9mIGFuIGFscmVhZHktYWxsb3dsaXN0ZWQgYGF0dGVzdGVyYC4gUmVxdWlyZXMKdGhlIGFkbWluJ3MgYXV0aG9yaXphdGlvbi4gVW5saWtlIGBhZGRfYXR0ZXN0ZXJfd2l0aF9pbmZvYCwgdGhpcwpuZXZlciBlbnJvbGxzIGEgbmV3IGF0dGVzdGVyOiBpdCBmYWlscyB3aXRoIGBFcnJvcjo6QXR0ZXN0ZXJOb3RGb3VuZGAKaWYgYGF0dGVzdGVyYCBpcyBub3QgY3VycmVudGx5IGFsbG93bGlzdGVkIChuZXZlciBhZGRlZCwgb3Igc2luY2UKcmVtb3ZlZCksIGFuZCBhbHdheXMgZW1pdHMgYEF0dGVzdGVySW5mb1VwZGF0ZWRgIHJhdGhlciB0aGFuCmBBdHRlc3RlckFkZGVkYCwgc28gcHJvZmlsZSBjaGFuZ2VzIGFyZSBkaXN0aW5ndWlzaGFibGUgZnJvbQplbnJvbGxtZW50LgAAAAAAABR1cGRhdGVfYXR0ZXN0ZXJfaW5mbwAAAAMAAAAAAAAACGF0dGVzdGVyAAAAEwAAAAAAAAAMbGljZW5zZV9oYXNoAAAD6AAAA+4AAAAgAAAAAAAAAAZyZWdpb24AAAAAA+gAAAARAAAAAQAAA+kAAAACAAAAAw==",
        "AAAAAAAAAN5BZGQgYGF0dGVzdGVyYCB3aXRoIG9wdGlvbmFsIG1ldGFkYXRhIHRvIHRoZSBhbGxvd2xpc3QuIFJlcXVpcmVzIHRoZSBhZG1pbidzIGF1dGhvcml6YXRpb24uCkZhaWxzIHdpdGggYEVycm9yOjpBbGxvd2xpc3RGdWxsYCBpZiB0aGUgYWxsb3dsaXN0IGlzIGF0IGNhcGFjaXR5IGFuZApgYXR0ZXN0ZXJgIGlzIG5vdCBhbHJlYWR5IHByZXNlbnQgKHNlZSBgc2V0X21heF9hdHRlc3RlcnNgKS4AAAAAABZhZGRfYXR0ZXN0ZXJfd2l0aF9pbmZvAAAAAAADAAAAAAAAAAhhdHRlc3RlcgAAABMAAAAAAAAADGxpY2Vuc2VfaGFzaAAAA+gAAAPuAAAAIAAAAAAAAAAGcmVnaW9uAAAAAAPoAAAAEQAAAAEAAAPpAAAAAgAAAAM=" ]),
      options
    )
  }
  public readonly fromJSON = {
    pause: this.txFromJSON<Result<void>>,
        migrate: this.txFromJSON<Result<void>>,
        unpause: this.txFromJSON<Result<void>>,
        upgrade: this.txFromJSON<Result<void>>,
        get_admin: this.txFromJSON<Result<string>>,
        is_paused: this.txFromJSON<boolean>,
        initialize: this.txFromJSON<Result<void>>,
        is_attester: this.txFromJSON<boolean>,
        accept_admin: this.txFromJSON<Result<void>>,
        add_attester: this.txFromJSON<Result<void>>,
        add_attesters: this.txFromJSON<Result<void>>,
        propose_admin: this.txFromJSON<Result<void>>,
        remove_attester: this.txFromJSON<Result<void>>,
        remove_attesters: this.txFromJSON<Result<void>>,
        suspend_attester: this.txFromJSON<Result<void>>,
        get_attester_info: this.txFromJSON<Option<AttesterInfo>>,
        get_max_attesters: this.txFromJSON<u32>,
        set_max_attesters: this.txFromJSON<Result<void>>,
        get_attester_count: this.txFromJSON<u32>,
        get_schema_version: this.txFromJSON<u32>,
        reinstate_attester: this.txFromJSON<Result<void>>,
        get_attester_status: this.txFromJSON<Option<AttesterStatus>>,
        update_attester_info: this.txFromJSON<Result<void>>,
        add_attester_with_info: this.txFromJSON<Result<void>>
  }
}