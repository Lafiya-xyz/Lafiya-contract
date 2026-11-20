import { readFileSync } from "node:fs";
import { fileURLToPath } from "node:url";
import { describe, expect, it } from "vitest";
import { StrKey, xdr } from "@stellar/stellar-sdk";
import {
  addSignatureToCeremony,
  attachSignatures,
  buildAuthPayload,
  createCeremony,
  finalizeCeremony,
  readCeremony,
  requiredSigners,
  writeCeremony,
} from "../dist/index.js";
import { fixtureEntry, randomKeypairs } from "./fixtures.js";

const NETWORK_PASSPHRASE = "Test SDF Network ; September 2015";
const VALID_UNTIL_LEDGER = 1000;

// Canonical cross-language vectors shared with the Rust contract tests
// (see vectors/multisig-auth.json and issue #704).
const VECTORS_PATH = fileURLToPath(new URL("../../../vectors/multisig-auth.json", import.meta.url));
const VECTORS = JSON.parse(readFileSync(VECTORS_PATH, "utf8"));

// Every vector must be exercised by at least one test below; the set of
// consumed ids is asserted against the file at the end of the suite.
const consumedVectorIds = new Set();

function consumeVector(id) {
  consumedVectorIds.add(id);
}

function decodeSignatureVec(signedEntry) {
  const scVal = signedEntry.credentials().address().signature();
  return scVal.vec().map((entryScVal) => {
    const map = entryScVal.map();
    const byKey = Object.fromEntries(map.map((e) => [e.key().sym().toString(), e.val()]));
    return {
      publicKey: StrKey.encodeEd25519PublicKey(byKey.public_key.bytes()),
      signature: byKey.signature.bytes(),
    };
  });
}

describe("buildAuthPayload", () => {
  it("returns a 32-byte hash", () => {
    const entry = fixtureEntry();
    const payload = buildAuthPayload(entry, NETWORK_PASSPHRASE, VALID_UNTIL_LEDGER);
    expect(payload).toHaveLength(32);
  });

  it("is deterministic for the same entry/network/ledger", () => {
    const entry = fixtureEntry({ nonce: 42 });
    const a = buildAuthPayload(entry, NETWORK_PASSPHRASE, VALID_UNTIL_LEDGER);
    const b = buildAuthPayload(entry, NETWORK_PASSPHRASE, VALID_UNTIL_LEDGER);
    expect(Buffer.from(a).equals(Buffer.from(b))).toBe(true);
  });

  it("changes if validUntilLedger changes (it is part of the signed preimage)", () => {
    const entry = fixtureEntry();
    const a = buildAuthPayload(entry, NETWORK_PASSPHRASE, VALID_UNTIL_LEDGER);
    const b = buildAuthPayload(entry, NETWORK_PASSPHRASE, VALID_UNTIL_LEDGER + 1);
    expect(Buffer.from(a).equals(Buffer.from(b))).toBe(false);
  });
});

describe("attachSignatures", () => {
  it("encodes signatures sorted by ascending raw public key, not input order", () => {
    const entry = fixtureEntry();
    const payload = buildAuthPayload(entry, NETWORK_PASSPHRASE, VALID_UNTIL_LEDGER);
    const [kp1, kp2, kp3] = randomKeypairs(3);
    const sigs = [kp1, kp2, kp3].map((kp) => ({
      publicKey: kp.publicKey(),
      signature: kp.sign(Buffer.from(payload)),
    }));
    // Deliberately shuffle the input order.
    const shuffled = [sigs[2], sigs[0], sigs[1]];

    const signed = attachSignatures(entry, shuffled);
    const decoded = decodeSignatureVec(signed);

    const rawKeys = decoded.map((d) => StrKey.decodeEd25519PublicKey(d.publicKey));
    for (let i = 1; i < rawKeys.length; i++) {
      expect(Buffer.compare(rawKeys[i - 1], rawKeys[i])).toBeLessThan(0);
    }
    expect(decoded).toHaveLength(3);
  });

  it("deduplicates an exact-duplicate public key", () => {
    const entry = fixtureEntry();
    const payload = buildAuthPayload(entry, NETWORK_PASSPHRASE, VALID_UNTIL_LEDGER);
    const [kp1, kp2] = randomKeypairs(2);
    const sig1 = { publicKey: kp1.publicKey(), signature: kp1.sign(Buffer.from(payload)) };
    const sig2 = { publicKey: kp2.publicKey(), signature: kp2.sign(Buffer.from(payload)) };

    const signed = attachSignatures(entry, [sig1, sig2, sig1]);
    expect(decodeSignatureVec(signed)).toHaveLength(2);
  });

  it("does not mutate the input entry", () => {
    const entry = fixtureEntry();
    const before = entry.toXDR("base64");
    const [kp] = randomKeypairs(1);
    attachSignatures(entry, [{ publicKey: kp.publicKey(), signature: Buffer.alloc(64) }]);
    expect(entry.toXDR("base64")).toBe(before);
  });

  it("leaves source-account credentials untouched", () => {
    const entry = xdr.SorobanAuthorizationEntry.fromXDR(fixtureEntry().toXDR());
    // Swap in source-account credentials for this one test.
    const withSourceAccount = new xdr.SorobanAuthorizationEntry({
      credentials: xdr.SorobanCredentials.sorobanCredentialsSourceAccount(),
      rootInvocation: entry.rootInvocation(),
    });
    const result = attachSignatures(withSourceAccount, []);
    expect(result.toXDR("base64")).toBe(withSourceAccount.toXDR("base64"));
  });
});

