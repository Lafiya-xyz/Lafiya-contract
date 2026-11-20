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
        self.restores = []

    def live_until(self, entry):
        return self.latest, self.live[entry["id"]]

    def extend(self, entry, ledgers, source):
        self.extensions.append(entry["id"])
        self.live[entry["id"]] = self.latest + ledgers
        return entry["est_cost_stroops"]

    def restore(self, entry, ledgers, source):
        self.restores.append(entry["id"])
        self.live[entry["id"]] = self.latest + ledgers
        return entry["est_cost_stroops"]


def _archived(chain, entry):
    """True when the entry's TTL has lapsed (latest >= live_until)."""
    latest, live = chain.live_until(entry)
    return latest >= live


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


def test_archived_entry_is_detected(tmp_path):
    chain = Chain({"a": 0})  # already lapsed at ledger 0
    entry = CFG["entries"][0]
    assert _archived(chain, entry)
    logs = []
    mod.run({**CFG, "entries": [entry]}, chain, chain.extend, tmp_path / "s.json", False, logs.append)
    assert any(r["event"] == "archived" and r["id"] == "a" for r in logs)


def test_archived_entry_is_restored(tmp_path):
    chain = Chain({"a": 0})
    cfg = {**CFG, "entries": CFG["entries"][:1], "daily_cap_stroops": 10_000}
    logs = []
    rc = mod.run(cfg, chain, chain.extend, tmp_path / "s.json", False, logs.append,
                 restore=chain.restore)
    assert chain.restores == ["a"]
    assert chain.extensions == []  # restored, not merely extended
    assert not _archived(chain, CFG["entries"][0])
    assert any(r["event"] == "restored" and r["id"] == "a" for r in logs)
    assert rc == 0


def test_restore_respects_spend_cap(tmp_path):
    chain = Chain({"a": 0, "b": 0, "c": 0})
    logs = []
    rc = mod.run(CFG, chain, chain.extend, tmp_path / "s.json", False, logs.append,
                 restore=chain.restore)
    assert chain.restores == ["a", "b"]  # third restore would exceed the 250 cap
    assert any(r["event"] == "cap_reached" for r in logs)
    assert rc == 2


def test_dry_run_does_not_restore(tmp_path):
    chain = Chain({"a": 0})
    cfg = {**CFG, "entries": CFG["entries"][:1]}
    mod.run(cfg, chain, chain.extend, tmp_path / "s.json", True, lambda r: None,
            restore=chain.restore)
    assert chain.restores == []


def test_restore_each_persistent_key_class(tmp_path):
    """Attestation, attester info and work item entries each archive and restore."""
    classes = [{"id": "attestation", "contract": "C1", "est_cost_stroops": 100},
               {"id": "attester_info", "contract": "C2", "est_cost_stroops": 100},
               {"id": "work_item", "contract": "C3", "est_cost_stroops": 100}]
    chain = Chain({c["id"]: 0 for c in classes})
    cfg = {**CFG, "entries": classes, "daily_cap_stroops": 10_000}
    for c in classes:
        assert _archived(chain, c)
    rc = mod.run(cfg, chain, chain.extend, tmp_path / "s.json", False, lambda r: None,
                 restore=chain.restore)
    assert chain.restores == ["attestation", "attester_info", "work_item"]
    for c in classes:
        assert not _archived(chain, c)
    assert rc == 0
