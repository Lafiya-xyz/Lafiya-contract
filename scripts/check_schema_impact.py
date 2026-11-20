#!/usr/bin/env python3
"""Fail when a commit changes the contract storage schema without a
`Schema-Impact:` trailer.

A commit "touches the schema" when, in a non-test `.rs` file, a changed line
mentions `DataKey` or `contracttype`, or a changed hunk lies inside a
`#[contracttype]` item (found through git's built-in Rust diff driver).
Comment-only and blank-line changes are ignored.
Such commits must carry a trailer such as

    Schema-Impact: none -- adds a variant read with a default

so the release CHANGELOG can state the upgrade impact. See docs/releasing.md.

Usage:
    scripts/check_schema_impact.py [--repo DIR] REV_RANGE
    scripts/check_schema_impact.py --cli-schemas [--repo DIR] BASE HEAD

The `--cli-schemas` mode compares the JSON schemas published under
`crates/lafiya-cli/schemas/` between BASE and HEAD. Removing a field, changing
a field's type, or adding a new required field is additive-breaking and fails
unless the schema's `$id` (which carries the contract version) is bumped. See
docs/cli-json-schema.md.
"""
import argparse
import json
import re
import subprocess
import sys
import tempfile
from dataclasses import dataclass, field
from pathlib import Path

SCHEMA_WORDS = re.compile(r"\b(DataKey|contracttype)\b")
CONTRACTTYPE_ITEM = re.compile(
    r"#\[contracttype\][^\n]*\n(?:\s*#\[[^\n]*\n|\s*///[^\n]*\n)*\s*pub\s+(?:struct|enum)\s+(\w+)"
)
HUNK = re.compile(r"^@@ [^@]* @@ ?(.*)$")
ITEM_NAME = re.compile(r"\b(?:struct|enum|impl)\s+(\w+)")
TEST_PATH = re.compile(r"(^|/)(tests?|benches)(/|\.rs$)|_test\.rs$")
CLI_SCHEMA_DIR = "crates/lafiya-cli/schemas"
RELEASE_MANIFEST = "release-manifest.json"
SCHEMA_VERSION_FILE = "crates/lafiya-contract/src/lib.rs"
SCHEMA_VERSION_RE = re.compile(r"pub\s+const\s+SCHEMA_VERSION\s*:\s*u32\s*=\s*(\d+)")
MIGRATION_DIR = "crates/lafiya-contract/src/migrations"
JSON_TYPE_NAMES = {
    "object": "object",
    "array": "array",
    "string": "string",
    "number": "number",
    "integer": "integer",
    "boolean": "boolean",
    "null": "null",
}


def git(repo: Path, *args: str) -> str:
    return subprocess.run(
        ["git", "-C", str(repo), *args], check=True, capture_output=True, text=True
    ).stdout


def contracttype_names(source: str) -> set[str]:
    return set(CONTRACTTYPE_ITEM.findall(source))


def touches_schema(diff: str, names_by_file: dict[str, set[str]]) -> bool:
    """`diff` is `git show -U0` output for one commit, restricted to .rs files.
    Comment-only and blank-line changes never count."""
    path, in_item = None, False
    for line in diff.splitlines():
        if line.startswith("+++ "):
            path = line[6:] if line.startswith("+++ b/") else None
        elif line.startswith("--- "):
            continue
        elif path is None or TEST_PATH.search(path):
            continue
        elif m := HUNK.match(line):
            item = ITEM_NAME.search(m.group(1))
            in_item = bool(item and item.group(1) in names_by_file.get(path, set()))
        elif line[:1] in "+-":
            code = line[1:].strip()
            if not code or code.startswith("//"):
                continue
            if in_item or SCHEMA_WORDS.search(code):
                return True
    return False


def has_trailer(repo: Path, sha: str) -> bool:
    value = git(repo, "log", "-1", "--format=%(trailers:key=Schema-Impact,valueonly)", sha)
    return bool(value.strip())


def offending_commits(repo: Path, rev_range: str) -> list[str]:
    bad = []
    with tempfile.NamedTemporaryFile("w", suffix=".gitattributes", delete=False) as attrs:
        attrs.write("*.rs diff=rust\n")
    try:
        for sha in git(repo, "rev-list", "--no-merges", rev_range).split():
            diff = git(
                repo, "-c", f"core.attributesFile={attrs.name}",
                "show", "--format=", "-U0", sha, "--", "*.rs",
            )
            names = {}
            for path in re.findall(r"^\+\+\+ b/(.+)$", diff, re.M):
                try:
                    names[path] = contracttype_names(git(repo, "show", f"{sha}:{path}"))
                except subprocess.CalledProcessError:
                    names[path] = set()
            if touches_schema(diff, names) and not has_trailer(repo, sha):
                bad.append(sha)
    finally:
        Path(attrs.name).unlink()
    return bad


