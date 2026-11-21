/**
 * Dispute signature v2 message (docs/specs/dispute-signature-v2.md),
 * byte-for-byte compatible with `attestation-registry::dispute_message_v2`
 * and checked against the shared test vectors. Uses WebCrypto.
 */
import { Address } from "@stellar/stellar-sdk";

export const DISPUTE_DOMAIN_V2 = "lafiya.attestation.dispute.v2";

export interface DisputeMessageInput {
  /** 32-byte network id: sha256 of the network passphrase. */
  networkId: Uint8Array;
  /** Registry contract strkey (`C...`). */
  contract: string;
  recordHash: Uint8Array;
  /** The `signer` argument as a strkey (`G...` or `C...`). */
  signer: string;
  /** 32-byte ed25519 public key that signs the message. */
  signerKey: Uint8Array;
  nonce: bigint;
  /** Unix seconds. */
  expiresAt: bigint;
}

function be64(value: bigint): Uint8Array {
  const out = new Uint8Array(8);
  new DataView(out.buffer).setBigUint64(0, value);
  return out;
}

/** Build the v2 preimage; sign `sha256(preimage)` (see {@link disputeMessageV2}). */
export function disputePreimageV2(input: DisputeMessageInput): Uint8Array {
  for (const [name, v] of [
    ["networkId", input.networkId],
    ["recordHash", input.recordHash],
    ["signerKey", input.signerKey],
  ] as const) {
    if (v.length !== 32) throw new RangeError(`${name} must be 32 bytes`);
  }
  const parts = [
    new TextEncoder().encode(DISPUTE_DOMAIN_V2),
    input.networkId,
    Address.fromString(input.contract).toScVal().toXDR(),
    input.recordHash,
    Address.fromString(input.signer).toScVal().toXDR(),
    input.signerKey,
    be64(input.nonce),
    be64(input.expiresAt),
  ];
  const out = new Uint8Array(parts.reduce((n, p) => n + p.length, 0));
  let offset = 0;
  for (const p of parts) {
    out.set(p, offset);
    offset += p.length;
  }
  return out;
}

/** The 32-byte message the patient's ed25519 key signs. */
export async function disputeMessageV2(input: DisputeMessageInput): Promise<Uint8Array> {
  const digest = await crypto.subtle.digest("SHA-256", disputePreimageV2(input));
  return new Uint8Array(digest);
}
