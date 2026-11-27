#!/usr/bin/env python3
"""Cross-check each contract's built-Wasm error enum against docs/error-codes.md.

    python3 scripts/conformance/check_error_docs.py [contract ...]

Catches the case a snapshot diff alone would not always make obvious at a
glance: a new `Error` variant added to the Rust enum but never added to the
docs table, a variant renamed in one place but not the other, or a code
value that has drifted out of sync between the contract and the docs a
`lafiya-web` or `lafiya-verifier` developer is reading.
"""

import re
import sys
from pathlib import Path
from typing import Any

from contracts import CONTRACTS, REPO_ROOT
from extract_interface import extract

ERROR_CODES_MD = REPO_ROOT / "docs" / "error-codes.md"


def parse_docs_table(markdown: str, contract_name: str) -> dict[int, str] | None:
    section = re.search(
        rf"^## `{re.escape(contract_name)}`\s*\n(.*?)(?=\n## |\Z)",
        markdown,
        re.DOTALL | re.MULTILINE,
    )
    if not section:
        return None
    # Rows may carry extra columns (e.g. a description or notes column) after
    # the code/name pair, so only anchor on the leading `code` | `name` cells.
    rows = re.findall(
        r"^\|\s*`(\d+)`\s*\|\s*`(\w+)`\s*\|",
        section.group(1),
        re.MULTILINE,
    )
    return {int(code): name for code, name in rows}


def wasm_error_cases(cfg: dict[str, Path]) -> dict[int, str]:
    for entry in extract(cfg["wasm_path"]):
        if entry["kind"] == "udt_error_enum_v0":
            cases: list[dict[str, Any]] = entry["spec"]["cases"]
            return {int(c["value"]): str(c["name"]) for c in cases}
    return {}


def check_one(name: str, cfg: dict[str, Path], markdown: str) -> bool:
    docs = parse_docs_table(markdown, name)
    wasm = wasm_error_cases(cfg)

    if docs is None:
        print(f"[{name}] no `## \\`{name}\\`` section found in {ERROR_CODES_MD.name}")
        return False

    ok = True
    for code, variant in sorted(wasm.items()):
        if code not in docs:
            print(f"[{name}] error {code} ({variant}) is in the Wasm but missing from docs")
            ok = False
        elif docs[code] != variant:
            print(
                f"[{name}] error {code} is `{variant}` in the Wasm but documented as `{docs[code]}`"
            )
            ok = False
    for code, variant in sorted(docs.items()):
        if code not in wasm:
            print(f"[{name}] error {code} ({variant}) is documented but no longer in the Wasm")
            ok = False

    if ok:
        print(f"[{name}] OK -- {len(wasm)} error code(s) match docs/error-codes.md")
    return ok


# Contracts whose error enums are extended by the multi-asset / push-disbursement
# / approval-void / identity-cap work (#695-#698). They must always be checked so
# a new variant cannot land without a matching docs row.
REQUIRED_CONTRACTS = ("incentive-pool", "attester-registry")


def main() -> None:
    names = sys.argv[1:] or list(CONTRACTS)
    unknown = [n for n in names if n not in CONTRACTS]
    if unknown:
        sys.exit(f"error: unknown contract(s): {', '.join(unknown)}")

    missing = [n for n in REQUIRED_CONTRACTS if n not in CONTRACTS]
    if missing:
        sys.exit(f"error: required contract(s) not configured: {', '.join(missing)}")

    # Always include the contracts touched by the advanced incentive-pool work,
    # even when a caller passes an explicit subset, so their error variants are
    # never silently skipped.
    for name in REQUIRED_CONTRACTS:
        if name not in names:
            names.append(name)

    markdown = ERROR_CODES_MD.read_text()
    ok = all(check_one(name, CONTRACTS[name], markdown) for name in names)
    sys.exit(0 if ok else 1)


if __name__ == "__main__":
    main()
