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
"""
import argparse
import re
import subprocess
import sys
import tempfile
from pathlib import Path

SCHEMA_WORDS = re.compile(r"\b(DataKey|contracttype)\b")
CONTRACTTYPE_ITEM = re.compile(
    r"#\[contracttype\][^\n]*\n(?:\s*#\[[^\n]*\n|\s*///[^\n]*\n)*\s*pub\s+(?:struct|enum)\s+(\w+)"
)
HUNK = re.compile(r"^@@ [^@]* @@ ?(.*)$")
ITEM_NAME = re.compile(r"\b(?:struct|enum|impl)\s+(\w+)")
TEST_PATH = re.compile(r"(^|/)(tests?|benches)(/|\.rs$)|_test\.rs$")


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


def main() -> int:
    parser = argparse.ArgumentParser(description=__doc__.splitlines()[0])
    parser.add_argument("--repo", type=Path, default=Path.cwd())
    parser.add_argument("rev_range")
    args = parser.parse_args()

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
