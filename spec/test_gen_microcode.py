"""Tests of gen_microcode.py (python3 -m unittest)."""

import unittest
from pathlib import Path

import gen_microcode as gm
import micro

ROOT = Path(__file__).resolve().parent.parent
ITS = ROOT / "3rdparty" / "its"

# A made-up word for the layout tests. The RAM file has 3 words (108 bits).
FIELDS = """\
.WIDTH/108
A/=<0:11>
B/=<12:35>
"""


def items(ramfile):
    a = micro.Assembler([("test", FIELDS + ".RAMFILE/" + ramfile + "\n")])
    a.assemble()
    return gm.ramfile_items(a)


class RamFile(unittest.TestCase):
    def test_items_are_packed_from_the_left(self):
        got = items("A, B, ODD PARITY<0:11,12:35>, 71/0")
        self.assertEqual(got[0], ("field", "A", 0, 12, 0))
        self.assertEqual(got[1], ("field", "B", 12, 24, 12))
        self.assertEqual(got[2], ("parity", True, ((0, 11), (12, 35)), 36))
        self.assertEqual(got[3], ("pad", 71, 37))

    def test_even_parity_and_a_one_bit_pad(self):
        got = items("A, 0, EVEN PARITY<0:10>, 94/0")
        self.assertEqual(got[1], ("pad", 1, 12))
        self.assertEqual(got[2], ("parity", False, ((0, 10),), 13))

    def test_the_source_lines(self):
        a = micro.Assembler([("test", FIELDS + ".RAMFILE/A, B, 72/0\n")])
        a.assemble()
        self.assertEqual(a.ramfile_source, "test:4")
        self.assertEqual(gm.fields(a, micro.UCODE), [("A", 0, 11, "test:2"), ("B", 12, 35, "test:3")])

    def test_rejects(self):
        cases = [
            ("A, B, ODD PARITY<0:1>, 35/0", gm.MicrocodeError, "72 bits, expected 108"),
            ("A, D, 96/0", micro.MicroError, "NOT DEFINED"),
            ("A, A, 0, B, 59/0", micro.MicroError, "CROSSES WORD BOUNDARY"),
        ]
        for text, error, msg in cases:
            with self.subTest(text=text):
                with self.assertRaisesRegex(error, msg):
                    items(text)


class Names(unittest.TestCase):
    def test_rust_names(self):
        self.assertEqual(gm.rs_ident("#"), "NUM")
        self.assertEqual(gm.rs_ident("# HIGH"), "NUM_HIGH")
        self.assertEqual(gm.rs_ident("S#"), "S_NUM")
        self.assertEqual(gm.rs_ident("FL-B"), "FL_B")
        self.assertEqual(gm.rs_ident("I.CO3"), "I_CO3")


@unittest.skipUnless((ITS / "src" / "kshack").is_dir(), "the submodule 3rdparty/its is not checked out")
class Sources(unittest.TestCase):
    def test_the_its_microcode(self):
        a = micro.assemble(ITS)
        got = gm.ramfile_items(a)
        fields = {i[1]: i for i in got if i[0] == "field"}
        # J is the next address, bits 0 to 11 (ks10, "MICROCODE FIELDS"),
        # in the third word of the RAM file.
        self.assertEqual(fields["J"][2:], (0, 12, 84))
        self.assertEqual(sum(1 for i in got if i[0] == "parity"), 2)
        ufields = gm.fields(a, micro.UCODE)
        self.assertIn(("J", 0, 11, "ks10.47:563"), ufields)
        # The dispatch ROM word has its own J, bits 12 to 23.
        self.assertIn(("J", 12, 23), [f[:3] for f in gm.fields(a, micro.DISP)])


if __name__ == "__main__":
    unittest.main()
