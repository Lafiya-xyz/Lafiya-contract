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
 * Errors returned by the multisig-account contract's public entry points.
 */
export const Errors = {
  /**
   * The configured threshold is zero or exceeds the signer count.
   */
  1: {message:"InvalidThreshold"},
  /**
   * The signer configuration contains duplicate public keys.
   */
  2: {message:"DuplicateSigner"},
  /**
   * The supplied signature count is below the configured threshold.
   */
  3: {message:"NotEnoughSigners"},
  /**
   * Signatures are not strictly ordered by ascending public key.
   */
  4: {message:"BadSignatureOrder"},
  /**
   * A signature corresponds to a public key that is not a configured signer.
   */
  5: {message:"UnknownSigner"},
  /**
   * The contract has not been initialized; threshold or signer count is unavailable.
   */
  6: {message:"NotInitialized"},
  /**
   * The supplied signature count exceeds the configured signer count.
   */
  7: {message:"TooManySigners"}
}


/**
 * A single ed25519 signature from one signer in the multisig set.
 */
export interface Signature {
  /**
 * The public key of the signer who created this signature.
 */
public_key: Buffer;
  /**
 * The ed25519 signature bytes.
 */
signature: Buffer;
}

export interface Client {
}
export class Client extends ContractClient {
  static async deploy<T = Client>(
        /** Constructor/Initialization Args for the contract's `__constructor` method */
        {signers, threshold}: {signers: Array<Buffer>, threshold: u32},
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
    return ContractClient.deploy({signers, threshold}, options)
  }
  constructor(public readonly options: ContractClientOptions) {
    super(
      new ContractSpec([ "AAAABAAAAEdFcnJvcnMgcmV0dXJuZWQgYnkgdGhlIG11bHRpc2lnLWFjY291bnQgY29udHJhY3QncyBwdWJsaWMgZW50cnkgcG9pbnRzLgAAAAAAAAAABUVycm9yAAAAAAAABwAAAD1UaGUgY29uZmlndXJlZCB0aHJlc2hvbGQgaXMgemVybyBvciBleGNlZWRzIHRoZSBzaWduZXIgY291bnQuAAAAAAAAEEludmFsaWRUaHJlc2hvbGQAAAABAAAAOFRoZSBzaWduZXIgY29uZmlndXJhdGlvbiBjb250YWlucyBkdXBsaWNhdGUgcHVibGljIGtleXMuAAAAD0R1cGxpY2F0ZVNpZ25lcgAAAAACAAAAP1RoZSBzdXBwbGllZCBzaWduYXR1cmUgY291bnQgaXMgYmVsb3cgdGhlIGNvbmZpZ3VyZWQgdGhyZXNob2xkLgAAAAAQTm90RW5vdWdoU2lnbmVycwAAAAMAAAA8U2lnbmF0dXJlcyBhcmUgbm90IHN0cmljdGx5IG9yZGVyZWQgYnkgYXNjZW5kaW5nIHB1YmxpYyBrZXkuAAAAEUJhZFNpZ25hdHVyZU9yZGVyAAAAAAAABAAAAEhBIHNpZ25hdHVyZSBjb3JyZXNwb25kcyB0byBhIHB1YmxpYyBrZXkgdGhhdCBpcyBub3QgYSBjb25maWd1cmVkIHNpZ25lci4AAAANVW5rbm93blNpZ25lcgAAAAAAAAUAAABQVGhlIGNvbnRyYWN0IGhhcyBub3QgYmVlbiBpbml0aWFsaXplZDsgdGhyZXNob2xkIG9yIHNpZ25lciBjb3VudCBpcyB1bmF2YWlsYWJsZS4AAAAOTm90SW5pdGlhbGl6ZWQAAAAAAAYAAABBVGhlIHN1cHBsaWVkIHNpZ25hdHVyZSBjb3VudCBleGNlZWRzIHRoZSBjb25maWd1cmVkIHNpZ25lciBjb3VudC4AAAAAAAAOVG9vTWFueVNpZ25lcnMAAAAAAAc=",
        "AAAAAQAAAD9BIHNpbmdsZSBlZDI1NTE5IHNpZ25hdHVyZSBmcm9tIG9uZSBzaWduZXIgaW4gdGhlIG11bHRpc2lnIHNldC4AAAAAAAAAAAlTaWduYXR1cmUAAAAAAAACAAAAOFRoZSBwdWJsaWMga2V5IG9mIHRoZSBzaWduZXIgd2hvIGNyZWF0ZWQgdGhpcyBzaWduYXR1cmUuAAAACnB1YmxpY19rZXkAAAAAA+4AAAAgAAAAHFRoZSBlZDI1NTE5IHNpZ25hdHVyZSBieXRlcy4AAAAJc2lnbmF0dXJlAAAAAAAD7gAAAEA=",
        "AAAAAAAAAqBWZXJpZnkgdGhlIGF1dGhvcml6YXRpb24gb2YgYSB0cmFuc2FjdGlvbiBieSBjaGVja2luZyBOLW9mLU0gZWQyNTUxOSBzaWduYXR1cmVzLgoKVmVyaWZpZXMgdGhhdCB0aGUgc3VwcGxpZWQgc2lnbmF0dXJlcyBtZWV0IHRoZSBjb25maWd1cmVkIHRocmVzaG9sZCBhbmQgZWFjaCBiZWxvbmdzIHRvCmFuIGF1dGhvcml6ZWQgc2lnbmVyLCB3aXRoIHNpZ25hdHVyZXMgb3JkZXJlZCBpbiBhc2NlbmRpbmcgcHVibGljLWtleSBvcmRlci4KCiMgQXJndW1lbnRzCiogYHNpZ25hdHVyZV9wYXlsb2FkYCDigJQgQSAzMi1ieXRlIGhhc2ggb2YgdGhlIHRyYW5zYWN0aW9uIHRvIGF1dGhvcml6ZS4KKiBgc2lnbmF0dXJlc2Ag4oCUIEEgdmVjdG9yIG9mIGVkMjU1MTkgc2lnbmF0dXJlcywgZWFjaCB3aXRoIGEgcHVibGljIGtleSBhbmQgc2lnbmF0dXJlIGJ5dGVzLCBvcmRlcmVkIGJ5IGFzY2VuZGluZyBwdWJsaWMga2V5LgoqIGBfYXV0aF9jb250ZXh0c2Ag4oCUIEludGVudGlvbmFsbHkgdW51c2VkOyBzZWUgW0FEUi0wMDA3XSguLi9hZHIvMDAwNy11bnNjb3BlZC1tdWx0aXNpZy1hdXRob3JpemF0aW9uLm1kKSBmb3Igd2h5IHRoaXMgYWNjb3VudCBkb2VzIG5vdCBzY29wZSBhdXRob3JpemF0aW9uIHRvIHNwZWNpZmljIGNvbnRyYWN0cyBvciBmdW5jdGlvbnMgZHVyaW5nIHByZS1hbHBoYS4AAAAMX19jaGVja19hdXRoAAAAAwAAAAAAAAARc2lnbmF0dXJlX3BheWxvYWQAAAAAAAPuAAAAIAAAAAAAAAAKc2lnbmF0dXJlcwAAAAAD6gAAB9AAAAAJU2lnbmF0dXJlAAAAAAAAAAAAAA1hdXRoX2NvbnRleHRzAAAAAAAD6gAAB9AAAAAHQ29udGV4dAAAAAABAAAD6QAAAAIAAAAD",
        "AAAAAAAAAUdJbml0aWFsaXplIHRoZSBtdWx0aXNpZyBhY2NvdW50IHdpdGggYSBzZXQgb2YgYXV0aG9yaXplZCBzaWduZXJzIGFuZCBhIHNpZ25hdHVyZSB0aHJlc2hvbGQuCgojIEFyZ3VtZW50cwoqIGBzaWduZXJzYCDigJQgQSB2ZWN0b3Igb2YgZWQyNTUxOSBwdWJsaWMga2V5cyAoMzIgYnl0ZXMgZWFjaCkgYXV0aG9yaXplZCB0byBzaWduIHRyYW5zYWN0aW9ucy4KKiBgdGhyZXNob2xkYCDigJQgVGhlIG1pbmltdW0gbnVtYmVyIG9mIHNpZ25hdHVyZXMgcmVxdWlyZWQgdG8gYXV0aG9yaXplIGEgdHJhbnNhY3Rpb247IG11c3QgYmUgPiAwIGFuZCDiiaQgdGhlIHNpZ25lciBjb3VudC4AAAAADV9fY29uc3RydWN0b3IAAAAAAAACAAAAAAAAAAdzaWduZXJzAAAAA+oAAAPuAAAAIAAAAAAAAAAJdGhyZXNob2xkAAAAAAAABAAAAAA=" ]),
      options
    )
  }
  public readonly fromJSON = {
    
  }
}

