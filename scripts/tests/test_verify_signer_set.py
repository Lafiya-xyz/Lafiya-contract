"""Tests for scripts/verify_signer_set.py against the committed dry-run record."""

import json
import pathlib
import subprocess
from typing import Any

import pytest
import verify_signer_set as vss
from conftest import completed

ROOT = pathlib.Path(__file__).resolve().parents[2]
RECORD = ROOT / "docs" / "security" / "ceremonies" / "2026-10-18-testnet-dry-run.json"
ADMIN_3 = "GCZFTV6RJVEAZDO2E25SODXWXQMGYVCO2YKGQMFQVCO3KE6AZLERGKEP"


def load() -> dict[str, Any]:
    record: dict[str, Any] = json.loads(RECORD.read_text())
    return record


def fake_chain(record: dict[str, Any]) -> vss.Invoke:
    """Chain state that matches `record`, keyed like the stellar CLI output."""
    accounts = {a["contract_id"]: a for a in record["accounts"].values()}
    admin_id = record["accounts"]["admin"]["contract_id"]
    guardian_id = record["accounts"]["guardian"]["contract_id"]

    def invoke(contract: str, call: str) -> Any:
        if contract in accounts:
            acct = accounts[contract]
            keys = sorted(vss.decode_account(s["public_key"]) for s in acct["signers"])
            t = acct["threshold"]
            return {
                "get_signers": [{"Ed25519": k.hex()} for k in keys],
                "get_threshold": t,
                "get_policy": {
                    "minimum_signers": t,
                    "weight_threshold": t,
                    "role_requirements": [],
                    "signers": [
                        {"public_key": {"Ed25519": k.hex()}, "role": None, "weight": 1}
                        for k in keys
                    ],
                },
            }[call]
        if call == "get_admin":
            return admin_id
        assert call == f"has_role --role Guardian --account {guardian_id}"
        return True

    return invoke


def run(
    record: dict[str, Any], tmp_path: pathlib.Path, invoke: vss.Invoke | None, *extra: str
) -> int:
    path = tmp_path / "record.json"
    path.write_text(json.dumps(record))
    return vss.main(["verify", str(path), *extra], invoke=invoke)


def test_strkey_round_trip_and_checksum() -> None:
    key = vss.decode_account(ADMIN_3)
    assert len(key) == 32 and vss.encode_account(key) == ADMIN_3
    with pytest.raises(ValueError, match="checksum"):
        vss.decode_account(ADMIN_3[:-1] + ("A" if ADMIN_3[-1] != "A" else "B"))
    with pytest.raises(ValueError, match="not an account"):
        vss.decode_account(load()["accounts"]["admin"]["contract_id"])


def test_hash_is_order_independent(capsys: pytest.CaptureFixture[str]) -> None:
    keys = [s["public_key"] for s in load()["accounts"]["admin"]["signers"]]
    assert vss.main(["hash", "--threshold", "2", *keys]) == 0
    forward = capsys.readouterr().out.strip()
    assert vss.main(["hash", "--threshold", "2", *reversed(keys)]) == 0
    assert (
        capsys.readouterr().out.strip() == forward == load()["accounts"]["admin"]["signer_set_hash"]
    )
    assert vss.main(["hash", "--threshold", "3", *keys]) == 0
    assert capsys.readouterr().out.strip() != forward


def test_committed_record_passes(
    tmp_path: pathlib.Path, capsys: pytest.CaptureFixture[str]
) -> None:
    record = load()
    assert run(record, tmp_path, fake_chain(record)) == 0
    out = capsys.readouterr().out
    assert "FAIL" not in out and "all checks passed" in out
    assert run(record, tmp_path, None, "--offline") == 0


def test_stale_record_fails_against_rotated_chain(
    tmp_path: pathlib.Path, capsys: pytest.CaptureFixture[str]
) -> None:
    chain = fake_chain(load())
    stale = load()
    admin = stale["accounts"]["admin"]
    admin["signers"][2] = {"seat": "admin-3", "public_key": ADMIN_3}
    admin["signer_set_hash"] = vss.signer_set_hash(
        2, [vss.decode_account(s["public_key"]) for s in admin["signers"]]
    )
    assert run(stale, tmp_path, chain) == 1
    out = capsys.readouterr().out
    assert "[FAIL] admin: on-chain signer set equals the ceremony record" in out
    assert "[FAIL] admin: signer-set hash recomputed from chain matches" in out


