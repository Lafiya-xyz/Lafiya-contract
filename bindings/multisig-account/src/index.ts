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
  7: {message:"TooManySigners"},
  /**
   * A context policy references a signer key that is not configured.
   */
  8: {message:"UnknownContextSigner"},
  /**
   * The requested signer replacement would leave the policy unsatisfiable.
   */
  9: {message:"UnsatisfiablePolicy"},
  /**
   * The replacement signer key is already present in the configuration.
   */
  10: {message:"SignerAlreadyPresent"},
  /**
   * The signer to be replaced was not found in the configuration.
   */
  11: {message:"SignerNotFound"}
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

/**
 * A signer policy: the minimum number of signatures required for a context.
 */
export interface SignerPolicy {
  /**
 * The minimum number of signatures required.
 */
threshold: u32;
}

/**
 * A context key identifying a (contract address, function name) pair.
 */
export interface ContextKey {
  /**
 * The address of the contract being invoked.
 */
contract: string;
  /**
 * The name of the function being invoked.
 */
fn_name: string;
}

/**
 * The configuration for a single signer, including weight and roles.
 */
export interface SignerConfig {
  /**
 * The public key of the signer.
 */
key: Buffer;
  /**
 * The weight of the signer.
 */
weight: u32;
  /**
 * The roles assigned to the signer.
 */
roles: Array<string>;
}

/**
 * Event emitted when a signer is atomically replaced.
 */
export interface SignerReplaced {
  /**
 * A fingerprint of the replaced signer key.
 */
old_fingerprint: Buffer;
  /**
 * A fingerprint of the new signer key.
 */
new_fingerprint: Buffer;
  /**
 * The configuration version after the replacement.
 */
config_version: u32;
}

