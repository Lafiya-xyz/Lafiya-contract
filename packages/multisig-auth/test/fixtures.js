import { Address, Keypair, xdr } from "@stellar/stellar-sdk";
import { readFileSync } from "node:fs";
import { fileURLToPath } from "node:url";
import { dirname, resolve } from "node:path";

const __dirname = dirname(fileURLToPath(import.meta.url));

/**
 * Path to the canonical, cross-language signature-payload test vectors.
 * The same file is consumed by the Rust tests in `multisig-account`, so
 * both implementations are pinned to identical inputs/outputs.
 */
export const VECTORS_PATH = resolve(
  __dirname,
  "../../../vectors/multisig-auth.json",
);

/**
 * Load the canonical `vectors/multisig-auth.json` file.
 *
 * Returns the parsed object: `{ version, ed25519: [...], secp256r1: [...] }`.
 * Each vector carries `name`, `kind` ("positive" | "negative"), the raw
 * inputs, and the expected result (or expected failure reason).
 */
export function loadVectors() {
  const raw = readFileSync(VECTORS_PATH, "utf8");
  return JSON.parse(raw);
}

/**
 * Return the total number of vectors in the canonical file. Used by CI to
 * assert that the Rust and TypeScript suites consume the same count.
 */
export function vectorCount(vectors = loadVectors()) {
  return (vectors.ed25519?.length ?? 0) + (vectors.secp256r1?.length ?? 0);
}

function contractFnInvocation({ contractAddress, functionName, args = [] }) {
  return new xdr.SorobanAuthorizedInvocation({
    function: xdr.SorobanAuthorizedFunction.sorobanAuthorizedFunctionTypeContractFn(
      new xdr.InvokeContractArgs({
        contractAddress,
        functionName,
        args,
      }),
    ),
    subInvocations: [],
  });
}

function addressCredentials({ address, nonce, signature = xdr.ScVal.scvVec([]) }) {
  return new xdr.SorobanAddressCredentials({
    address,
    nonce: new xdr.Int64(nonce),
    signatureExpirationLedger: 0,
    signature,
  });
}

/**
 * Build a minimal, syntactically valid `SorobanAuthorizationEntry` for a
 * call to `multisig-account`'s own address (as its own `__check_auth`
 * signer), with `ADDRESS` credentials -- enough to exercise
 * `buildAuthPayload`/`attachSignatures` without a network.
 */
export function fixtureEntry({ nonce = 1 } = {}) {
  const accountKeypair = Keypair.random();
  const address = new Address(accountKeypair.publicKey()).toScAddress();
  const credentials = addressCredentials({ address, nonce });
  const invocation = contractFnInvocation({
    contractAddress: address,
    functionName: "add_attester",
  });
  return new xdr.SorobanAuthorizationEntry({
    credentials: xdr.SorobanCredentials.sorobanCredentialsAddress(credentials),
    rootInvocation: invocation,
  });
}

/**
 * Build an authorization entry whose root invocation carries multiple
 * sub-invocations, each targeting a distinct `(contract, function)` pair.
 * Used to exercise context-policy selection: the strictest policy across
 * all contexts must be required.
 *
 * `contexts` is an array of `{ contractAddress, functionName, args? }`.
 * `contractAddress` may be a `Keypair` (its public key is used), an
 * `Address`, or a raw `ScAddress`.
 */
export function multiContextEntry({ nonce = 1, contexts = [] } = {}) {
  const accountKeypair = Keypair.random();
  const address = new Address(accountKeypair.publicKey()).toScAddress();

  const subInvocations = contexts.map((ctx) =>
    contractFnInvocation({
      contractAddress: toScAddress(ctx.contractAddress),
      functionName: ctx.functionName,
      args: ctx.args ?? [],
    }),
  );

  const invocation = new xdr.SorobanAuthorizedInvocation({
    function: xdr.SorobanAuthorizedFunction.sorobanAuthorizedFunctionTypeContractFn(
      new xdr.InvokeContractArgs({
        contractAddress: address,
        functionName: "add_attester",
        args: [],
      }),
    ),
    subInvocations,
  });

  const credentials = addressCredentials({ address, nonce });
  return new xdr.SorobanAuthorizationEntry({
    credentials: xdr.SorobanCredentials.sorobanCredentialsAddress(credentials),
    rootInvocation: invocation,
  });
}