def schema_version(schema: dict) -> str | None:
    """The contract version is the trailing path segment of `$id`."""
    schema_id = schema.get("$id")
    if not isinstance(schema_id, str) or not schema_id:
        return None
    return schema_id.rstrip("/").rsplit("/", 1)[-1]


def _type_names(schema: dict) -> set[str]:
    declared = schema.get("type")
    if declared is None:
        return set()
    if isinstance(declared, str):
        return {declared}
    return {t for t in declared if isinstance(t, str)}


def _resolve(schema: dict, node: dict) -> dict:
    """Follow a single local `$ref` (`#/$defs/Name`) if present."""
    ref = node.get("$ref")
    if isinstance(ref, str) and ref.startswith("#/"):
        target: object = schema
        for part in ref[2:].split("/"):
            if not isinstance(target, dict):
                return node
            target = target.get(part)
        if isinstance(target, dict):
            return target
    return node


def _compare_node(schema: dict, base: dict, head: dict, path: str) -> list[str]:
    base, head = _resolve(schema, base), _resolve(schema, head)
    problems: list[str] = []

    base_types, head_types = _type_names(base), _type_names(head)
    if base_types and head_types and not (base_types & head_types):
        problems.append(
            f"{path}: type changed from {'/'.join(sorted(base_types))} "
            f"to {'/'.join(sorted(head_types))}"
        )

    base_props = base.get("properties")
    head_props = head.get("properties")
    if isinstance(base_props, dict):
        head_props = head_props if isinstance(head_props, dict) else {}
        base_required = set(base.get("required") or [])
        head_required = set(head.get("required") or [])
        for name, base_prop in base_props.items():
            child = f"{path}.{name}"
            if name not in head_props:
                problems.append(f"{child}: field removed")
                continue
            problems.extend(
                _compare_node(schema, base_prop, head_props[name], child)
            )
        for name in head_props:
            if name not in base_props and name in head_required:
                problems.append(
                    f"{path}.{name}: new required field (additive-breaking)"
                )

    base_items = base.get("items")
    head_items = head.get("items")
    if isinstance(base_items, dict) and isinstance(head_items, dict):
        problems.extend(_compare_node(schema, base_items, head_items, f"{path}[]"))

    return problems


def schema_breaking_changes(base: dict, head: dict) -> list[str]:
    """Return human-readable additive-breaking changes from `base` to `head`."""
    return _compare_node(head, base, head, "$")


def cli_schema_impact(repo: Path, base_rev: str, head_rev: str) -> list[str]:
    """Compare every published CLI schema between two revisions."""
    try:
        listing = git(repo, "ls-tree", "-r", "--name-only", head_rev, "--", CLI_SCHEMA_DIR)
    except subprocess.CalledProcessError:
        return []
    errors: list[str] = []
    for path in listing.split():
        if not path.endswith(".json"):
            continue
        try:
            head = json.loads(git(repo, "show", f"{head_rev}:{path}"))
        except (subprocess.CalledProcessError, json.JSONDecodeError):
            continue
        try:
            base = json.loads(git(repo, "show", f"{base_rev}:{path}"))
        except (subprocess.CalledProcessError, json.JSONDecodeError):
            continue  # newly added schema: nothing to break
        problems = schema_breaking_changes(base, head)
        if not problems:
            continue
        base_version, head_version = schema_version(base), schema_version(head)
        if base_version is not None and head_version != base_version:
            continue  # version bumped: breaking change is allowed
        for problem in problems:
            errors.append(f"{path}: {problem}")
        errors.append(
            f"{path}: bump the schema $id version (currently {base_version!r}) "
            "to allow these changes"
        )
    return errors


# --- Static storage-layout compatibility checker ---------------------------
#
# Contract specs embed the XDR UDT definitions (`SCSpecUdtStructV0`,
# `SCSpecUdtUnionV0`). We extract them from the last released wasm and from
# HEAD, follow `DataKey` value types to find storage-reachable types, and
# classify each change as compatible, additive or breaking. Breaking changes to
# storage-reachable types require a `SCHEMA_VERSION` bump plus a migration.


