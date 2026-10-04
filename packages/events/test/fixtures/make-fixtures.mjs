// Builds test/fixtures/events.json: one raw `getEvents` event (topic/value
// as base64 XDR, exactly as Soroban RPC returns them) per Lafiya event kind,
// plus the payload it must decode to. Re-run after regenerating decoders:
//   node test/fixtures/make-fixtures.mjs
// Replace entries with events captured from a local-quickstart run when
// one is available; the test format is the same.
import { Address, Keypair, StrKey, nativeToScVal, xdr } from "@stellar/stellar-sdk";
import { writeFileSync } from "node:fs";
import { EVENT_SPECS } from "../../dist/generated.js";

const CONTRACT = StrKey.encodeContract(Buffer.alloc(32, 9));
const accounts = [1, 2, 3].map((i) =>
  Keypair.fromRawEd25519Seed(Buffer.alloc(32, i)).publicKey(),
);

function sample(type, i) {
  if (type === "address") return [accounts[i % 3], (v) => Address.fromString(v).toScVal()];
  if (type === "u64") return ["1765900800", (v) => nativeToScVal(BigInt(v), { type: "u64" })];
  if (typeof type === "object" && "bytes_n" in type) {
    const hex = Buffer.alloc(type.bytes_n.n, 0xa0 + i).toString("hex");
    return [hex, (v) => xdr.ScVal.scvBytes(Buffer.from(v, "hex"))];
  }
  throw new Error(`no sample for ${JSON.stringify(type)}`);
}

const fixtures = Object.entries(EVENT_SPECS).map(([kind, [spec]], n) => {
  const expected = {};
  const topic = spec.prefixTopics.map((t) => xdr.ScVal.scvSymbol(t));
  const data = [];
  spec.fields.forEach((field, i) => {
    const [value, toScVal] = sample(field.type, i);
    expected[field.key] = value;
    if (field.location === "topic") topic.push(toScVal(value));
    else data.push(new xdr.ScMapEntry({ key: xdr.ScVal.scvSymbol(field.name), val: toScVal(value) }));
  });
  const ledger = 1000 + n;
  return {
    kind,
    raw: {
      id: `${(BigInt(ledger) << 32n).toString().padStart(19, "0")}-0000000000`,
      ledger,
      ledgerClosedAt: "2026-09-25T00:00:00Z",
      txHash: Buffer.alloc(32, n).toString("hex"),
      contractId: CONTRACT,
      topic: topic.map((t) => t.toXDR("base64")),
      value: xdr.ScVal.scvMap(data).toXDR("base64"),
    },
    expected,
  };
});

writeFileSync(new URL("./events.json", import.meta.url), JSON.stringify(fixtures, null, 2) + "\n");
