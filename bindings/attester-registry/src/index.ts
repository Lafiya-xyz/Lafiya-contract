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




export const Errors = {
  1: {message:"NotInitialized"},
  2: {message:"AlreadyInitialized"}
}


/**
 * Metadata associated with an allowlisted attester.
 */
export interface AttesterInfo {
  license_hash: Option<Buffer>;
  region: Option<string>;
}



export interface Client {
  /**
   * Construct and simulate a initialize transaction. Returns an `AssembledTransaction` object which will have a `result` field containing the result of the simulation. If this transaction changes contract state, you will need to call `signAndSend()` on the returned object.
   * Set the admin address authorized to manage the allowlist. Can only
   * be called once; the caller must authorize as the given `admin`.
   */
  initialize: ({admin}: {admin: string}, options?: MethodOptions) => Promise<AssembledTransaction<Result<void>>>

  /**
   * Construct and simulate a is_attester transaction. Returns an `AssembledTransaction` object which will have a `result` field containing the result of the simulation. If this transaction changes contract state, you will need to call `signAndSend()` on the returned object.
   * Whether `attester` is currently allowlisted. Callable by anyone,
   * including other contracts (e.g. `attestation-registry`).
   */
  is_attester: ({attester}: {attester: string}, options?: MethodOptions) => Promise<AssembledTransaction<boolean>>

  /**
   * Construct and simulate a add_attester transaction. Returns an `AssembledTransaction` object which will have a `result` field containing the result of the simulation. If this transaction changes contract state, you will need to call `signAndSend()` on the returned object.
   * Add `attester` to the allowlist. Requires the admin's authorization.
   */
  add_attester: ({attester}: {attester: string}, options?: MethodOptions) => Promise<AssembledTransaction<Result<void>>>

  /**
   * Construct and simulate a remove_attester transaction. Returns an `AssembledTransaction` object which will have a `result` field containing the result of the simulation. If this transaction changes contract state, you will need to call `signAndSend()` on the returned object.
   * Remove `attester` from the allowlist. Requires the admin's
   * authorization. A no-op if the attester was never allowlisted.
   */
  remove_attester: ({attester}: {attester: string}, options?: MethodOptions) => Promise<AssembledTransaction<Result<void>>>

  /**
   * Construct and simulate a get_attester_info transaction. Returns an `AssembledTransaction` object which will have a `result` field containing the result of the simulation. If this transaction changes contract state, you will need to call `signAndSend()` on the returned object.
   * Get the optional metadata associated with `attester` if they are allowlisted.
   */
  get_attester_info: ({attester}: {attester: string}, options?: MethodOptions) => Promise<AssembledTransaction<Option<AttesterInfo>>>

