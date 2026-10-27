import importlib.util
import json
from pathlib import Path

ROOT = Path(__file__).resolve().parent.parent.parent
spec = importlib.util.spec_from_file_location("resource_headroom", ROOT / "scripts" / "ops" / "resource_headroom.py")
mod = importlib.util.module_from_spec(spec)
spec.loader.exec_module(mod)

BASELINE = json.loads((ROOT / "budgets" / "baseline.json").read_text())
ACCEPTED = json.loads((ROOT / "config" / "network-limits.json").read_text())


def test_current_limits_are_quiet():
    rows, alerts = mod.evaluate(BASELINE, ACCEPTED, ACCEPTED, 0.2)
    assert rows and alerts == []


def test_injected_limit_reduction_alerts():
    reduced = {**ACCEPTED, "cpu_instructions": 100_000}  # baseline add_attester uses 125000
    _, alerts = mod.evaluate(BASELINE, reduced, ACCEPTED, 0.2)
    assert any("cpu_instructions headroom" in a for a in alerts)
    assert any("network limit cpu_instructions changed" in a for a in alerts)
    assert all(mod.PLAYBOOK in a for a in alerts)


def test_metrics_export():
    rows, alerts = mod.evaluate(BASELINE, ACCEPTED, ACCEPTED, 0.2)
    text = mod.metrics(rows, ACCEPTED, alerts)
    assert "lafiya_resource_headroom_ratio{" in text and "lafiya_headroom_alerts 0" in text
