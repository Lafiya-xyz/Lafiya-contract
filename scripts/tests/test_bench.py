"""Tests for scripts/bench_history.py and scripts/bench_bisect.py.

A throwaway git repo stands in for the contracts. Each commit carries a
`metrics.json` in lafiya-bench's output shape, and `cat metrics.json` stands
in for the build-and-measure step. That gives the bisect a synthetic
regression at a known commit.
"""

import json
import pathlib
import subprocess
from collections.abc import Sequence
from typing import Any

import bench_bisect
import bench_history
import pytest

KEY = "attester-registry/add_attester/cpu_instructions"
CAT = "cat metrics.json"
CFG: dict[str, Any] = json.loads(bench_history.DEFAULT_CONFIG.read_text())


def sh(repo: pathlib.Path, *args: str) -> str:
    return subprocess.run(
        ["git", "-C", str(repo), *args], check=True, capture_output=True, text=True
    ).stdout.strip()


def make_repo(tmp_path: pathlib.Path, values: list[int | None]) -> tuple[pathlib.Path, list[str]]:
    """One commit per value; None writes a commit that cannot be measured."""
    repo = tmp_path / "repo"
    repo.mkdir()
    sh(repo, "init", "-q", "-b", "main")
    sh(repo, "config", "user.email", "bench@example.invalid")
    sh(repo, "config", "user.name", "bench")
    commits = []
    for i, value in enumerate(values):
        body = (
            f"not json {i}"
            if value is None
            else json.dumps({"metrics": {KEY: value}, "errors": {}})
        )
        (repo / "metrics.json").write_text(body)
        sh(repo, "add", "metrics.json")
        sh(repo, "commit", "-q", "--allow-empty", "-m", f"commit {i}")
        commits.append(sh(repo, "rev-parse", "HEAD"))
    return repo, commits


@pytest.mark.parametrize(
    ("culprit", "skips"),
    [(7, []), (1, []), (11, []), (7, [5, 6]), (7, [7])],
)
def test_bisect_finds_synthetic_regression(
    tmp_path: pathlib.Path, culprit: int, skips: list[int]
) -> None:
    values: list[int | None] = [1000 + i if i < culprit else 1200 + i for i in range(12)]
    for i in skips:
        values[i] = None
    repo, commits = make_repo(tmp_path, values)
    b = bench_bisect.Bisector(repo, KEY, CAT)
    first, reference = bench_bisect.bisect(b, commits[0], commits[-1], 0.05)
    assert reference == 1000
    # A skipped culprit cannot be pinned; the next measurable commit is named.
    expected = culprit + 1 if culprit in skips else culprit
    assert first == commits[expected]
    # Binary search: far fewer measurements than commits.
    assert len(b.cache) <= 7


def test_bisect_cli(tmp_path: pathlib.Path, capsys: pytest.CaptureFixture[str]) -> None:
    repo, commits = make_repo(tmp_path, [100, 100, 100, None, 150, 151])
    argv = ["--repo", str(repo), "--good", commits[0], "--bad", commits[-1]]
    assert bench_bisect.main([*argv, "--metric", KEY, "--measure-cmd", CAT]) == 0
    out = capsys.readouterr().out
    assert "first regressed commit" in out and "commit 4" in out
    assert "100 -> 150 (+50.0%)" in out
    assert (
        bench_bisect.main([*argv, "--metric", KEY, "--measure-cmd", CAT, "--threshold", "1"]) == 1
    )
    assert "no regression" in capsys.readouterr().out
    with pytest.raises(SystemExit):
        bench_bisect.main([*argv, "--metric", "missing", "--measure-cmd", CAT])
    with pytest.raises(SystemExit):
        bench_bisect.main(
            [
                "--repo",
                str(repo),
                "--good",
                commits[-1],
                "--bad",
                commits[-1],
                "--metric",
                KEY,
                "--measure-cmd",
                CAT,
            ]
        )


def points(values: Sequence[float]) -> list[tuple[str, float]]:
    return [(f"c{i:02d}", v) for i, v in enumerate(values)]


def test_single_spike_does_not_alert() -> None:
    alert, events = bench_history.evaluate(points([100] * 10 + [200] + [100] * 5), KEY, CFG)
    assert alert is None and events == []


def test_sustained_step_raises_and_holds() -> None:
    alert, events = bench_history.evaluate(points([100] * 10 + [110] * 30), KEY, CFG)
    assert alert is not None
    assert alert.since == "c10" and alert.reference == 100 and alert.current == 110
    # The reference is frozen while raised, so the alert does not age out.
    assert [e.kind for e in events] == ["raise"]


