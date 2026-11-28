"""Tests of gen_impldef.py (python3 -m unittest)."""

import copy
import unittest
from pathlib import Path

import gen_impldef as gi

ROOT = Path(__file__).resolve().parent.parent

DOCS = {
    "discrepancy": "## CIRC\n\ntext\n",
    "unknown": "- Floating point with odd operands.\n",
    "difference": "- Bits 16 and 17 of the page fail word.\n",
}
NAMES = {"CIRC", "ASH"}
CASE = {
    "name": "circ",
    "title": "CIRC",
    "area": "processor",
    "level": "A",
    "state": "decided",
    "discrepancy": "CIRC",
    "instructions": ["CIRC"],
    "when": {"pred": "any"},
    "differs": ["ac"],
    "sources": [
        {"who": "microcode", "mark": "microcode", "ref": "simple.42, CIRC", "does": "it exists"},
        {"who": "SIMH", "mark": "emulator", "ref": "kx10_cpu.c", "does": "nothing"},
    ],
    "choice": {"uop": "microcode", "soft": "microcode", "pipelined": "microcode"},
    "compare": {"simh": "skip", "klh10": "exact", "uop_soft": "exact"},
    "program": "\tCIRC 3,3\n",
}


def case(**change):
    c = copy.deepcopy(CASE)
    for k, v in change.items():
        if v is None:
            c.pop(k)
        else:
            c[k] = v
    return c


class CheckTest(unittest.TestCase):
    def rejects(self, c, text):
        with self.assertRaisesRegex(gi.ImpldefError, text):
            gi.check_case(c, DOCS, NAMES)

    def test_a_good_case_passes(self):
        gi.check_case(case(), DOCS, NAMES)

    def test_the_origin_must_exist(self):
        self.rejects(case(discrepancy="FDV"), "has no")
        self.rejects(case(discrepancy=None), "give one of")
        self.rejects(case(unknown="Floating point with odd operands"), "give one of")

    def test_level_b_needs_its_reason(self):
        self.rejects(case(level="B"), "level_b")
        self.rejects(case(level_b="no reason"), "level_b")
        gi.check_case(case(level="B", level_b="The PRM does not say."), DOCS, NAMES)

    def test_instructions_come_from_the_instruction_table(self):
        self.rejects(case(instructions=["CIRCX"]), "instructions")

    def test_the_predicate_and_its_parameters(self):
        self.rejects(case(when={"pred": "every"}), "predicate")
        self.rejects(case(when={"pred": "exponent_difference_over"}), "needs")
        gi.check_case(case(instructions=["ASH"], when={"pred": "left_shift_over", "ASH": 28}),
                      DOCS, NAMES)
        self.rejects(case(instructions=["ASH"], when={"pred": "left_shift_over", "ASHC": 29}),
                     "needs")

    def test_a_drawing_needs_its_revision(self):
        s = copy.deepcopy(CASE["sources"])
        s[0]["ref"] = "M8620-0-DPE9, E60"
        self.rejects(case(sources=s), "revision")

    def test_a_source_needs_an_evidence_mark(self):
        s = copy.deepcopy(CASE["sources"])
        s[0]["mark"] = "reading"
        self.rejects(case(sources=s), "mark")

    def test_a_mask_needs_the_fields(self):
        self.rejects(case(differs=[], compare={"simh": "mask", "klh10": "exact",
                                               "uop_soft": "exact"}), "mask needs")

    def test_a_difference_of_the_two_models_needs_level_b(self):
        diff = case(choice={"uop": "microcode", "soft": "PRM", "pipelined": "open"})
        self.rejects(diff, "level B or state open")
        gi.check_case(case(choice=diff["choice"], state="open"), DOCS, NAMES)
        gi.check_case(case(choice=diff["choice"], level="B", level_b="Decision."), DOCS, NAMES)
        masked = case(compare={"simh": "exact", "klh10": "exact", "uop_soft": "mask"})
        self.rejects(masked, "level B or state open")

    def test_a_device_case_has_its_own_choice(self):
        dev = case(area="device", choice={"machine": "KSDEFS"})
        self.rejects(dev, "no instructions")
        for k in ("instructions", "compare", "when"):
            dev.pop(k)
        gi.check_case(dev, DOCS, NAMES)


class TableTest(unittest.TestCase):
    def test_the_table_of_the_repository_passes(self):
        cases = gi.load(ROOT)
        self.assertTrue(cases)
        self.assertIn("### fdv-quotient", gi.render(cases))


if __name__ == "__main__":
    unittest.main()
