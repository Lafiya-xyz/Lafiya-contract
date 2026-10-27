#!/usr/bin/env python3
"""Lint docs/adr: filename pattern, unique ids, and index completeness."""
import re
import sys
from collections import defaultdict
from pathlib import Path

ADR_DIR = Path(__file__).resolve().parent.parent / "docs" / "adr"
NAME = re.compile(r"^(\d{4})-[a-z0-9]+(-[a-z0-9]+)*\.md$")
REDIRECT = "<!-- adr-redirect:"


def check(adr_dir: Path = ADR_DIR) -> list[str]:
    errors = []
    ids = defaultdict(list)
    for path in sorted(adr_dir.glob("*.md")):
        if path.name == "README.md":
            continue
        m = NAME.match(path.name)
        if not m:
            errors.append(f"{path.name}: does not match NNNN-kebab-case.md")
            continue
        if path.read_text(encoding="utf-8").startswith(REDIRECT):
            continue
        ids[m.group(1)].append(path.name)
    for adr_id, names in ids.items():
        if len(names) > 1:
            errors.append(f"duplicate ADR id {adr_id}: {', '.join(names)}")
    index = (adr_dir / "README.md").read_text(encoding="utf-8")
    for adr_id, names in ids.items():
        if adr_id == "0000":
            continue
        for name in names:
            if f"]({name})" not in index:
                errors.append(f"{name}: missing from docs/adr/README.md index")
    return errors


if __name__ == "__main__":
    errs = check()
    for e in errs:
        print(f"error: {e}", file=sys.stderr)
    sys.exit(1 if errs else 0)
