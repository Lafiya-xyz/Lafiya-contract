"""Tests for check_error_docs.py and gen_events_doc.py."""

import pathlib
from typing import Any

import pytest

import check_error_docs
import gen_events_doc

CFG: dict[str, pathlib.Path] = {"wasm_path": pathlib.Path("x.wasm")}

ERROR_ENUM = {
    "kind": "udt_error_enum_v0",
    "name": "Error",
    "spec": {"cases": [{"name": "NotInitialized", "value": 1}, {"name": "Paused", "value": 2}]},
}
EVENT = {
    "kind": "event_v0",
    "name": "AttesterAdded",
    "spec": {
        "name": "AttesterAdded",
        "prefix_topics": ["attester_added"],
        "data_format": "map",
        "params": [
            {"name": "attester", "type": "address", "location": "topic_list"},
            {"name": "region", "type": {"option": {"value_type": "symbol"}}, "location": "data"},
        ],
    },
}

DOCS = """# Errors

## `demo`

| Code | Variant | Description |
|---|---|---|
| `1` | `NotInitialized` | x |
| `2` | `Paused` | y |

## `other`

| `1` | `Other` | z |
"""


@pytest.fixture
def one_contract(monkeypatch: pytest.MonkeyPatch) -> None:
    for mod in (check_error_docs, gen_events_doc):
        monkeypatch.setattr(mod, "CONTRACTS", {"demo": dict(CFG)})
    monkeypatch.setattr(check_error_docs, "extract", lambda _: [ERROR_ENUM])
    monkeypatch.setattr(gen_events_doc, "extract", lambda _: [EVENT])


# --- check_error_docs ------------------------------------------------------


def test_parse_docs_table_is_scoped_to_section() -> None:
    assert check_error_docs.parse_docs_table(DOCS, "demo") == {1: "NotInitialized", 2: "Paused"}
    assert check_error_docs.parse_docs_table(DOCS, "other") == {1: "Other"}
    assert check_error_docs.parse_docs_table(DOCS, "missing") is None


def test_error_docs_match(one_contract: None, capsys: pytest.CaptureFixture[str]) -> None:
    assert check_error_docs.check_one("demo", CFG, DOCS)
    assert "OK -- 2 error code(s)" in capsys.readouterr().out


@pytest.mark.parametrize(
    ("docs", "message"),
    [
        (DOCS.replace("| `2` | `Paused` | y |\n", ""), "missing from docs"),
        (DOCS.replace("`Paused`", "`Halted`"), "documented as `Halted`"),
        (DOCS.replace("| `2` |", "| `3` | `Gone` | g |\n| `2` |"), "no longer in the Wasm"),
        (DOCS.replace("## `demo`", "## `renamed`"), "no `## \\`demo\\`` section"),
    ],
)
def test_error_docs_drift(
    one_contract: None, docs: str, message: str, capsys: pytest.CaptureFixture[str]
) -> None:
    assert not check_error_docs.check_one("demo", CFG, docs)
    assert message in capsys.readouterr().out


def test_wasm_without_error_enum(monkeypatch: pytest.MonkeyPatch) -> None:
    monkeypatch.setattr(check_error_docs, "extract", lambda _: [EVENT])
    assert check_error_docs.wasm_error_cases(CFG) == {}


def test_error_docs_main(
    one_contract: None, tmp_path: pathlib.Path, monkeypatch: pytest.MonkeyPatch
) -> None:
    docs = tmp_path / "error-codes.md"
    docs.write_text(DOCS)
    monkeypatch.setattr(check_error_docs, "ERROR_CODES_MD", docs)
    monkeypatch.setattr("sys.argv", ["check_error_docs.py"])
    with pytest.raises(SystemExit) as exc:
        check_error_docs.main()
    assert exc.value.code == 0
    monkeypatch.setattr("sys.argv", ["check_error_docs.py", "nope"])
    with pytest.raises(SystemExit, match="unknown contract"):
        check_error_docs.main()


# --- gen_events_doc ---------------------------------------------------------


def test_render_contract(one_contract: None) -> None:
    text = gen_events_doc.render_contract("demo", CFG)
    assert "### `AttesterAdded`" in text
    assert "- **Prefix topics:** `attester_added`" in text
    assert "| `region` | `Option<Symbol>` | data |" in text


def test_render_contract_without_events(monkeypatch: pytest.MonkeyPatch) -> None:
    monkeypatch.setattr(gen_events_doc, "extract", lambda _: [ERROR_ENUM])
    assert "_No events declared._" in gen_events_doc.render_contract("demo", CFG)


def test_events_doc_write_then_check(
    one_contract: None, tmp_path: pathlib.Path, monkeypatch: pytest.MonkeyPatch
) -> None:
    doc = tmp_path / "events.md"
    monkeypatch.setattr(gen_events_doc, "EVENTS_MD", doc)

    monkeypatch.setattr("sys.argv", ["gen_events_doc.py", "--check"])
    with pytest.raises(SystemExit):
        gen_events_doc.main()

    monkeypatch.setattr("sys.argv", ["gen_events_doc.py"])
    gen_events_doc.main()
    assert doc.read_text().startswith("# Contract Event Schemas")

    monkeypatch.setattr("sys.argv", ["gen_events_doc.py", "--check"])
    gen_events_doc.main()