describe("shared signature-payload vectors", () => {
  it("loads a canonical vector file with both key types", () => {
    expect(Array.isArray(VECTORS.vectors)).toBe(true);
    expect(VECTORS.vectors.length).toBeGreaterThan(0);
    const keyTypes = new Set(VECTORS.vectors.map((v) => v.keyType));
    expect(keyTypes.has("ed25519")).toBe(true);
    expect(keyTypes.has("secp256r1")).toBe(true);
  });

  it("covers positive and negative cases for each key type", () => {
    for (const keyType of ["ed25519", "secp256r1"]) {
      const cases = VECTORS.vectors.filter((v) => v.keyType === keyType);
      expect(cases.some((v) => v.expected === "valid")).toBe(true);
      expect(cases.some((v) => v.expected !== "valid")).toBe(true);
    }
  });

  it("reproduces the payload hash for every vector", () => {
    for (const vector of VECTORS.vectors) {
      consumeVector(vector.id);
      const entry = xdr.SorobanAuthorizationEntry.fromXDR(vector.entryXdr, "base64");
      const payload = buildAuthPayload(entry, vector.networkPassphrase, vector.validUntilLedger);
      expect(Buffer.from(payload).toString("hex")).toBe(vector.payloadHex);
    }
  });

  it("accepts positive vectors and rejects negative vectors", () => {
    for (const vector of VECTORS.vectors) {
      consumeVector(vector.id);
      const entry = xdr.SorobanAuthorizationEntry.fromXDR(vector.entryXdr, "base64");
      const payload = Buffer.from(vector.payloadHex, "hex");
      const signature = Buffer.from(vector.signatureHex, "hex");
      const signed = attachSignatures(entry, [{ publicKey: vector.publicKey, signature }]);
      const decoded = decodeSignatureVec(signed);
      if (vector.expected === "valid") {
        expect(decoded).toHaveLength(1);
        expect(decoded[0].signature).toEqual(signature);
      } else {
        // Negative vectors must not produce a verifiable signature entry.
        expect(decoded).toHaveLength(0);
      }
      expect(payload).toHaveLength(32);
    }
  });

  it("exercises every vector in the canonical file", () => {
    const allIds = VECTORS.vectors.map((v) => v.id);
    const missing = allIds.filter((id) => !consumedVectorIds.has(id));
    expect(missing).toEqual([]);
  });
});

