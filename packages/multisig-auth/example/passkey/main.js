// Browser example for the passkey helpers in @lafiya/multisig-auth.
// Serve with any bundler that resolves bare imports, e.g. from the package
// directory: `npx vite example/passkey` (http://localhost:5173).
//
// The private key is created and kept inside the authenticator; this page
// only ever handles the public key, the challenge, and the assertion.
import { Buffer } from "buffer";
import { xdr } from "@stellar/stellar-sdk";
import {
  decodeIntent,
  parsePasskeyAssertion,
  passkeyChallenge,
  readCeremony,
  secp256r1PublicKeyFromSpki,
} from "../../dist/index.js";

const $ = (id) => document.getElementById(id);
const toHex = (bytes) => Buffer.from(bytes).toString("hex");
const rpId = window.location.hostname;
let ceremony = null;

$("register").onclick = async () => {
  const credential = await navigator.credentials.create({
    publicKey: {
      rp: { id: rpId, name: "Lafiya multisig" },
      user: { id: crypto.getRandomValues(new Uint8Array(16)), name: "signer", displayName: "Signer" },
      challenge: crypto.getRandomValues(new Uint8Array(32)),
      pubKeyCredParams: [{ type: "public-key", alg: -7 }], // ES256 = P-256
      authenticatorSelection: { residentKey: "preferred", userVerification: "required" },
    },
  });
  const spki = new Uint8Array(credential.response.getPublicKey());
  $("public-key").textContent = toHex(secp256r1PublicKeyFromSpki(spki));
};

$("load").onclick = () => {
  ceremony = readCeremony($("ceremony").value);
  const entry = xdr.SorobanAuthorizationEntry.fromXDR(ceremony.entryXdr, "base64");
  const intent = decodeIntent(entry, ceremony.validUntilLedger);
  $("intent").textContent = JSON.stringify(
    {
      network: ceremony.networkPassphrase,
      account: ceremony.contractId,
      validUntilLedger: intent.validUntilLedger,
      invocation: intent.invocation,
      payloadHex: ceremony.payloadHex,
    },
    null,
    2,
  );
  $("reviewed").checked = false;
  $("reviewed").disabled = false;
  $("sign").disabled = true;
};

$("reviewed").onchange = () => {
  $("sign").disabled = !$("reviewed").checked;
};

$("sign").onclick = async () => {
  const payload = Uint8Array.from(Buffer.from(ceremony.payloadHex, "hex"));
  const credential = await navigator.credentials.get({
    publicKey: { challenge: passkeyChallenge(payload), rpId, userVerification: "required" },
  });
  const parsed = parsePasskeyAssertion(
    {
      authenticatorData: new Uint8Array(credential.response.authenticatorData),
      clientDataJSON: new Uint8Array(credential.response.clientDataJSON),
      signature: new Uint8Array(credential.response.signature),
    },
    payload,
    { origin: window.location.origin, rpId, requireUserVerification: true },
  );
  $("result").textContent = JSON.stringify(
    {
      signature: toHex(parsed.signature),
      normalizedFromHighS: parsed.normalized,
      signedDigest: toHex(parsed.signedDigest),
      userVerified: parsed.userVerified,
    },
    null,
    2,
  );
};