def test_gradual_drift_alerts() -> None:
    # +1% per commit never trips a 5% per-PR budget, but adds up.
    values = [100.0] * 10 + [100 * 1.01**i for i in range(1, 30)]
    alert, _ = bench_history.evaluate(points(values), KEY, CFG)
    assert alert is not None and alert.change > CFG["tolerance"]


def test_hysteresis_clears_only_when_back_within_clear_tolerance() -> None:
    # 104 is inside the raise tolerance but outside the clear tolerance, so
    # the alert holds. It clears once usage is back near 100.
    values = [100] * 10 + [110] * 5 + [104] * 5 + [100] * 3 + [104] * 5
    alert, events = bench_history.evaluate(points(values), KEY, CFG)
    assert [e.kind for e in events] == ["raise", "clear"]
    assert events[1].commit == "c22"
    assert alert is None


def test_zero_reference() -> None:
    assert bench_history.ratio(0, 0) == 0
    assert bench_history.ratio(5, 0) == float("inf")
    alert, _ = bench_history.evaluate(points([0] * 5 + [64] * 5), KEY, CFG)
    assert alert is not None


def entry(i: int, metrics: dict[str, int]) -> dict[str, Any]:
    return {"commit": f"{i:040x}", "position": i, "subject": f"c{i}", "metrics": metrics}


def test_acknowledged_rebaselines() -> None:
    entries = [entry(i, {KEY: 100 if i < 10 else 120}) for i in range(20)]
    active, _ = bench_history.all_alerts(entries, CFG)
    assert [a.metric for a in active] == [KEY]
    acked = {**CFG, "acknowledged": {KEY: f"{10:040x}"[:12]}}
    active, _ = bench_history.all_alerts(entries, acked)
    assert active == []


def test_metric_selection_ignores_untracked() -> None:
    entries = [entry(0, {KEY: 1, "x/y/not_tracked": 2, "x/wasm/size_bytes": 3})]
    assert bench_history.metric_keys(entries, CFG) == [KEY, "x/wasm/size_bytes"]


def test_backfill_record_alerts_render(
    tmp_path: pathlib.Path, capsys: pytest.CaptureFixture[str]
) -> None:
    repo, commits = make_repo(tmp_path, [100] * 8 + [None] + [130] * 6)
    history = tmp_path / "history"
    base = ["--repo", str(repo)]
    assert (
        bench_history.main(
            [*base, "backfill", "--history", str(history), "--count", "10", "--measure-cmd", CAT]
        )
        == 0
    )
    assert "10 commits, 1 skipped" in capsys.readouterr().out
    # Only missing commits are measured again; the older ones are new here.
    assert (
        bench_history.main(
            [*base, "backfill", "--history", str(history), "--count", "50", "--measure-cmd", CAT]
        )
        == 0
    )
    loaded = bench_history.load_history(history)
    assert len(loaded) == 14
    assert [e["position"] for e in loaded] == sorted(e["position"] for e in loaded)

    metrics = tmp_path / "m.json"
    metrics.write_text(json.dumps({"metrics": {KEY: 131}, "errors": {}}))
    assert (
        bench_history.main(
            [
                *base,
                "record",
                "--history",
                str(history),
                "--commit",
                commits[-1],
                "--metrics",
                str(metrics),
            ]
        )
        == 0
    )
    assert bench_history.load_history(history)[-1]["metrics"][KEY] == 131

    summary, report = tmp_path / "alerts.md", tmp_path / "alerts.json"
    assert (
        bench_history.main(
            [
                *base,
                "alerts",
                "--history",
                str(history),
                "--json",
                str(report),
                "--markdown",
                str(summary),
            ]
        )
        == 0
    )
    assert json.loads(report.read_text())["active"][0]["metric"] == KEY
    assert f"`{KEY}`" in summary.read_text() and "+31.0%" in summary.read_text()

    page = tmp_path / "site" / "index.html"
    assert bench_history.main([*base, "render", "--history", str(history), "--out", str(page)]) == 0
    html = page.read_text()
    assert "15 commits" not in html and "14 commits" in html
    assert "1 active drift alert(s)" in html and "</script>" in html


def test_alerts_markdown_when_quiet() -> None:
    assert bench_history.alerts_markdown([]) == "No sustained benchmark drift.\n"


def test_render_empty_history() -> None:
    assert "0 commits; 0 active" in bench_history.render_page([], CFG)


def test_measure_failure_is_a_skip(tmp_path: pathlib.Path) -> None:
    repo, commits = make_repo(tmp_path, [1])
    assert bench_history.measure(repo, commits[0], "false") is None
    assert bench_history.measure(repo, commits[0], CAT) == {"metrics": {KEY: 1}, "errors": {}}