describe("requiredSigners", () => {
  const DEFAULT_POLICY = { threshold: 2, signers: ["GA", "GB", "GC"] };
  const STRICT_POLICY = { threshold: 3, signers: ["GA", "GB", "GC"] };
  const CONTEXT_POLICIES = {
    "CCONTRACT:upgrade": STRICT_POLICY,
    "CCONTRACT:add_attester": { threshold: 1, signers: ["GA", "GB", "GC"] },
  };

  function invocationTree(contexts) {
    // Build a nested invocation tree from a list of (contract, fn) contexts.
    let tree = null;
    for (const [contract, fn] of contexts) {
      const call = xdr.SorobanAuthorizedInvocation.sorobanAuthorizedInvocation({
        function: xdr.SorobanAuthorizedFunction.sorobanAuthorizedFunctionTypeContractFn(
          new xdr.InvokeContractArgs({
            contractAddress: StrKey.decodeContract ? xdr.ScAddress.scAddressTypeContract(StrKey.decodeContract(contract)) : undefined,
            functionName: fn,
            args: [],
          }),
        ),
        subInvocations: tree ? [tree] : [],
      });
      tree = call;
    }
    return tree;
  }

  it("uses the default policy for a single unknown context", () => {
    const tree = invocationTree([["CCONTRACT", "unknown_fn"]]);
    const signers = requiredSigners(tree, DEFAULT_POLICY, CONTEXT_POLICIES);
    expect(signers.threshold).toBe(DEFAULT_POLICY.threshold);
  });

  it("uses the context override when a matching policy exists", () => {
    const tree = invocationTree([["CCONTRACT", "add_attester"]]);
    const signers = requiredSigners(tree, DEFAULT_POLICY, CONTEXT_POLICIES);
    expect(signers.threshold).toBe(1);
  });

  it("applies the strictest policy across multiple contexts", () => {
    const tree = invocationTree([
      ["CCONTRACT", "add_attester"],
      ["CCONTRACT", "upgrade"],
    ]);
    const signers = requiredSigners(tree, DEFAULT_POLICY, CONTEXT_POLICIES);
    expect(signers.threshold).toBe(STRICT_POLICY.threshold);
  });

  it("falls back to the strictest configured policy for unknown contexts", () => {
    const tree = invocationTree([["CCONTRACT", "mystery"]]);
    const signers = requiredSigners(tree, DEFAULT_POLICY, CONTEXT_POLICIES);
    expect(signers.threshold).toBe(STRICT_POLICY.threshold);
  });

  it("falls back to the strictest configured policy for contract-creation contexts", () => {
    const tree = xdr.SorobanAuthorizedInvocation.sorobanAuthorizedInvocation({
      function: xdr.SorobanAuthorizedFunction.sorobanAuthorizedFunctionTypeCreateContractHostFn(
        new xdr.CreateContractArgs({
          executable: xdr.ContractExecutable.contractExecutableWasm(Buffer.alloc(32)),
          contractIdPreimage: xdr.ContractIdPreimage.contractIdPreimageFromAddress(
            new xdr.ContractIdPreimageFromAddress({
              address: xdr.ScAddress.scAddressTypeAccount(
                xdr.PublicKey.publicKeyTypeEd25519(StrKey.decodeEd25519PublicKey("GA")),
              ),
              salt: Buffer.alloc(32),
            }),
          ),
        }),
      ),
      subInvocations: [],
    });
    const signers = requiredSigners(tree, DEFAULT_POLICY, CONTEXT_POLICIES);
    expect(signers.threshold).toBe(STRICT_POLICY.threshold);
  });
});

describe("ceremony files", () => {
  it("round-trips: create -> write -> read -> sign -> finalize", () => {
    const entry = fixtureEntry();
    const ceremony = createCeremony(entry, "CCONTRACT", NETWORK_PASSPHRASE, VALID_UNTIL_LEDGER);

    const roundTripped = readCeremony(writeCeremony(ceremony));
    expect(roundTripped).toEqual(ceremony);

    const [kp1, kp2] = randomKeypairs(2);
    const payload = Buffer.from(roundTripped.payloadHex, "hex");
    let withSigs = addSignatureToCeremony(roundTripped, kp1.publicKey(), kp1.sign(payload));
    withSigs = addSignatureToCeremony(withSigs, kp2.publicKey(), kp2.sign(payload));
    expect(withSigs.signatures).toHaveLength(2);

    const finalized = finalizeCeremony(withSigs);
    expect(decodeSignatureVec(finalized)).toHaveLength(2);
  });

  it("rejects a signature that does not verify against the ceremony payload", () => {
    const entry = fixtureEntry();
    const ceremony = createCeremony(entry, "CCONTRACT", NETWORK_PASSPHRASE, VALID_UNTIL_LEDGER);
    const [kp] = randomKeypairs(1);
    const wrongPayload = Buffer.alloc(32, 7);
    expect(() => addSignatureToCeremony(ceremony, kp.publicKey(), kp.sign(wrongPayload))).toThrow();
  });

  it("rejects a ceremony file with an unsupported version", () => {
    expect(() => readCeremony(JSON.stringify({ version: 2 }))).toThrow(/unsupported ceremony file version/);
  });
});
