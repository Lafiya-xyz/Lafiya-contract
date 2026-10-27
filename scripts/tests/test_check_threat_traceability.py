"""Tests for scripts/check_threat_traceability.py."""

import copy
import pathlib
from typing import Any

import check_threat_traceability as ctt
import pytest
import yaml

DOC = """# Model

### 4.1 Indexer

| T-901 | S | thing |
"""

ENTRY: dict[str, Any] = {
    "title": "thing",
    "component": "demo",
    "stride": "S,T",
    "boundary": "TB4 -> demo",
    "dfd": "model.md#41-indexer",
    "mitigation": "checked",
    "owner": "@someone",
    "status": "mitigated",
    "code": ["src/lib.rs::guard"],
    "tests": ["src/lib.rs::guard_rejects", "test/app.test.ts::rejects bad input"],
}


@pytest.fixture
def root(tmp_path: pathlib.Path) -> pathlib.Path:
    (tmp_path / "src").mkdir()
    (tmp_path / "test").mkdir()
    (tmp_path / "model.md").write_text(DOC)
    (tmp_path / "src" / "lib.rs").write_text("fn guard() {}\n#[test]\nfn guard_rejects() {}\n")
    (tmp_path / "test" / "app.test.ts").write_text('it("rejects bad input", () => {});\n')
    return tmp_path


def errors_for(root: pathlib.Path, **changes: Any) -> list[str]:
    entry = copy.deepcopy(ENTRY)
    for key, value in changes.items():
        if value is None:
            entry.pop(key, None)
        else:
            entry[key] = value
    errors, _ = ctt.check({"threats": {"T-901": entry}}, root)
    return errors


def test_valid_entry_passes(root: pathlib.Path) -> None:
    assert errors_for(root) == []


@pytest.mark.parametrize(
    ("changes", "expected"),
    [
        ({"owner": None}, "missing owner"),
        ({"status": "done"}, "status 'done'"),
        ({"stride": "S,X"}, "is not STRIDE letters"),
        ({"dfd": "missing.md#41-indexer"}, "dfd missing.md does not exist"),
        ({"dfd": "model.md#99-nowhere"}, "dfd section #99-nowhere not found"),
        ({"code": ["src/gone.rs"]}, "src/gone.rs does not exist"),
        ({"code": ["src/lib.rs::absent"]}, "'absent' not found in src/lib.rs"),
        ({"tests": ["test/app.test.ts::some other title"]}, "'some other title' not found"),
        ({"tests": ["src/lib.rs"]}, "must be `path::name`"),
        ({"code": [], "tests": []}, "must point at code or tests"),
        ({"status": "partial"}, "partial threat needs `gap`"),
        ({"status": "open", "gap": "x"}, "open threat needs `issue`"),
        ({"status": "accepted"}, "accepted threat needs a `rationale`"),
    ],
)
def test_each_rule_fires(root: pathlib.Path, changes: dict[str, Any], expected: str) -> None:
    errors = errors_for(root, **changes)
    assert any(expected in e for e in errors), errors


def test_threat_missing_from_doc(root: pathlib.Path) -> None:
    (root / "model.md").write_text(DOC.replace("T-901", "T-902"))
    errors, _ = ctt.check({"threats": {"T-901": ENTRY}}, root)
    assert "T-901: T-901 is not listed in model.md" in errors
    assert "model.md cites T-902, which is not in the registry" in errors


def test_legacy_entries_only_warn(root: pathlib.Path) -> None:
    legacy = {"title": "old", "tests": ["guard_rejects", "never_written"]}
    errors, warnings = ctt.check({"threats": {"T-001": legacy}}, root)
    assert errors == []
    assert warnings == ["T-001: 1 legacy test(s) not written yet"]


def test_main_reports_and_exits(
    root: pathlib.Path, tmp_path: pathlib.Path, capsys: pytest.CaptureFixture[str]
) -> None:
    registry = tmp_path / "threats.yaml"
    registry.write_text(yaml.safe_dump({"threats": {"T-901": ENTRY}}))
    assert ctt.main(["--registry", str(registry), "--root", str(root)]) == 0
    assert "OK (1 threats traced" in capsys.readouterr().out
    bad = copy.deepcopy(ENTRY)
    bad["code"] = ["src/gone.rs"]
    registry.write_text(yaml.safe_dump({"threats": {"T-901": bad}}))
    assert ctt.main(["--registry", str(registry), "--root", str(root)]) == 1
    assert "1 traceability error(s)" in capsys.readouterr().out


def test_committed_registry_is_traceable() -> None:
    assert ctt.main([]) == 0


def test_python_symbols(tmp_path: pathlib.Path) -> None:
    script = tmp_path / "x.py"
    script.write_text("def test_each_rule_fires():\n    pass\n")
    assert ctt.check_ref(tmp_path, "x.py::test_each_rule_fires") is None
    assert ctt.check_ref(tmp_path, "x.py::missing") is not None
