import { Buffer } from "buffer";
import { Client as ContractClient, Spec as ContractSpec, } from "@stellar/stellar-sdk/contract";
export * from "@stellar/stellar-sdk";
export * as contract from "@stellar/stellar-sdk/contract";
export * as rpc from "@stellar/stellar-sdk/rpc";
if (typeof window !== "undefined") {
    //@ts-ignore Buffer exists
    window.Buffer = window.Buffer || Buffer;
}
export const Errors = {
    1: { message: "NotInitialized" },
    2: { message: "AlreadyInitialized" },
    3: { message: "AttesterNotAllowlisted" }
};
export class Client extends ContractClient {
    options;
    static async deploy(
    /** Options for initializing a Client as well as for calling a method, with extras specific to deploying. */
    options) {
        return ContractClient.deploy(null, options);
    }
    constructor(options) {
        super(new ContractSpec(["AAAABAAAAAAAAAAAAAAABUVycm9yAAAAAAAAAwAAAAAAAAAOTm90SW5pdGlhbGl6ZWQAAAAAAAEAAAAAAAAAEkFscmVhZHlJbml0aWFsaXplZAAAAAAAAgAAAAAAAAAWQXR0ZXN0ZXJOb3RBbGxvd2xpc3RlZAAAAAAAAw==",
            "AAAAAQAAAKJBIHNpbmdsZSBhdHRlc3RhdGlvbjogcHJvb2YgdGhhdCBgYXR0ZXN0ZXJgIHZlcmlmaWVkIHRoZSBvZmYtY2hhaW4KcmVjb3JkIHdob3NlIGhhc2ggaXMgdGhlIGxvb2t1cCBrZXksIGF0IGB0aW1lc3RhbXBgLiBOZXZlciBjb250YWlucyB0aGUKdW5kZXJseWluZyBoZWFsdGggZGF0YS4AAAAAAAAAAAALQXR0ZXN0YXRpb24AAAAAAgAAAAAAAAAIYXR0ZXN0ZXIAAAATAAAAAAAAAAl0aW1lc3RhbXAAAAAAAAAG",
            "AAAABQAAAAAAAAAAAAAAE0F0dGVzdGF0aW9uUmVjb3JkZWQAAAAAAQAAABRhdHRlc3RhdGlvbl9yZWNvcmRlZAAAAAMAAAAAAAAAC3JlY29yZF9oYXNoAAAAA+4AAAAgAAAAAQAAAAAAAAAIYXR0ZXN0ZXIAAAATAAAAAAAAAAAAAAAJdGltZXN0YW1wAAAAAAAABgAAAAAAAAAC",
            "AAAAAAAAAPlSZWNvcmQgdGhhdCBgYXR0ZXN0ZXJgIHZlcmlmaWVkIHRoZSByZWNvcmQgaGFzaGluZyB0byBgcmVjb3JkX2hhc2hgLgpSZXF1aXJlcyBgYXR0ZXN0ZXJgJ3MgYXV0aG9yaXphdGlvbiBhbmQgdGhhdCBgYXR0ZXN0ZXJgIGlzCmN1cnJlbnRseSBhbGxvd2xpc3RlZCBpbiB0aGUgY29uZmlndXJlZCBgYXR0ZXN0ZXItcmVnaXN0cnlgLgpPdmVyd3JpdGVzIGFueSBwcmlvciBhdHRlc3RhdGlvbiBmb3IgdGhlIHNhbWUgYHJlY29yZF9oYXNoYC4AAAAAAAAGYXR0ZXN0AAAAAAACAAAAAAAAAAhhdHRlc3RlcgAAABMAAAAAAAAAC3JlY29yZF9oYXNoAAAAA+4AAAAgAAAAAQAAA+kAAAfQAAAAC0F0dGVzdGF0aW9uAAAAAAM=",
            "AAAAAAAAAKhTZXQgdGhlIGFkbWluIGFuZCB0aGUgYGF0dGVzdGVyLXJlZ2lzdHJ5YCBjb250cmFjdCB0aGlzIHJlZ2lzdHJ5CmNvbnN1bHRzIGZvciBhbGxvd2xpc3QgY2hlY2tzLiBDYW4gb25seSBiZSBjYWxsZWQgb25jZTsgdGhlIGNhbGxlcgptdXN0IGF1dGhvcml6ZSBhcyB0aGUgZ2l2ZW4gYGFkbWluYC4AAAAKaW5pdGlhbGl6ZQAAAAAAAgAAAAAAAAAFYWRtaW4AAAAAAAATAAAAAAAAABFhdHRlc3Rlcl9yZWdpc3RyeQAAAAAAABMAAAABAAAD6QAAAAIAAAAD",
            "AAAAAAAAAK9Mb29rIHVwIHRoZSBsYXRlc3QgYXR0ZXN0YXRpb24gZm9yIGByZWNvcmRfaGFzaGAsIGlmIGFueS4gQ2FsbGFibGUKYnkgYW55b25lIOKAlCB0aGlzIGlzIHdoYXQgbGV0cyBhIHJlc3BvbmRlcidzIFFSIHNjYW4gaW5kZXBlbmRlbnRseQpjaGVjayBhIGNhcmQgd2l0aG91dCBhbiBleHRlcm5hbCBvcmFjbGUuAAAAAA9nZXRfYXR0ZXN0YXRpb24AAAAAAQAAAAAAAAALcmVjb3JkX2hhc2gAAAAD7gAAACAAAAABAAAD6AAAB9AAAAALQXR0ZXN0YXRpb24A"]), options);
        this.options = options;
    }
    fromJSON = {
        attest: (this.txFromJSON),
        initialize: (this.txFromJSON),
        get_attestation: (this.txFromJSON)
    };
}
