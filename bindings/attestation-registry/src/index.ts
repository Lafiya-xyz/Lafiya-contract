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
 * Errors returned by the attestation registry's public entry points.
 */
export const Errors = {
  /**
   * `initialize` has not been called yet.
   */
  1: {message:"NotInitialized"},
  /**
   * `initialize` was called more than once.
   */
  2: {message:"AlreadyInitialized"},
  /**
   * The caller is not allowlisted by the `attester-registry` contract.
   */
  3: {message:"AttesterNotAllowlisted"},
  /**
   * `accept_admin` was called with no pending admin transfer. Admin transfer is a
   * two-step flow: the current admin must first call `propose_admin` to nominate a
   * successor, then the nominated address must call `accept_admin` to complete the
   * transfer. This error is returned when `accept_admin` is called before a
   * corresponding `propose_admin` call has set a pending admin.
   */
  4: {message:"NoPendingTransfer"},
  /**
   * The configured `attester-registry` address does not implement the expected interface. Re-run `set_attester_registry` with the correct address, or check your network configuration.
   */
  5: {message:"InvalidRegistryWiring"},
  /**
   * No attestation exists for the given record hash / sequence.
   */
  6: {message:"AttestationNotFound"},
  /**
   * The requested operation is blocked while the contract is paused.
   */
  7: {message:"ContractPaused"}
}




/**
 * A single attestation: proof that `attester` verified the off-chain
 * record whose hash is the lookup key, at `timestamp`. Never contains the
 * underlying health data.
 */
export interface Attestation {
  /**
 * The allowlisted attester that verified the record.
 */
attester: string;
  /**
 * Ledger timestamp at which the attestation was recorded.
 */
timestamp: u64;
}





export interface Client {
  /**
   * Construct and simulate a pause transaction. Returns an `AssembledTransaction` object which will have a `result` field containing the result of the simulation. If this transaction changes contract state, you will need to call `signAndSend()` on the returned object.
   * Pause the contract, blocking `attest` until `unpause` is called.
   * Requires the admin's authorization.
   */
  pause: (options?: MethodOptions) => Promise<AssembledTransaction<Result<void>>>

  /**
   * Construct and simulate a attest transaction. Returns an `AssembledTransaction` object which will have a `result` field containing the result of the simulation. If this transaction changes contract state, you will need to call `signAndSend()` on the returned object.
   * Record that `attester` verified the record hashing to `record_hash`.
   * Requires `attester`'s authorization and that `attester` is
   * currently allowlisted in the configured `attester-registry`.
   * Stores the attestation with an incrementing sequence number,
   * maintaining a bounded history (MAX_HISTORY entries per hash).
   */
  attest: ({attester, record_hash}: {attester: string, record_hash: Buffer}, options?: MethodOptions) => Promise<AssembledTransaction<Result<Attestation>>>

  /**
   * Construct and simulate a unpause transaction. Returns an `AssembledTransaction` object which will have a `result` field containing the result of the simulation. If this transaction changes contract state, you will need to call `signAndSend()` on the returned object.
   * Resume normal operation after a `pause`. Requires the admin's authorization.
   */
  unpause: (options?: MethodOptions) => Promise<AssembledTransaction<Result<void>>>

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
   * Set the admin and the `attester-registry` contract this registry
   * consults for allowlist checks. Can only be called once; the caller
   * must authorize as the given `admin`.
   * 
   * ## Best-effort interface check
   * 
   * This function performs a lightweight sanity check against
   * `attester_registry`: it calls `is_attester` with a throwaway address
   * and confirms the call does not trap. This confirms the address
   * implements the expected interface — it does **not** prove the address
   * is the canonical, trusted `attester-registry` deployment. A malicious
   * contract that happens to expose `is_attester` would pass this check.
   */
  initialize: ({admin, attester_registry}: {admin: string, attester_registry: string}, options?: MethodOptions) => Promise<AssembledTransaction<Result<void>>>

  /**
   * Construct and simulate a accept_admin transaction. Returns an `AssembledTransaction` object which will have a `result` field containing the result of the simulation. If this transaction changes contract state, you will need to call `signAndSend()` on the returned object.
   * Accept the proposed admin transfer. The caller must authorize as the pending admin.
   */
  accept_admin: (options?: MethodOptions) => Promise<AssembledTransaction<Result<void>>>

