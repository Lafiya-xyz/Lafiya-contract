#!/usr/bin/env python3
"""Generate CycloneDX 1.6 SBOMs for the Rust release artifacts.

One SBOM per contract wasm (dependency closure for wasm32v1-none) and one for
the `lafiya-cli` binary (closure for the host target). Only normal
dependencies are listed: build and dev dependencies never ship in the
artifact. The indexer container image and the npm packages are scanned with
syft in the release workflow, into the same output directory and naming
scheme (see SBOM_KINDS).

Output is reproducible: the serial number is derived from the artifact name
and git commit, and the timestamp is the commit time (or SOURCE_DATE_EPOCH).

Usage:
    scripts/generate_sboms.py [-o DIR]

Writes DIR/<kind>-<name>.sbom.cdx.json (default DIR: dist).
"""

import argparse
import hashlib
import json
import os
import subprocess
import tomllib
import uuid
from datetime import UTC, datetime
from pathlib import Path
from typing import Any

Json = dict[str, Any]

ROOT = Path(__file__).resolve().parents[1]
WASM_DIR = ROOT / "target" / "wasm32v1-none" / "release"
SPEC_VERSION = "1.6"
SBOM_SUFFIX = ".sbom.cdx.json"

# File-name prefix -> artifact kind recorded in the release manifest.
SBOM_KINDS = {
    "contract": "contract-wasm",
    "cli": "cli",
    "image": "container-image",
    "npm": "npm-package",
}

# name -> (wasm file stem); kept in step with generate_release_manifest.CONTRACTS
CONTRACTS = {
    "attester-registry": "attester_registry",
    "attestation-registry": "attestation_registry",
    "multisig-account": "multisig_account",
}
CLI_PACKAGE = "lafiya-cli"
HOST_TARGET = "x86_64-unknown-linux-gnu"


def run(*args: str) -> str:
    return subprocess.run(args, cwd=ROOT, capture_output=True, text=True, check=True).stdout


def cargo_tree(package: str, target: str) -> str:
    """`cargo tree` output for the artifact's runtime closure: features are
    resolved for this package alone (as `cargo build -p` would), and build,
    dev and proc-macro edges are dropped because none of them ship."""
    return run(
        "cargo",
        "tree",
        "--locked",
        "-p",
        package,
        "--target",
        target,
        "-e",
        "normal,no-proc-macro",
        "--prefix",
        "depth",
        "--format",
        "{p}|{l}",
    )


def lock_checksums() -> dict[tuple[str, str], str]:
    with open(ROOT / "Cargo.lock", "rb") as f:
        lock = tomllib.load(f)
    return {
        (p["name"], p["version"]): p["checksum"] for p in lock.get("package", []) if "checksum" in p
    }


def purl(pkg: Json) -> str:
    return f"pkg:cargo/{pkg['name']}@{pkg['version']}"


def parse_tree(tree: str) -> tuple[Json, list[Json], dict[str, list[str]]]:
    """Parse `cargo tree --prefix depth --format '{p}|{l}'` into the root
    package, the packages it reaches, and dependency edges keyed by purl."""
    packages: dict[str, Json] = {}
    edges: dict[str, set[str]] = {}
    stack: list[str] = []
    root: Json | None = None
    for line in tree.splitlines():
        if not line.strip():
            continue
        digits = len(line) - len(line.lstrip("0123456789"))
        depth = int(line[:digits])
        spec, _, license_ = line[digits:].partition("|")
        parts = spec.replace(" (*)", "").split(" ", 2)
        pkg: Json = {
            "name": parts[0],
            "version": parts[1].removeprefix("v"),
            # A local path in parentheses marks a workspace (path) package.
            "workspace": len(parts) > 2 and parts[2].startswith("(/"),
            "license": license_ or None,
        }
        ref = purl(pkg)
        packages.setdefault(ref, pkg)
        edges.setdefault(ref, set())
        del stack[depth:]
        if stack:
            edges[stack[-1]].add(ref)
        else:
            root = pkg
        stack.append(ref)
    if root is None:
        raise ValueError("empty cargo tree output")
    closure = sorted((p for r, p in packages.items() if r != purl(root)), key=purl)
    return root, closure, {r: sorted(d) for r, d in sorted(edges.items())}


