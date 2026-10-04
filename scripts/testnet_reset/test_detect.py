#!/usr/bin/env python3
"""Unit tests for detect.classify against fixture RPC responses.

    python3 -m unittest discover -s scripts/testnet_reset
"""
import json
import unittest
from pathlib import Path

from detect import (
    CONTRACT_MISSING,
    HEALTHY,
    NETWORK_RESET,
    classify,
    contract_id_to_hash,
    instance_ledger_key,
)

FIXTURES = Path(__file__).parent / "fixtures"


def load(name):
    return json.loads((FIXTURES / name).read_text())


def run(fixture, state):
    fx = load(fixture)
    return classify(
        state,
        fx["getLatestLedger"],
        fx["getNetwork"],
        fx["getLedgerEntries"],
        fx["contract_ids"],
    )[0]


class ClassifyTest(unittest.TestCase):
    def setUp(self):
        self.state = load("state.json")

    def test_healthy(self):
        self.assertEqual(run("healthy.json", self.state), HEALTHY)

    def test_contract_missing(self):
        self.assertEqual(run("contract-missing.json", self.state), CONTRACT_MISSING)

    def test_network_reset_on_ledger_drop(self):
        self.assertEqual(run("network-reset.json", self.state), NETWORK_RESET)

    def test_network_reset_on_passphrase_change(self):
        self.state["passphrase"] = "Some Other Network"
        self.assertEqual(run("healthy.json", self.state), NETWORK_RESET)

    def test_first_run_without_state_is_not_a_reset(self):
        self.assertEqual(run("network-reset.json", None), CONTRACT_MISSING)
        self.assertEqual(run("healthy.json", None), HEALTHY)

    def test_no_configured_contracts_is_healthy(self):
        fx = load("healthy.json")
        status, _ = classify(
            self.state, fx["getLatestLedger"], fx["getNetwork"], {"entries": []}, []
        )
        self.assertEqual(status, HEALTHY)


class LedgerKeyTest(unittest.TestCase):
    def test_contract_id_round_trip(self):
        cid = load("healthy.json")["contract_ids"][0]
        self.assertEqual(contract_id_to_hash(cid), bytes([1]) * 32)
        self.assertTrue(instance_ledger_key(cid).startswith("AAAABgAAAAE"))

    def test_rejects_account_strkey(self):
        with self.assertRaises(ValueError):
            contract_id_to_hash("G" + "A" * 55)


if __name__ == "__main__":
    unittest.main()
