"""Tests for the dashboard generator on synthetic data.

Run: python3 -m unittest discover -s dashboard
"""

import filecmp
import json
import os
import subprocess
import sys
import tempfile
import unittest

HERE = os.path.dirname(os.path.abspath(__file__))
sys.path.insert(0, HERE)

import generate as g  # noqa: E402

FIX = os.path.join(HERE, "fixtures")
REG, ATT = "C" + "R" * 55, "C" + "A" * 55
ARGS = [
    "--events", os.path.join(FIX, "events.jsonl"),
    "--attesters", os.path.join(FIX, "attesters.json"),
    "--manifest", os.path.join(HERE, "..", "docs", "release-manifest", "examples", "v0.1.0-dev.json"),
    "--network", "synthetic",
    "--ledger-from", "1000",
    "--ledger-to", "60000",
    "--contracts", f"{REG},{ATT}",
]


def ev(ledger, name, topics, data="", contract=ATT):
    return {"ledger": ledger, "contract": contract, "name": name, "topics": topics,
            "data": data, "closed_at": "2026-09-02T00:00:00Z"}


class KAnonymity(unittest.TestCase):
    def test_small_cells_are_suppressed(self):
        cells = {("2026-W36", "lagos"): {"new": 3, "reverified": 0, "revoked": 0},
                 ("2026-W36", "kano"): {"new": 40, "reverified": 0, "revoked": 0},
                 ("2026-W36", "oyo"): {"new": 25, "reverified": 0, "revoked": 0}}
        out = g.suppress(cells)
        self.assertIs(out[("2026-W36", "lagos")]["new"], g.SUPPRESSED)
        # Secondary suppression hides the next-smallest cell in the week.
        self.assertIs(out[("2026-W36", "oyo")]["new"], g.SUPPRESSED)
        self.assertEqual(out[("2026-W36", "kano")]["new"], 40)

    def test_no_published_value_below_k(self):
        with tempfile.TemporaryDirectory() as out:
            g.main(ARGS + ["--out", out])
            with open(os.path.join(out, "data", "verification_activity.json")) as f:
                rows = json.load(f)
        self.assertTrue(rows)
        for row in rows:
            for m in ("new", "reverified", "revoked"):
                v = row[m]
                self.assertTrue(v is None or v == 0 or v >= g.K_ANONYMITY, row)

    def test_suppressed_cells_render_as_below_k(self):
        html = g.render_html(g.build(
            [ev(1, "attestation_recorded", ["r1"], "GA")], {"GA": "lagos"}, [],
            {"network": "t", "ledger_from": 0, "ledger_to": 2}))
        self.assertIn("&lt;10", html)


class Aggregates(unittest.TestCase):
    def test_new_vs_reverified_vs_revoked(self):
        events = [ev(1, "attestation_recorded", ["r1"], "GA"),
                  ev(2, "attestation_recorded", ["r1"], "GA"),
                  ev(3, "attestation_revoked", ["r1"])]
        cells = g.verification_activity(events, {"GA": "lagos"})
        self.assertEqual(cells[("2026-W36", "lagos")],
                         {"new": 1, "reverified": 1, "revoked": 1})

    def test_unknown_attester_region(self):
        cells = g.verification_activity([ev(1, "attestation_recorded", ["r"], "GX")], {})
        self.assertIn(("2026-W36", g.UNKNOWN_REGION), cells)

    def test_attester_lifecycle_counts(self):
        events = [ev(1, "attester_added", ["A"], contract=REG),
                  ev(2, "attester_added", ["B"], contract=REG),
                  ev(3, "attester_suspended", ["A"], contract=REG),
                  ev(4, "attester_removed", ["B"], contract=REG)]
        self.assertEqual(g.attester_counts(events),
                         [{"week": "2026-W36", "active": 0, "suspended": 1, "removed": 1}])

    def test_contract_health_flags_unknown_wasm(self):
        events = [ev(1, "admin_transferred", ["GOLD", "GNEW"], contract=REG),
                  ev(2, "upgraded", ["AB" * 32], contract=REG),
                  ev(3, "paused", ["GNEW"], contract=REG)]
        [h] = g.contract_health(events, {"cd" * 32})
        self.assertEqual(h["admin"], "GNEW")
        self.assertFalse(h["wasm_in_manifest"])
        self.assertTrue(h["paused"])
        self.assertEqual(h["last_admin_action"]["event"], "paused")


class Reproducibility(unittest.TestCase):
    """The committed fixtures/expected site is exactly what the generator
    produces from the pinned synthetic ledger range."""

    def test_regenerated_site_matches_committed_output(self):
        with tempfile.TemporaryDirectory() as tmp:
            subprocess.run([sys.executable, os.path.join(FIX, "make_synthetic.py")], check=True)
            g.main(ARGS + ["--out", tmp])
            expected = os.path.join(FIX, "expected")
            for rel in ["index.html"] + [os.path.join("data", f) for f in
                                          sorted(os.listdir(os.path.join(expected, "data")))]:
                self.assertTrue(filecmp.cmp(os.path.join(tmp, rel),
                                            os.path.join(expected, rel), shallow=False),
                                f"{rel} differs; regenerate with dashboard/README.md")


if __name__ == "__main__":
    unittest.main()
