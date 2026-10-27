import json
import os
import tempfile
import unittest

import feed

HERE = os.path.dirname(os.path.abspath(__file__))
EVENTS = os.path.join(HERE, "..", "fixtures", "events.jsonl")


class FeedTest(unittest.TestCase):
    def test_roundtrip_and_tamper(self):
        with tempfile.TemporaryDirectory() as d:
            self.assertEqual(feed.main(["--events", EVENTS, "--out", d]), 0)
            self.assertEqual(feed.main(["--events", EVENTS, "--out", d, "--verify"]), 0)
            f = json.load(open(os.path.join(d, "feed.json")))
            self.assertEqual(f["format"], feed.FORMAT)
            f["locale"] = "en"
            if f["entries"]:
                f["entries"][0]["summary"] = "tampered"
                json.dump(f, open(os.path.join(d, "feed.json"), "w"))
                self.assertEqual(feed.main(["--events", EVENTS, "--out", d, "--verify"]), 1)

    def test_only_privileged(self):
        f = feed.build_feed(feed.load_events(EVENTS), "https://x/tx")
        self.assertTrue(all(e["event"] in feed.PRIVILEGED for e in f["entries"]))


if __name__ == "__main__":
    unittest.main()
