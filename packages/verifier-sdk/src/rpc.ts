import { Client as AttestationClient } from "@lafiya/attestation-registry";
import { Client as AttesterClient } from "@lafiya/attester-registry";
import type { AttesterState, ChainReader, OnChainAttestation } from "./chain.js";
import type { NetworkTrust } from "./trust.js";

/** Reads contract state over Soroban RPC through the generated bindings. */
export class BindingsChainReader implements ChainReader {
  readonly source = "rpc" as const;

  constructor(private readonly options: { allowHttp?: boolean } = {}) {}

  private clientOptions(trust: NetworkTrust, contractId: string) {
    return {
      contractId,
      networkPassphrase: trust.networkPassphrase,
      rpcUrl: trust.rpcUrl,
      allowHttp: this.options.allowHttp,
    };
  }

  async getAttestation(trust: NetworkTrust, recordHash: string) {
    const client = new AttestationClient(this.clientOptions(trust, trust.attestationRegistry));
    const tx = await client.get_attestation({ record_hash: recordHashArg(recordHash) });
    return tx.result ? toAttestation(tx.result) : null;
  }

  async getAttestationHistory(trust: NetworkTrust, recordHash: string) {
    const client = new AttestationClient(this.clientOptions(trust, trust.attestationRegistry));
    const tx = await client.get_attestation_history({
      record_hash: recordHashArg(recordHash),
    });
    return tx.result.map(toAttestation);
  }

  async getAttesterStatus(trust: NetworkTrust, attester: string): Promise<AttesterState | null> {
    const client = new AttesterClient(this.clientOptions(trust, trust.attesterRegistry));
    const status = (await client.get_attester_status({ attester })).result;
    if (!status) return null;
    return {
      suspended: status.suspended,
      ...(status.info.region ? { region: String(status.info.region) } : {}),
    };
  }
}

type RecordHashArg = Parameters<AttestationClient["get_attestation"]>[0]["record_hash"];

/**
 * The bindings type `BytesN` arguments as `Buffer`, but the contract spec
 * encoder accepts any `Uint8Array`. Passing one keeps the `buffer` polyfill
 * out of this package's dependencies.
 */
function recordHashArg(hex: string): RecordHashArg {
  const bytes = new Uint8Array(hex.length / 2);
  for (let i = 0; i < bytes.length; i++) bytes[i] = parseInt(hex.slice(i * 2, i * 2 + 2), 16);
  return bytes as unknown as RecordHashArg;
}

function toAttestation(a: { attester: string; timestamp: bigint | number }): OnChainAttestation {
  return { attester: a.attester, timestamp: BigInt(a.timestamp) };
}
