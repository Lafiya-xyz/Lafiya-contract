import { Buffer } from "buffer";
import { Client as ContractClient, Spec as ContractSpec, } from "@stellar/stellar-sdk/contract";
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
    1: { message: "NotInitialized" },
    /**
     * `initialize` was called more than once.
     */
    2: { message: "AlreadyInitialized" },
    /**
     * The caller is not allowlisted by the `attester-registry` contract.
     */
    3: { message: "AttesterNotAllowlisted" },
    /**
     * `accept_admin` was called with no pending admin transfer. Admin transfer is a
     * two-step flow: the current admin must first call `propose_admin` to nominate a
     * successor, then the nominated address must call `accept_admin` to complete the
     * transfer. This error is returned when `accept_admin` is called before a
     * corresponding `propose_admin` call has set a pending admin.
     */
    4: { message: "NoPendingTransfer" },
    /**
     * The configured `attester-registry` address does not implement the expected interface. Re-run `set_attester_registry` with the correct address, or check your network configuration.
     */
    5: { message: "InvalidRegistryWiring" },
    /**
     * No attestation exists for the given record hash / sequence.
     */
    6: { message: "AttestationNotFound" },
    /**
     * The requested operation is blocked while the contract is paused.
     */
    7: { message: "ContractPaused" },
    /**
     * The attester has no active attestation for the given record hash.
     */
    8: { message: "AttestationNotOwned" },
    /**
     * The supplied previous hash or version relationship is invalid.
     */
    9: { message: "InvalidRecordVersion" },
    /**
     * The batch contains more requests than the supported maximum.
     */
    10: { message: "BatchTooLarge" },
    /**
     * The stored schema version is already current.
     */
    11: { message: "MigrationNotRequired" }
};
export class Client extends ContractClient {
    options;
    static async deploy(
    /** Options for initializing a Client as well as for calling a method, with extras specific to deploying. */
    options) {
        return ContractClient.deploy(null, options);
    }
    constructor(options) {
        super(new ContractSpec(["AAAABAAAAEJFcnJvcnMgcmV0dXJuZWQgYnkgdGhlIGF0dGVzdGF0aW9uIHJlZ2lzdHJ5J3MgcHVibGljIGVudHJ5IHBvaW50cy4AAAAAAAAAAAAFRXJyb3IAAAAAAAALAAAAJWBpbml0aWFsaXplYCBoYXMgbm90IGJlZW4gY2FsbGVkIHlldC4AAAAAAAAOTm90SW5pdGlhbGl6ZWQAAAAAAAEAAAAnYGluaXRpYWxpemVgIHdhcyBjYWxsZWQgbW9yZSB0aGFuIG9uY2UuAAAAABJBbHJlYWR5SW5pdGlhbGl6ZWQAAAAAAAIAAABCVGhlIGNhbGxlciBpcyBub3QgYWxsb3dsaXN0ZWQgYnkgdGhlIGBhdHRlc3Rlci1yZWdpc3RyeWAgY29udHJhY3QuAAAAAAAWQXR0ZXN0ZXJOb3RBbGxvd2xpc3RlZAAAAAAAAwAAAW9gYWNjZXB0X2FkbWluYCB3YXMgY2FsbGVkIHdpdGggbm8gcGVuZGluZyBhZG1pbiB0cmFuc2Zlci4gQWRtaW4gdHJhbnNmZXIgaXMgYQp0d28tc3RlcCBmbG93OiB0aGUgY3VycmVudCBhZG1pbiBtdXN0IGZpcnN0IGNhbGwgYHByb3Bvc2VfYWRtaW5gIHRvIG5vbWluYXRlIGEKc3VjY2Vzc29yLCB0aGVuIHRoZSBub21pbmF0ZWQgYWRkcmVzcyBtdXN0IGNhbGwgYGFjY2VwdF9hZG1pbmAgdG8gY29tcGxldGUgdGhlCnRyYW5zZmVyLiBUaGlzIGVycm9yIGlzIHJldHVybmVkIHdoZW4gYGFjY2VwdF9hZG1pbmAgaXMgY2FsbGVkIGJlZm9yZSBhCmNvcnJlc3BvbmRpbmcgYHByb3Bvc2VfYWRtaW5gIGNhbGwgaGFzIHNldCBhIHBlbmRpbmcgYWRtaW4uAAAAABFOb1BlbmRpbmdUcmFuc2ZlcgAAAAAAAAQAAACzVGhlIGNvbmZpZ3VyZWQgYGF0dGVzdGVyLXJlZ2lzdHJ5YCBhZGRyZXNzIGRvZXMgbm90IGltcGxlbWVudCB0aGUgZXhwZWN0ZWQgaW50ZXJmYWNlLiBSZS1ydW4gYHNldF9hdHRlc3Rlcl9yZWdpc3RyeWAgd2l0aCB0aGUgY29ycmVjdCBhZGRyZXNzLCBvciBjaGVjayB5b3VyIG5ldHdvcmsgY29uZmlndXJhdGlvbi4AAAAAFUludmFsaWRSZWdpc3RyeVdpcmluZwAAAAAAAAUAAAA7Tm8gYXR0ZXN0YXRpb24gZXhpc3RzIGZvciB0aGUgZ2l2ZW4gcmVjb3JkIGhhc2ggLyBzZXF1ZW5jZS4AAAAAE0F0dGVzdGF0aW9uTm90Rm91bmQAAAAABgAAAEBUaGUgcmVxdWVzdGVkIG9wZXJhdGlvbiBpcyBibG9ja2VkIHdoaWxlIHRoZSBjb250cmFjdCBpcyBwYXVzZWQuAAAADkNvbnRyYWN0UGF1c2VkAAAAAAAHAAAAQVRoZSBhdHRlc3RlciBoYXMgbm8gYWN0aXZlIGF0dGVzdGF0aW9uIGZvciB0aGUgZ2l2ZW4gcmVjb3JkIGhhc2guAAAAAAAAE0F0dGVzdGF0aW9uTm90T3duZWQAAAAACAAAAD5UaGUgc3VwcGxpZWQgcHJldmlvdXMgaGFzaCBvciB2ZXJzaW9uIHJlbGF0aW9uc2hpcCBpcyBpbnZhbGlkLgAAAAAAFEludmFsaWRSZWNvcmRWZXJzaW9uAAAACQAAADxUaGUgYmF0Y2ggY29udGFpbnMgbW9yZSByZXF1ZXN0cyB0aGFuIHRoZSBzdXBwb3J0ZWQgbWF4aW11bS4AAAANQmF0Y2hUb29MYXJnZQAAAAAAAAoAAAAtVGhlIHN0b3JlZCBzY2hlbWEgdmVyc2lvbiBpcyBhbHJlYWR5IGN1cnJlbnQuAAAAAAAAFE1pZ3JhdGlvbk5vdFJlcXVpcmVkAAAACw==",
            "AAAABQAAADJFbWl0dGVkIHdoZW4gc3RhdGUtY2hhbmdpbmcgb3BlcmF0aW9ucyBhcmUgcGF1c2VkLgAAAAAAAAAAAAZQYXVzZWQAAAAAAAEAAAAGcGF1c2VkAAAAAAABAAAAAAAAAAJieQAAAAAAEwAAAAEAAAAC",
            "AAAABQAAADRFbWl0dGVkIHdoZW4gc3RhdGUtY2hhbmdpbmcgb3BlcmF0aW9ucyBhcmUgdW5wYXVzZWQuAAAAAAAAAAhVbnBhdXNlZAAAAAEAAAAIdW5wYXVzZWQAAAABAAAAAAAAAAJieQAAAAAAEwAAAAEAAAAC",
            "AAAAAQAAAKJBIHNpbmdsZSBhdHRlc3RhdGlvbjogcHJvb2YgdGhhdCBgYXR0ZXN0ZXJgIHZlcmlmaWVkIHRoZSBvZmYtY2hhaW4KcmVjb3JkIHdob3NlIGhhc2ggaXMgdGhlIGxvb2t1cCBrZXksIGF0IGB0aW1lc3RhbXBgLiBOZXZlciBjb250YWlucyB0aGUKdW5kZXJseWluZyBoZWFsdGggZGF0YS4AAAAAAAAAAAALQXR0ZXN0YXRpb24AAAAAAgAAADJUaGUgYWxsb3dsaXN0ZWQgYXR0ZXN0ZXIgdGhhdCB2ZXJpZmllZCB0aGUgcmVjb3JkLgAAAAAACGF0dGVzdGVyAAAAEwAAADdMZWRnZXIgdGltZXN0YW1wIGF0IHdoaWNoIHRoZSBhdHRlc3RhdGlvbiB3YXMgcmVjb3JkZWQuAAAAAAl0aW1lc3RhbXAAAAAAAAAG",
            "AAAABQAAAERFbWl0dGVkIHdoZW4gYWRtaW4gb3duZXJzaGlwIGZpbmlzaGVzIHRyYW5zZmVycmluZyB0byBhIG5ldyBhZGRyZXNzLgAAAAAAAAAQQWRtaW5UcmFuc2ZlcnJlZAAAAAEAAAARYWRtaW5fdHJhbnNmZXJyZWQAAAAAAAACAAAAAAAAAA5wcmV2aW91c19hZG1pbgAAAAAAEwAAAAEAAAAAAAAACW5ld19hZG1pbgAAAAAAABMAAAABAAAAAg==",
            "AAAAAgAAAH1TdW1tYXJ5IHN0YXRlIGZvciBhIHJlY29yZCBoYXNoLCBkaXN0aW5ndWlzaGluZyBhYnNlbnQgdmVyaWZpY2F0aW9uIGZyb20KYW4gZXhwbGljaXQgd2l0aGRyYXdhbCBvciBhZG1pbmlzdHJhdGl2ZSByZXZvY2F0aW9uLgAAAAAAAAAAAAARQXR0ZXN0YXRpb25TdGF0dXMAAAAAAAAEAAAAAAAAAAAAAAANTmV2ZXJBdHRlc3RlZAAAAAAAAAAAAAAAAAAACFZlcmlmaWVkAAAAAAAAAAAAAAAJV2l0aGRyYXduAAAAAAAAAAAAAAAAAAAHUmV2b2tlZAA=",
            "AAAAAQAAAFVPbmUgYXR0ZXN0YXRpb24gdG8gc3VibWl0IGluIGEgYmF0Y2gsIG9wdGlvbmFsbHkgbGlua2VkIHRvIGEgcHJldmlvdXMKcmVjb3JkIHZlcnNpb24uAAAAAAAAAAAAABJBdHRlc3RhdGlvblJlcXVlc3QAAAAAAAMAAAAAAAAACGF0dGVzdGVyAAAAEwAAAAAAAAAUcHJldmlvdXNfcmVjb3JkX2hhc2gAAAPoAAAD7gAAACAAAAAAAAAAC3JlY29yZF9oYXNoAAAAA+4AAAAg",
            "AAAABQAAACdFbWl0dGVkIHdoZW4gYW4gYXR0ZXN0YXRpb24gaXMgcmV2b2tlZC4AAAAAAAAAABJBdHRlc3RhdGlvblJldm9rZWQAAAAAAAEAAAATYXR0ZXN0YXRpb25fcmV2b2tlZAAAAAABAAAAAAAAAAtyZWNvcmRfaGFzaAAAAAPuAAAAIAAAAAEAAAAC",
            "AAAABQAAAD1FbWl0dGVkIHdoZW4gYSBuZXcgYXR0ZXN0YXRpb24gaXMgcmVjb3JkZWQgZm9yIGEgcmVjb3JkIGhhc2guAAAAAAAAAAAAABNBdHRlc3RhdGlvblJlY29yZGVkAAAAAAEAAAAUYXR0ZXN0YXRpb25fcmVjb3JkZWQAAAADAAAAAAAAAAtyZWNvcmRfaGFzaAAAAAPuAAAAIAAAAAEAAAAyVGhlIGFsbG93bGlzdGVkIGF0dGVzdGVyIHRoYXQgdmVyaWZpZWQgdGhlIHJlY29yZC4AAAAAAAhhdHRlc3RlcgAAABMAAAAAAAAAN0xlZGdlciB0aW1lc3RhbXAgYXQgd2hpY2ggdGhlIGF0dGVzdGF0aW9uIHdhcyByZWNvcmRlZC4AAAAACXRpbWVzdGFtcAAAAAAAAAYAAAAAAAAAAg==",
            "AAAABQAAAEFFbWl0dGVkIHdoZW4gYSBuZXcgcmVjb3JkIGhhc2ggaXMgbGlua2VkIHRvIGl0cyBwcmV2aW91cyB2ZXJzaW9uLgAAAAAAAAAAAAATUmVjb3JkVmVyc2lvbkxpbmtlZAAAAAABAAAAFXJlY29yZF92ZXJzaW9uX2xpbmtlZAAAAAAAAAIAAAAAAAAAFHByZXZpb3VzX3JlY29yZF9oYXNoAAAD7gAAACAAAAABAAAAAAAAAAtyZWNvcmRfaGFzaAAAAAPuAAAAIAAAAAEAAAAC",
            "AAAABQAAADlFbWl0dGVkIHdoZW4gYW4gYXR0ZXN0ZXIgd2l0aGRyYXdzIHRoZWlyIG93biBhdHRlc3RhdGlvbi4AAAAAAAAAAAAAFEF0dGVzdGF0aW9uV2l0aGRyYXduAAAAAQAAABVhdHRlc3RhdGlvbl93aXRoZHJhd24AAAAAAAACAAAAAAAAAAtyZWNvcmRfaGFzaAAAAAPuAAAAIAAAAAEAAAAAAAAACGF0dGVzdGVyAAAAEwAAAAEAAAAC",
            "AAAAAgAAADhTdGF0dXMgb2Ygb25lIGF0dGVzdGVyJ3MgdmVyaWZpY2F0aW9uIGZvciBhIHJlY29yZCBoYXNoLgAAAAAAAAAZQXR0ZXN0ZXJBdHRlc3RhdGlvblN0YXR1cwAAAAAAAAQAAAAAAAAAAAAAAA1OZXZlckF0dGVzdGVkAAAAAAAAAAAAAAAAAAAGQWN0aXZlAAAAAAAAAAAAAAAAAAlXaXRoZHJhd24AAAAAAAAAAAAAAAAAAAdSZXZva2VkAA==",
            "AAAABQAAAFJFbWl0dGVkIHdoZW4gdGhlIGBhdHRlc3Rlci1yZWdpc3RyeWAgY29udHJhY3QgdGhpcyByZWdpc3RyeSBjb25zdWx0cyBpcyByZXBvaW50ZWQuAAAAAAAAAAAAGUF0dGVzdGVyUmVnaXN0cnlSZXBvaW50ZWQAAAAAAAABAAAAG2F0dGVzdGVyX3JlZ2lzdHJ5X3JlcG9pbnRlZAAAAAACAAAAAAAAAAhwcmV2aW91cwAAABMAAAABAAAAAAAAAANuZXcAAAAAEwAAAAEAAAAC",
            "AAAAAAAAAGRQYXVzZSB0aGUgY29udHJhY3QsIGJsb2NraW5nIGBhdHRlc3RgIHVudGlsIGB1bnBhdXNlYCBpcyBjYWxsZWQuClJlcXVpcmVzIHRoZSBhZG1pbidzIGF1dGhvcml6YXRpb24uAAAABXBhdXNlAAAAAAAAAAAAAAEAAAPpAAAAAgAAAAM=",
            "AAAAAAAAATdSZWNvcmQgdGhhdCBgYXR0ZXN0ZXJgIHZlcmlmaWVkIHRoZSByZWNvcmQgaGFzaGluZyB0byBgcmVjb3JkX2hhc2hgLgpSZXF1aXJlcyBgYXR0ZXN0ZXJgJ3MgYXV0aG9yaXphdGlvbiBhbmQgdGhhdCBgYXR0ZXN0ZXJgIGlzCmN1cnJlbnRseSBhbGxvd2xpc3RlZCBpbiB0aGUgY29uZmlndXJlZCBgYXR0ZXN0ZXItcmVnaXN0cnlgLgpTdG9yZXMgdGhlIGF0dGVzdGF0aW9uIHdpdGggYW4gaW5jcmVtZW50aW5nIHNlcXVlbmNlIG51bWJlciwKbWFpbnRhaW5pbmcgYSBib3VuZGVkIGhpc3RvcnkgKE1BWF9ISVNUT1JZIGVudHJpZXMgcGVyIGhhc2gpLgAAAAAGYXR0ZXN0AAAAAAACAAAAAAAAAAhhdHRlc3RlcgAAABMAAAAAAAAAC3JlY29yZF9oYXNoAAAAA+4AAAAgAAAAAQAAA+kAAAfQAAAAC0F0dGVzdGF0aW9uAAAAAAM=",
            "AAAAAAAAAMFNYXJrIHRoZSBhZGRpdGl2ZSB2ZXJzaW9uLTIgc3RvcmFnZSBzY2hlbWEgYXMgYXZhaWxhYmxlIGFmdGVyIHVwZ3JhZGUuCk5vIGRhdGEgcmVzaGFwaW5nIGlzIHJlcXVpcmVkOyBhbGwgbmV3bHkgaW50cm9kdWNlZCBrZXlzIGFyZSBvcHRpb25hbAp1bnRpbCB0aGUgY29ycmVzcG9uZGluZyBvcGVyYXRpb24gZmlyc3Qgd3JpdGVzIHRoZW0uAAAAAAAAB21pZ3JhdGUAAAAAAAAAAAEAAAPpAAAAAgAAAAM=",
            "AAAAAAAAAExSZXN1bWUgbm9ybWFsIG9wZXJhdGlvbiBhZnRlciBhIGBwYXVzZWAuIFJlcXVpcmVzIHRoZSBhZG1pbidzIGF1dGhvcml6YXRpb24uAAAAB3VucGF1c2UAAAAAAAAAAAEAAAPpAAAAAgAAAAM=",
            "AAAAAAAAACFSZXR1cm4gdGhlIGN1cnJlbnQgYWRtaW4gYWRkcmVzcy4AAAAAAAAJZ2V0X2FkbWluAAAAAAAAAAAAAAEAAAPpAAAAEwAAAAM=",
            "AAAAAAAAAClXaGV0aGVyIHRoZSBjb250cmFjdCBpcyBjdXJyZW50bHkgcGF1c2VkLgAAAAAAAAlpc19wYXVzZWQAAAAAAAAAAAAAAQAAAAE=",
            "AAAAAAAAAlpTZXQgdGhlIGFkbWluIGFuZCB0aGUgYGF0dGVzdGVyLXJlZ2lzdHJ5YCBjb250cmFjdCB0aGlzIHJlZ2lzdHJ5CmNvbnN1bHRzIGZvciBhbGxvd2xpc3QgY2hlY2tzLiBDYW4gb25seSBiZSBjYWxsZWQgb25jZTsgdGhlIGNhbGxlcgptdXN0IGF1dGhvcml6ZSBhcyB0aGUgZ2l2ZW4gYGFkbWluYC4KCiMjIEJlc3QtZWZmb3J0IGludGVyZmFjZSBjaGVjawoKVGhpcyBmdW5jdGlvbiBwZXJmb3JtcyBhIGxpZ2h0d2VpZ2h0IHNhbml0eSBjaGVjayBhZ2FpbnN0CmBhdHRlc3Rlcl9yZWdpc3RyeWA6IGl0IGNhbGxzIGBpc19hdHRlc3RlcmAgd2l0aCBhIHRocm93YXdheSBhZGRyZXNzCmFuZCBjb25maXJtcyB0aGUgY2FsbCBkb2VzIG5vdCB0cmFwLiBUaGlzIGNvbmZpcm1zIHRoZSBhZGRyZXNzCmltcGxlbWVudHMgdGhlIGV4cGVjdGVkIGludGVyZmFjZSDigJQgaXQgZG9lcyAqKm5vdCoqIHByb3ZlIHRoZSBhZGRyZXNzCmlzIHRoZSBjYW5vbmljYWwsIHRydXN0ZWQgYGF0dGVzdGVyLXJlZ2lzdHJ5YCBkZXBsb3ltZW50LiBBIG1hbGljaW91cwpjb250cmFjdCB0aGF0IGhhcHBlbnMgdG8gZXhwb3NlIGBpc19hdHRlc3RlcmAgd291bGQgcGFzcyB0aGlzIGNoZWNrLgAAAAAACmluaXRpYWxpemUAAAAAAAIAAAAAAAAABWFkbWluAAAAAAAAEwAAAAAAAAARYXR0ZXN0ZXJfcmVnaXN0cnkAAAAAAAATAAAAAQAAA+kAAAACAAAAAw==",
            "AAAAAAAAAFNBY2NlcHQgdGhlIHByb3Bvc2VkIGFkbWluIHRyYW5zZmVyLiBUaGUgY2FsbGVyIG11c3QgYXV0aG9yaXplIGFzIHRoZSBwZW5kaW5nIGFkbWluLgAAAAAMYWNjZXB0X2FkbWluAAAAAAAAAAEAAAPpAAAAAgAAAAM=",
            "AAAAAAAAAKxSZWNvcmQgdXAgdG8gYEJBVENIX0xJTUlUYCBhdHRlc3RhdGlvbnMgaW4gb25lIHRyYW5zYWN0aW9uLiBFYWNoCmF0dGVzdGVyIG11c3QgYXV0aG9yaXplIHRoZWlyIHJlcXVlc3Q7IGFsbG93bGlzdCBzdGF0dXMgaXMgY2hlY2tlZCBvbmNlCnBlciBkaXN0aW5jdCBhdHRlc3RlciBpbiB0aGUgYmF0Y2guAAAADGJhdGNoX2F0dGVzdAAAAAEAAAAAAAAACHJlcXVlc3RzAAAD6gAAB9AAAAASQXR0ZXN0YXRpb25SZXF1ZXN0AAAAAAABAAAD6QAAA+oAAAfQAAAAC0F0dGVzdGF0aW9uAAAAAAM=",
            "AAAAAAAAAExQcm9wb3NlIGEgbmV3IGFkbWluIGFkZHJlc3MuIFRoZSBjYWxsZXIgbXVzdCBhdXRob3JpemUgYXMgdGhlIGN1cnJlbnQgYWRtaW4uAAAADXByb3Bvc2VfYWRtaW4AAAAAAAABAAAAAAAAAAluZXdfYWRtaW4AAAAAAAATAAAAAQAAA+kAAAACAAAAAw==",
            "AAAAAAAAAIVSZWNvcmQgYSB2ZXJpZmljYXRpb24gYW5kIGV4cGxpY2l0bHkgbGluayB0aGlzIHJlY29yZCBoYXNoIHRvIGl0cwpwcmV2aW91cyB2ZXJzaW9uLiBUaGUgcHJldmlvdXMgaGFzaCBtdXN0IGFscmVhZHkgaGF2ZSBhdHRlc3RhdGlvbnMuAAAAAAAADmF0dGVzdF92ZXJzaW9uAAAAAAADAAAAAAAAAAhhdHRlc3RlcgAAABMAAAAAAAAAC3JlY29yZF9oYXNoAAAAA+4AAAAgAAAAAAAAABRwcmV2aW91c19yZWNvcmRfaGFzaAAAA+4AAAAgAAAAAQAAA+kAAAfQAAAAC0F0dGVzdGF0aW9uAAAAAAM=",
            "AAAAAAAAALZMb29rIHVwIHRoZSBsYXRlc3QgYWN0aXZlIGF0dGVzdGF0aW9uIGZvciBgcmVjb3JkX2hhc2hgLCBpZiBhbnkuIENhbGxhYmxlCmJ5IGFueW9uZSDigJQgdGhpcyBpcyB3aGF0IGxldHMgYSByZXNwb25kZXIncyBRUiBzY2FuIGluZGVwZW5kZW50bHkKY2hlY2sgYSBjYXJkIHdpdGhvdXQgYW4gZXh0ZXJuYWwgb3JhY2xlLgAAAAAAD2dldF9hdHRlc3RhdGlvbgAAAAABAAAAAAAAAAtyZWNvcmRfaGFzaAAAAAPuAAAAIAAAAAEAAAPoAAAH0AAAAAtBdHRlc3RhdGlvbgA=",
            "AAAAAAAAADxRdWVyeSB0aGUgc3RvcmFnZSBzY2hlbWEgdmVyc2lvbiBmb3IgdGhpcyBjb250cmFjdCBpbnN0YW5jZS4AAAASZ2V0X3NjaGVtYV92ZXJzaW9uAAAAAAAAAAAAAQAAAAQ=",
            "AAAAAAAAAG9SZXZva2UgYWxsIGF0dGVzdGF0aW9ucyBmb3IgYHJlY29yZF9oYXNoYCB3aXRob3V0IGVyYXNpbmcgdGhlCmhpc3RvcmljYWwgZW50cmllcy4gR2F0ZWQgYnkgYWRtaW4gYXV0aG9yaXphdGlvbi4AAAAAEnJldm9rZV9hdHRlc3RhdGlvbgAAAAAAAQAAAAAAAAALcmVjb3JkX2hhc2gAAAAD7gAAACAAAAABAAAD6QAAAAIAAAAD",
            "AAAAAAAAADlSZXR1cm4gdGhlIGV4cGxpY2l0bHkgbGlua2VkIG5leHQgcmVjb3JkIHZlcnNpb24sIGlmIGFueS4AAAAAAAAUZ2V0X25leHRfcmVjb3JkX2hhc2gAAAABAAAAAAAAAAtyZWNvcmRfaGFzaAAAAAPuAAAAIAAAAAEAAAPoAAAD7gAAACA=",
            "AAAAAAAAAHFXaXRoZHJhdyBhbGwgb2YgdGhlIGNhbGxlcidzIGFjdGl2ZSBhdHRlc3RhdGlvbnMgZm9yIGByZWNvcmRfaGFzaGAuCk90aGVyIGF0dGVzdGVycycgYXR0ZXN0YXRpb25zIGFyZSB1bmFmZmVjdGVkLgAAAAAAABR3aXRoZHJhd19hdHRlc3RhdGlvbgAAAAIAAAAAAAAACGF0dGVzdGVyAAAAEwAAAAAAAAALcmVjb3JkX2hhc2gAAAAD7gAAACAAAAABAAAD6QAAAAIAAAAD",
            "AAAAAAAAADlSZXR1cm4gdGhlIGNvbmZpZ3VyZWQgYXR0ZXN0ZXItcmVnaXN0cnkgY29udHJhY3QgYWRkcmVzcy4AAAAAAAAVZ2V0X2F0dGVzdGVyX3JlZ2lzdHJ5AAAAAAAAAAAAAAEAAAPpAAAAEwAAAAM=",
            "AAAAAAAAALZDaGFuZ2UgdGhlIGF0dGVzdGVyLXJlZ2lzdHJ5IGNvbnRyYWN0IHRoaXMgcmVnaXN0cnkgY29uc3VsdHMgZm9yCmFsbG93bGlzdCBjaGVja3MuIFJlcXVpcmVzIHRoZSBhZG1pbidzIGF1dGhvcml6YXRpb24uIEVtaXRzCmBBdHRlc3RlclJlZ2lzdHJ5UmVwb2ludGVkYCBmb3IgaW5kZXhlci9hdWRpdCB2aXNpYmlsaXR5LgAAAAAAFXNldF9hdHRlc3Rlcl9yZWdpc3RyeQAAAAAAAAEAAAAAAAAADG5ld19yZWdpc3RyeQAAABMAAAABAAAD6QAAAAIAAAAD",
            "AAAAAAAAAIhSZXR1cm4gd2hldGhlciBhIHJlY29yZCBoYXNoIGlzIHVudmVyaWZpZWQsIHZlcmlmaWVkLCB3aXRoZHJhd24sIG9yCmV4cGxpY2l0bHkgcmV2b2tlZC4gVGhpcyByZW1haW5zIGluZm9ybWF0aXZlIGFmdGVyIGFkbWluIHJldm9jYXRpb24uAAAAFmdldF9hdHRlc3RhdGlvbl9zdGF0dXMAAAAAAAEAAAAAAAAAC3JlY29yZF9oYXNoAAAAA+4AAAAgAAAAAQAAB9AAAAARQXR0ZXN0YXRpb25TdGF0dXMAAAA=",
            "AAAAAAAAAI9Mb29rIHVwIHRoZSBmdWxsIGF0dGVzdGF0aW9uIGhpc3RvcnkgZm9yIGByZWNvcmRfaGFzaGAsIGlmIGFueS4KUmV0dXJucyBhdHRlc3RhdGlvbnMgaW4gY2hyb25vbG9naWNhbCBvcmRlciAob2xkZXN0IGZpcnN0KS4KQ2FsbGFibGUgYnkgYW55b25lLgAAAAAXZ2V0X2F0dGVzdGF0aW9uX2hpc3RvcnkAAAAAAQAAAAAAAAALcmVjb3JkX2hhc2gAAAAD7gAAACAAAAABAAAD6gAAB9AAAAALQXR0ZXN0YXRpb24A",
            "AAAAAAAAAD1SZXR1cm4gdGhlIGV4cGxpY2l0bHkgbGlua2VkIHByZXZpb3VzIHJlY29yZCB2ZXJzaW9uLCBpZiBhbnkuAAAAAAAAGGdldF9wcmV2aW91c19yZWNvcmRfaGFzaAAAAAEAAAAAAAAAC3JlY29yZF9oYXNoAAAAA+4AAAAgAAAAAQAAA+gAAAPuAAAAIA==",
            "AAAAAAAAAF5SZXR1cm4gd2hldGhlciBgYXR0ZXN0ZXJgIGhhcyBhbiBhY3RpdmUsIHdpdGhkcmF3biwgb3IgcmV2b2tlZAp2ZXJpZmljYXRpb24gZm9yIGByZWNvcmRfaGFzaGAuAAAAAAAfZ2V0X2F0dGVzdGVyX2F0dGVzdGF0aW9uX3N0YXR1cwAAAAACAAAAAAAAAAtyZWNvcmRfaGFzaAAAAAPuAAAAIAAAAAAAAAAIYXR0ZXN0ZXIAAAATAAAAAQAAB9AAAAAZQXR0ZXN0ZXJBdHRlc3RhdGlvblN0YXR1cwAAAA=="]), options);
        this.options = options;
    }
    fromJSON = {
        pause: (this.txFromJSON),
        attest: (this.txFromJSON),
        migrate: (this.txFromJSON),
        unpause: (this.txFromJSON),
        get_admin: (this.txFromJSON),
        is_paused: (this.txFromJSON),
        initialize: (this.txFromJSON),
        accept_admin: (this.txFromJSON),
        batch_attest: (this.txFromJSON),
        propose_admin: (this.txFromJSON),
        attest_version: (this.txFromJSON),
        get_attestation: (this.txFromJSON),
        get_schema_version: (this.txFromJSON),
        revoke_attestation: (this.txFromJSON),
        get_next_record_hash: (this.txFromJSON),
        withdraw_attestation: (this.txFromJSON),
        get_attester_registry: (this.txFromJSON),
        set_attester_registry: (this.txFromJSON),
        get_attestation_status: (this.txFromJSON),
        get_attestation_history: (this.txFromJSON),
        get_previous_record_hash: (this.txFromJSON),
        get_attester_attestation_status: (this.txFromJSON)
    };
}
