import { Buffer } from "buffer";
import { AssembledTransaction, Client as ContractClient, ClientOptions as ContractClientOptions, MethodOptions } from "@stellar/stellar-sdk/contract";
import type { u32 } from "@stellar/stellar-sdk/contract";
export * from "@stellar/stellar-sdk";
export * as contract from "@stellar/stellar-sdk/contract";
export * as rpc from "@stellar/stellar-sdk/rpc";
/**
 * Errors returned by the multisig-account contract's public entry points.
 */
export declare const Errors: {
    /**
     * The configured threshold is zero or exceeds the signer count.
     */
    1: {
        message: string;
    };
    /**
     * The signer configuration contains duplicate public keys.
     */
    2: {
        message: string;
    };
    /**
     * The supplied signature count is below the configured threshold.
     */
    3: {
        message: string;
    };
    /**
     * Signatures are not strictly ordered by ascending public key.
     */
    4: {
        message: string;
    };
    /**
     * A signature corresponds to a public key that is not a configured signer.
     */
    5: {
        message: string;
    };
    /**
     * The contract has not been initialized; threshold or signer count is unavailable.
     */
    6: {
        message: string;
    };
    /**
     * The supplied signature count exceeds the configured signer count.
     */
    7: {
        message: string;
    };
};
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
export declare class Client extends ContractClient {
    readonly options: ContractClientOptions;
    static deploy<T = Client>(
    /** Constructor/Initialization Args for the contract's `__constructor` method */
    { signers, threshold }: {
        signers: Array<Buffer>;
        threshold: u32;
    }, 
    /** Options for initializing a Client as well as for calling a method, with extras specific to deploying. */
    options: MethodOptions & Omit<ContractClientOptions, "contractId"> & {
        /** The hash of the Wasm blob, which must already be installed on-chain. */
        wasmHash: Buffer | string;
        /** Salt used to generate the contract's ID. Passed through to {@link Operation.createCustomContract}. Default: random. */
        salt?: Buffer | Uint8Array;
        /** The format used to decode `wasmHash`, if it's provided as a string. */
        format?: "hex" | "base64";
    }): Promise<AssembledTransaction<T>>;
    constructor(options: ContractClientOptions);
    readonly fromJSON: {};
}
