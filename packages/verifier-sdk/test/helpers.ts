import type { AttesterState, ChainReader, OnChainAttestation } from "../src/index.js";
import type { NetworkTrust, TrustAnchors } from "../src/index.js";

export const REGISTRY = "C" + "A".repeat(55);
export const ATTESTER_REGISTRY = "C" + "B".repeat(55);
export const OTHER_CONTRACT = "C" + "D".repeat(55);
export const CHW = "G" + "C".repeat(55);
export const NOW = new Date("2026-09-25T00:00:00Z");
export const NOW_S = BigInt(NOW.getTime() / 1000);

export const TRUST: TrustAnchors = {
  testnet: {
    networkPassphrase: "Test SDF Network ; September 2015",
    rpcUrl: "https://rpc.invalid",
    attestationRegistry: REGISTRY,
    attesterRegistry: ATTESTER_REGISTRY,
  },
};

export interface FakeState {
  history?: OnChainAttestation[];
  attester?: AttesterState | null;
  revoked?: boolean;
  fail?: boolean;
}

/** In-memory ChainReader standing in for RPC. */
export function fakeChain(state: FakeState): ChainReader & { calls: string[] } {
  const calls: string[] = [];
  const guard = (name: string) => {
    calls.push(name);
    if (state.fail) throw new Error("rpc down");
  };
  return {
    source: "rpc",
    calls,
    async getAttestation(_t: NetworkTrust) {
      guard("getAttestation");
      return state.history?.at(-1) ?? null;
    },
    async getAttestationHistory() {
      guard("getAttestationHistory");
      return state.history ?? [];
    },
    async getAttesterStatus() {
      guard("getAttesterStatus");
      return state.attester === undefined ? { suspended: false } : state.attester;
    },
    async isRevoked() {
      guard("isRevoked");
      return state.revoked ?? false;
    },
  };
}
