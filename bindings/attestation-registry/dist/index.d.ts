import { Buffer } from "buffer";
import { AssembledTransaction, Client as ContractClient, ClientOptions as ContractClientOptions, MethodOptions, Result } from "@stellar/stellar-sdk/contract";
import type { u64, Option } from "@stellar/stellar-sdk/contract";
export * from "@stellar/stellar-sdk";
export * as contract from "@stellar/stellar-sdk/contract";
export * as rpc from "@stellar/stellar-sdk/rpc";
export declare const Errors: {
    1: {
        message: string;
    };
    2: {
        message: string;
    };
    3: {
        message: string;
    };
};
/**
 * A single attestation: proof that `attester` verified the off-chain
 * record whose hash is the lookup key, at `timestamp`. Never contains the
 * underlying health data.
 */
export interface Attestation {
    attester: string;
    timestamp: u64;
}
export interface Client {
    /**
     * Construct and simulate a attest transaction. Returns an `AssembledTransaction` object which will have a `result` field containing the result of the simulation. If this transaction changes contract state, you will need to call `signAndSend()` on the returned object.
     * Record that `attester` verified the record hashing to `record_hash`.
     * Requires `attester`'s authorization and that `attester` is
     * currently allowlisted in the configured `attester-registry`.
     * Overwrites any prior attestation for the same `record_hash`.
     */
    attest: ({ attester, record_hash }: {
        attester: string;
        record_hash: Buffer;
    }, options?: MethodOptions) => Promise<AssembledTransaction<Result<Attestation>>>;
    /**
     * Construct and simulate a initialize transaction. Returns an `AssembledTransaction` object which will have a `result` field containing the result of the simulation. If this transaction changes contract state, you will need to call `signAndSend()` on the returned object.
     * Set the admin and the `attester-registry` contract this registry
     * consults for allowlist checks. Can only be called once; the caller
     * must authorize as the given `admin`.
     */
    initialize: ({ admin, attester_registry }: {
        admin: string;
        attester_registry: string;
    }, options?: MethodOptions) => Promise<AssembledTransaction<Result<void>>>;
    /**
     * Construct and simulate a get_attestation transaction. Returns an `AssembledTransaction` object which will have a `result` field containing the result of the simulation. If this transaction changes contract state, you will need to call `signAndSend()` on the returned object.
     * Look up the latest attestation for `record_hash`, if any. Callable
     * by anyone — this is what lets a responder's QR scan independently
     * check a card without an external oracle.
     */
    get_attestation: ({ record_hash }: {
        record_hash: Buffer;
    }, options?: MethodOptions) => Promise<AssembledTransaction<Option<Attestation>>>;
}
export declare class Client extends ContractClient {
    readonly options: ContractClientOptions;
    static deploy<T = Client>(
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
    readonly fromJSON: {
        attest: (json: string) => AssembledTransaction<Result<Attestation, import("@stellar/stellar-sdk/contract").ErrorMessage>>;
        initialize: (json: string) => AssembledTransaction<Result<void, import("@stellar/stellar-sdk/contract").ErrorMessage>>;
        get_attestation: (json: string) => AssembledTransaction<Option<Attestation>>;
    };
}
