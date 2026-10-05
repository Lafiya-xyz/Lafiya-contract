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
