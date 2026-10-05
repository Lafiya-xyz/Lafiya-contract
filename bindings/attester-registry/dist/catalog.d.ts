export type ContractKind = "attestation-registry" | "attester-registry";
export type ErrorSeverity = "user" | "operator" | "bug";
export interface ContractErrorInfo {
    contract: ContractKind;
    code: number;
    name: string;
    doc: string;
    severity: ErrorSeverity;
    retryable: boolean;
    i18n_key: string;
    since: string;
    deprecated: string | null;
}
export declare const CONTRACT_ERRORS: readonly ContractErrorInfo[];
/** Decode a contract error code; codes overlap between contracts, so the kind is required. */
export declare function decodeContractError(contractKind: ContractKind, code: number): ContractErrorInfo | undefined;
