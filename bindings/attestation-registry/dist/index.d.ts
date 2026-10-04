import { Buffer } from "buffer";
import { AssembledTransaction, Client as ContractClient, ClientOptions as ContractClientOptions, MethodOptions, Result } from "@stellar/stellar-sdk/contract";
import type { u64, Option } from "@stellar/stellar-sdk/contract";
export * from "@stellar/stellar-sdk";
export * as contract from "@stellar/stellar-sdk/contract";
export * as rpc from "@stellar/stellar-sdk/rpc";
export * from "./record.js";
export declare const Errors: {
    /**
     * `initialize` has not been called yet.
     */
    1: {
        message: string;
    };
    /**
     * `initialize` was called more than once.
     */
    2: {
        message: string;
    };
    /**
     * The caller is not allowlisted by the `attester-registry` contract.
     */
    3: {
        message: string;
    };
    /**
     * `accept_admin` was called with no pending admin transfer. Admin transfer is a
     * two-step flow: the current admin must first call `propose_admin` to nominate a
     * successor, then the nominated address must call `accept_admin` to complete the
     * transfer. This error is returned when `accept_admin` is called before a
     * corresponding `propose_admin` call has set a pending admin.
     */
    4: {
        message: string;
    };
    /**
     * The configured `attester-registry` address does not implement the expected interface. Re-run `set_attester_registry` with the correct address, or check your network configuration.
     */
    5: {
        message: string;
    };
    /**
     * No attestation exists for the given record hash / sequence.
     */
    6: {
        message: string;
    };
    /**
     * The requested operation is blocked while the contract is paused.
     */
    7: {
        message: string;
    };
};
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
    pause: (options?: MethodOptions) => Promise<AssembledTransaction<Result<void>>>;
    /**
     * Construct and simulate a attest transaction. Returns an `AssembledTransaction` object which will have a `result` field containing the result of the simulation. If this transaction changes contract state, you will need to call `signAndSend()` on the returned object.
     * Record that `attester` verified the record hashing to `record_hash`.
     * Requires `attester`'s authorization and that `attester` is
     * currently allowlisted in the configured `attester-registry`.
     * Stores the attestation with an incrementing sequence number,
     * maintaining a bounded history (MAX_HISTORY entries per hash).
     */
    attest: ({ attester, record_hash }: {
        attester: string;
        record_hash: Buffer;
    }, options?: MethodOptions) => Promise<AssembledTransaction<Result<Attestation>>>;
    /**
     * Construct and simulate a unpause transaction. Returns an `AssembledTransaction` object which will have a `result` field containing the result of the simulation. If this transaction changes contract state, you will need to call `signAndSend()` on the returned object.
     * Resume normal operation after a `pause`. Requires the admin's authorization.
     */
    unpause: (options?: MethodOptions) => Promise<AssembledTransaction<Result<void>>>;
    /**
     * Construct and simulate a get_admin transaction. Returns an `AssembledTransaction` object which will have a `result` field containing the result of the simulation. If this transaction changes contract state, you will need to call `signAndSend()` on the returned object.
     * Return the current admin address.
     */
    get_admin: (options?: MethodOptions) => Promise<AssembledTransaction<Result<string>>>;
    /**
     * Construct and simulate a is_paused transaction. Returns an `AssembledTransaction` object which will have a `result` field containing the result of the simulation. If this transaction changes contract state, you will need to call `signAndSend()` on the returned object.
     * Whether the contract is currently paused.
     */
    is_paused: (options?: MethodOptions) => Promise<AssembledTransaction<boolean>>;
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
    initialize: ({ admin, attester_registry }: {
        admin: string;
        attester_registry: string;
    }, options?: MethodOptions) => Promise<AssembledTransaction<Result<void>>>;
    /**
     * Construct and simulate a accept_admin transaction. Returns an `AssembledTransaction` object which will have a `result` field containing the result of the simulation. If this transaction changes contract state, you will need to call `signAndSend()` on the returned object.
     * Accept the proposed admin transfer. The caller must authorize as the pending admin.
     */
    accept_admin: (options?: MethodOptions) => Promise<AssembledTransaction<Result<void>>>;
    /**
     * Construct and simulate a propose_admin transaction. Returns an `AssembledTransaction` object which will have a `result` field containing the result of the simulation. If this transaction changes contract state, you will need to call `signAndSend()` on the returned object.
     * Propose a new admin address. The caller must authorize as the current admin.
     */
    propose_admin: ({ new_admin }: {
        new_admin: string;
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
    /**
     * Construct and simulate a revoke_attestation transaction. Returns an `AssembledTransaction` object which will have a `result` field containing the result of the simulation. If this transaction changes contract state, you will need to call `signAndSend()` on the returned object.
     * Revoke all attestations for `record_hash`. Gated by admin authorization.
     */
    revoke_attestation: ({ record_hash }: {
        record_hash: Buffer;
    }, options?: MethodOptions) => Promise<AssembledTransaction<Result<void>>>;
    /**
     * Construct and simulate a get_attester_registry transaction. Returns an `AssembledTransaction` object which will have a `result` field containing the result of the simulation. If this transaction changes contract state, you will need to call `signAndSend()` on the returned object.
     * Return the configured attester-registry contract address.
     */
    get_attester_registry: (options?: MethodOptions) => Promise<AssembledTransaction<Result<string>>>;
    /**
     * Construct and simulate a set_attester_registry transaction. Returns an `AssembledTransaction` object which will have a `result` field containing the result of the simulation. If this transaction changes contract state, you will need to call `signAndSend()` on the returned object.
     * Change the attester-registry contract this registry consults for
     * allowlist checks. Requires the admin's authorization. Emits
     * `AttesterRegistryRepointed` for indexer/audit visibility.
     */
    set_attester_registry: ({ new_registry }: {
        new_registry: string;
    }, options?: MethodOptions) => Promise<AssembledTransaction<Result<void>>>;
    /**
     * Construct and simulate a get_attestation_history transaction. Returns an `AssembledTransaction` object which will have a `result` field containing the result of the simulation. If this transaction changes contract state, you will need to call `signAndSend()` on the returned object.
     * Look up the full attestation history for `record_hash`, if any.
     * Returns attestations in chronological order (oldest first).
     * Callable by anyone.
     */
    get_attestation_history: ({ record_hash }: {
        record_hash: Buffer;
    }, options?: MethodOptions) => Promise<AssembledTransaction<Array<Attestation>>>;
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
        pause: (json: string) => AssembledTransaction<Result<void, import("@stellar/stellar-sdk/contract").ErrorMessage>>;
        attest: (json: string) => AssembledTransaction<Result<Attestation, import("@stellar/stellar-sdk/contract").ErrorMessage>>;
        unpause: (json: string) => AssembledTransaction<Result<void, import("@stellar/stellar-sdk/contract").ErrorMessage>>;
        get_admin: (json: string) => AssembledTransaction<Result<string, import("@stellar/stellar-sdk/contract").ErrorMessage>>;
        is_paused: (json: string) => AssembledTransaction<boolean>;
        initialize: (json: string) => AssembledTransaction<Result<void, import("@stellar/stellar-sdk/contract").ErrorMessage>>;
        accept_admin: (json: string) => AssembledTransaction<Result<void, import("@stellar/stellar-sdk/contract").ErrorMessage>>;
        propose_admin: (json: string) => AssembledTransaction<Result<void, import("@stellar/stellar-sdk/contract").ErrorMessage>>;
        get_attestation: (json: string) => AssembledTransaction<Option<Attestation>>;
        revoke_attestation: (json: string) => AssembledTransaction<Result<void, import("@stellar/stellar-sdk/contract").ErrorMessage>>;
        get_attester_registry: (json: string) => AssembledTransaction<Result<string, import("@stellar/stellar-sdk/contract").ErrorMessage>>;
        set_attester_registry: (json: string) => AssembledTransaction<Result<void, import("@stellar/stellar-sdk/contract").ErrorMessage>>;
        get_attestation_history: (json: string) => AssembledTransaction<Attestation[]>;
    };
}
