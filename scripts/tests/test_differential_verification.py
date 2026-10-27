import json
import sys
from pathlib import Path

import pytest

import differential_verification as dv
from verification_model import TABLE

SCRIPTS = Path(__file__).resolve().parents[1]

# An implementation that agrees with the reference except that it ignores
# supersession: the harness must catch it and shrink to `superseded: true`.
BUGGY = f"""
import json, sys
sys.path.insert(0, {str(SCRIPTS)!r})
from verification_model import explain
for line in sys.stdin:
    s = json.loads(line)
    v, r, q = explain({{**s, "superseded": False}})
    print(json.dumps({{"verdict": v, "reason": r, "qualifier": q}}))
"""

CORRECT = BUGGY.replace('"superseded": False', '"superseded": s["superseded"]')


def impl(tmp_path: Path, name: str, source: str) -> str:
    path = tmp_path / f"{name}.py"
    path.write_text(source)
    return f"{name}={sys.executable} {path}"


def test_domains_come_from_the_model() -> None:
    table = json.loads(TABLE.read_text())
    domains = dv.input_domains(table)
    assert domains["attester_status"] == ["active", "removed", "suspended"]
    assert domains["attestation_read"] == ["failed", "ok"]
    assert domains["offline"] == [False, True]
    assert set(domains) == set(table["default_input"])


def test_generation_is_deterministic_per_seed() -> None:
    table = json.loads(TABLE.read_text())
    assert dv.generate(table, [7], 50, 0) == dv.generate(table, [7], 50, 0)
    assert dv.generate(table, [7], 50, 0) != dv.generate(table, [8], 50, 0)


def test_agreeing_implementation_passes(tmp_path: Path) -> None:
    args = ["--seeds", "1", "--samples", "50", "--impl", impl(tmp_path, "ok", CORRECT)]
    assert dv.main(args) == 0


def test_discrepancy_fails_with_minimal_scenario(
    tmp_path: Path, capsys: pytest.CaptureFixture[str]
) -> None:
    code = dv.main(["--seeds", "1", "--samples", "50", "--impl", impl(tmp_path, "bug", BUGGY)])
    out = capsys.readouterr().out
    assert code == 1
    assert "DISCREPANCY (bug)" in out
    assert 'input: {"superseded": true}' in out


def test_bad_impl_argument() -> None:
    with pytest.raises(SystemExit):
        dv.parse_impls(["no-command"])