export interface Client {
  /**
   * Set the policy override for a (contract, function) context. Requires the strictest policy.
   */
  set_context_policy: (args: { context: ContextKey; policy: SignerPolicy }, options?: MethodOptions) => Promise<AssembledTransaction<null>>;
  /**
   * Remove a context policy override. Requires the strictest policy.
   */
  remove_context_policy: (args: { context: ContextKey }, options?: MethodOptions) => Promise<AssembledTransaction<null>>;
  /**
   * Set the default policy applied to unknown contexts. Requires the strictest policy.
   */
  set_default_policy: (args: { policy: SignerPolicy }, options?: MethodOptions) => Promise<AssembledTransaction<null>>;
  /**
   * Atomically replace a signer, preserving weight and roles unless overridden.
   */
  replace_signer: (args: { old: Buffer; new: SignerConfig }, options?: MethodOptions) => Promise<AssembledTransaction<null>>;
  /**
   * Return the policy configured for a context, if any.
   */
  get_context_policy: (args: { context: ContextKey }, options?: MethodOptions) => Promise<AssembledTransaction<Option<SignerPolicy>>>;
  /**
   * Return the default policy applied to unknown contexts.
   */
  get_default_policy: (options?: MethodOptions) => Promise<AssembledTransaction<SignerPolicy>>;
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
        "AAAAAAAAAUdJbml0aWFsaXplIHRoZSBtdWx0aXNpZyBhY2NvdW50IHdpdGggYSBzZXQgb2YgYXV0aG9yaXplZCBzaWduZXJzIGFuZCBhIHNpZ25hdHVyZSB0aHJlc2hvbGQuCgojIEFyZ3VtZW50cwoqIGBzaWduZXJzYCDigJQgQSB2ZWN0b3Igb2YgZWQyNTUxOSBwdWJsaWMga2V5cyAoMzIgYnl0ZXMgZWFjaCkgYXV0aG9yaXplZCB0byBzaWduIHRyYW5zYWN0aW9ucy4KKiBgdGhyZXNob2xkYCDigJQgVGhlIG1pbmltdW0gbnVtYmVyIG9mIHNpZ25hdHVyZXMgcmVxdWlyZWQgdG8gYXV0aG9yaXplIGEgdHJhbnNhY3Rpb247IG11c3QgYmUgPiAwIGFuZCDiiaQgdGhlIHNpZ25lciBjb3VudC4AAAAADV9fY29uc3RydWN0b3IAAAAAAAACAAAAAAAAAAdzaWduZXJzAAAAA+oAAAPuAAAAIAAAAAAAAAAJdGhyZXNob2xkAAAAAAAABAAAAAA=",
        "AAAAAQAAAEhBIHNpZ25lciBwb2xpY3k6IHRoZSBtaW5pbXVtIG51bWJlciBvZiBzaWduYXR1cmVzIHJlcXVpcmVkIGZvciBhIGNvbnRleHQuAAAAAAxTaWduZXJQb2xpY3kAAAABAAAAHFRoZSBtaW5pbXVtIG51bWJlciBvZiBzaWduYXR1cmVzIHJlcXVpcmVkLgAAAAl0aHJlc2hvbGQAAAAAAAAEAAAAAA==",
        "AAAAAQAAAEZBIHNpZ25lciBjb25maWd1cmF0aW9uIGluY2x1ZGluZyB3ZWlnaHQgYW5kIHJvbGVzLgAAAAAADFNpZ25lckNvbmZpZwAAAAMAAAAcVGhlIHB1YmxpYyBrZXkgb2YgdGhlIHNpZ25lci4AAAADa2V5AAAAA+4AAAAgAAAAHFRoZSB3ZWlnaHQgb2YgdGhlIHNpZ25lci4AAAAWd2VpZ2h0AAAAAAAEAAAAHFRoZSByb2xlcyBhc3NpZ25lZCB0byB0aGUgc2lnbmVyLgAAAAVyb2xlcwAAAAAAA+oAAAARAAAAAA==",
        "AAAAAQAAADlBIChjb250cmFjdCBhZGRyZXNzLCBmdW5jdGlvbiBuYW1lKSBjb250ZXh0IGtleS4AAAAAAAAKQ29udGV4dEtleQAAAAAAAgAAABxUaGUgYWRkcmVzcyBvZiB0aGUgY29udHJhY3QgYmVpbmcgaW52b2tlZC4AAAAPY29udHJhY3QAAAAAEwAAABxUaGUgbmFtZSBvZiB0aGUgZnVuY3Rpb24gYmVpbmcgaW52b2tlZC4AAAAHZm5fbmFtZQAAAAARAAAAAA==",
        "AAAAAQAAADxFdmVudCBlbWl0dGVkIHdoZW4gYSBzaWduZXIgaXMgYXRvbWljYWxseSByZXBsYWNlZC4AAAAOU2lnbmVyUmVwbGFjZWQAAAAAAAMAAAAhQSBmaW5nZXJwcmludCBvZiB0aGUgcmVwbGFjZWQgc2lnbmVyIGtleS4AAAAAAA9vbGRfZmluZ2VycHJpbnQAAAAADu4AAAAgAAAAH0EgZmluZ2VycHJpbnQgb2YgdGhlIG5ldyBzaWduZXIga2V5LgAAAAAPbmV3X2ZpbmdlcnByaW50AAAAAA7uAAAAIAAAAB5UaGUgY29uZmlndXJhdGlvbiB2ZXJzaW9uIGFmdGVyIHRoZSByZXBsYWNlbWVudC4AAAAADmNvbmZpZ192ZXJzaW9uAAAAAAAEAAAAAA==",
        "AAAAAAAAAFZTZXQgdGhlIHBvbGljeSBvdmVycmlkZSBmb3IgYSAoY29udHJhY3QsIGZ1bmN0aW9uKSBjb250ZXh0LiBSZXF1aXJlcyB0aGUgc3RyaWN0ZXN0IHBvbGljeS4AAAAAEnNldF9jb250ZXh0X3BvbGljeQAAAAAAAgAAAAAAAAAHY29udGV4dAAAAAfQAAAACkNvbnRleHRLZXkAAAAAAAAABnBvbGljeQAAAAAH0AAAAAxTaWduZXJQb2xpY3kAAAAAAA==",
        "AAAAAAAAAD1SZW1vdmUgYSBjb250ZXh0IHBvbGljeSBvdmVycmlkZS4gUmVxdWlyZXMgdGhlIHN0cmljdGVzdCBwb2xpY3kuAAAAAAAADnJlbW92ZV9jb250ZXh0X3BvbGljeQAAAAAAAQAAAAAAAAAHY29udGV4dAAAAAfQAAAACkNvbnRleHRLZXkAAAAA",
        "AAAAAAAAAEBTZXQgdGhlIGRlZmF1bHQgcG9saWN5IGFwcGxpZWQgdG8gdW5rbm93biBjb250ZXh0cy4gUmVxdWlyZXMgdGhlIHN0cmljdGVzdCBwb2xpY3kuAAAAEnNldF9kZWZhdWx0X3BvbGljeQAAAAAAAQAAAAAAAAAGcG9saWN5AAAAAAfQAAAADFNpZ25lclBvbGljeQAA",
        "AAAAAAAAAFZBdG9taWNhbGx5IHJlcGxhY2UgYSBzaWduZXIsIHByZXNlcnZpbmcgd2VpZ2h0IGFuZCByb2xlcyB1bmxlc3Mgb3ZlcnJpZGRlbi4AAAAAEnJlcGxhY2Vfc2lnbmVyAAAAAAACAAAAAAAAAANvbGQAAAAA7gAAACAAAAAAAAAABG5ldwAAAAfQAAAADFNpZ25lckNvbmZpZwAA",
        "AAAAAAAAADZSZXR1cm4gdGhlIHBvbGljeSBjb25maWd1cmVkIGZvciBhIGNvbnRleHQsIGlmIGFueS4AAAAAEmdldF9jb250ZXh0X3BvbGljeQAAAAAAAQAAAAAAAAAHY29udGV4dAAAAAfQAAAACkNvbnRleHRLZXkAAAAAAQAAA+kAAAfQAAAADFNpZ25lclBvbGljeQAA",
        "AAAAAAAAADJSZXR1cm4gdGhlIGRlZmF1bHQgcG9saWN5IGFwcGxpZWQgdG8gdW5rbm93biBjb250ZXh0cy4AAAAAEmdldF9kZWZhdWx0X3BvbGljeQAAAAAAAAABAAAH0AAAAAxTaWduZXJQb2xpY3kA" ]),
      options
    )
  }
  public readonly fromJSON = {
    set_context_policy: this.txFromJSON<null>,
    remove_context_policy: this.txFromJSON<null>,
    set_default_policy: this.txFromJSON<null>,
    replace_signer: this.txFromJSON<null>,
    get_context_policy: this.txFromJSON<Option<SignerPolicy>>,
    get_default_policy: this.txFromJSON<SignerPolicy>
  }
}