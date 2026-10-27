import json
import os
import tempfile
import unittest

import status_page


def rec(ts, loc, ok):
    return {"ts": ts, "location": loc, "deployment": "testnet",
            "probes": [{"probe": "rpc_liveness", "ok": ok, "ms": 1, "detail": ""}]}


class StatusTest(unittest.TestCase):
    def test_single_location_failure_is_not_down(self):
        cells = status_page.summarize([rec(1000, "eu", False), rec(1010, "us", True)])
        self.assertEqual(list(cells.values())[0], {3: True})

    def test_all_locations_failing_is_down(self):
        cells = status_page.summarize([rec(1000, "eu", False), rec(1010, "us", False)])
        self.assertEqual(list(cells.values())[0], {3: False})

    def test_generates_page(self):
        with tempfile.TemporaryDirectory() as d:
            h = os.path.join(d, "h.jsonl")
            open(h, "w").write(json.dumps(rec(1000, "eu", True)) + "\n")
            status_page.main(["--history", h, "--out", d, "--now", "1100"])
            self.assertIn("rpc_liveness", open(os.path.join(d, "index.html")).read())


if __name__ == "__main__":
    unittest.main()