/**
 * Round-trip test helpers for #727.
 *
 * These helpers decode a Rust-generated ScVal fixture (XDR base64) using the
 * contract spec, re-encode it, and assert byte-exact equality. They are used by
 * the TS round-trip tests to fuzz the encoding of every `#[contracttype]`
 * across the Rust/TypeScript language boundary in both directions.
 *
 * Fixtures are produced by the Rust generator (see
 * `bindings/multisig-account/test/fixtures/`); only synthetic data is used.
 */
export interface RoundTripFixture {
  /** The name of the contracttype this fixture exercises. */
  type: string;
  /** XDR base64 of the ScVal produced by the Rust bindings. */
  xdr: string;
}

/**
 * Decode an XDR base64 ScVal, re-encode it, and return the re-encoded base64.
 *
 * Throws if the decoded value cannot be re-encoded, which surfaces any
 * mismatch between the Rust and TypeScript bindings.
 */
export function roundTripScVal(xdr: string): string {
  const spec = new ContractSpec([
    "AAAABAAAAEdFcnJvcnMgcmV0dXJuZWQgYnkgdGhlIG11bHRpc2lnLWFjY291bnQgY29udHJhY3QncyBwdWJsaWMgZW50cnkgcG9pbnRzLgAAAAAAAAAABUVycm9yAAAAAAAABwAAAD1UaGUgY29uZmlndXJlZCB0aHJlc2hvbGQgaXMgemVybyBvciBleGNlZWRzIHRoZSBzaWduZXIgY291bnQuAAAAAAAAEEludmFsaWRUaHJlc2hvbGQAAAABAAAAOFRoZSBzaWduZXIgY29uZmlndXJhdGlvbiBjb250YWlucyBkdXBsaWNhdGUgcHVibGljIGtleXMuAAAAD0R1cGxpY2F0ZVNpZ25lcgAAAAACAAAAP1RoZSBzdXBwbGllZCBzaWduYXR1cmUgY291bnQgaXMgYmVsb3cgdGhlIGNvbmZpZ3VyZWQgdGhyZXNob2xkLgAAAAAQTm90RW5vdWdoU2lnbmVycwAAAAMAAAA8U2lnbmF0dXJlcyBhcmUgbm90IHN0cmljdGx5IG9yZGVyZWQgYnkgYXNjZW5kaW5nIHB1YmxpYyBrZXkuAAAAEUJhZFNpZ25hdHVyZU9yZGVyAAAAAAAABAAAAEhBIHNpZ25hdHVyZSBjb3JyZXNwb25kcyB0byBhIHB1YmxpYyBrZXkgdGhhdCBpcyBub3QgYSBjb25maWd1cmVkIHNpZ25lci4AAAANVW5rbm93blNpZ25lcgAAAAAAAAUAAABQVGhlIGNvbnRyYWN0IGhhcyBub3QgYmVlbiBpbml0aWFsaXplZDsgdGhyZXNob2xkIG9yIHNpZ25lciBjb3VudCBpcyB1bmF2YWlsYWJsZS4AAAAOTm90SW5pdGlhbGl6ZWQAAAAAAAYAAABBVGhlIHN1cHBsaWVkIHNpZ25hdHVyZSBjb3VudCBleGNlZWRzIHRoZSBjb25maWd1cmVkIHNpZ25lciBjb3VudC4AAAAAAAAOVG9vTWFueVNpZ25lcnMAAAAAAAc=",
    "AAAAAQAAAD9BIHNpbmdsZSBlZDI1NTE5IHNpZ25hdHVyZSBmcm9tIG9uZSBzaWduZXIgaW4gdGhlIG11bHRpc2lnIHNldC4AAAAAAAAAAAlTaWduYXR1cmUAAAAAAAACAAAAOFRoZSBwdWJsaWMga2V5IG9mIHRoZSBzaWduZXIgd2hvIGNyZWF0ZWQgdGhpcyBzaWduYXR1cmUuAAAACnB1YmxpY19rZXkAAAAAA+4AAAAgAAAAHFRoZSBlZDI1NTE5IHNpZ25hdHVyZSBieXRlcy4AAAAJc2lnbmF0dXJlAAAAAAAD7gAAAEA=",
    "AAAAAAAAAqBWZXJpZnkgdGhlIGF1dGhvcml6YXRpb24gb2YgYSB0cmFuc2FjdGlvbiBieSBjaGVja2luZyBOLW9mLU0gZWQyNTUxOSBzaWduYXR1cmVzLgoKVmVyaWZpZXMgdGhhdCB0aGUgc3VwcGxpZWQgc2lnbmF0dXJlcyBtZWV0IHRoZSBjb25maWd1cmVkIHRocmVzaG9sZCBhbmQgZWFjaCBiZWxvbmdzIHRvCmFuIGF1dGhvcml6ZWQgc2lnbmVyLCB3aXRoIHNpZ25hdHVyZXMgb3JkZXJlZCBpbiBhc2NlbmRpbmcgcHVibGljLWtleSBvcmRlci4KCiMgQXJndW1lbnRzCiogYHNpZ25hdHVyZV9wYXlsb2FkYCDigJQgQSAzMi1ieXRlIGhhc2ggb2YgdGhlIHRyYW5zYWN0aW9uIHRvIGF1dGhvcml6ZS4KKiBgc2lnbmF0dXJlc2Ag4oCUIEEgdmVjdG9yIG9mIGVkMjU1MTkgc2lnbmF0dXJlcywgZWFjaCB3aXRoIGEgcHVibGljIGtleSBhbmQgc2lnbmF0dXJlIGJ5dGVzLCBvcmRlcmVkIGJ5IGFzY2VuZGluZyBwdWJsaWMga2V5LgoqIGBfYXV0aF9jb250ZXh0c2Ag4oCUIEludGVudGlvbmFsbHkgdW51c2VkOyBzZWUgW0FEUi0wMDA3XSguLi9hZHIvMDAwNy11bnNjb3BlZC1tdWx0aXNpZy1hdXRob3JpemF0aW9uLm1kKSBmb3Igd2h5IHRoaXMgYWNjb3VudCBkb2VzIG5vdCBzY29wZSBhdXRob3JpemF0aW9uIHRvIHNwZWNpZmljIGNvbnRyYWN0cyBvciBmdW5jdGlvbnMgZHVyaW5nIHByZS1hbHBoYS4AAAAMX19jaGVja19hdXRoAAAAAwAAAAAAAAARc2lnbmF0dXJlX3BheWxvYWQAAAAAAAPuAAAAIAAAAAAAAAAKc2lnbmF0dXJlcwAAAAAD6gAAB9AAAAAJU2lnbmF0dXJlAAAAAAAAAAAAAA1hdXRoX2NvbnRleHRzAAAAAAAD6gAAB9AAAAAHQ29udGV4dAAAAAABAAAD6QAAAAIAAAAD",
    "AAAAAAAAAUdJbml0aWFsaXplIHRoZSBtdWx0aXNpZyBhY2NvdW50IHdpdGggYSBzZXQgb2YgYXV0aG9yaXplZCBzaWduZXJzIGFuZCBhIHNpZ25hdHVyZSB0aHJlc2hvbGQuCgojIEFyZ3VtZW50cwoqIGBzaWduZXJzYCDigJQgQSB2ZWN0b3Igb2YgZWQyNTUxOSBwdWJsaWMga2V5cyAoMzIgYnl0ZXMgZWFjaCkgYXV0aG9yaXplZCB0byBzaWduIHRyYW5zYWN0aW9ucy4KKiBgdGhyZXNob2xkYCDigJQgVGhlIG1pbmltdW0gbnVtYmVyIG9mIHNpZ25hdHVyZXMgcmVxdWlyZWQgdG8gYXV0aG9yaXplIGEgdHJhbnNhY3Rpb247IG11c3QgYmUgPiAwIGFuZCDiiaQgdGhlIHNpZ25lciBjb3VudC4AAAAADV9fY29uc3RydWN0b3IAAAAAAAACAAAAAAAAAAdzaWduZXJzAAAAA+oAAAPuAAAAIAAAAAAAAAAJdGhyZXNob2xkAAAAAAAABAAAAAA=",
  ]);
  const scVal = spec.funcArgsToScVals("__check_auth", [
    Buffer.from(xdr, "base64"),
  ]);
  // Re-encode the decoded value and return its base64 representation.
  return Buffer.from(scVal[0].toXDR()).toString("base64");
}

/**
 * Assert that a Rust-generated fixture round-trips byte-exactly through the
 * TypeScript bindings.
 */
export function assertRoundTrip(fixture: RoundTripFixture): void {
  const reencoded = roundTripScVal(fixture.xdr);
  if (reencoded !== fixture.xdr) {
    throw new Error(
      `Round-trip mismatch for ${fixture.type}: expected ${fixture.xdr}, got ${reencoded}`
    );
  }
}