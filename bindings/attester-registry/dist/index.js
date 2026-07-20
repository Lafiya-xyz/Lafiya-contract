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
    2: { message: "AlreadyInitialized" }
};
export class Client extends ContractClient {
    options;
    static async deploy(
    /** Options for initializing a Client as well as for calling a method, with extras specific to deploying. */
    options) {
        return ContractClient.deploy(null, options);
    }
    constructor(options) {
        super(new ContractSpec(["AAAABAAAAAAAAAAAAAAABUVycm9yAAAAAAAAAgAAAAAAAAAOTm90SW5pdGlhbGl6ZWQAAAAAAAEAAAAAAAAAEkFscmVhZHlJbml0aWFsaXplZAAAAAAAAg==",
            "AAAAAQAAADFNZXRhZGF0YSBhc3NvY2lhdGVkIHdpdGggYW4gYWxsb3dsaXN0ZWQgYXR0ZXN0ZXIuAAAAAAAAAAAAAAxBdHRlc3RlckluZm8AAAACAAAAAAAAAAxsaWNlbnNlX2hhc2gAAAPoAAAD7gAAACAAAAAAAAAABnJlZ2lvbgAAAAAD6AAAABE=",
            "AAAABQAAAAAAAAAAAAAADUF0dGVzdGVyQWRkZWQAAAAAAAABAAAADmF0dGVzdGVyX2FkZGVkAAAAAAABAAAAAAAAAAhhdHRlc3RlcgAAABMAAAABAAAAAg==",
            "AAAABQAAAAAAAAAAAAAAD0F0dGVzdGVyUmVtb3ZlZAAAAAABAAAAEGF0dGVzdGVyX3JlbW92ZWQAAAABAAAAAAAAAAhhdHRlc3RlcgAAABMAAAABAAAAAg==",
            "AAAAAAAAAIJTZXQgdGhlIGFkbWluIGFkZHJlc3MgYXV0aG9yaXplZCB0byBtYW5hZ2UgdGhlIGFsbG93bGlzdC4gQ2FuIG9ubHkKYmUgY2FsbGVkIG9uY2U7IHRoZSBjYWxsZXIgbXVzdCBhdXRob3JpemUgYXMgdGhlIGdpdmVuIGBhZG1pbmAuAAAAAAAKaW5pdGlhbGl6ZQAAAAAAAQAAAAAAAAAFYWRtaW4AAAAAAAATAAAAAQAAA+kAAAACAAAAAw==",
            "AAAAAAAAAHlXaGV0aGVyIGBhdHRlc3RlcmAgaXMgY3VycmVudGx5IGFsbG93bGlzdGVkLiBDYWxsYWJsZSBieSBhbnlvbmUsCmluY2x1ZGluZyBvdGhlciBjb250cmFjdHMgKGUuZy4gYGF0dGVzdGF0aW9uLXJlZ2lzdHJ5YCkuAAAAAAAAC2lzX2F0dGVzdGVyAAAAAAEAAAAAAAAACGF0dGVzdGVyAAAAEwAAAAEAAAAB",
            "AAAAAAAAAERBZGQgYGF0dGVzdGVyYCB0byB0aGUgYWxsb3dsaXN0LiBSZXF1aXJlcyB0aGUgYWRtaW4ncyBhdXRob3JpemF0aW9uLgAAAAxhZGRfYXR0ZXN0ZXIAAAABAAAAAAAAAAhhdHRlc3RlcgAAABMAAAABAAAD6QAAAAIAAAAD",
            "AAAAAAAAAHhSZW1vdmUgYGF0dGVzdGVyYCBmcm9tIHRoZSBhbGxvd2xpc3QuIFJlcXVpcmVzIHRoZSBhZG1pbidzCmF1dGhvcml6YXRpb24uIEEgbm8tb3AgaWYgdGhlIGF0dGVzdGVyIHdhcyBuZXZlciBhbGxvd2xpc3RlZC4AAAAPcmVtb3ZlX2F0dGVzdGVyAAAAAAEAAAAAAAAACGF0dGVzdGVyAAAAEwAAAAEAAAPpAAAAAgAAAAM=",
            "AAAAAAAAAE1HZXQgdGhlIG9wdGlvbmFsIG1ldGFkYXRhIGFzc29jaWF0ZWQgd2l0aCBgYXR0ZXN0ZXJgIGlmIHRoZXkgYXJlIGFsbG93bGlzdGVkLgAAAAAAABFnZXRfYXR0ZXN0ZXJfaW5mbwAAAAAAAAEAAAAAAAAACGF0dGVzdGVyAAAAEwAAAAEAAAPoAAAH0AAAAAxBdHRlc3RlckluZm8=",
            "AAAAAAAAAFtBZGQgYGF0dGVzdGVyYCB3aXRoIG9wdGlvbmFsIG1ldGFkYXRhIHRvIHRoZSBhbGxvd2xpc3QuIFJlcXVpcmVzIHRoZSBhZG1pbidzIGF1dGhvcml6YXRpb24uAAAAABZhZGRfYXR0ZXN0ZXJfd2l0aF9pbmZvAAAAAAADAAAAAAAAAAhhdHRlc3RlcgAAABMAAAAAAAAADGxpY2Vuc2VfaGFzaAAAA+gAAAPuAAAAIAAAAAAAAAAGcmVnaW9uAAAAAAPoAAAAEQAAAAEAAAPpAAAAAgAAAAM="]), options);
        this.options = options;
    }
    fromJSON = {
        initialize: (this.txFromJSON),
        is_attester: (this.txFromJSON),
        add_attester: (this.txFromJSON),
        remove_attester: (this.txFromJSON),
        get_attester_info: (this.txFromJSON),
        add_attester_with_info: (this.txFromJSON)
    };
}
