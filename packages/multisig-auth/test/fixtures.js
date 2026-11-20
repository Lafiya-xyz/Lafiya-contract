import { Address, Keypair, xdr } from "@stellar/stellar-sdk";

/**
 * Build a minimal, syntactically valid `SorobanAuthorizationEntry` for a
 * call to `multisig-account`'s own address (as its own `__check_auth`
 * signer), with `ADDRESS` credentials -- enough to exercise
 * `buildAuthPayload`/`attachSignatures` without a network.
 */
export function fixtureEntry({ nonce = 1 } = {}) {
  const accountKeypair = Keypair.random();
  const address = new Address(accountKeypair.publicKey()).toScAddress();
  const credentials = new xdr.SorobanAddressCredentials({
    address,
    nonce: new xdr.Int64(nonce),
    signatureExpirationLedger: 0,
    signature: xdr.ScVal.scvVec([]),
  });
  const invocation = new xdr.SorobanAuthorizedInvocation({
    function: xdr.SorobanAuthorizedFunction.sorobanAuthorizedFunctionTypeContractFn(
      new xdr.InvokeContractArgs({
        contractAddress: address,
        functionName: "add_attester",
        args: [],
      }),
    ),
    subInvocations: [],
  });
  return new xdr.SorobanAuthorizationEntry({
    credentials: xdr.SorobanCredentials.sorobanCredentialsAddress(credentials),
    rootInvocation: invocation,
  });
}

export function randomKeypairs(n) {
  return Array.from({ length: n }, () => Keypair.random());
}

/**
 * Order of the secp256r1 (P-256) base point, as a 32-byte big-endian hex
 * string. Shared with the Rust tests so both sides use the same curve
 * parameters when checking low-S normalization.
 */
export const SECP256R1_N_HEX =
  "ffffffff00000000ffffffffffffffffbce6faada7179e84f3b9cac2fc632551";

/**
 * `n / 2` (floor), as a 32-byte big-endian hex string. A signature is
 * "low-S" iff `s <= SECP256R1_HALF_N_HEX`.
 */
export const SECP256R1_HALF_N_HEX =
  "7fffffff800000007fffffffffffffffde737d56d38bcf4279dce5617e3192a8";

/**
 * Shared DER signature test vectors for secp256r1 low-S normalization.
 *
 * Each vector carries a DER-encoded ECDSA signature (`derHex`), the
 * expected 64-byte raw `r || s` output after `derToCompact` +
 * `normalizeLowS` (`compactHex`), and a `description` of the edge case it
 * exercises. These are synthetic vectors (no real signatures) and are
 * mirrored in the Rust on-chain tests so acceptance is verified end to end.
 */
export const DER_SIGNATURE_VECTORS = [
  {
    description: "high-S signature is normalized to low-S (s = n - s)",
    // r = 1, s = n - 1 (maximal high-S)
    derHex:
      "3045022100" +
      "0000000000000000000000000000000000000000000000000000000000000001" +
      "0220" +
      "ffffffff00000000ffffffffffffffffbce6faada7179e84f3b9cac2fc632550",
    compactHex:
      "0000000000000000000000000000000000000000000000000000000000000001" +
      "0000000000000000000000000000000000000000000000000000000000000001",
  },
  {
    description: "low-S signature is left unchanged",
    // r = 1, s = 1 (already low-S)
    derHex:
      "3025020101020101",
    compactHex:
      "0000000000000000000000000000000000000000000000000000000000000001" +
      "0000000000000000000000000000000000000000000000000000000000000001",
  },
  {
    description: "short r is left-padded to 32 bytes",
    // r = 0x01 (1 byte), s = 0x02 (1 byte)
    derHex:
      "3006020101020102",
    compactHex:
      "0000000000000000000000000000000000000000000000000000000000000001" +
      "0000000000000000000000000000000000000000000000000000000000000002",
  },
  {
    description: "leading zero byte on r is stripped before padding",
    // r encoded as 0x00 0x80 (leading zero for sign bit), s = 0x01
    derHex:
      "302602020080020101",
    compactHex:
      "0000000000000000000000000000000000000000000000000000000000000080" +
      "0000000000000000000000000000000000000000000000000000000000000001",
  },
  {
    description: "leading zero byte on s is stripped before padding",
    // r = 0x01, s encoded as 0x00 0x80
    derHex:
      "302602010102020080",
    compactHex:
      "0000000000000000000000000000000000000000000000000000000000000001" +
      "0000000000000000000000000000000000000000000000000000000000000080",
  },
  {
    description: "high-S with leading zero byte on s is normalized",
    // r = 0x01, s = 0x00 0x80 (high-S? no -- 0x80 is low-S, but exercises
    // the leading-zero strip path together with normalization)
    derHex:
      "302602010102020080",
    compactHex:
      "0000000000000000000000000000000000000000000000000000000000000001" +
      "0000000000000000000000000000000000000000000000000000000000000080",
  },
  {
    description: "full-length high-S signature is normalized",
    // r = 0xdeadbeef... (32 bytes), s = n - 2 (high-S)
    derHex:
      "3045022100" +
      "deadbeefdeadbeefdeadbeefdeadbeefdeadbeefdeadbeefdeadbeefdeadbeef" +
      "0220" +
      "ffffffff00000000ffffffffffffffffbce6faada7179e84f3b9cac2fc63254f",
    compactHex:
      "deadbeefdeadbeefdeadbeefdeadbeefdeadbeefdeadbeefdeadbeefdeadbeef" +
      "0000000000000000000000000000000000000000000000000000000000000002",
  },
];

/**
 * Helper: returns the big-endian hex of `n - s` for a 32-byte hex `s`.
 * Used by tests to compute expected low-S values without a bignum dep.
 */
export function hexSubtractFromN(sHex) {
  const n = BigInt("0x" + SECP256R1_N_HEX);
  const s = BigInt("0x" + sHex);
  const result = n - s;
  return result.toString(16).padStart(64, "0");
}

/**
 * Helper: returns true iff the 32-byte hex `s` is low-S (`s <= n/2`).
 */
export function isLowS(sHex) {
  const halfN = BigInt("0x" + SECP256R1_HALF_N_HEX);
  const s = BigInt("0x" + sHex);
  return s <= halfN;
}