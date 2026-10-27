"""Tests for generate_sboms.py, validate_sboms.py and the SBOM/VEX parts of
the release manifest."""

import copy
import hashlib
import json
import pathlib
from typing import Any

import pytest

import generate_release_manifest as manifest_gen
import generate_sboms as gen
import validate_release_manifest as manifest_validate
import validate_sboms as validate

VEX_PATH = gen.ROOT / "security" / "vex" / "vex.cdx.json"
VEX: dict[str, Any] = json.loads(VEX_PATH.read_text())

TREE = """\
0attester-registry v0.1.0 (/src/contracts/attester-registry)|MIT
1soroban-sdk v28.0.0|Apache-2.0
2stellar-strkey v0.0.16|Apache-2.0
3data-encoding v2.11.1|MIT
2data-encoding v2.11.1|MIT
1lafiya-contract-common v0.1.0 (/src/crates/lafiya-contract-common)|
2soroban-sdk v28.0.0|Apache-2.0 (*)
"""


def run_main(monkeypatch: pytest.MonkeyPatch, main: Any, *argv: object) -> Any:
    monkeypatch.setattr("sys.argv", ["prog", *map(str, argv)])
    return main()


def sample_sbom(monkeypatch: pytest.MonkeyPatch, artifact_file: pathlib.Path | None = None) -> Any:
    monkeypatch.setattr(gen, "lock_checksums", lambda: {("soroban-sdk", "28.0.0"): "ab" * 32})
    return gen.build_sbom(
        "attester-registry",
        "contract-wasm",
        TREE,
        "wasm32v1-none",
        "0" * 40,
        "2026-10-18T00:00:00Z",
        artifact_file,
    )


# --- generate_sboms ---------------------------------------------------------


def test_parse_tree_builds_closure_and_edges() -> None:
    root, closure, edges = gen.parse_tree(TREE)
    assert root["name"] == "attester-registry" and root["workspace"]
    assert [gen.purl(p) for p in closure] == [
        "pkg:cargo/data-encoding@2.11.1",
        "pkg:cargo/lafiya-contract-common@0.1.0",
        "pkg:cargo/soroban-sdk@28.0.0",
        "pkg:cargo/stellar-strkey@0.0.16",
    ]
    assert edges["pkg:cargo/soroban-sdk@28.0.0"] == [
        "pkg:cargo/data-encoding@2.11.1",
        "pkg:cargo/stellar-strkey@0.0.16",
    ]
    assert edges["pkg:cargo/lafiya-contract-common@0.1.0"] == ["pkg:cargo/soroban-sdk@28.0.0"]


def test_parse_tree_rejects_empty_output() -> None:
    with pytest.raises(ValueError):
        gen.parse_tree("\n")


def test_build_sbom_is_valid_cyclonedx(monkeypatch: pytest.MonkeyPatch, wasm: pathlib.Path) -> None:
    sbom = sample_sbom(monkeypatch, wasm)
    assert validate.schema_errors(validate.schema_validator(), sbom, "sbom") == []
    main = sbom["metadata"]["component"]
    assert main["hashes"][0]["content"] == hashlib.sha256(b"\0asm").hexdigest()
    sdk = next(c for c in sbom["components"] if c["name"] == "soroban-sdk")
    assert sdk["hashes"] == [{"alg": "SHA-256", "content": "ab" * 32}]
    common = next(c for c in sbom["components"] if c["name"] == "lafiya-contract-common")
    assert "licenses" not in common and common["properties"][0]["value"] == "workspace"


def test_build_sbom_is_reproducible(monkeypatch: pytest.MonkeyPatch) -> None:
    assert sample_sbom(monkeypatch) == sample_sbom(monkeypatch)


def test_generate_writes_one_sbom_per_rust_artifact(
    monkeypatch: pytest.MonkeyPatch, tmp_path: pathlib.Path
) -> None:
    def fake_run(*args: str) -> str:
        if args[:2] == ("cargo", "tree"):
            package = args[args.index("-p") + 1]
            return TREE.replace("attester-registry", package)
        return "1760745600\n" if args[1] == "log" else "0" * 40 + "\n"

    monkeypatch.setattr(gen, "run", fake_run)
    monkeypatch.setattr(gen, "lock_checksums", lambda: {})
    monkeypatch.delenv("SOURCE_DATE_EPOCH", raising=False)
    run_main(monkeypatch, gen.main, "-o", tmp_path)
    names = sorted(p.name for p in tmp_path.iterdir())
    assert names == [
        "cli-lafiya-cli.sbom.cdx.json",
        "contract-attestation-registry.sbom.cdx.json",
        "contract-attester-registry.sbom.cdx.json",
        "contract-multisig-account.sbom.cdx.json",
    ]
    cli = json.loads((tmp_path / "cli-lafiya-cli.sbom.cdx.json").read_text())
    assert cli["metadata"]["timestamp"] == "2025-10-18T00:00:00Z"


# --- validate_sboms ---------------------------------------------------------


