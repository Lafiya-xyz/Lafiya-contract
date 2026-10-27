import unittest

import capacity_model as cm


class CapacityModelTest(unittest.TestCase):
    def setUp(self):
        self.model, self.fees = cm.load()
        self.scn, self.asm = self.model["scenario"], self.model["assumptions"]

    def test_reproducible(self):
        a = cm.forecast(self.scn, self.asm, self.fees)
        b = cm.forecast(self.scn, self.asm, self.fees)
        self.assertEqual(a, b)

    def test_scales_with_cards(self):
        base = cm.forecast(self.scn, self.asm, self.fees)
        scn = dict(self.scn, cards=self.scn["cards"] * 2)
        self.assertGreater(cm.forecast(scn, self.asm, self.fees)["rent_stroops_per_year"],
                           base["rent_stroops_per_year"])

    def test_top_three_drivers(self):
        self.assertEqual(len(cm.sensitivity(self.scn, self.asm, self.fees)[:3]), 3)


if __name__ == "__main__":
    unittest.main()