@pytest.mark.parametrize(
    ("mutate", "failure"),
    [
        (lambda r: r["accounts"]["admin"].update(threshold=1), "threshold 1-of-3 within policy"),
        (lambda r: r["accounts"]["admin"].update(threshold=3), "threshold 3-of-3 within policy"),
        (
            lambda r: r["accounts"]["admin"].update(signer_set_hash="00" * 32),
            "recorded signer-set hash",
        ),
        (
            lambda r: r["accounts"]["guardian"]["signers"].__setitem__(
                0, r["accounts"]["admin"]["signers"][0]
            ),
            "admin and guardian share no keys",
        ),
        (
            lambda r: r["accounts"]["admin"]["signers"].append(
                r["accounts"]["admin"]["signers"][0]
            ),
            "admin: no duplicate keys",
        ),
        (
            lambda r: r["accounts"]["admin"]["signers"][0].update(public_key=ADMIN_3[:-1] + "A"),
            "admin-1 public key",
        ),
    ],
)
def test_offline_policy_checks(
    tmp_path: pathlib.Path,
    capsys: pytest.CaptureFixture[str],
    mutate: Any,
    failure: str,
) -> None:
    record = load()
    mutate(record)
    assert run(record, tmp_path, None, "--offline") == 1
    assert failure in "\n".join(
        line for line in capsys.readouterr().out.splitlines() if line.startswith("[FAIL]")
    )


def test_chain_drift_is_reported(
    tmp_path: pathlib.Path, capsys: pytest.CaptureFixture[str]
) -> None:
    record = load()
    honest = fake_chain(record)

    def drifted(contract: str, call: str) -> Any:
        if call == "get_admin":
            return "CATTACKER"
        if call.startswith("has_role"):
            return False
        result = honest(contract, call)
        if call == "get_policy":
            result["role_requirements"] = [{"role": "ops", "minimum": 1}]
        return result

    assert run(record, tmp_path, drifted) == 1
    out = capsys.readouterr().out
    assert "[FAIL] attester-registry: admin is the admin account: CATTACKER" in out
    assert "[FAIL] attestation-registry: guardian account holds Guardian" in out
    assert "[FAIL] admin: policy is plain 2-of-n" in out

    def passkey(contract: str, call: str) -> Any:
        return [{"Secp256r1": "04" * 65}] if call == "get_signers" else honest(contract, call)

    assert run(record, tmp_path, passkey) == 1
    assert "only Ed25519 ceremony keys" in capsys.readouterr().out


def test_stellar_cli_invocation(monkeypatch: pytest.MonkeyPatch, tmp_path: pathlib.Path) -> None:
    calls: list[list[str]] = []

    def fake_run(cmd: list[str], **_: Any) -> subprocess.CompletedProcess[str]:
        calls.append(cmd)
        return completed('"CABC"')  # type: ignore[no-any-return]

    monkeypatch.setattr(subprocess, "run", fake_run)
    invoke = vss.stellar_invoke("testnet", "ops")
    assert invoke("CXYZ", "has_role --role Guardian --account CG") == "CABC"
    assert calls[0] == [
        "stellar",
        "contract",
        "invoke",
        "--id",
        "CXYZ",
        "--network",
        "testnet",
        "--source",
        "ops",
        "--send=no",
        "--",
        "has_role",
        "--role",
        "Guardian",
        "--account",
        "CG",
    ]


def test_online_requires_source(tmp_path: pathlib.Path, monkeypatch: pytest.MonkeyPatch) -> None:
    monkeypatch.delenv("STELLAR_ACCOUNT", raising=False)
    path = tmp_path / "r.json"
    path.write_text(json.dumps(load()))
    with pytest.raises(SystemExit):
        vss.main(["verify", str(path)])
