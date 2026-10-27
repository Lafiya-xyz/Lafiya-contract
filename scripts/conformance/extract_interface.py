#!/usr/bin/env python3
"""Extract a normalized interface snapshot from a built contract .wasm file.

Wraps `stellar contract info interface --output json`, which reads the
`contractspecv0` custom section embedded in the compiled Wasm binary (the
actual artifact that gets deployed) rather than parsing Rust source. This is
what lets the conformance checks catch drift between what the contract
*source* implies and what the *build artifact* actually exposes -- e.g. a
merge that silently dropped a function body (see issue #103) would still
show up here because the spec is read back out of the compiled binary.

Output is normalized (docstrings stripped, entries sorted by kind+name) so
that snapshot diffs only fire on actual interface changes, not comment
edits or non-deterministic ordering.
"""

import json
import shutil
import subprocess
import sys
from pathlib import Path
from typing import Any

KIND_ORDER = {
    "function_v0": 0,
    "udt_error_enum_v0": 1,
    "event_v0": 2,
    "udt_struct_v0": 3,
    "udt_union_v0": 4,
    "udt_enum_v0": 5,
}

# Contracts whose interface snapshots feed the generated TypeScript bindings
# and the `packages/events` decoder spec. Kept in sync with the drift gate in
# CI so a new contract can't be added to one without the other.
BINDING_CONTRACTS = (
    "registry",
    "multisig",
    "incentive-pool",
)

# Event kinds that the `packages/events` generator knows how to decode.
EVENT_KINDS = ("event_v0",)


def _strip_docs(node: Any) -> Any:
    """Recursively drop `doc` fields so wording-only edits don't diff."""
    if isinstance(node, dict):
        return {k: _strip_docs(v) for k, v in node.items() if k != "doc"}
    if isinstance(node, list):
        return [_strip_docs(v) for v in node]
    return node


def _entry_name(kind: str, body: dict[str, Any]) -> str:
    if kind == "function_v0":
        return str(body["name"])
    return str(body.get("name", ""))


def _normalize_type_keys(node: Any) -> Any:
    """Recursively normalize type references so snapshots are stable.

    `stellar contract info interface` emits type references as either a bare
    string or a single-key object depending on the CLI version; collapse both
    shapes to a canonical `{"type": ...}` form so the drift gate doesn't fire
    on tooling upgrades.
    """
    if isinstance(node, dict):
        if len(node) == 1:
            ((key, value),) = node.items()
            if key in ("type", "udt", "option", "result", "vec", "map", "tuple"):
                return {key: _normalize_type_keys(value)}
        return {k: _normalize_type_keys(v) for k, v in node.items()}
    if isinstance(node, list):
        return [_normalize_type_keys(v) for v in node]
    return node


def extract_events(snapshot: list[dict[str, Any]]) -> list[dict[str, Any]]:
    """Filter an interface snapshot down to its event entries.

    This is the handoff point to `packages/events`: the generator consumes
    exactly this list, so every pool event that appears in the built wasm
    must survive this filter or the decoder tests will silently miss it.
    """
    return [entry for entry in snapshot if entry["kind"] in EVENT_KINDS]


def extract_views(snapshot: list[dict[str, Any]]) -> list[dict[str, Any]]:
    """Filter an interface snapshot down to its callable view functions."""
    return [entry for entry in snapshot if entry["kind"] == "function_v0"]


def require_stellar_cli() -> None:
    if shutil.which("stellar") is None:
        sys.exit(
            "error: `stellar` CLI not found on PATH.\n"
            "Install it (https://developers.stellar.org/docs/tools/cli/install-cli) "
            "-- the same tool used by `make bindings`."
        )


def extract(wasm_path: Path) -> list[dict[str, Any]]:
    """Return a normalized, deterministic interface snapshot for one contract.

    Raises FileNotFoundError if the wasm hasn't been built yet, and exits
    with a clear message if the wasm has no embedded spec at all (e.g. it
    isn't actually a Soroban contract build).
    """
    require_stellar_cli()
    if not wasm_path.exists():
        raise FileNotFoundError(f"{wasm_path} does not exist -- build it first, e.g. `make wasm`")

    proc = subprocess.run(
        [
            "stellar",
            "contract",
            "info",
            "interface",
            "--wasm",
            str(wasm_path),
            "--output",
            "json",
        ],
        capture_output=True,
        text=True,
    )
    if proc.returncode != 0:
        sys.exit(f"error: `stellar contract info interface` failed for {wasm_path}:\n{proc.stderr}")

    raw = json.loads(proc.stdout)
    entries: list[tuple[str, str, Any]] = []
    for entry in raw:
        ((kind, body),) = entry.items()
        normalized = _normalize_type_keys(_strip_docs(body))
        entries.append((kind, _entry_name(kind, body), normalized))

    entries.sort(key=lambda e: (KIND_ORDER.get(e[0], 99), e[1]))
    return [{"kind": k, "name": n, "spec": s} for k, n, s in entries]


def main() -> None:
    if len(sys.argv) not in (2, 3):
        sys.exit(f"usage: {sys.argv[0]} <path-to-wasm> [events|views]")

    snapshot = extract(Path(sys.argv[1]))
    if len(sys.argv) == 3:
        section = sys.argv[2]
        if section == "events":
            snapshot = extract_events(snapshot)
        elif section == "views":
            snapshot = extract_views(snapshot)
        else:
            sys.exit(f"error: unknown section {section!r} (expected 'events' or 'views')")

    json.dump(snapshot, sys.stdout, indent=2, sort_keys=True)
    sys.stdout.write("\n")


if __name__ == "__main__":
    main()
