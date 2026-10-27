import importlib.util
from concurrent.futures import ThreadPoolExecutor
from pathlib import Path

spec = importlib.util.spec_from_file_location(
    "ttl_keeper", Path(__file__).resolve().parent.parent / "ops" / "ttl_keeper.py"
)
mod = importlib.util.module_from_spec(spec)
spec.loader.exec_module(mod)

CFG = {"extend_below_ledgers": 1000, "alert_below_ledgers": 200, "extend_to_ledgers": 5000,
       "daily_cap_stroops": 250, "source_identity": "keeper",
       "entries": [{"id": "a", "contract": "C1", "est_cost_stroops": 100},
                   {"id": "b", "contract": "C2", "est_cost_stroops": 100},
                   {"id": "c", "contract": "C3", "est_cost_stroops": 100}]}


class Chain:
    """Fake chain: entry is archived when latest passes live_until."""

    def __init__(self, live_until):
        self.latest, self.live = 0, dict(live_until)
        self.extensions = []

    def live_until(self, entry):
        return self.latest, self.live[entry["id"]]

    def extend(self, entry, ledgers, source):
        self.extensions.append(entry["id"])
        self.live[entry["id"]] = self.latest + ledgers
        return entry["est_cost_stroops"]


def test_keeper_prevents_simulated_archival(tmp_path):
    chain = Chain({"a": 3000})
    cfg = {**CFG, "entries": CFG["entries"][:1], "daily_cap_stroops": 10_000}
    for _ in range(10):  # ledger time advances; a keeper pass runs every 1500 ledgers
        chain.latest += 1500
        mod.run(cfg, chain, chain.extend, tmp_path / "s.json", False, lambda r: None)
        assert chain.latest < chain.live["a"], "entry was archived"


def test_dry_run_does_not_extend(tmp_path):
    chain = Chain({"a": 500, "b": 5000, "c": 5000})
    mod.run(CFG, chain, chain.extend, tmp_path / "s.json", True, lambda r: None)
    assert chain.extensions == []


def test_spend_cap_enforced_and_logged(tmp_path):
    chain, logs = Chain({"a": 500, "b": 500, "c": 100}), []
    rc = mod.run(CFG, chain, chain.extend, tmp_path / "s.json", False, logs.append)
    assert chain.extensions == ["a", "b"]  # third would exceed the 250 cap
    assert any(r["event"] == "cap_reached" for r in logs)
    assert rc == 2  # entry c is still below the alert threshold


def test_idempotent_across_concurrent_instances(tmp_path):
    chain = Chain({"a": 500})
    cfg = {**CFG, "entries": CFG["entries"][:1]}
    with ThreadPoolExecutor(4) as ex:
        list(ex.map(lambda _: mod.run(cfg, chain, chain.extend, tmp_path / "s.json", False, lambda r: None), range(4)))
    assert chain.extensions == ["a"]
