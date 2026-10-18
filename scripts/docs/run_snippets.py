#!/usr/bin/env python3
"""Extract and run the fenced doc snippets marked `testable`.

    python3 scripts/docs/run_snippets.py [--list] [path ...]

Scans Markdown files (default: every tracked `*.md`) for fences whose info
string is a supported language followed by `testable`:

    ```bash testable          ```ts testable

Shell snippets (`bash`, `sh`, `shell`) run under `bash -euo pipefail`;
TypeScript snippets (`ts`, `typescript`) run with Node's built-in type
stripping. Each snippet runs in a sandbox: a fresh temporary directory as
the working directory and `HOME`, a minimal environment (only `PATH` is
inherited, plus `REPO_ROOT` pointing at the checkout for snippets that need
repository files), and a timeout. Snippets are independent; any failure
fails the run.
"""

import argparse
import os
import re
import subprocess
import sys
import tempfile
from pathlib import Path

REPO_ROOT = Path(__file__).resolve().parents[2]
FENCE = re.compile(r"^(?P<indent> {0,3})(?P<fence>`{3,}|~{3,})\s*(?P<info>[^\n`]*)$")
SHELL = {"bash", "sh", "shell"}
TYPESCRIPT = {"ts", "typescript"}
TIMEOUT_SECONDS = 120


def snippets(path: Path) -> list[tuple[int, str, str]]:
    """Return (line, language, body) for each `testable` fence in `path`."""
    found, lines = [], path.read_text(encoding="utf-8").splitlines()
    i = 0
    while i < len(lines):
        m = FENCE.match(lines[i])
        if not m:
            i += 1
            continue
        info = m["info"].split()
        start, fence, body = i + 1, m["fence"], []
        i += 1
        while i < len(lines) and not lines[i].strip().startswith(fence):
            body.append(lines[i])
            i += 1
        i += 1
        if len(info) >= 2 and "testable" in info[1:] and info[0] in SHELL | TYPESCRIPT:
            found.append((start, info[0], "\n".join(body) + "\n"))
    return found


def run(lang: str, body: str) -> subprocess.CompletedProcess[str]:
    with tempfile.TemporaryDirectory(prefix="lafiya-snippet-") as sandbox:
        env = {"PATH": os.environ.get("PATH", ""), "HOME": sandbox, "REPO_ROOT": str(REPO_ROOT)}
        if lang in SHELL:
            argv = ["bash", "-euo", "pipefail", "-c", body]
        else:
            script = Path(sandbox) / "snippet.ts"
            script.write_text(body, encoding="utf-8")
            argv = ["node", "--experimental-strip-types", "--no-warnings", str(script)]
        return subprocess.run(
            argv, cwd=sandbox, env=env, capture_output=True, text=True, timeout=TIMEOUT_SECONDS
        )


def tracked_markdown() -> list[Path]:
    out = subprocess.run(
        ["git", "ls-files", "*.md"], cwd=REPO_ROOT, capture_output=True, text=True, check=True
    ).stdout
    return [REPO_ROOT / p for p in out.splitlines()]


def main() -> None:
    parser = argparse.ArgumentParser(description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter)
    parser.add_argument("paths", nargs="*", type=Path)
    parser.add_argument("--list", action="store_true", help="list snippets without running them")
    args = parser.parse_args()

    failures = total = 0
    for path in args.paths or tracked_markdown():
        for line, lang, body in snippets(path):
            total += 1
            where = f"{path.resolve().relative_to(REPO_ROOT)}:{line}"
            if args.list:
                print(f"{where} [{lang}]")
                continue
            try:
                result = run(lang, body)
            except subprocess.TimeoutExpired:
                print(f"[snippets] FAIL {where}: timed out after {TIMEOUT_SECONDS}s")
                failures += 1
                continue
            if result.returncode == 0:
                print(f"[snippets] ok   {where}")
            else:
                failures += 1
                print(f"[snippets] FAIL {where} (exit {result.returncode})")
                print(result.stdout + result.stderr)
    if not args.list:
        print(f"[snippets] {total - failures}/{total} testable snippets passed")
    if failures:
        sys.exit(1)


if __name__ == "__main__":
    main()
