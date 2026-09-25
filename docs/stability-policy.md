# Contract Interface Stability Policy

## CI enforcement

The following rules are **enforced in CI** by the `conformance` job in
[`.github/workflows/ci.yml`](../.github/workflows/ci.yml), which builds the
contract Wasm and runs `make conformance` (see
[`scripts/conformance/README.md`](../scripts/conformance/README.md)). A pull
request fails CI if it changes a contract's public interface without the
matching updates:

| Rule | Enforced by |
| --- | --- |
| Function, error, event, and type changes update `scripts/conformance/snapshots/<contract>.json` | `check_snapshot.py` |
| Error enum changes update `docs/error-codes.md` | `check_error_docs.py` |
| Event schema changes regenerate `docs/events.md` | `gen_events_doc.py --check` |
| Function and error changes regenerate `bindings/<contract>` | `check_bindings_drift.py` |

After a deliberate interface change, run `make conformance-update` and
`make bindings`, then commit the results.
