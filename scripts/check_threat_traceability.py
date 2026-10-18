#!/usr/bin/env python3
"""Check that every modelled threat points at real code, tests, and a DFD.

Entries in security/threats.yaml that carry a `dfd` (the off-chain model,
docs/security/threat-model-offchain.md) must satisfy all of these:

  - required fields are present: STRIDE letters, boundary, owner, status;
  - `dfd` names an existing section (`path#anchor`) of the model, and the
    model lists the threat id;
  - every `code` entry (`path` or `path::symbol`) exists;
  - every `tests` entry (`path::name`) exists: a Rust `fn name`, or a
    TypeScript/Python test whose title or function name matches;
  - mitigated and partial threats point at code or tests;
  - partial and open threats name the `gap` and the `issue` tracking it;
  - accepted threats give a `rationale`.

Older entries without `dfd` predate the registry convention. They are listed
as warnings when their tests do not exist yet, but they do not fail the check.

Usage:
    scripts/check_threat_traceability.py [--registry PATH]
"""

import argparse
import re
import sys
from pathlib import Path
from typing import Any

import yaml

ROOT = Path(__file__).resolve().parents[1]
DEFAULT_REGISTRY = ROOT / "security" / "threats.yaml"
STATUSES = {"mitigated", "partial", "open", "accepted"}
REQUIRED = ("title", "component", "stride", "boundary", "mitigation", "owner", "status")


def anchors(markdown: str) -> set[str]:
    """GitHub-style heading anchors."""
    out = set()
    for line in markdown.splitlines():
        m = re.match(r"#{1,6}\s+(.*)", line)
        if m:
            slug = re.sub(r"[^\w\- ]", "", m.group(1).strip().lower()).replace(" ", "-")
            out.add(slug)
    return out


def symbol_exists(path: Path, symbol: str) -> bool:
    text = path.read_text(encoding="utf-8")
    if path.suffix == ".rs":
        return re.search(rf"\bfn\s+{re.escape(symbol)}\b", text) is not None or (
            re.search(rf"\b(const|static|struct|enum)\s+{re.escape(symbol)}\b", text) is not None
        )
    if path.suffix == ".py":
        return re.search(rf"\bdef\s+{re.escape(symbol)}\b", text) is not None
    # TypeScript: a test title, or an exported/declared name.
    return (
        f'"{symbol}"' in text
        or f"`{symbol}`" in text
        or re.search(rf"\b(function|class|const)\s+{re.escape(symbol)}\b", text) is not None
    )


def check_ref(root: Path, ref: str) -> str | None:
    path_s, _, symbol = ref.partition("::")
    path = root / path_s
    if not path.is_file():
        return f"{path_s} does not exist"
    if symbol and not symbol_exists(path, symbol):
        return f"{symbol!r} not found in {path_s}"
    return None


def check_entry(root: Path, tid: str, t: dict[str, Any]) -> list[str]:
    errors = [f"missing {key}" for key in REQUIRED if not t.get(key)]
    status = t.get("status")
    if status not in STATUSES:
        errors.append(f"status {status!r} not one of {sorted(STATUSES)}")
    letters = {s.strip() for s in str(t.get("stride", "")).split(",")}
    if not letters <= set("STRIDE"):
        errors.append(f"stride {t.get('stride')!r} is not STRIDE letters")

    doc_s, _, anchor = str(t["dfd"]).partition("#")
    doc = root / doc_s
    if not doc.is_file():
        errors.append(f"dfd {doc_s} does not exist")
    else:
        text = doc.read_text(encoding="utf-8")
        if anchor not in anchors(text):
            errors.append(f"dfd section #{anchor} not found in {doc_s}")
        if tid not in text:
            errors.append(f"{tid} is not listed in {doc_s}")

    refs = [*t.get("code", []), *t.get("tests", [])]
    errors += [e for ref in refs if (e := check_ref(root, ref))]
    for ref in t.get("tests", []):
        if "::" not in ref:
            errors.append(f"test {ref!r} must be `path::name`")
    if status in ("mitigated", "partial") and not refs:
        errors.append("a mitigation must point at code or tests")
    if status in ("partial", "open"):
        errors += [f"{status} threat needs `{key}`" for key in ("gap", "issue") if not t.get(key)]
    if status == "accepted" and not t.get("rationale"):
        errors.append("accepted threat needs a `rationale`")
    return errors


def check(registry: dict[str, Any], root: Path) -> tuple[list[str], list[str]]:
    errors: list[str] = []
    warnings: list[str] = []
    threats: dict[str, dict[str, Any]] = registry["threats"]
    rust_sources: str | None = None
    for tid, t in threats.items():
        if "dfd" in t:
            errors += [f"{tid}: {e}" for e in check_entry(root, tid, t)]
            continue
        if t.get("tests") and rust_sources is None:
            rust_sources = "\n".join(
                p.read_text(encoding="utf-8", errors="ignore")
                for p in root.glob("**/*.rs")
                if "target" not in p.parts
            )
        missing = [test for test in t.get("tests", []) if f"fn {test}" not in (rust_sources or "")]
        if missing:
            warnings.append(f"{tid}: {len(missing)} legacy test(s) not written yet")
    # Every threat id cited by an off-chain model must be in the registry.
    for doc in {str(t["dfd"]).partition("#")[0] for t in threats.values() if "dfd" in t}:
        path = root / doc
        if path.is_file():
            for tid in sorted(set(re.findall(r"\bT-\d{3}\b", path.read_text(encoding="utf-8")))):
                if tid not in threats:
                    errors.append(f"{doc} cites {tid}, which is not in the registry")
    return errors, warnings


def main(argv: list[str] | None = None) -> int:
    parser = argparse.ArgumentParser(description=__doc__.splitlines()[0])
    parser.add_argument("--registry", type=Path, default=DEFAULT_REGISTRY)
    parser.add_argument("--root", type=Path, default=ROOT)
    args = parser.parse_args(argv)

    registry = yaml.safe_load(args.registry.read_text(encoding="utf-8"))
    errors, warnings = check(registry, args.root)
    for w in warnings:
        print(f"warning: {w}")
    for e in errors:
        print(f"error: {e}")
    traced = sum(1 for t in registry["threats"].values() if "dfd" in t)
    if errors:
        print(f"{len(errors)} traceability error(s)")
        return 1
    print(f"threat traceability: OK ({traced} threats traced to code/tests)")
    return 0


if __name__ == "__main__":
    sys.exit(main())
