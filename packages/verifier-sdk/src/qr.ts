/**
 * Lafiya card QR payload, version 1:
 *
 *   lafiya:v1?network=<name>&contract=<C... attestation registry>&record=<64 hex>[&attested=<unix seconds>]
 *
 * `network` and `contract` are *claims*: the verifier only proceeds if they
 * match its trust anchors. `attested` is the attestation timestamp the card
 * was issued with, used to detect a superseded card.
 */
export interface QrPayload {
  network: string;
  contractId: string;
  recordHash: string;
  attestedAt?: bigint;
}

export class QrPayloadError extends Error {
  constructor(message: string) {
    super(message);
    this.name = "QrPayloadError";
  }
}

const PREFIX = "lafiya:v1?";

export function parseQrPayload(text: string): QrPayload {
  const trimmed = text.trim();
  if (!trimmed.startsWith(PREFIX)) {
    throw new QrPayloadError("not a lafiya:v1 payload");
  }
  const params = new URLSearchParams(trimmed.slice(PREFIX.length));
  const network = params.get("network") ?? "";
  const contractId = params.get("contract") ?? "";
  const recordHash = (params.get("record") ?? "").toLowerCase();
  const attested = params.get("attested");

  if (!/^[a-z][a-z0-9_-]{0,31}$/.test(network)) {
    throw new QrPayloadError("missing or invalid network");
  }
  if (!/^C[A-Z2-7]{55}$/.test(contractId)) {
    throw new QrPayloadError("missing or invalid contract id");
  }
  if (!/^[0-9a-f]{64}$/.test(recordHash)) {
    throw new QrPayloadError("record must be a 32-byte hex hash");
  }
  if (attested !== null && !/^\d{1,20}$/.test(attested)) {
    throw new QrPayloadError("attested must be unix seconds");
  }
  return {
    network,
    contractId,
    recordHash,
    ...(attested !== null ? { attestedAt: BigInt(attested) } : {}),
  };
}

export function encodeQrPayload(payload: QrPayload): string {
  const params = new URLSearchParams({
    network: payload.network,
    contract: payload.contractId,
    record: payload.recordHash,
  });
  if (payload.attestedAt !== undefined) {
    params.set("attested", payload.attestedAt.toString());
  }
  return PREFIX + params.toString();
}
