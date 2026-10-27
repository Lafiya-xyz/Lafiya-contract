import { xdr } from "@stellar/stellar-sdk";
/** One node of the decoded invocation tree. */
export interface DecodedInvocation {
    /** Contract strkey (C...), or a label for non-contract-call functions. */
    contract: string;
    function: string;
    /** Arguments converted with `scValToNative`, then JSON-safe stringified. */
    args: string[];
    subInvocations: DecodedInvocation[];
}
export interface DecodedIntent {
    /** Account whose authorization is requested (the multisig contract). */
    signer: string | null;
    validUntilLedger: number;
    invocation: DecodedInvocation;
}
/** Decode `entry` into the call tree a signer is approving. */
export declare function decodeIntent(entry: xdr.SorobanAuthorizationEntry, validUntilLedger: number): DecodedIntent;