  /**
   * Construct and simulate a propose_admin transaction. Returns an `AssembledTransaction` object which will have a `result` field containing the result of the simulation. If this transaction changes contract state, you will need to call `signAndSend()` on the returned object.
   * Propose a new admin address. The caller must authorize as the current admin.
   */
  propose_admin: ({new_admin}: {new_admin: string}, options?: MethodOptions) => Promise<AssembledTransaction<Result<void>>>

  /**
   * Construct and simulate a get_attestation transaction. Returns an `AssembledTransaction` object which will have a `result` field containing the result of the simulation. If this transaction changes contract state, you will need to call `signAndSend()` on the returned object.
   * Look up the latest attestation for `record_hash`, if any. Callable
   * by anyone — this is what lets a responder's QR scan independently
   * check a card without an external oracle.
   */
  get_attestation: ({record_hash}: {record_hash: Buffer}, options?: MethodOptions) => Promise<AssembledTransaction<Option<Attestation>>>

  /**
   * Construct and simulate a revoke_attestation transaction. Returns an `AssembledTransaction` object which will have a `result` field containing the result of the simulation. If this transaction changes contract state, you will need to call `signAndSend()` on the returned object.
   * Revoke all attestations for `record_hash`. Gated by admin authorization.
   */
  revoke_attestation: ({record_hash}: {record_hash: Buffer}, options?: MethodOptions) => Promise<AssembledTransaction<Result<void>>>

  /**
   * Construct and simulate a get_attester_registry transaction. Returns an `AssembledTransaction` object which will have a `result` field containing the result of the simulation. If this transaction changes contract state, you will need to call `signAndSend()` on the returned object.
   * Return the configured attester-registry contract address.
   */
  get_attester_registry: (options?: MethodOptions) => Promise<AssembledTransaction<Result<string>>>

  /**
   * Construct and simulate a set_attester_registry transaction. Returns an `AssembledTransaction` object which will have a `result` field containing the result of the simulation. If this transaction changes contract state, you will need to call `signAndSend()` on the returned object.
   * Change the attester-registry contract this registry consults for
   * allowlist checks. Requires the admin's authorization. Emits
   * `AttesterRegistryRepointed` for indexer/audit visibility.
   */
  set_attester_registry: ({new_registry}: {new_registry: string}, options?: MethodOptions) => Promise<AssembledTransaction<Result<void>>>

