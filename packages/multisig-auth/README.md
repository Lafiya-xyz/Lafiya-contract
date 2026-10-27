# @lafiya/multisig-auth

Browser/Node helper for authorizing a Soroban call against the
[`multisig-account`](../../contracts/multisig-account) contract, without
hand-encoding `ScVal`s or hand-sorting public keys.

## Why

`multisig-account`'s `__check_auth` requires a `Vec<Signature>` (each
`{ public_key, signature }`) **strictly ordered by ascending public key**; any
other order fails with `BadSignatureOrder`
(`contracts/multisig-account/src/lib.rs`). A browser-based admin console (a
governance dashboard where signers approve with passkeys or browser wallets)
has no generated-bindings-level help for this, since `multisig-account` is an
account contract, not one your app calls directly -- see
[`bindings/multisig-account`](../../bindings/multisig-account) for the
generated client, which covers the contract's data type, not this
authorization-building step.

## API

```ts
import { buildAuthPayload, attachSignatures } from "@lafiya/multisig-auth";

// entry: an unsigned xdr.SorobanAuthorizationEntry for the call you want to
// authorize (e.g. from an AssembledTransaction's `.needsNonInvokerSigningBy()`
// flow, same as you'd pass to the SDK's own `authorizeEntry`).
const payload = buildAuthPayload(entry, networkPassphrase, validUntilLedger);

// Each signer signs `payload` with their ed25519 key out of band (passkey,
// browser wallet, hardware key, or `stellar keys sign`) and you collect:
const signed = attachSignatures(entry, [
  { publicKey: "GABC...", signature: sigBytesFromAlice },
  { publicKey: "GXYZ...", signature: sigBytesFromBob },
]);
// `signed` now carries a Vec<Signature> sorted and deduplicated by public
// key -- ready to include in the transaction's `auth` list.
```

`attachSignatures` sorts by the signers' **raw** ed25519 public key bytes
(matching the byte-wise `BytesN<32>` comparison `__check_auth` performs, not
string order on the `G...` address) and drops exact duplicates, so callers
never have to get the ordering right themselves.

## Ceremony files

A signing ceremony collects N-of-M signatures over one payload, often across
multiple sessions or people. `createCeremony`/`readCeremony`/
`writeCeremony`/`addSignatureToCeremony`/`finalizeCeremony` share one JSON
format (`CeremonyFile`, `version: 1`) so a browser signer and a CLI-based
signer can take part in the same ceremony by passing the same file back and
forth:

```ts
import {
  createCeremony,
  readCeremony,
  writeCeremony,
  addSignatureToCeremony,
  finalizeCeremony,
} from "@lafiya/multisig-auth";

// Start a ceremony (usually done once, by whoever assembles the call):
const ceremony = createCeremony(entry, contractId, networkPassphrase, validUntilLedger);
fs.writeFileSync("ceremony.json", writeCeremony(ceremony));

// Each signer, in turn:
const loaded = readCeremony(fs.readFileSync("ceremony.json", "utf8"));
const mySignature = /* sign Buffer.from(loaded.payloadHex, "hex") out of band */;
const updated = addSignatureToCeremony(loaded, myPublicKey, mySignature);
fs.writeFileSync("ceremony.json", writeCeremony(updated));

// Once threshold is met:
const signedEntry = finalizeCeremony(updated);
```

`addSignatureToCeremony` verifies each signature against the ceremony's own
recorded payload hash before accepting it, so a signature collected against a
stale payload (e.g. after `validUntilLedger` changed) is rejected rather than
silently merged in.

## Air-gapped signing

When signers' keys live on a machine that never touches the network, move the
ceremony across the air gap as files or animated QR instead of signing on the
submitting host. `lafiya-cli` drives this in three steps:

```sh
# 1. Online host: build a self-describing envelope for the call.
lafiya-cli tx prepare --contract <CONTRACT_ID> --network testnet \
  --valid-until-ledger <LEDGER> --out envelope.json

# 2. Offline host (no network): verify, then sign.
lafiya-cli tx verify --envelope envelope.json        # re-decode + integrity check
lafiya-cli tx sign --offline --envelope envelope.json \
  --key <SIGNER_SECRET> --out signature-bundle.json

# 3. Online host: assemble the bundle(s) and submit.
lafiya-cli tx submit --envelope envelope.json --signatures signature-bundle.json
```

The envelope is self-describing: it records the network passphrase, contract
id, `validUntilLedger`, the unsigned XDR, and a human-readable decode of the
call so the offline signer can read exactly what they are approving. It also
carries an integrity hash over those fields.

**Verify before signing.** Always run `tx verify` on the offline device first.
It re-decodes the envelope and recomputes the integrity hash, so any tampering
in transit (edited XDR, swapped contract id, changed expiry, altered network
passphrase) is reported and signing is refused. Only sign an envelope whose
decoded contents match what the ceremony coordinator announced out of band.

`tx sign --offline` never contacts the network; it emits a detached
`signature-bundle.json` (signer public key, signature, and the envelope hash it
was produced against). `tx submit` checks each bundle's envelope hash against
the envelope before assembling the sorted `Vec<Signature>` and sending, so a
bundle signed against a different or stale envelope is rejected rather than
merged in -- the same guarantee `addSignatureToCeremony` gives for the JSON
ceremony format.

### QR transport

For payloads too large for a single QR code, `tx prepare` and `tx sign
--offline` can emit animated multi-frame QR sequences:

```sh
lafiya-cli tx prepare ... --qr-out envelope-qr/        # one PNG per frame
lafiya-cli tx sign --offline ... --qr-out bundle-qr/
```

Each frame carries its index and total count plus a hash of the full payload;
the receiving side reassembles the frames and verifies the hash before use, so
a dropped or reordered frame is detected. Scan the frames into a file on the
other side of the air gap, then feed that file to `tx verify`/`tx submit` as
above.

## Testing

`test/index.test.js` covers signature ordering/deduplication, the
struct/vec ScVal shape `attachSignatures` produces, and a full
create → sign → finalize ceremony round trip, all against a locally
constructed `SorobanAuthorizationEntry` (no network required).

**Not included** in this change: a live 2-of-3 browser-to-chain integration
test against a running `stellar-cli` quickstart network, and a payload-hash
comparison against the Rust CLI for the same entry (the CLI has no
`__check_auth`-authorizing command yet to compare against). Both are called
out as open follow-ups in the tracking issue.
