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

KIND_ORDER = {
    "function_v0": 0,
    "udt_error_enum_v0": 1,
    "event_v0": 2,
    "udt_struct_v0": 3,
    "udt_union_v0": 4,
    "udt_enum_v0": 5,
}


def _strip_docs(node):
    """Recursively drop `doc` fields so wording-only edits don't diff."""
    if isinstance(node, dict):
        return {k: _strip_docs(v) for k, v in node.items() if k != "doc"}
    if isinstance(node, list):
        return [_strip_docs(v) for v in node]
    return node


def _entry_name(kind, body):
    if kind == "function_v0":
        return body["name"]
    return body.get("name", "")


def require_stellar_cli():
    if shutil.which("stellar") is None:
        sys.exit(
            "error: `stellar` CLI not found on PATH.\n"
            "Install it (https://developers.stellar.org/docs/tools/cli/install-cli) "
            "-- the same tool used by `make bindings`."
        )


def extract(wasm_path):
    """Return a normalized, deterministic interface snapshot for one contract.

    Raises FileNotFoundError if the wasm hasn't been built yet, and exits
    with a clear message if the wasm has no embedded spec at all (e.g. it
    isn't actually a Soroban contract build).
    """
    require_stellar_cli()
    if not wasm_path.exists():
        raise FileNotFoundError(
            f"{wasm_path} does not exist -- build it first, e.g. `make wasm`"
        )

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
        sys.exit(
            f"error: `stellar contract info interface` failed for {wasm_path}:\n"
            f"{proc.stderr}"
        )

    raw = json.loads(proc.stdout)
    entries = []
    for entry in raw:
        ((kind, body),) = entry.items()
        entries.append((kind, _entry_name(kind, body), _strip_docs(body)))

    entries.sort(key=lambda e: (KIND_ORDER.get(e[0], 99), e[1]))
    return [{"kind": k, "name": n, "spec": s} for k, n, s in entries]


def main():
    if len(sys.argv) != 2:
        sys.exit(f"usage: {sys.argv[0]} <path-to-wasm>")
    from pathlib import Path

    snapshot = extract(Path(sys.argv[1]))
    json.dump(snapshot, sys.stdout, indent=2, sort_keys=True)
    sys.stdout.write("\n")


if __name__ == "__main__":
    main()