  /**
   * Construct and simulate a get_attestation_history transaction. Returns an `AssembledTransaction` object which will have a `result` field containing the result of the simulation. If this transaction changes contract state, you will need to call `signAndSend()` on the returned object.
   * Look up the full attestation history for `record_hash`, if any.
   * Returns attestations in chronological order (oldest first).
   * Callable by anyone.
   */
  get_attestation_history: ({record_hash}: {record_hash: Buffer}, options?: MethodOptions) => Promise<AssembledTransaction<Array<Attestation>>>

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
      new ContractSpec([ "AAAABAAAAEJFcnJvcnMgcmV0dXJuZWQgYnkgdGhlIGF0dGVzdGF0aW9uIHJlZ2lzdHJ5J3MgcHVibGljIGVudHJ5IHBvaW50cy4AAAAAAAAAAAAFRXJyb3IAAAAAAAAHAAAAJWBpbml0aWFsaXplYCBoYXMgbm90IGJlZW4gY2FsbGVkIHlldC4AAAAAAAAOTm90SW5pdGlhbGl6ZWQAAAAAAAEAAAAnYGluaXRpYWxpemVgIHdhcyBjYWxsZWQgbW9yZSB0aGFuIG9uY2UuAAAAABJBbHJlYWR5SW5pdGlhbGl6ZWQAAAAAAAIAAABCVGhlIGNhbGxlciBpcyBub3QgYWxsb3dsaXN0ZWQgYnkgdGhlIGBhdHRlc3Rlci1yZWdpc3RyeWAgY29udHJhY3QuAAAAAAAWQXR0ZXN0ZXJOb3RBbGxvd2xpc3RlZAAAAAAAAwAAAW9gYWNjZXB0X2FkbWluYCB3YXMgY2FsbGVkIHdpdGggbm8gcGVuZGluZyBhZG1pbiB0cmFuc2Zlci4gQWRtaW4gdHJhbnNmZXIgaXMgYQp0d28tc3RlcCBmbG93OiB0aGUgY3VycmVudCBhZG1pbiBtdXN0IGZpcnN0IGNhbGwgYHByb3Bvc2VfYWRtaW5gIHRvIG5vbWluYXRlIGEKc3VjY2Vzc29yLCB0aGVuIHRoZSBub21pbmF0ZWQgYWRkcmVzcyBtdXN0IGNhbGwgYGFjY2VwdF9hZG1pbmAgdG8gY29tcGxldGUgdGhlCnRyYW5zZmVyLiBUaGlzIGVycm9yIGlzIHJldHVybmVkIHdoZW4gYGFjY2VwdF9hZG1pbmAgaXMgY2FsbGVkIGJlZm9yZSBhCmNvcnJlc3BvbmRpbmcgYHByb3Bvc2VfYWRtaW5gIGNhbGwgaGFzIHNldCBhIHBlbmRpbmcgYWRtaW4uAAAAABFOb1BlbmRpbmdUcmFuc2ZlcgAAAAAAAAQAAACzVGhlIGNvbmZpZ3VyZWQgYGF0dGVzdGVyLXJlZ2lzdHJ5YCBhZGRyZXNzIGRvZXMgbm90IGltcGxlbWVudCB0aGUgZXhwZWN0ZWQgaW50ZXJmYWNlLiBSZS1ydW4gYHNldF9hdHRlc3Rlcl9yZWdpc3RyeWAgd2l0aCB0aGUgY29ycmVjdCBhZGRyZXNzLCBvciBjaGVjayB5b3VyIG5ldHdvcmsgY29uZmlndXJhdGlvbi4AAAAAFUludmFsaWRSZWdpc3RyeVdpcmluZwAAAAAAAAUAAAA7Tm8gYXR0ZXN0YXRpb24gZXhpc3RzIGZvciB0aGUgZ2l2ZW4gcmVjb3JkIGhhc2ggLyBzZXF1ZW5jZS4AAAAAE0F0dGVzdGF0aW9uTm90Rm91bmQAAAAABgAAAEBUaGUgcmVxdWVzdGVkIG9wZXJhdGlvbiBpcyBibG9ja2VkIHdoaWxlIHRoZSBjb250cmFjdCBpcyBwYXVzZWQuAAAADkNvbnRyYWN0UGF1c2VkAAAAAAAH",
        "AAAABQAAADJFbWl0dGVkIHdoZW4gc3RhdGUtY2hhbmdpbmcgb3BlcmF0aW9ucyBhcmUgcGF1c2VkLgAAAAAAAAAAAAZQYXVzZWQAAAAAAAEAAAAGcGF1c2VkAAAAAAABAAAAAAAAAAJieQAAAAAAEwAAAAEAAAAC",
        "AAAABQAAADRFbWl0dGVkIHdoZW4gc3RhdGUtY2hhbmdpbmcgb3BlcmF0aW9ucyBhcmUgdW5wYXVzZWQuAAAAAAAAAAhVbnBhdXNlZAAAAAEAAAAIdW5wYXVzZWQAAAABAAAAAAAAAAJieQAAAAAAEwAAAAEAAAAC",
        "AAAAAQAAAKJBIHNpbmdsZSBhdHRlc3RhdGlvbjogcHJvb2YgdGhhdCBgYXR0ZXN0ZXJgIHZlcmlmaWVkIHRoZSBvZmYtY2hhaW4KcmVjb3JkIHdob3NlIGhhc2ggaXMgdGhlIGxvb2t1cCBrZXksIGF0IGB0aW1lc3RhbXBgLiBOZXZlciBjb250YWlucyB0aGUKdW5kZXJseWluZyBoZWFsdGggZGF0YS4AAAAAAAAAAAALQXR0ZXN0YXRpb24AAAAAAgAAADJUaGUgYWxsb3dsaXN0ZWQgYXR0ZXN0ZXIgdGhhdCB2ZXJpZmllZCB0aGUgcmVjb3JkLgAAAAAACGF0dGVzdGVyAAAAEwAAADdMZWRnZXIgdGltZXN0YW1wIGF0IHdoaWNoIHRoZSBhdHRlc3RhdGlvbiB3YXMgcmVjb3JkZWQuAAAAAAl0aW1lc3RhbXAAAAAAAAAG",
        "AAAABQAAAERFbWl0dGVkIHdoZW4gYWRtaW4gb3duZXJzaGlwIGZpbmlzaGVzIHRyYW5zZmVycmluZyB0byBhIG5ldyBhZGRyZXNzLgAAAAAAAAAQQWRtaW5UcmFuc2ZlcnJlZAAAAAEAAAARYWRtaW5fdHJhbnNmZXJyZWQAAAAAAAACAAAAAAAAAA5wcmV2aW91c19hZG1pbgAAAAAAEwAAAAEAAAAAAAAACW5ld19hZG1pbgAAAAAAABMAAAABAAAAAg==",
        "AAAABQAAACdFbWl0dGVkIHdoZW4gYW4gYXR0ZXN0YXRpb24gaXMgcmV2b2tlZC4AAAAAAAAAABJBdHRlc3RhdGlvblJldm9rZWQAAAAAAAEAAAATYXR0ZXN0YXRpb25fcmV2b2tlZAAAAAABAAAAAAAAAAtyZWNvcmRfaGFzaAAAAAPuAAAAIAAAAAEAAAAC",
        "AAAABQAAAD1FbWl0dGVkIHdoZW4gYSBuZXcgYXR0ZXN0YXRpb24gaXMgcmVjb3JkZWQgZm9yIGEgcmVjb3JkIGhhc2guAAAAAAAAAAAAABNBdHRlc3RhdGlvblJlY29yZGVkAAAAAAEAAAAUYXR0ZXN0YXRpb25fcmVjb3JkZWQAAAADAAAAAAAAAAtyZWNvcmRfaGFzaAAAAAPuAAAAIAAAAAEAAAAyVGhlIGFsbG93bGlzdGVkIGF0dGVzdGVyIHRoYXQgdmVyaWZpZWQgdGhlIHJlY29yZC4AAAAAAAhhdHRlc3RlcgAAABMAAAAAAAAAN0xlZGdlciB0aW1lc3RhbXAgYXQgd2hpY2ggdGhlIGF0dGVzdGF0aW9uIHdhcyByZWNvcmRlZC4AAAAACXRpbWVzdGFtcAAAAAAAAAYAAAAAAAAAAg==",
        "AAAABQAAAFJFbWl0dGVkIHdoZW4gdGhlIGBhdHRlc3Rlci1yZWdpc3RyeWAgY29udHJhY3QgdGhpcyByZWdpc3RyeSBjb25zdWx0cyBpcyByZXBvaW50ZWQuAAAAAAAAAAAAGUF0dGVzdGVyUmVnaXN0cnlSZXBvaW50ZWQAAAAAAAABAAAAG2F0dGVzdGVyX3JlZ2lzdHJ5X3JlcG9pbnRlZAAAAAACAAAAAAAAAAhwcmV2aW91cwAAABMAAAABAAAAAAAAAANuZXcAAAAAEwAAAAEAAAAC",
        "AAAAAAAAAGRQYXVzZSB0aGUgY29udHJhY3QsIGJsb2NraW5nIGBhdHRlc3RgIHVudGlsIGB1bnBhdXNlYCBpcyBjYWxsZWQuClJlcXVpcmVzIHRoZSBhZG1pbidzIGF1dGhvcml6YXRpb24uAAAABXBhdXNlAAAAAAAAAAAAAAEAAAPpAAAAAgAAAAM=",
        "AAAAAAAAATdSZWNvcmQgdGhhdCBgYXR0ZXN0ZXJgIHZlcmlmaWVkIHRoZSByZWNvcmQgaGFzaGluZyB0byBgcmVjb3JkX2hhc2hgLgpSZXF1aXJlcyBgYXR0ZXN0ZXJgJ3MgYXV0aG9yaXphdGlvbiBhbmQgdGhhdCBgYXR0ZXN0ZXJgIGlzCmN1cnJlbnRseSBhbGxvd2xpc3RlZCBpbiB0aGUgY29uZmlndXJlZCBgYXR0ZXN0ZXItcmVnaXN0cnlgLgpTdG9yZXMgdGhlIGF0dGVzdGF0aW9uIHdpdGggYW4gaW5jcmVtZW50aW5nIHNlcXVlbmNlIG51bWJlciwKbWFpbnRhaW5pbmcgYSBib3VuZGVkIGhpc3RvcnkgKE1BWF9ISVNUT1JZIGVudHJpZXMgcGVyIGhhc2gpLgAAAAAGYXR0ZXN0AAAAAAACAAAAAAAAAAhhdHRlc3RlcgAAABMAAAAAAAAAC3JlY29yZF9oYXNoAAAAA+4AAAAgAAAAAQAAA+kAAAfQAAAAC0F0dGVzdGF0aW9uAAAAAAM=",
        "AAAAAAAAAExSZXN1bWUgbm9ybWFsIG9wZXJhdGlvbiBhZnRlciBhIGBwYXVzZWAuIFJlcXVpcmVzIHRoZSBhZG1pbidzIGF1dGhvcml6YXRpb24uAAAAB3VucGF1c2UAAAAAAAAAAAEAAAPpAAAAAgAAAAM=",
        "AAAAAAAAACFSZXR1cm4gdGhlIGN1cnJlbnQgYWRtaW4gYWRkcmVzcy4AAAAAAAAJZ2V0X2FkbWluAAAAAAAAAAAAAAEAAAPpAAAAEwAAAAM=",
        "AAAAAAAAAClXaGV0aGVyIHRoZSBjb250cmFjdCBpcyBjdXJyZW50bHkgcGF1c2VkLgAAAAAAAAlpc19wYXVzZWQAAAAAAAAAAAAAAQAAAAE=",
        "AAAAAAAAAlpTZXQgdGhlIGFkbWluIGFuZCB0aGUgYGF0dGVzdGVyLXJlZ2lzdHJ5YCBjb250cmFjdCB0aGlzIHJlZ2lzdHJ5CmNvbnN1bHRzIGZvciBhbGxvd2xpc3QgY2hlY2tzLiBDYW4gb25seSBiZSBjYWxsZWQgb25jZTsgdGhlIGNhbGxlcgptdXN0IGF1dGhvcml6ZSBhcyB0aGUgZ2l2ZW4gYGFkbWluYC4KCiMjIEJlc3QtZWZmb3J0IGludGVyZmFjZSBjaGVjawoKVGhpcyBmdW5jdGlvbiBwZXJmb3JtcyBhIGxpZ2h0d2VpZ2h0IHNhbml0eSBjaGVjayBhZ2FpbnN0CmBhdHRlc3Rlcl9yZWdpc3RyeWA6IGl0IGNhbGxzIGBpc19hdHRlc3RlcmAgd2l0aCBhIHRocm93YXdheSBhZGRyZXNzCmFuZCBjb25maXJtcyB0aGUgY2FsbCBkb2VzIG5vdCB0cmFwLiBUaGlzIGNvbmZpcm1zIHRoZSBhZGRyZXNzCmltcGxlbWVudHMgdGhlIGV4cGVjdGVkIGludGVyZmFjZSDigJQgaXQgZG9lcyAqKm5vdCoqIHByb3ZlIHRoZSBhZGRyZXNzCmlzIHRoZSBjYW5vbmljYWwsIHRydXN0ZWQgYGF0dGVzdGVyLXJlZ2lzdHJ5YCBkZXBsb3ltZW50LiBBIG1hbGljaW91cwpjb250cmFjdCB0aGF0IGhhcHBlbnMgdG8gZXhwb3NlIGBpc19hdHRlc3RlcmAgd291bGQgcGFzcyB0aGlzIGNoZWNrLgAAAAAACmluaXRpYWxpemUAAAAAAAIAAAAAAAAABWFkbWluAAAAAAAAEwAAAAAAAAARYXR0ZXN0ZXJfcmVnaXN0cnkAAAAAAAATAAAAAQAAA+kAAAACAAAAAw==",
        "AAAAAAAAAFNBY2NlcHQgdGhlIHByb3Bvc2VkIGFkbWluIHRyYW5zZmVyLiBUaGUgY2FsbGVyIG11c3QgYXV0aG9yaXplIGFzIHRoZSBwZW5kaW5nIGFkbWluLgAAAAAMYWNjZXB0X2FkbWluAAAAAAAAAAEAAAPpAAAAAgAAAAM=",
        "AAAAAAAAAExQcm9wb3NlIGEgbmV3IGFkbWluIGFkZHJlc3MuIFRoZSBjYWxsZXIgbXVzdCBhdXRob3JpemUgYXMgdGhlIGN1cnJlbnQgYWRtaW4uAAAADXByb3Bvc2VfYWRtaW4AAAAAAAABAAAAAAAAAAluZXdfYWRtaW4AAAAAAAATAAAAAQAAA+kAAAACAAAAAw==",
        "AAAAAAAAAK9Mb29rIHVwIHRoZSBsYXRlc3QgYXR0ZXN0YXRpb24gZm9yIGByZWNvcmRfaGFzaGAsIGlmIGFueS4gQ2FsbGFibGUKYnkgYW55b25lIOKAlCB0aGlzIGlzIHdoYXQgbGV0cyBhIHJlc3BvbmRlcidzIFFSIHNjYW4gaW5kZXBlbmRlbnRseQpjaGVjayBhIGNhcmQgd2l0aG91dCBhbiBleHRlcm5hbCBvcmFjbGUuAAAAAA9nZXRfYXR0ZXN0YXRpb24AAAAAAQAAAAAAAAALcmVjb3JkX2hhc2gAAAAD7gAAACAAAAABAAAD6AAAB9AAAAALQXR0ZXN0YXRpb24A",
        "AAAAAAAAAEhSZXZva2UgYWxsIGF0dGVzdGF0aW9ucyBmb3IgYHJlY29yZF9oYXNoYC4gR2F0ZWQgYnkgYWRtaW4gYXV0aG9yaXphdGlvbi4AAAAScmV2b2tlX2F0dGVzdGF0aW9uAAAAAAABAAAAAAAAAAtyZWNvcmRfaGFzaAAAAAPuAAAAIAAAAAEAAAPpAAAAAgAAAAM=",
        "AAAAAAAAADlSZXR1cm4gdGhlIGNvbmZpZ3VyZWQgYXR0ZXN0ZXItcmVnaXN0cnkgY29udHJhY3QgYWRkcmVzcy4AAAAAAAAVZ2V0X2F0dGVzdGVyX3JlZ2lzdHJ5AAAAAAAAAAAAAAEAAAPpAAAAEwAAAAM=",
        "AAAAAAAAALZDaGFuZ2UgdGhlIGF0dGVzdGVyLXJlZ2lzdHJ5IGNvbnRyYWN0IHRoaXMgcmVnaXN0cnkgY29uc3VsdHMgZm9yCmFsbG93bGlzdCBjaGVja3MuIFJlcXVpcmVzIHRoZSBhZG1pbidzIGF1dGhvcml6YXRpb24uIEVtaXRzCmBBdHRlc3RlclJlZ2lzdHJ5UmVwb2ludGVkYCBmb3IgaW5kZXhlci9hdWRpdCB2aXNpYmlsaXR5LgAAAAAAFXNldF9hdHRlc3Rlcl9yZWdpc3RyeQAAAAAAAAEAAAAAAAAADG5ld19yZWdpc3RyeQAAABMAAAABAAAD6QAAAAIAAAAD",
        "AAAAAAAAAI9Mb29rIHVwIHRoZSBmdWxsIGF0dGVzdGF0aW9uIGhpc3RvcnkgZm9yIGByZWNvcmRfaGFzaGAsIGlmIGFueS4KUmV0dXJucyBhdHRlc3RhdGlvbnMgaW4gY2hyb25vbG9naWNhbCBvcmRlciAob2xkZXN0IGZpcnN0KS4KQ2FsbGFibGUgYnkgYW55b25lLgAAAAAXZ2V0X2F0dGVzdGF0aW9uX2hpc3RvcnkAAAAAAQAAAAAAAAALcmVjb3JkX2hhc2gAAAAD7gAAACAAAAABAAAD6gAAB9AAAAALQXR0ZXN0YXRpb24A" ]),
      options
    )
  }
  public readonly fromJSON = {
    pause: this.txFromJSON<Result<void>>,
        attest: this.txFromJSON<Result<Attestation>>,
        unpause: this.txFromJSON<Result<void>>,
        get_admin: this.txFromJSON<Result<string>>,
        is_paused: this.txFromJSON<boolean>,
        initialize: this.txFromJSON<Result<void>>,
        accept_admin: this.txFromJSON<Result<void>>,
        propose_admin: this.txFromJSON<Result<void>>,
        get_attestation: this.txFromJSON<Option<Attestation>>,
        revoke_attestation: this.txFromJSON<Result<void>>,
        get_attester_registry: this.txFromJSON<Result<string>>,
        set_attester_registry: this.txFromJSON<Result<void>>,
        get_attestation_history: this.txFromJSON<Array<Attestation>>
  }
}