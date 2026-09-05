#!/usr/bin/env python3
"""Generate (or check) `docs/events.md`, the canonical event schema
reference, from each contract's built-Wasm interface.

    python3 scripts/conformance/gen_events_doc.py            # write docs/events.md
    python3 scripts/conformance/gen_events_doc.py --check     # diff, don't write

`docs/architecture/event-indexing.md` names the events consumers should
expect but does not pin their topic/data shape; this file is the
machine-generated source of truth for that shape, meant to be regenerated
(`make conformance-update`) whenever a contract's events change, and to fail
CI (`--check`) if a change lands without the doc being regenerated.
"""
import sys
from pathlib import Path

from contracts import CONTRACTS, REPO_ROOT
from extract_interface import extract
from spec_types import render_type

EVENTS_MD = REPO_ROOT / "docs" / "events.md"

HEADER = """# Contract Event Schemas

**Generated file -- do not hand-edit.** Regenerate with:

```bash
make conformance-update
```

This reference is produced directly from the `contractspecv0` section of
each contract's built Wasm (`stellar contract info interface`), so it
reflects what the deployed artifact actually emits, not just what the Rust
source declares. See [`docs/architecture/event-indexing.md`](architecture/event-indexing.md)
for how these events are consumed.
"""


def render_contract(name, cfg):
    lines = [f"## `{name}`\n"]
    events = [e for e in extract(cfg["wasm_path"]) if e["kind"] == "event_v0"]
    if not events:
        lines.append("_No events declared._\n")
        return "\n".join(lines)

    for entry in events:
        spec = entry["spec"]
        lines.append(f"### `{spec['name']}`\n")
        topics = ", ".join(f"`{t}`" for t in spec["prefix_topics"])
        lines.append(f"- **Prefix topics:** {topics or '_none_'}")
        lines.append(f"- **Data format:** `{spec['data_format']}`")
        lines.append("")
        lines.append("| Field | Type | Location |")
        lines.append("|---|---|---|")
        for p in spec["params"]:
            lines.append(f"| `{p['name']}` | `{render_type(p['type_'])}` | {p['location']} |")
        lines.append("")
    return "\n".join(lines)


def render_doc():
    parts = [HEADER]
    for name in sorted(CONTRACTS):
        parts.append(render_contract(name, CONTRACTS[name]))
    return "\n".join(parts).rstrip() + "\n"


def main():
    check = "--check" in sys.argv[1:]
    rendered = render_doc()

    if check:
        if not EVENTS_MD.exists() or EVENTS_MD.read_text() != rendered:
            print(f"[events-doc] {EVENTS_MD} is stale -- run `make conformance-update`")
            sys.exit(1)
        print(f"[events-doc] OK -- {EVENTS_MD.name} matches the built contracts")
        return

    EVENTS_MD.write_text(rendered)
    print(f"[events-doc] wrote {EVENTS_MD}")


if __name__ == "__main__":
    main()