def test_committed_vex_is_valid() -> None:
    assert validate.schema_errors(validate.schema_validator(), VEX, "vex") == []
    assert validate.vex_errors(VEX, None) == []


def test_vex_requires_advisory_id_and_justification() -> None:
    vex = copy.deepcopy(VEX)
    vex["vulnerabilities"][0]["id"] = "not-an-advisory"
    del vex["vulnerabilities"][1]["analysis"]["justification"]
    vex["vulnerabilities"][2]["analysis"]["detail"] = "n/a"
    vex["vulnerabilities"][3]["affects"] = []
    del vex["vulnerabilities"][4]["analysis"]["state"]
    vex["vulnerabilities"].append(copy.deepcopy(vex["vulnerabilities"][1]))
    errors = "\n".join(validate.vex_errors(vex, None))
    assert "must be a RUSTSEC-, GHSA- or CVE- advisory id" in errors
    assert "needs analysis.justification" in errors
    assert "needs a reachability analysis.detail" in errors
    assert "affects must list" in errors
    assert "analysis.state is required" in errors
    assert "duplicate statement" in errors


def test_vex_affects_must_be_in_an_sbom_unless_code_not_present() -> None:
    errors = validate.vex_errors(VEX, set())
    assert errors and all("pkg:cargo/rustls@" in e for e in errors)
    assert validate.vex_errors(VEX, {"pkg:cargo/rustls@0.23.45"}) == []


def test_audit_findings_need_vex_statements() -> None:
    audit = {
        "vulnerabilities": {"list": [{"advisory": {"id": "RUSTSEC-2026-0346"}}]},
        "warnings": {
            "unmaintained": [{"advisory": {"id": "RUSTSEC-2099-0001"}}],
            "yanked": [{"advisory": None}],
        },
    }
    assert validate.audit_errors(audit, VEX) == [
        "cargo audit reports RUSTSEC-2099-0001 but the VEX document has no statement for it"
    ]


def test_validate_main(
    monkeypatch: pytest.MonkeyPatch, tmp_path: pathlib.Path, capsys: pytest.CaptureFixture[str]
) -> None:
    sbom = sample_sbom(monkeypatch)
    sbom["components"].append(
        {"type": "library", "name": "rustls", "purl": "pkg:cargo/rustls@0.23.45"}
    )
    sbom_path = tmp_path / "cli-x.sbom.cdx.json"
    sbom_path.write_text(json.dumps(sbom))
    audit_path = tmp_path / "audit.json"
    audit_path.write_text(json.dumps({"vulnerabilities": {"list": []}, "warnings": {}}))
    args = ("--vex", VEX_PATH, "--audit-json", audit_path, sbom_path)
    assert run_main(monkeypatch, validate.main, *args) == 0

    sbom["bomFormat"] = "SPDX"
    sbom_path.write_text(json.dumps(sbom))
    assert run_main(monkeypatch, validate.main, *args) == 1
    assert "schema error at bomFormat" in capsys.readouterr().err


# --- release manifest -------------------------------------------------------


def test_manifest_records_sboms_and_vex(
    monkeypatch: pytest.MonkeyPatch, tmp_path: pathlib.Path
) -> None:
    sbom = sample_sbom(monkeypatch)
    (tmp_path / "contract-attester-registry.sbom.cdx.json").write_text(json.dumps(sbom))
    vex_path = tmp_path / "vex.cdx.json"
    vex_path.write_text(VEX_PATH.read_text())

    manifest = manifest_gen.generate(None, tmp_path, vex_path)
    assert manifest["sboms"][0]["kind"] == "contract-wasm"
    assert manifest["sboms"][0]["artifact"] == "attester-registry"
    assert [s["id"] for s in manifest["vex"]["statements"]] == [
        v["id"] for v in VEX["vulnerabilities"]
    ]

    manifest_path = tmp_path / "release-manifest.json"
    manifest_path.write_text(json.dumps(manifest))
    assert run_main(monkeypatch, manifest_validate.main, manifest_path) == 0

    vex_path.write_text("{}")
    assert manifest_validate.asset_errors(manifest, tmp_path) == [
        "vex.sha256 does not match vex.cdx.json"
    ]


def test_manifest_rejects_unknown_sbom_prefix(tmp_path: pathlib.Path) -> None:
    (tmp_path / "firmware-x.sbom.cdx.json").write_text("{}")
    with pytest.raises(ValueError, match="unknown SBOM kind"):
        manifest_gen.build_sboms(tmp_path)


def test_manifest_flags_duplicate_vex_statements() -> None:
    statement = {"id": "RUSTSEC-2026-0346", "state": "not_affected"}
    manifest = {"vex": {"path": "missing.json", "sha256": "0", "statements": [statement] * 2}}
    errors = manifest_validate.asset_errors(manifest, pathlib.Path("/nonexistent"))
    assert errors == ["vex.statements has more than one statement for RUSTSEC-2026-0346"]