@dataclass
class UdtField:
    name: str
    type: str


@dataclass
class Udt:
    kind: str  # "struct" | "union"
    name: str
    fields: list[UdtField] = field(default_factory=list)
    cases: list[str] = field(default_factory=list)  # union variant names


@dataclass
class UdtDiff:
    type_name: str
    severity: str  # "compatible" | "additive" | "breaking"
    detail: str


def _spec_udts(spec: dict) -> dict[str, Udt]:
    """Extract UDT definitions from a parsed contract spec JSON."""
    udts: dict[str, Udt] = {}
    for entry in spec.get("udtStructV0", []) or []:
        name = entry.get("name")
        if not isinstance(name, str):
            continue
        fields = [
            UdtField(f.get("name", ""), f.get("type", ""))
            for f in entry.get("fields", []) or []
        ]
        udts[name] = Udt("struct", name, fields=fields)
    for entry in spec.get("udtUnionV0", []) or []:
        name = entry.get("name")
        if not isinstance(name, str):
            continue
        cases = [c.get("name", "") for c in entry.get("cases", []) or []]
        udts[name] = Udt("union", name, cases=cases)
    return udts


def _datakey_reachable(udts: dict[str, Udt]) -> set[str]:
    """Types reachable from `DataKey` value types (transitively)."""
    reachable: set[str] = set()
    pending: list[str] = []
    for udt in udts.values():
        if udt.name == "DataKey":
            pending.extend(f.type for f in udt.fields)
    while pending:
        name = pending.pop()
        if name in reachable or name not in udts:
            continue
        reachable.add(name)
        udt = udts[name]
        pending.extend(f.type for f in udt.fields)
    return reachable


def _diff_udt(base: Udt, head: Udt) -> UdtDiff:
    if base.kind != head.kind:
        return UdtDiff(base.name, "breaking", f"kind changed {base.kind} -> {head.kind}")
    if base.kind == "struct":
        base_names = [f.name for f in base.fields]
        head_names = [f.name for f in head.fields]
        if base_names != head_names:
            if set(base_names) == set(head_names):
                return UdtDiff(base.name, "breaking", "field reordered")
            removed = [n for n in base_names if n not in head_names]
            if removed:
                return UdtDiff(base.name, "breaking", f"field removed: {', '.join(removed)}")
            return UdtDiff(base.name, "additive", "field added")
        for b, h in zip(base.fields, head.fields):
            if b.type != h.type:
                return UdtDiff(
                    base.name, "breaking", f"field {b.name} type {b.type} -> {h.type}"
                )
        return UdtDiff(base.name, "compatible", "unchanged")
    # union
    if base.cases != head.cases:
        removed = [c for c in base.cases if c not in head.cases]
        if removed:
            return UdtDiff(base.name, "breaking", f"variant removed: {', '.join(removed)}")
        return UdtDiff(base.name, "additive", "variant added")
    return UdtDiff(base.name, "compatible", "unchanged")


def storage_layout_impact(
    base_spec: dict, head_spec: dict
) -> list[UdtDiff]:
    """Classify UDT changes between two contract specs."""
    base_udts, head_udts = _spec_udts(base_spec), _spec_udts(head_spec)
    reachable = _datakey_reachable(base_udts) | _datakey_reachable(head_udts)
    diffs: list[UdtDiff] = []
    for name in sorted(reachable):
        if name not in base_udts:
            diffs.append(UdtDiff(name, "additive", "type added"))
            continue
        if name not in head_udts:
            diffs.append(UdtDiff(name, "breaking", "type removed"))
            continue
        diffs.append(_diff_udt(base_udts[name], head_udts[name]))
    return diffs


def _schema_version_at(repo: Path, rev: str) -> int | None:
    try:
        source = git(repo, "show", f"{rev}:{SCHEMA_VERSION_FILE}")
    except subprocess.CalledProcessError:
        return None
    m = SCHEMA_VERSION_RE.search(source)
    return int(m.group(1)) if m else None


def _has_migration(repo: Path, base_rev: str, head_rev: str) -> bool:
    try:
        listing = git(repo, "diff", "--name-only", base_rev, head_rev, "--", MIGRATION_DIR)
    except subprocess.CalledProcessError:
        return False
    return any(line.strip().endswith(".rs") for line in listing.splitlines())


