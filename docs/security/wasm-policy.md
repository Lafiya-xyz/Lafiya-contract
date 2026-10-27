# Wasm artifact policy

Size budgets say how big a contract is. They say nothing about what it can do.
The host functions a Soroban wasm imports are its whole capability surface. A
dependency bump can quietly add `update_current_contract_wasm` or a
cross-contract call, and nothing else in CI would flag it.
`scripts/check_wasm_policy.py` pins that surface for each contract.

## Rules

The check reads every built `.wasm` and fails if any of these hold:

| Rule | Why |
|---|---|
| Imports a host function that is not on the contract's allowlist in [`security/wasm-policy.json`](../../security/wasm-policy.json) | Widens the host surface. |
| Imports anything other than a function (memory, table, global) | A Soroban contract owns its memory. Any other import kind is unexpected. |
| Declares a start function | Code would run at instantiation, outside any entry point and its auth checks. |
| Has an `f32`/`f64`/`v128` type in a signature, local, global or block | Soroban rejects floats, which are non-deterministic across hosts. |
| Contains a float, SIMD (`0xFD`), atomics (`0xFE`), bulk-memory/`0xFC`, reference-type or unknown opcode | Only the MVP integer subset plus sign-extension is allowed. Anything else is a toolchain or dependency change worth a look. |
| Is larger than the contract's `max_size_bytes` | Coarse guard. Fee budgets live in `budgets/`. |

If an allowlisted import is no longer used, the check prints a note but does
not fail. Delete the entry to tighten the list.

Each rule has a negative fixture in `scripts/tests/test_check_wasm_policy.py`.
Each fixture is a minimal module that breaks only that one rule.

## Where it runs

- **CI**: the `Build (wasm32v1-none)` job. The per-contract diff is written to
  the job summary: `+` marks an import that is not allowlisted, `-` marks an
  allowlisted import that is no longer used, and `!` marks any other
  violation. Host functions are labeled by name, e.g.
  `l.6  ledger::update_current_contract_wasm`.
- **Release**: the tag pipeline runs the check on the reproducible build,
  after the provenance metadata assertion and before the release manifest is
  generated. A release cannot ship wasm that breaks the policy.

## Changing the allowlist

The allowlist and the checker are listed in `.github/CODEOWNERS`. With
"Require review from Code Owners" enabled on `main`, a PR cannot widen the
host surface without a code owner's approval.

To regenerate the allowlist from a build:

```bash
make wasm
python3 scripts/check_wasm_policy.py --update \
  --env-json "$(find ~/.cargo/registry/src -path '*soroban-env-common-*' -name env.json | sort -V | tail -1)" \
  target/wasm32v1-none/release/*.wasm
git diff security/wasm-policy.json
```

`--update` sets the allowlist to the imports the build actually uses. It sets
the size limit to the larger of the current limit and the measured size plus
10%. In the PR, explain every new import: which change needs it, and why the
contract should be allowed to call it.