def component(pkg: Json, checksums: dict[tuple[str, str], str]) -> Json:
    comp: Json = {
        "type": "library",
        "bom-ref": purl(pkg),
        "name": pkg["name"],
        "version": pkg["version"],
        "purl": purl(pkg),
    }
    if pkg.get("license"):
        comp["licenses"] = [{"expression": pkg["license"]}]
    checksum = checksums.get((pkg["name"], pkg["version"]))
    if checksum:
        comp["hashes"] = [{"alg": "SHA-256", "content": checksum}]
    if pkg["workspace"]:
        comp["properties"] = [{"name": "lafiya:source", "value": "workspace"}]
    return comp


def build_timestamp() -> str:
    epoch = os.environ.get("SOURCE_DATE_EPOCH") or run("git", "log", "-1", "--format=%ct").strip()
    return datetime.fromtimestamp(int(epoch), UTC).strftime("%Y-%m-%dT%H:%M:%SZ")


def build_sbom(
    artifact: str,
    kind: str,
    tree: str,
    target: str,
    git_commit: str,
    timestamp: str,
    artifact_file: Path | None = None,
) -> Json:
    root, closure, edges = parse_tree(tree)
    checksums = lock_checksums()

    main: Json = {
        "type": "application",
        "bom-ref": purl(root),
        "name": artifact,
        "version": root["version"],
        "purl": purl(root),
        "properties": [
            {"name": "lafiya:artifact-kind", "value": kind},
            {"name": "lafiya:target", "value": target},
            {"name": "lafiya:git-commit", "value": git_commit},
        ],
    }
    if root.get("license"):
        main["licenses"] = [{"expression": root["license"]}]
    if artifact_file is not None and artifact_file.is_file():
        main["hashes"] = [
            {"alg": "SHA-256", "content": hashlib.sha256(artifact_file.read_bytes()).hexdigest()}
        ]

    serial = uuid.uuid5(uuid.NAMESPACE_URL, f"lafiya:{artifact}:{git_commit}")
    return {
        "bomFormat": "CycloneDX",
        "specVersion": SPEC_VERSION,
        "serialNumber": f"urn:uuid:{serial}",
        "version": 1,
        "metadata": {
            "timestamp": timestamp,
            "tools": {"components": [{"type": "application", "name": "generate_sboms.py"}]},
            "component": main,
        },
        "components": [component(p, checksums) for p in closure],
        "dependencies": [{"ref": ref, "dependsOn": deps} for ref, deps in edges.items()],
    }


def generate(out_dir: Path) -> list[Path]:
    git_commit = run("git", "rev-parse", "HEAD").strip()
    timestamp = build_timestamp()
    out_dir.mkdir(parents=True, exist_ok=True)
    written: list[Path] = []

    for name, stem in CONTRACTS.items():
        sbom = build_sbom(
            name,
            SBOM_KINDS["contract"],
            cargo_tree(name, "wasm32v1-none"),
            "wasm32v1-none",
            git_commit,
            timestamp,
            WASM_DIR / f"{stem}.wasm",
        )
        written.append(write(out_dir / f"contract-{name}{SBOM_SUFFIX}", sbom))

    sbom = build_sbom(
        CLI_PACKAGE,
        SBOM_KINDS["cli"],
        cargo_tree(CLI_PACKAGE, HOST_TARGET),
        HOST_TARGET,
        git_commit,
        timestamp,
        ROOT / "target" / "release" / CLI_PACKAGE,
    )
    written.append(write(out_dir / f"cli-{CLI_PACKAGE}{SBOM_SUFFIX}", sbom))
    return written


def write(path: Path, sbom: Json) -> Path:
    path.write_text(json.dumps(sbom, indent=2) + "\n", encoding="utf-8")
    return path


def main() -> None:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("-o", "--output-dir", type=Path, default=ROOT / "dist")
    args = parser.parse_args()
    for path in generate(args.output_dir):
        print(f"wrote {path}")


if __name__ == "__main__":
    main()
