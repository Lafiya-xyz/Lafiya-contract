"""Fail if README.md's `soroban-sdk` version drifts from Cargo.toml.

Cargo.toml's `[workspace.dependencies]` pin is the source of truth. Every
`soroban-sdk <major>.x` mention in README.md must match its major version.
"""
import pathlib
import re
import sys

ROOT = pathlib.Path(__file__).resolve().parents[2]


def main():
    cargo = (ROOT / "Cargo.toml").read_text(encoding="utf-8")
    m = re.search(r'^soroban-sdk\s*=\s*(?:\{[^}]*version\s*=\s*)?"=?\^?~?(\d+)\.', cargo, re.M)
    if not m:
        print("error: could not find the soroban-sdk pin in Cargo.toml")
        return 1
    pinned = m.group(1)

    readme = (ROOT / "README.md").read_text(encoding="utf-8")
    mentions = re.findall(r"`soroban-sdk`\s+(\d+)\.x", readme)
    if not mentions:
        print("error: README.md does not state the soroban-sdk version (expected `soroban-sdk` N.x)")
        return 1
    stale = [v for v in mentions if v != pinned]
    if stale:
        print(
            f"error: README.md mentions `soroban-sdk` {', '.join(v + '.x' for v in stale)} "
            f"but Cargo.toml pins {pinned}.x -- update README.md"
        )
        return 1
    print(f"OK -- README.md soroban-sdk version matches Cargo.toml ({pinned}.x)")
    return 0


if __name__ == "__main__":
    sys.exit(main())
