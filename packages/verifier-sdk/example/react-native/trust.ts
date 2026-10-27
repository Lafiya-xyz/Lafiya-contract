import type { TrustAnchors } from "@lafiya/verifier";

// Replace with values from the release manifest (docs/release-manifest).
export const TRUST: TrustAnchors = {
  testnet: {
    networkPassphrase: "Test SDF Network ; September 2015",
    rpcUrl: "https://soroban-testnet.stellar.org",
    attestationRegistry: "C...ATTESTATION_REGISTRY",
    attesterRegistry: "C...ATTESTER_REGISTRY",
  },
};
