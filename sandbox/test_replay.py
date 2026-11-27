"""Tests for replay.py. Run: python3 -m unittest discover -s sandbox"""

import json
import os
import subprocess
import sys
import tempfile
import unittest

HERE = os.path.dirname(os.path.abspath(__file__))
sys.path.insert(0, HERE)

import replay  # noqa: E402

EVENTS = os.path.join(HERE, "..", "dashboard", "fixtures", "events.jsonl")
REGIONS = os.path.join(HERE, "..", "dashboard", "fixtures", "attesters.json")


def render(lo, hi):
    with open(REGIONS, encoding="utf-8") as f:
        regions = json.load(f)
    return replay.render(replay.load_window(EVENTS, lo, hi), regions)


class Replay(unittest.TestCase):
    def test_1000_event_window_is_reproducible(self):
        window = replay.load_window(EVENTS, 1000, 60000)
        self.assertGreaterEqual(len(window), 1000)
        self.assertEqual(render(1000, 60000), render(1000, 60000))

    def test_input_order_does_not_matter(self):
        window = replay.load_window(EVENTS, 1100, 4100)
        with tempfile.NamedTemporaryFile("w", suffix=".jsonl", delete=False) as f:
            for e in reversed(window):
                f.write(json.dumps(e) + "\n")
        try:
            shuffled = replay.render(replay.load_window(f.name, 1100, 4100), {})
        finally:
            os.unlink(f.name)
        self.assertEqual(shuffled, replay.render(window, {}))

    def test_committed_regression_scenario_is_current(self):
        path = os.path.join(HERE, "scenarios", "replay-regression.toml")
        with open(path, encoding="utf-8") as f:
            self.assertEqual(f.read(), render(1100, 4100) + "\n")

    def test_revoked_record_is_kept(self):
        self.assertIn("[[revocations]]", render(1100, 4100))

    def test_final_attester_status_wins(self):
        def ev(ledger, name, topics, data=""):
            return {"ledger": ledger, "name": name, "topics": topics, "data": data}
        text = replay.render([ev(1, "attester_added", ["GA"]),
                              ev(2, "attestation_recorded", ["r1"], "GA"),
                              ev(3, "attester_removed", ["GA"])], {})
        self.assertIn('status = "removed"', text)
        self.assertIn("[[attestations]]", text)

    def test_constructor_deploy_has_no_initialize_event(self):
        def ev(ledger, name, topics, data=""):
            return {"ledger": ledger, "name": name, "topics": topics, "data": data}
        text = replay.render([ev(1, "incentive_pool_deployed", ["GADMIN"]),
                              ev(2, "attester_added", ["GA"])], {})
        self.assertIn("incentive_pool_deployed", text)
        self.assertNotIn("initialize", text)

    def test_payout_cancellation_is_replayed(self):
        def ev(ledger, name, topics, data=""):
            return {"ledger": ledger, "name": name, "topics": topics, "data": data}
        text = replay.render([ev(1, "attester_added", ["GA"]),
                              ev(2, "payout_address_scheduled", ["GA"], "GB"),
                              ev(3, "payout_address_cancelled", ["GA"])], {})
        self.assertIn("payout_address_cancelled", text)
        self.assertNotIn('payout_address = "GB"', text)

    def test_drift_resolution_is_replayed(self):
        def ev(ledger, name, topics, data=""):
            return {"ledger": ledger, "name": name, "topics": topics, "data": data}
        text = replay.render([ev(1, "attester_added", ["GA"]),
                              ev(2, "drift_detected", []),
                              ev(3, "reservation_cancelled", ["r1"])], {})
        self.assertIn("drift_detected", text)
        self.assertIn("reservation_cancelled", text)


if __name__ == "__main__":
    unittest.main()
