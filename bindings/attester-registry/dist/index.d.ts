import { Buffer } from "buffer";
import { AssembledTransaction, Client as ContractClient, ClientOptions as ContractClientOptions, MethodOptions, Result } from "@stellar/stellar-sdk/contract";
import type { u32, Option } from "@stellar/stellar-sdk/contract";
export * from "@stellar/stellar-sdk";
export * as contract from "@stellar/stellar-sdk/contract";
export * as rpc from "@stellar/stellar-sdk/rpc";
/**
 * Errors returned by the attester registry's public entry points.
 */
export declare const Errors: {
    /**
     * `initialize` has not been called yet; call
     * `initialize(admin: Address)` before using the contract.
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
     * `accept_admin` was called with no pending admin transfer. Admin transfer is a
     * two-step flow: the current admin must first call `propose_admin` to nominate a
     * successor, then the nominated address must call `accept_admin` to complete the
     * transfer. This error is returned when `accept_admin` is called before a
     * corresponding `propose_admin` call has set a pending admin.
     */
    3: {
        message: string;
    };
    /**
     * The requested operation is blocked while the contract is paused.
     */
    4: {
        message: string;
    };
    /**
     * The allowlist is at its configured maximum size. Raise the cap via `set_max_attesters`, or free a slot via `remove_attester`.
     */
    5: {
        message: string;
    };
    /**
     * `migrate()` was called while the stored schema version is already
     * `>= SCHEMA_VERSION`. Only call `migrate()` after `upgrade()` to a
     * build that bumps `SCHEMA_VERSION`; this error is a safe no-op signal
     * that there is nothing pending, not a failure to react to.
     */
    6: {
        message: string;
    };
    /**
     * The referenced attester is not currently allowlisted (never added,
     * or since removed).
     */
    7: {
        message: string;
    };
    /**
     * The supplied batch exceeds `BATCH_LIMIT` addresses.
     */
    8: {
        message: string;
    };
};
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
    pause: (options?: MethodOptions) => Promise<AssembledTransaction<Result<void>>>;
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
    migrate: (options?: MethodOptions) => Promise<AssembledTransaction<Result<void>>>;
    /**
     * Construct and simulate a unpause transaction. Returns an `AssembledTransaction` object which will have a `result` field containing the result of the simulation. If this transaction changes contract state, you will need to call `signAndSend()` on the returned object.
     * Resume normal operation after a `pause`. Requires the admin's authorization.
     */
    unpause: (options?: MethodOptions) => Promise<AssembledTransaction<Result<void>>>;
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
    upgrade: ({ new_wasm_hash }: {
        new_wasm_hash: Buffer;
    }, options?: MethodOptions) => Promise<AssembledTransaction<Result<void>>>;
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
     * Set the admin address authorized to manage the allowlist. Can only
     * be called once; the caller must authorize as the given `admin`.
     */
    initialize: ({ admin }: {
        admin: string;
    }, options?: MethodOptions) => Promise<AssembledTransaction<Result<void>>>;
    /**
     * Construct and simulate a is_attester transaction. Returns an `AssembledTransaction` object which will have a `result` field containing the result of the simulation. If this transaction changes contract state, you will need to call `signAndSend()` on the returned object.
     * Whether `attester` is currently allowlisted (and not suspended). Callable by anyone,
     * including other contracts (e.g. `attestation-registry`).
     */
    is_attester: ({ attester }: {
        attester: string;
    }, options?: MethodOptions) => Promise<AssembledTransaction<boolean>>;
    /**
     * Construct and simulate a accept_admin transaction. Returns an `AssembledTransaction` object which will have a `result` field containing the result of the simulation. If this transaction changes contract state, you will need to call `signAndSend()` on the returned object.
     * Accept the proposed admin transfer. The caller must authorize as the pending admin.
     */
    accept_admin: (options?: MethodOptions) => Promise<AssembledTransaction<Result<void>>>;
    /**
     * Construct and simulate a add_attester transaction. Returns an `AssembledTransaction` object which will have a `result` field containing the result of the simulation. If this transaction changes contract state, you will need to call `signAndSend()` on the returned object.
     * Add `attester` to the allowlist. Requires the admin's authorization.
     * Fails with `Error::AllowlistFull` if the allowlist is at capacity and
     * `attester` is not already present (see `set_max_attesters`).
     */
    add_attester: ({ attester }: {
        attester: string;
    }, options?: MethodOptions) => Promise<AssembledTransaction<Result<void>>>;
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
    add_attesters: ({ attesters }: {
        attesters: Array<string>;
    }, options?: MethodOptions) => Promise<AssembledTransaction<Result<void>>>;
    /**
     * Construct and simulate a propose_admin transaction. Returns an `AssembledTransaction` object which will have a `result` field containing the result of the simulation. If this transaction changes contract state, you will need to call `signAndSend()` on the returned object.
     * Propose a new admin address. The caller must authorize as the current admin.
     * Calling this a second time before `accept_admin` overwrites any pending proposal — the most recent call wins.
     */
    propose_admin: ({ new_admin }: {
        new_admin: string;
    }, options?: MethodOptions) => Promise<AssembledTransaction<Result<void>>>;
    /**
     * Construct and simulate a remove_attester transaction. Returns an `AssembledTransaction` object which will have a `result` field containing the result of the simulation. If this transaction changes contract state, you will need to call `signAndSend()` on the returned object.
     * Remove `attester` from the allowlist. Requires the admin's
     * authorization. A no-op if the attester was never allowlisted.
     */
    remove_attester: ({ attester }: {
        attester: string;
    }, options?: MethodOptions) => Promise<AssembledTransaction<Result<void>>>;
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
    remove_attesters: ({ attesters }: {
        attesters: Array<string>;
    }, options?: MethodOptions) => Promise<AssembledTransaction<Result<void>>>;
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
    suspend_attester: ({ attester }: {
        attester: string;
    }, options?: MethodOptions) => Promise<AssembledTransaction<Result<void>>>;
    /**
     * Construct and simulate a get_attester_info transaction. Returns an `AssembledTransaction` object which will have a `result` field containing the result of the simulation. If this transaction changes contract state, you will need to call `signAndSend()` on the returned object.
     * Get the optional metadata associated with `attester` if they are allowlisted.
     */
    get_attester_info: ({ attester }: {
        attester: string;
    }, options?: MethodOptions) => Promise<AssembledTransaction<Option<AttesterInfo>>>;
    /**
     * Construct and simulate a get_max_attesters transaction. Returns an `AssembledTransaction` object which will have a `result` field containing the result of the simulation. If this transaction changes contract state, you will need to call `signAndSend()` on the returned object.
     * The current soft cap on the number of allowlisted attesters.
     */
    get_max_attesters: (options?: MethodOptions) => Promise<AssembledTransaction<u32>>;
    /**
     * Construct and simulate a set_max_attesters transaction. Returns an `AssembledTransaction` object which will have a `result` field containing the result of the simulation. If this transaction changes contract state, you will need to call `signAndSend()` on the returned object.
     * Set the soft cap on the number of allowlisted attesters. Requires the
     * admin's authorization. Does not evict existing attesters if lowered
     * below the current count; it only blocks further `add_attester` calls.
     */
    set_max_attesters: ({ max_attesters }: {
        max_attesters: u32;
    }, options?: MethodOptions) => Promise<AssembledTransaction<Result<void>>>;
    /**
     * Construct and simulate a get_attester_count transaction. Returns an `AssembledTransaction` object which will have a `result` field containing the result of the simulation. If this transaction changes contract state, you will need to call `signAndSend()` on the returned object.
     * The current number of allowlisted attesters.
     */
    get_attester_count: (options?: MethodOptions) => Promise<AssembledTransaction<u32>>;
    /**
     * Construct and simulate a get_schema_version transaction. Returns an `AssembledTransaction` object which will have a `result` field containing the result of the simulation. If this transaction changes contract state, you will need to call `signAndSend()` on the returned object.
     * Query the current storage schema version of the contract.
     */
    get_schema_version: (options?: MethodOptions) => Promise<AssembledTransaction<u32>>;
    /**
     * Construct and simulate a reinstate_attester transaction. Returns an `AssembledTransaction` object which will have a `result` field containing the result of the simulation. If this transaction changes contract state, you will need to call `signAndSend()` on the returned object.
     * Reinstate a suspended attester. Requires the admin's authorization.
     */
    reinstate_attester: ({ attester }: {
        attester: string;
    }, options?: MethodOptions) => Promise<AssembledTransaction<Result<void>>>;
    /**
     * Construct and simulate a get_attester_status transaction. Returns an `AssembledTransaction` object which will have a `result` field containing the result of the simulation. If this transaction changes contract state, you will need to call `signAndSend()` on the returned object.
     * Get `attester`'s metadata together with its current suspension state
     * in a single call. Returns `None` if `attester` is not currently
     * allowlisted (never added, or since removed).
     */
    get_attester_status: ({ attester }: {
        attester: string;
    }, options?: MethodOptions) => Promise<AssembledTransaction<Option<AttesterStatus>>>;
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
    update_attester_info: ({ attester, license_hash, region }: {
        attester: string;
        license_hash: Option<Buffer>;
        region: Option<string>;
    }, options?: MethodOptions) => Promise<AssembledTransaction<Result<void>>>;
    /**
     * Construct and simulate a add_attester_with_info transaction. Returns an `AssembledTransaction` object which will have a `result` field containing the result of the simulation. If this transaction changes contract state, you will need to call `signAndSend()` on the returned object.
     * Add `attester` with optional metadata to the allowlist. Requires the admin's authorization.
     * Fails with `Error::AllowlistFull` if the allowlist is at capacity and
     * `attester` is not already present (see `set_max_attesters`).
     */
    add_attester_with_info: ({ attester, license_hash, region }: {
        attester: string;
        license_hash: Option<Buffer>;
        region: Option<string>;
    }, options?: MethodOptions) => Promise<AssembledTransaction<Result<void>>>;
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
        migrate: (json: string) => AssembledTransaction<Result<void, import("@stellar/stellar-sdk/contract").ErrorMessage>>;
        unpause: (json: string) => AssembledTransaction<Result<void, import("@stellar/stellar-sdk/contract").ErrorMessage>>;
        upgrade: (json: string) => AssembledTransaction<Result<void, import("@stellar/stellar-sdk/contract").ErrorMessage>>;
        get_admin: (json: string) => AssembledTransaction<Result<string, import("@stellar/stellar-sdk/contract").ErrorMessage>>;
        is_paused: (json: string) => AssembledTransaction<boolean>;
        initialize: (json: string) => AssembledTransaction<Result<void, import("@stellar/stellar-sdk/contract").ErrorMessage>>;
        is_attester: (json: string) => AssembledTransaction<boolean>;
        accept_admin: (json: string) => AssembledTransaction<Result<void, import("@stellar/stellar-sdk/contract").ErrorMessage>>;
        add_attester: (json: string) => AssembledTransaction<Result<void, import("@stellar/stellar-sdk/contract").ErrorMessage>>;
        add_attesters: (json: string) => AssembledTransaction<Result<void, import("@stellar/stellar-sdk/contract").ErrorMessage>>;
        propose_admin: (json: string) => AssembledTransaction<Result<void, import("@stellar/stellar-sdk/contract").ErrorMessage>>;
        remove_attester: (json: string) => AssembledTransaction<Result<void, import("@stellar/stellar-sdk/contract").ErrorMessage>>;
        remove_attesters: (json: string) => AssembledTransaction<Result<void, import("@stellar/stellar-sdk/contract").ErrorMessage>>;
        suspend_attester: (json: string) => AssembledTransaction<Result<void, import("@stellar/stellar-sdk/contract").ErrorMessage>>;
        get_attester_info: (json: string) => AssembledTransaction<Option<AttesterInfo>>;
        get_max_attesters: (json: string) => AssembledTransaction<number>;
        set_max_attesters: (json: string) => AssembledTransaction<Result<void, import("@stellar/stellar-sdk/contract").ErrorMessage>>;
        get_attester_count: (json: string) => AssembledTransaction<number>;
        get_schema_version: (json: string) => AssembledTransaction<number>;
        reinstate_attester: (json: string) => AssembledTransaction<Result<void, import("@stellar/stellar-sdk/contract").ErrorMessage>>;
        get_attester_status: (json: string) => AssembledTransaction<Option<AttesterStatus>>;
        update_attester_info: (json: string) => AssembledTransaction<Result<void, import("@stellar/stellar-sdk/contract").ErrorMessage>>;
        add_attester_with_info: (json: string) => AssembledTransaction<Result<void, import("@stellar/stellar-sdk/contract").ErrorMessage>>;
    };
}
