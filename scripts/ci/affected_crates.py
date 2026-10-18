#!/usr/bin/env python3
"""Print the cargo package selection affected by a diff.

    python3 scripts/ci/affected_crates.py <base-ref>     # e.g. origin/main

Maps each file changed since the merge base with `<base-ref>` to the
workspace crate that owns it, then adds every workspace crate that depends
on one of those (transitively), so a change to a shared crate still tests
its dependents. Prints `-p crate-a -p crate-b`, or `--workspace` when a
change can affect every crate (workspace manifest, lockfile, toolchain,
nextest/CI config, or a file outside any crate such as root `tests/`).
"""

import json
import subprocess
import sys
from pathlib import Path

REPO_ROOT = Path(__file__).resolve().parents[2]
# Changes under these paths invalidate the whole workspace.
GLOBAL = (
    "Cargo.toml",
    "Cargo.lock",
    "rust-toolchain.toml",
    ".cargo/",
    ".config/",
    ".github/workflows/ci.yml",
    "tests/",
)
RUST_ROOTS = ("contracts/", "crates/", "xtask/")


def sh(*argv: str) -> str:
    return subprocess.run(argv, cwd=REPO_ROOT, capture_output=True, text=True, check=True).stdout


def workspace() -> tuple[dict[str, str], dict[str, set[str]]]:
    """Return ({crate: relative dir}, {crate: workspace crates it depends on})."""
    meta = json.loads(sh("cargo", "metadata", "--format-version", "1", "--no-deps"))
    dirs = {
        p["name"]: Path(p["manifest_path"]).parent.relative_to(REPO_ROOT).as_posix() + "/"
        for p in meta["packages"]
    }
    deps = {
        p["name"]: {d["name"] for d in p["dependencies"] if d["name"] in dirs}
        for p in meta["packages"]
    }
    return dirs, deps


def select(changed: list[str], dirs: dict[str, str], deps: dict[str, set[str]]) -> str:
    affected: set[str] = set()
    for path in changed:
        if path.startswith(GLOBAL):
            return "--workspace"
        owners = [name for name, d in dirs.items() if path.startswith(d)]
        if owners:
            affected.add(max(owners, key=lambda n: len(dirs[n])))
        elif path.startswith(RUST_ROOTS):
            return "--workspace"
    grew = True
    while grew:
        dependents = {n for n, ds in deps.items() if ds & affected} - affected
        affected |= dependents
        grew = bool(dependents)
    if not affected:
        return "--workspace"
    return " ".join(f"-p {name}" for name in sorted(affected))


def main() -> None:
    if len(sys.argv) != 2:
        sys.exit(__doc__)
    changed = sh("git", "diff", "--name-only", f"{sys.argv[1]}...HEAD").split()
    print(select(changed, *workspace()))


if __name__ == "__main__":
    main()
