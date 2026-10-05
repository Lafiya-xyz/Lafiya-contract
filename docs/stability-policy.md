# Interface Stability and Deprecation Policy

This policy covers what external consumers (`lafiya-web`, indexers, verifier
SDKs, operators) can rely on across Lafiya contract releases, and how
contributors make a breaking change. [ADR-0010](adr/0010-release-manifest-and-compatibility.md)
defines the release manifest and the machinery for checking compatibility.
This document defines the **rules** that machinery enforces.

Lafiya is **pre-alpha** (`0.x`). Under SemVer, a `0.MINOR` bump may break
things. We still follow the deprecation windows below from now on so
consumers can build habits and tooling around them. From `1.0.0` they are a
hard commitment.

## Surfaces

| Surface | Versioning | Breaking change examples | Deprecation window | How signalled |
| --- | --- | --- | --- | --- |
| **Contract functions** (`#[contractimpl]`) | Contract `workspace_version` (SemVer, ADR-0010). A per-contract `interface_version` is planned in [#443](https://github.com/Lafiya-xyz/Lafiya-contract/issues/443) | Removing or renaming a function; changing argument order or types, return type, error set, or `require_auth` semantics | At least **1 minor release** that ships both the old and new function | `CHANGELOG.md`, `get_interface().features` (once #443 lands), release manifest |
| **Contract storage** (`DataKey`, `#[contracttype]`) | `SCHEMA_VERSION` per contract | Reordering, removing, or renumbering any `DataKey` variant or struct field. **Never allowed**, because it corrupts reads of existing data | n/a. Any schema change needs a migration (`migrate()`) | `SCHEMA_VERSION` bump, `CHANGELOG.md`, and a `Schema-Impact:` trailer on the commit that changes storage |
| **Events** | Event name plus positional `#[topic]` fields (ADR-0010 §2). A per-event `event_version` is planned | Removing an event; adding, removing, or reordering `#[topic]` fields; removing or retyping a data field | At least **2 minor releases** emitting the old shape (or both shapes) | [`events.md`](events.md), the event catalog ([#442](https://github.com/Lafiya-xyz/Lafiya-contract/issues/442)), and `events[].compatibility` in the release manifest |
| **Error codes** (`#[contracterror]`) | Per-contract enum, `u32` codes | Renumbering, reusing, or changing the meaning of an existing code. **Never allowed**. Add new codes at the end | Never. Retired codes stay reserved forever | [`error-codes.md`](error-codes.md) and the error catalog (#442) |
| **TypeScript bindings / SDK** (`bindings/*`) | npm SemVer (`package.json` `version`) | Any generated type or method change that follows from a breaking contract change; removing exports | Same as the contract change that caused it. The old major keeps working against old deployments | `CHANGELOG.md`, `npm deprecate` on superseded versions once packages are published ([PUBLISHING.md](../PUBLISHING.md)) |
| **CLI** (`lafiya-cli`, `scripts/*.sh`) | SemVer of the `lafiya-cli` crate | Removing or renaming a subcommand or flag; changing output formats that scripts parse | At least **1 minor release** with the old spelling still working and printing a deprecation warning to stderr | `--help` text, stderr warnings, `CHANGELOG.md` |
| **Commitment scheme** (LRC, [ADR-0008](adr/0008-record-commitment-canonicalization.md)) | Version byte inside the commitment preimage (`VERSION_V1`, ...) | Any change to canonicalization or encoding. This always produces a **new version**, never an edit to an existing one | Old versions remain verifiable by every verifier for at least **10 years** (the lifetime of an emergency health card) | ADR-0008 or its successor ADR, new test vectors, `CHANGELOG.md` |

Adding things is always allowed without a deprecation window: new functions,
new events, new trailing error codes, new appended `DataKey` variants, new
event data fields, new CLI subcommands, and new commitment versions. These
are minor releases under ADR-0010.

## Deprecation mechanics

**Decision: deprecation is documentation-only on-chain. Deprecated contract
functions do not emit a `Deprecated { fn_name }` event.**

- An event on every call to a deprecated function adds resource cost for
  every caller for the whole window, and the caller gets nothing from it. The
  people who need to know are the integrators, and they read the changelog,
  the manifest, and (once #443 lands) the interface view, not event streams.
- Deprecated functions keep working exactly as before until they are removed
  in the next allowed release.
- To deprecate a contract function, the PR must:
  1. add `/// **Deprecated since vX.Y:** use `new_fn` instead.` to its rustdoc, which flows into the contract spec and the generated bindings' JSDoc;
  2. add a `### Deprecated` entry to `CHANGELOG.md` naming the removal release;
  3. once #443 lands, remove its feature flag from `get_interface().features` in the release that removes it (not the one that deprecates it).
- CLI deprecations print `warning: <old> is deprecated and will be removed in vX.Y; use <new>` to stderr and keep a zero exit code.

## Making a breaking change

1. Check the table above. If the change is "never allowed" (renumbering
   errors, reordering storage), redesign it as an addition instead.
2. Ship the new surface alongside the old one, and deprecate the old one as
   described above.
3. Tick **Breaking change?** in the PR template and describe the impact and
   migration path.
4. After the deprecation window, remove the old surface in a release whose
   version bump follows ADR-0010 (major from `1.0.0`, minor while `0.x`).

## Enforcement

| Rule | Status |
| --- | --- |
| Function signature and error-set changes must update the golden spec snapshot | **Enforced locally** by `scripts/conformance/check_snapshot.py` (`make conformance`). **Not yet in CI:** [#448](https://github.com/Lafiya-xyz/Lafiya-contract/issues/448) |
| Error codes are never renumbered or reused | **Partly enforced**: `check_snapshot.py` flags any change to the error enum and `check_error_docs.py` flags code/doc mismatches (local, see #448). A strict "never reuse" check is part of the error catalog: [#442](https://github.com/Lafiya-xyz/Lafiya-contract/issues/442) |
| Event schema changes are documented and classified | **Enforced locally** by `gen_events_doc.py --check` (see #448). **Enforced on release** by the release manifest's `events[].compatibility` classification (`release-manifest.yml`, ADR-0010) |
| Events keep the old shape for 2 releases; `event_version` | **Not yet enforced**: [#442](https://github.com/Lafiya-xyz/Lafiya-contract/issues/442) |
| Function deprecation window; `interface_version` / `get_interface().features` | **Not yet enforced**: [#443](https://github.com/Lafiya-xyz/Lafiya-contract/issues/443) |
| Storage is append-only; `SCHEMA_VERSION` bump and `Schema-Impact` trailer | **Reviewer-enforced** (runbook and `DataKey` doc comments). Automated upgrade-from-every-release test: [#375](https://github.com/Lafiya-xyz/Lafiya-contract/issues/375) |
| Bindings match the contract spec | **Enforced locally** by `check_bindings_drift.py` (see #448). npm deprecation is not applicable until packages are published ([PUBLISHING.md](../PUBLISHING.md)) |
| CLI deprecation warnings and window | **Reviewer-enforced**. Consolidating the scripts into the CLI: [#402](https://github.com/Lafiya-xyz/Lafiya-contract/issues/402), full subcommand coverage: [#394](https://github.com/Lafiya-xyz/Lafiya-contract/issues/394) |
| Commitment encodings are only ever added as new versions | **Reviewer-enforced** under ADR-0008. Cross-implementation vectors in CI: [#387](https://github.com/Lafiya-xyz/Lafiya-contract/issues/387) |
| Consumers can check a release against their requirements | **Available** through `scripts/check_manifest_compatibility.py` (ADR-0010), which consumers run in their own CI |
