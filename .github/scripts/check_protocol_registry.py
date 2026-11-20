"""Fail if a domain-separation string or version constant is not registered.

`docs/specs/registry.md` is the single source of truth for every versioned
identifier in the protocol (LRC versions, interface versions, event schema
versions, domain-separation tags). This check scans the Rust sources for
domain-separation string literals and version constants and fails when one is
missing from the registry, so a new identifier cannot land unregistered.

Usage:
    python .github/scripts/check_protocol_registry.py
"""
import pathlib
import re
import sys

ROOT = pathlib.Path(__file__).resolve().parents[2]
REGISTRY = ROOT / "docs" / "specs" / "registry.md"

SCAN_DIRS = ("contracts", "crates")
SKIP_PARTS = {"target", "dist", "node_modules", "test", "tests"}

# Domain-separation string literals: `b"lafiya:..."` or `"lafiya...."`.
DOMAIN_RE = re.compile(r'b?"(lafiya[:.][A-Za-z0-9:._-]+)"')
# Version constants: `const NAME_VERSION: u8 = 0x01;` / `pub const VERSION_V1: u8 = 1;`
VERSION_RE = re.compile(
    r"^\s*(?:pub\s+)?const\s+([A-Z0-9_]*VERSION[A-Z0-9_]*)\s*:\s*[A-Za-z0-9]+\s*=\s*([^;]+);",
    re.M,
)


def rust_files():
    for base in SCAN_DIRS:
        for path in (ROOT / base).rglob("*.rs"):
            if SKIP_PARTS & set(path.parts):
                continue
            yield path


def main():
    if not REGISTRY.exists():
        print(f"error: {REGISTRY.relative_to(ROOT)} is missing")
        return 1
    registry = REGISTRY.read_text(encoding="utf-8")

    missing = []
    for path in rust_files():
        text = path.read_text(encoding="utf-8")
        rel = path.relative_to(ROOT)
        for literal in DOMAIN_RE.findall(text):
            if literal not in registry:
                missing.append(f"{rel}: domain-separation string `{literal}`")
        for name, value in VERSION_RE.findall(text):
            value = value.strip()
            if name not in registry:
                missing.append(f"{rel}: version constant `{name} = {value}`")

    if missing:
        print("error: unregistered protocol identifiers -- add them to docs/specs/registry.md:")
        for item in sorted(set(missing)):
            print(f"  - {item}")
        return 1
    print("OK -- all domain-separation strings and version constants are registered")
    return 0


if __name__ == "__main__":
    sys.exit(main())
