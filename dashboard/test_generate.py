"""Tests for the dashboard generator on synthetic data.

Run: python3 -m unittest discover -s dashboard
"""

import filecmp
import json
import os
import random
import shutil
import subprocess
import sys
import tempfile
import unittest

HERE = os.path.dirname(os.path.abspath(__file__))
sys.path.insert(0, HERE)

import generate as g  # noqa: E402
import snapshot  # noqa: E402

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


class SmallCellAttacks(unittest.TestCase):
    """Property tests on synthetic small-cell datasets (seeded, reproducible)."""

    METRICS = ("new", "reverified", "revoked")

    def test_random_small_cells_never_leak(self):
        rng = random.Random(577)
        for k in (2, 5, 10, 25):
            for _ in range(200):
                cells = {
                    (f"2026-W{w:02d}", f"r{r}"): {m: rng.choice([0, 1, 2, k - 1, k, k + 1, 3 * k])
                                                  for m in self.METRICS}
                    for w in range(1, 4) for r in range(rng.randint(1, 5))
                }
                out = g.suppress(cells, k)
                for v in out.values():
                    for m in self.METRICS:
                        self.assertTrue(v[m] is None or v[m] == 0 or v[m] >= k, (k, v))

    def test_complementary_suppression_blocks_subtraction(self):
        # Attacker knows the weekly total (e.g. from chain data) and 2 of 3 cells.
        cells = {("2026-W36", "a"): {"new": 4, "reverified": 0, "revoked": 0},
                 ("2026-W36", "b"): {"new": 30, "reverified": 0, "revoked": 0},
                 ("2026-W36", "c"): {"new": 12, "reverified": 0, "revoked": 0}}
        out = g.suppress(cells, 10)
        hidden = [r for (_, r), v in out.items() if v["new"] is None]
        self.assertGreaterEqual(len(hidden), 2)

    def test_open_week_is_not_published(self):
        # Two snapshots of the same growing week would differ by 7 < k records.
        def snap(n):
            closed = [ev(i, "attestation_recorded", [f"c{i}"], "GA") for i in range(12)]
            growing = [ev(1000 + i, "attestation_recorded", [f"g{i}"], "GA") for i in range(n)]
            for e in growing:
                e["closed_at"] = "2026-09-09T00:00:00Z"  # newer ISO week, still open
            return g.build(closed + growing, {"GA": "lagos"}, [], {})["verification_activity"]
        a, b = snap(12), snap(19)
        self.assertEqual(a, b)
        self.assertEqual({row["week"] for row in a}, {"2026-W36"})

    def test_threshold_is_configurable(self):
        cells = {("2026-W36", "a"): {"new": 5, "reverified": 0, "revoked": 0}}
        self.assertEqual(g.suppress(cells, 5)[("2026-W36", "a")]["new"], 5)
        self.assertIsNone(g.suppress(cells, 6)[("2026-W36", "a")]["new"])


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


@unittest.skipUnless(shutil.which("openssl"), "openssl CLI required")
class SignedSnapshot(unittest.TestCase):
    def setUp(self):
        self.tmp = tempfile.TemporaryDirectory()
        t = self.tmp.name
        self.key, self.pub = os.path.join(t, "k.pem"), os.path.join(t, "k.pub")
        subprocess.run(["openssl", "genpkey", "-algorithm", "ed25519", "-out", self.key], check=True)
        subprocess.run(["openssl", "pkey", "-in", self.key, "-pubout", "-out", self.pub], check=True)
        self.site = os.path.join(t, "site")
        g.main(ARGS + ["--out", self.site])
        self.data = os.path.join(self.site, "data")
        snapshot.sign(self.data, self.key)

    def tearDown(self):
        self.tmp.cleanup()

    def test_valid_snapshot_reproduces_from_public_data(self):
        self.assertEqual(snapshot.verify(self.data, self.pub, ARGS), [])

    def test_manifest_fields(self):
        with open(os.path.join(self.data, "snapshot.json")) as f:
            m = json.load(f)
        self.assertEqual(m["ledger_height"], 60000)
        self.assertEqual(len(m["index_version"]), 64)
        self.assertEqual(m["content_hash"], snapshot.content_hash(self.data))

    def test_tampered_data_is_detected(self):
        with open(os.path.join(self.data, "attesters.csv"), "a") as f:
            f.write("tamper\n")
        self.assertTrue(snapshot.verify(self.data, self.pub))

    def test_wrong_key_is_detected(self):
        other = os.path.join(self.tmp.name, "o.pem")
        subprocess.run(["openssl", "genpkey", "-algorithm", "ed25519", "-out", other], check=True)
        pub2 = os.path.join(self.tmp.name, "o.pub")
        subprocess.run(["openssl", "pkey", "-in", other, "-pubout", "-out", pub2], check=True)
        self.assertTrue(any("signature" in m for m in snapshot.verify(self.data, pub2)))


if __name__ == "__main__":
    unittest.main()
