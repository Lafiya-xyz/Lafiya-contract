import { Client as AttestationClient } from "@lafiya/attestation-registry";
import { Client as AttesterClient } from "@lafiya/attester-registry";
import { Buffer } from "buffer";
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
    const tx = await client.get_attestation({ record_hash: Buffer.from(recordHash, "hex") });
    return tx.result ? toAttestation(tx.result) : null;
  }

  async getAttestationHistory(trust: NetworkTrust, recordHash: string) {
    const client = new AttestationClient(this.clientOptions(trust, trust.attestationRegistry));
    const tx = await client.get_attestation_history({
      record_hash: Buffer.from(recordHash, "hex"),
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

function toAttestation(a: { attester: string; timestamp: bigint | number }): OnChainAttestation {
  return { attester: a.attester, timestamp: BigInt(a.timestamp) };
}