/**
 * Build an authorization entry whose root invocation is a
 * `CreateContractHostFn` (contract deployment without a constructor).
 * These contexts have no `(contract, function)` pair, so the account must
 * fall back to the strictest configured policy.
 */
export function createContractEntry({ nonce = 1, executable = null } = {}) {
  const accountKeypair = Keypair.random();
  const address = new Address(accountKeypair.publicKey()).toScAddress();

  const contractExecutable =
    executable ??
    xdr.ContractExecutable.contractExecutableWasm(
      xdr.Hash.fromXDR(Buffer.alloc(32, 0x11)),
    );

  const invocation = new xdr.SorobanAuthorizedInvocation({
    function:
      xdr.SorobanAuthorizedFunction.sorobanAuthorizedFunctionTypeCreateContractHostFn(
        new xdr.CreateContractArgs({
          contractIdPreimage:
            xdr.ContractIdPreimage.contractIdPreimageFromAddress(
              new xdr.ContractIdPreimageFromAddress({
                address,
                salt: xdr.Hash.fromXDR(Buffer.alloc(32, 0x22)),
              }),
            ),
          executable: contractExecutable,
        }),
      ),
    subInvocations: [],
  });

  const credentials = addressCredentials({ address, nonce });
  return new xdr.SorobanAuthorizationEntry({
    credentials: xdr.SorobanCredentials.sorobanCredentialsAddress(credentials),
    rootInvocation: invocation,
  });
}

/**
 * Build an authorization entry whose root invocation is a
 * `CreateContractWithCtorHostFn` (contract deployment with a constructor).
 * Like `createContractEntry`, this context has no `(contract, function)`
 * pair and must fall back to the strictest configured policy.
 */
export function createContractWithCtorEntry({
  nonce = 1,
  executable = null,
  constructorArgs = [],
} = {}) {
  const accountKeypair = Keypair.random();
  const address = new Address(accountKeypair.publicKey()).toScAddress();

  const contractExecutable =
    executable ??
    xdr.ContractExecutable.contractExecutableWasm(
      xdr.Hash.fromXDR(Buffer.alloc(32, 0x33)),
    );

  const invocation = new xdr.SorobanAuthorizedInvocation({
    function:
      xdr.SorobanAuthorizedFunction.sorobanAuthorizedFunctionTypeCreateContractV2HostFn(
        new xdr.CreateContractArgsV2({
          contractIdPreimage:
            xdr.ContractIdPreimage.contractIdPreimageFromAddress(
              new xdr.ContractIdPreimageFromAddress({
                address,
                salt: xdr.Hash.fromXDR(Buffer.alloc(32, 0x44)),
              }),
            ),
          executable: contractExecutable,
          constructorArgs,
        }),
      ),
    subInvocations: [],
  });

  const credentials = addressCredentials({ address, nonce });
  return new xdr.SorobanAuthorizationEntry({
    credentials: xdr.SorobanCredentials.sorobanCredentialsAddress(credentials),
    rootInvocation: invocation,
  });
}

/**
 * Normalize a contract address input into an `xdr.ScAddress`.
 * Accepts a `Keypair`, an `Address`, or an already-built `ScAddress`.
 */
export function toScAddress(input) {
  if (input instanceof xdr.ScAddress) {
    return input;
  }
  if (input instanceof Address) {
    return input.toScAddress();
  }
  if (input instanceof Keypair) {
    return new Address(input.publicKey()).toScAddress();
  }
  if (typeof input === "string") {
    return new Address(input).toScAddress();
  }
  throw new TypeError("Unsupported contract address input");
}

export function randomKeypairs(n) {
  return Array.from({ length: n }, () => Keypair.random());
}