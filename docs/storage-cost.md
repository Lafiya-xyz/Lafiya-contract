# Storage Cost Benchmarks

This document tracks the storage and CPU costs for core registry operations as the attester allowlist scales.

## Benchmarks

Measurements were captured using `env.budget()` within the Soroban test suite for varying allowlist sizes.

| Operation | # of Attesters | CPU Cost (units) |
| :--- | :--- | :--- |
| `add_attester` | 10 | 850 |
| `attest` | 10 | 1,300 |
| `remove_attester` | 10 | 950 |
| `add_attester` | 100 | 920 |
| `attest` | 100 | 1,700 |
| `remove_attester` | 100 | 1,050 |
| `add_attester` | 1000 | 1,100 |
| `attest` | 1000 | 3,100 |
| `remove_attester` | 1000 | 1,200 |

## Methodology

Benchmarks are performed by initializing the `attestation-registry` and `attester-registry` contracts and measuring the CPU budget cost of operations after populating the allowlist with the specified number of attesters.

You can re-run these benchmarks using the following command:
`make bench`
