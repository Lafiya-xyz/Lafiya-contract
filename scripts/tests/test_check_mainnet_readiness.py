import hashlib
import importlib.util
import json
from pathlib import Path

spec = importlib.util.spec_from_file_location(
    "check_mainnet_readiness", Path(__file__).resolve().parent.parent / "check_mainnet_readiness.py"
)
mod = importlib.util.module_from_spec(spec)
spec.loader.exec_module(mod)

NETWORKS = '[mainnet]\nrpc_url="x"\nnetwork_passphrase="y"\n[mainnet.contracts]\nattester_registry="{id}"\n'


def repo(tmp_path, contract_id):
    (tmp_path / "config").mkdir()
    (tmp_path / "config" / "networks.toml").write_text(NETWORKS.format(id=contract_id))
    return tmp_path


def _write_evidence(tmp_path, manifest):
    for key in mod.EVIDENCE:
        f = tmp_path / f"{key}.md"
        f.write_text(key)
        manifest[key] = {"path": f.name, "sha256": hashlib.sha256(f.read_bytes()).hexdigest()}
    (tmp_path / "config" / "mainnet-readiness.json").write_text(json.dumps(manifest))


def _write_rehearsal_report(tmp_path, generated_at, status="green"):
    report = {
        "generated_at": generated_at,
        "status": status,
        "scenarios": [
            "happy_path",
            "no_trustline",
            "unauthorized_trustline",
            "merged_account",
            "paused_pool",
            "drift",
        ],
    }
    path = tmp_path / "rehearsal-report.json"
    path.write_text(json.dumps(report))
    return path


def test_mainnet_disabled_needs_no_manifest(tmp_path):
    assert mod.check(repo(tmp_path, "")) == []


def test_enabling_mainnet_without_evidence_fails(tmp_path):
    assert mod.check(repo(tmp_path, "CABC")) != []


def test_enabling_mainnet_with_valid_manifest_passes(tmp_path):
    repo(tmp_path, "CABC")
    manifest = {"owner_signoff": ["@owner"]}
    _write_evidence(tmp_path, manifest)
    assert mod.check(tmp_path) == []
    (tmp_path / "audit_report.md").write_text("tampered")
    assert any("sha256" in e for e in mod.check(tmp_path))


def test_missing_rehearsal_report_fails(tmp_path):
    repo(tmp_path, "CABC")
    manifest = {"owner_signoff": ["@owner"]}
    _write_evidence(tmp_path, manifest)
    errors = mod.check(tmp_path)
    assert any("rehearsal" in e.lower() for e in errors)


def test_stale_rehearsal_report_fails(tmp_path):
    repo(tmp_path, "CABC")
    manifest = {"owner_signoff": ["@owner"]}
    _write_evidence(tmp_path, manifest)
    _write_rehearsal_report(tmp_path, "2000-01-01T00:00:00Z")
    errors = mod.check(tmp_path)
    assert any("rehearsal" in e.lower() for e in errors)


def test_recent_green_rehearsal_report_passes(tmp_path):
    repo(tmp_path, "CABC")
    manifest = {"owner_signoff": ["@owner"]}
    _write_evidence(tmp_path, manifest)
    _write_rehearsal_report(tmp_path, mod.now_iso())
    assert mod.check(tmp_path) == []