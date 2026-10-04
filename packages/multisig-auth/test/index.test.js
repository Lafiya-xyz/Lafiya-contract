import { describe, expect, it } from "vitest";
import { StrKey, xdr } from "@stellar/stellar-sdk";
import {
  addSignatureToCeremony,
  attachSignatures,
  buildAuthPayload,
  createCeremony,
  finalizeCeremony,
  readCeremony,
  writeCeremony,
} from "../dist/index.js";
import { fixtureEntry, randomKeypairs } from "./fixtures.js";

const NETWORK_PASSPHRASE = "Test SDF Network ; September 2015";
const VALID_UNTIL_LEDGER = 1000;

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