def _latest_release_wasm(repo: Path) -> tuple[str, str] | None:
    """Return (version, wasm_path) for the latest release manifest entry."""
    try:
        manifest = json.loads(git(repo, "show", f"HEAD:{RELEASE_MANIFEST}"))
    except (subprocess.CalledProcessError, json.JSONDecodeError):
        return None
    releases = manifest.get("releases") or []
    if not releases:
        return None
    latest = releases[-1]
    version = latest.get("version")
    wasm = latest.get("wasm")
    if not isinstance(version, str) or not isinstance(wasm, str):
        return None
    return version, wasm


def _spec_from_wasm(repo: Path, wasm_path: str) -> dict | None:
    """Extract the contract spec from a wasm file using `stellar contract info`."""
    try:
        out = subprocess.run(
            ["stellar", "contract", "info", "spec", "--wasm", str(repo / wasm_path)],
            check=True, capture_output=True, text=True,
        ).stdout
    except (subprocess.CalledProcessError, FileNotFoundError):
        return None
    try:
        return json.loads(out)
    except json.JSONDecodeError:
        return None


def storage_layout_impact_check(repo: Path, base_rev: str, head_rev: str) -> list[str]:
    """Fail on breaking storage-layout changes without a schema bump + migration."""
    release = _latest_release_wasm(repo)
    if release is None:
        return []  # no released wasm to compare against
    version, wasm_path = release
    base_spec = _spec_from_wasm(repo, wasm_path)
    if base_spec is None:
        return []
    try:
        head_spec = json.loads(git(repo, "show", f"{head_rev}:{wasm_path}"))
    except (subprocess.CalledProcessError, json.JSONDecodeError):
        return []
    diffs = storage_layout_impact(base_spec, head_spec)
    breaking = [d for d in diffs if d.severity == "breaking"]
    if not breaking:
        return []
    base_version = _schema_version_at(repo, base_rev)
    head_version = _schema_version_at(repo, head_rev)
    bumped = base_version is not None and head_version is not None and head_version > base_version
    if bumped and _has_migration(repo, base_rev, head_rev):
        return []
    errors = [
        f"storage-layout: {d.type_name}: {d.detail} (breaking vs release {version})"
        for d in breaking
    ]
    if not bumped:
        errors.append(
            f"storage-layout: bump SCHEMA_VERSION (currently {base_version!r}) "
            "to allow breaking changes"
        )
    if not _has_migration(repo, base_rev, head_rev):
        errors.append(
            f"storage-layout: add a migration under {MIGRATION_DIR}/ "
            "to allow breaking changes"
        )
    return errors


def main() -> int:
    parser = argparse.ArgumentParser(description=__doc__.splitlines()[0])
    parser.add_argument("--repo", type=Path, default=Path.cwd())
    parser.add_argument("--cli-schemas", action="store_true")
    parser.add_argument("--storage-layout", action="store_true")
    parser.add_argument("rev_range", nargs="?")
    parser.add_argument("head", nargs="?")
    args = parser.parse_args()

    if args.storage_layout:
        if not args.rev_range or not args.head:
            parser.error("--storage-layout requires BASE and HEAD revisions")
        errors = storage_layout_impact_check(args.repo, args.rev_range, args.head)
        for error in errors:
            print(f"::error::{error}")
        if errors:
            print("Breaking storage-layout change without a schema bump + migration -- see docs/releasing.md.")
            return 1
        print(f"storage-layout: OK ({args.rev_range}..{args.head})")
        return 0

    if args.cli_schemas:
        if not args.rev_range or not args.head:
            parser.error("--cli-schemas requires BASE and HEAD revisions")
        errors = cli_schema_impact(args.repo, args.rev_range, args.head)
        for error in errors:
            print(f"::error::{error}")
        if errors:
            print("Additive-breaking CLI schema change without a version bump -- see docs/cli-json-schema.md.")
            return 1
        print(f"cli-schema-impact: OK ({args.rev_range}..{args.head})")
        return 0

    if not args.rev_range:
        parser.error("REV_RANGE is required")

    bad = offending_commits(args.repo, args.rev_range)
    for sha in bad:
        subject = git(args.repo, "log", "-1", "--format=%h %s", sha).strip()
        print(f"::error::{subject}: changes DataKey/#[contracttype] but has no Schema-Impact: trailer")
    if bad:
        print("Add the trailer (git commit --amend --trailer 'Schema-Impact: ...') -- see docs/releasing.md.")
        return 1
    print(f"schema-impact: OK ({args.rev_range})")
    return 0


if __name__ == "__main__":
    sys.exit(main())
