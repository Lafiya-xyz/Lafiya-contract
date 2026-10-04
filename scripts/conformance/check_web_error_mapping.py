#!/usr/bin/env python3
"""Check the lafiya-web error-to-UX mapping against docs/error-codes.md.

    python3 scripts/conformance/check_web_error_mapping.py

Every error of the registries lafiya-web talks to must appear in
docs/integration/lafiya-web.md with the same code and variant name, and the
guide must not list errors that no longer exist. Pure docs check; needs no
built Wasm.
"""
import re
import sys
from pathlib import Path

REPO_ROOT = Path(__file__).resolve().parents[2]
ERROR_CODES_MD = REPO_ROOT / "docs" / "error-codes.md"
GUIDE_MD = REPO_ROOT / "docs" / "integration" / "lafiya-web.md"
CONTRACTS = ("attester-registry", "attestation-registry")


def documented_errors(markdown):
    errors = set()
    for contract in CONTRACTS:
        section = re.search(
            rf"^## `{re.escape(contract)}`\s*\n(.*?)(?=\n## |\Z)",
            markdown,
            re.DOTALL | re.MULTILINE,
        )
        if not section:
            sys.exit(f"error: section for `{contract}` missing from {ERROR_CODES_MD}")
        for code, name in re.findall(
            r"^\|\s*`(\d+)`\s*\|\s*`(\w+)`\s*\|", section.group(1), re.MULTILINE
        ):
            errors.add((contract, int(code), name))
    return errors


def guide_errors(markdown):
    return {
        (contract, int(code), name)
        for contract, code, name in re.findall(
            r"^\|\s*`([\w-]+)`\s*\|\s*`(\d+)`\s*\|\s*`(\w+)`\s*\|",
            markdown,
            re.MULTILINE,
        )
        if contract in CONTRACTS
    }


def main():
    docs = documented_errors(ERROR_CODES_MD.read_text())
    guide = guide_errors(GUIDE_MD.read_text())
    ok = True
    for contract, code, name in sorted(docs - guide):
        print(f"missing from lafiya-web guide: {contract} {code} {name}")
        ok = False
    for contract, code, name in sorted(guide - docs):
        print(f"stale in lafiya-web guide: {contract} {code} {name}")
        ok = False
    if ok:
        print(f"lafiya-web error mapping matches docs/error-codes.md ({len(docs)} errors)")
    return 0 if ok else 1


if __name__ == "__main__":
    sys.exit(main())
