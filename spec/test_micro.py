"""Tests of micro.py (python3 -m unittest)."""

import unittest
from pathlib import Path

import micro

ROOT = Path(__file__).resolve().parent.parent
ITS = ROOT / "3rdparty" / "its"
# A listing of MICRO 116 from 1987 (KS10FPGA, microcode/ITS/mcr.262). It has
# the words of the current sources; some comment lines differ.
HISTORICAL = ROOT / "3rdparty" / "KS10FPGA" / "microcode" / "ITS" / "mcr.262"

_ITS = None


def its_assembly():
    global _ITS
    if _ITS is None:
        _ITS = micro.assemble(ITS)
    return _ITS


def assemble(text):
    a = micro.Assembler([("test", text)])
    a.assemble()
    return a


# A made-up microword: J, and two fields that overlap, as SPEC and SHSTYLE
# do in ks10.
SMALL = """\
.WIDTH/108
J/=<0:11>+
SPEC/=<51:56>D,0
\tASHOV=44
SHSTYLE/=<54:56>
\tASHC=4
"""


class Names(unittest.TestCase):
    def test_the_order_is_the_signed_compare_of_the_words(self):
        # The first word of "A" is negative: 7-bit codes from 100 up sort
        # before "#" (SRCFLD, CAMLE).
        self.assertLess(micro.name_words("A"), micro.name_words("#"))
        self.assertLess(micro.name_words("SV.ARX"), micro.name_words("SV.AR"))
        self.assertLess(micro.name_words("AB"), micro.name_words("AC"))


class Words(unittest.TestCase):
    def test_a_field_can_overlap_a_field_with_the_same_bits(self):
        a = assemble(SMALL + "1:\tSHSTYLE/ASHC, SPEC/ASHOV, J/1\n")
        spec = (a.cram[1] >> (107 - 56)) & 0o77
        self.assertEqual(spec, 0o44)

    def test_a_conflict_is_an_error(self):
        with self.assertRaisesRegex(micro.MicroError, "CONFLICTING"):
            assemble(SMALL + "1:\tSHSTYLE/ASHC, SPEC/10, J/1\n")

    def test_defaults_and_the_next_address(self):
        # J/+ (PCINC) is the address of the next word; SPEC/D,0.
        a = assemble(SMALL + "=0\nX:\tSHSTYLE/ASHC\n\tJ/X\n")
        self.assertEqual(sorted(a.cram), [0, 1])
        self.assertEqual((a.cram[0] >> 96, a.cram[1] >> 96), (1, 0))
        self.assertEqual((a.cram[0] >> (107 - 56)) & 0o77, 0o04)
        self.assertEqual(a.cram[1], 0)

    def test_the_word_has_the_groups_up_to_the_last_field(self):
        # PNTB3: without .WIDTH, the last bit of a field gives the width.
        a = micro.Assembler([("test", SMALL.replace(".WIDTH/108\n", "") + "1:\tSPEC/ASHOV, J/1\n")])
        a.assemble()
        self.assertEqual(a.cram[1], (1 << 48) | (0o44 << 3))

    def test_an_undefined_symbol_is_an_error(self):
        with self.assertRaisesRegex(micro.MicroError, "SYMBOL NOT DEFINED"):
            assemble(SMALL + "1:\tSPEC/NONE, J/1\n")


@unittest.skipUnless((ITS / "src" / "kshack").is_dir(), "the submodule 3rdparty/its is missing")
class Its(unittest.TestCase):
    def test_the_word_counts(self):
        a = its_assembly()
        self.assertEqual((len(a.cram), len(a.drom)), (1980, 512))

    @unittest.skipUnless(HISTORICAL.is_file(), "the submodule 3rdparty/KS10FPGA is missing")
    def test_the_words_and_symbols_agree_with_the_1987_listing(self):
        a = its_assembly()
        text = HISTORICAL.read_bytes().decode("latin-1")
        self.assertEqual(micro.compare(a, text, lines=False), [])

    @unittest.skipUnless(HISTORICAL.is_file(), "the submodule 3rdparty/KS10FPGA is missing")
    def test_the_comparison_finds_a_changed_word(self):
        a = its_assembly()
        text = HISTORICAL.read_bytes().decode("latin-1")
        old = "U 0000, 0002,3771,0012,4374,4007,0700,0000,0037,7777"
        self.assertIn(old, text)
        diffs = micro.compare(a, text.replace(old, old[:-1] + "6"), lines=False)
        self.assertEqual(len(diffs), 1)
        self.assertTrue(diffs[0].startswith("U 0000:"))


if __name__ == "__main__":
    unittest.main()