  /**
   * Construct and simulate a add_attester_with_info transaction. Returns an `AssembledTransaction` object which will have a `result` field containing the result of the simulation. If this transaction changes contract state, you will need to call `signAndSend()` on the returned object.
   * Add `attester` with optional metadata to the allowlist. Requires the admin's authorization.
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
      new ContractSpec([ "AAAABAAAAAAAAAAAAAAABUVycm9yAAAAAAAAAgAAAAAAAAAOTm90SW5pdGlhbGl6ZWQAAAAAAAEAAAAAAAAAEkFscmVhZHlJbml0aWFsaXplZAAAAAAAAg==",
        "AAAAAQAAADFNZXRhZGF0YSBhc3NvY2lhdGVkIHdpdGggYW4gYWxsb3dsaXN0ZWQgYXR0ZXN0ZXIuAAAAAAAAAAAAAAxBdHRlc3RlckluZm8AAAACAAAAAAAAAAxsaWNlbnNlX2hhc2gAAAPoAAAD7gAAACAAAAAAAAAABnJlZ2lvbgAAAAAD6AAAABE=",
        "AAAABQAAAAAAAAAAAAAADUF0dGVzdGVyQWRkZWQAAAAAAAABAAAADmF0dGVzdGVyX2FkZGVkAAAAAAABAAAAAAAAAAhhdHRlc3RlcgAAABMAAAABAAAAAg==",
        "AAAABQAAAAAAAAAAAAAAD0F0dGVzdGVyUmVtb3ZlZAAAAAABAAAAEGF0dGVzdGVyX3JlbW92ZWQAAAABAAAAAAAAAAhhdHRlc3RlcgAAABMAAAABAAAAAg==",
        "AAAAAAAAAIJTZXQgdGhlIGFkbWluIGFkZHJlc3MgYXV0aG9yaXplZCB0byBtYW5hZ2UgdGhlIGFsbG93bGlzdC4gQ2FuIG9ubHkKYmUgY2FsbGVkIG9uY2U7IHRoZSBjYWxsZXIgbXVzdCBhdXRob3JpemUgYXMgdGhlIGdpdmVuIGBhZG1pbmAuAAAAAAAKaW5pdGlhbGl6ZQAAAAAAAQAAAAAAAAAFYWRtaW4AAAAAAAATAAAAAQAAA+kAAAACAAAAAw==",
        "AAAAAAAAAHlXaGV0aGVyIGBhdHRlc3RlcmAgaXMgY3VycmVudGx5IGFsbG93bGlzdGVkLiBDYWxsYWJsZSBieSBhbnlvbmUsCmluY2x1ZGluZyBvdGhlciBjb250cmFjdHMgKGUuZy4gYGF0dGVzdGF0aW9uLXJlZ2lzdHJ5YCkuAAAAAAAAC2lzX2F0dGVzdGVyAAAAAAEAAAAAAAAACGF0dGVzdGVyAAAAEwAAAAEAAAAB",
        "AAAAAAAAAERBZGQgYGF0dGVzdGVyYCB0byB0aGUgYWxsb3dsaXN0LiBSZXF1aXJlcyB0aGUgYWRtaW4ncyBhdXRob3JpemF0aW9uLgAAAAxhZGRfYXR0ZXN0ZXIAAAABAAAAAAAAAAhhdHRlc3RlcgAAABMAAAABAAAD6QAAAAIAAAAD",
        "AAAAAAAAAHhSZW1vdmUgYGF0dGVzdGVyYCBmcm9tIHRoZSBhbGxvd2xpc3QuIFJlcXVpcmVzIHRoZSBhZG1pbidzCmF1dGhvcml6YXRpb24uIEEgbm8tb3AgaWYgdGhlIGF0dGVzdGVyIHdhcyBuZXZlciBhbGxvd2xpc3RlZC4AAAAPcmVtb3ZlX2F0dGVzdGVyAAAAAAEAAAAAAAAACGF0dGVzdGVyAAAAEwAAAAEAAAPpAAAAAgAAAAM=",
        "AAAAAAAAAE1HZXQgdGhlIG9wdGlvbmFsIG1ldGFkYXRhIGFzc29jaWF0ZWQgd2l0aCBgYXR0ZXN0ZXJgIGlmIHRoZXkgYXJlIGFsbG93bGlzdGVkLgAAAAAAABFnZXRfYXR0ZXN0ZXJfaW5mbwAAAAAAAAEAAAAAAAAACGF0dGVzdGVyAAAAEwAAAAEAAAPoAAAH0AAAAAxBdHRlc3RlckluZm8=",
        "AAAAAAAAAFtBZGQgYGF0dGVzdGVyYCB3aXRoIG9wdGlvbmFsIG1ldGFkYXRhIHRvIHRoZSBhbGxvd2xpc3QuIFJlcXVpcmVzIHRoZSBhZG1pbidzIGF1dGhvcml6YXRpb24uAAAAABZhZGRfYXR0ZXN0ZXJfd2l0aF9pbmZvAAAAAAADAAAAAAAAAAhhdHRlc3RlcgAAABMAAAAAAAAADGxpY2Vuc2VfaGFzaAAAA+gAAAPuAAAAIAAAAAAAAAAGcmVnaW9uAAAAAAPoAAAAEQAAAAEAAAPpAAAAAgAAAAM=" ]),
      options
    )
  }
  public readonly fromJSON = {
    initialize: this.txFromJSON<Result<void>>,
        is_attester: this.txFromJSON<boolean>,
        add_attester: this.txFromJSON<Result<void>>,
        remove_attester: this.txFromJSON<Result<void>>,
        get_attester_info: this.txFromJSON<Option<AttesterInfo>>,
        add_attester_with_info: this.txFromJSON<Result<void>>
  }
}