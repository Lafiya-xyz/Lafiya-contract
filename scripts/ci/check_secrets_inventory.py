#!/usr/bin/env python3
"""Fail if a workflow references a secret missing from docs/security/ci-secrets.md."""
import pathlib
import re
import sys

root = pathlib.Path(__file__).resolve().parents[2]
inventory = (root / "docs/security/ci-secrets.md").read_text()
used = {}
for wf in sorted((root / ".github/workflows").glob("*.yml")):
    for name in re.findall(r"secrets\.([A-Za-z0-9_]+)", wf.read_text()):
        used.setdefault(name, wf.name)
missing = [f"{n} (used in {w})" for n, w in used.items() if f"`{n}`" not in inventory]
if missing:
    sys.exit("secrets missing from docs/security/ci-secrets.md: " + ", ".join(missing))
print(f"ok: {len(used)} secrets inventoried")
