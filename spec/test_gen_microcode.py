"""Tests of gen_microcode.py (python3 -m unittest)."""

import unittest
from pathlib import Path

import gen_microcode as gm

ROOT = Path(__file__).resolve().parent.parent
KSHACK = ROOT / "3rdparty" / "its" / "src" / "kshack"

# Fields of a made-up word for the layout tests: 108 bits in total.
FIELDS = [("A", 0, 35, "x:1"), ("B", 36, 70, "x:2"), ("C", 71, 106, "x:3")]


class RamFile(unittest.TestCase):
    def test_items_are_packed_from_the_left(self):
        items = gm.parse_ramfile("A, B, ODD PARITY<0:35,36:70>, 36/0", FIELDS)
        self.assertEqual(items[0], ("field", "A", 0, 36, 0))
        self.assertEqual(items[1], ("field", "B", 36, 35, 36))
        self.assertEqual(items[2], ("parity", True, ((0, 35), (36, 70)), 71))
        self.assertEqual(items[3], ("pad", 36, 72))

    def test_even_parity_and_a_one_bit_pad(self):
        items = gm.parse_ramfile("A, 0, B, EVEN PARITY<0:10>, 35/0", FIELDS)
        self.assertEqual(items[1], ("pad", 1, 36))
        self.assertEqual(items[3][:2], ("parity", False))

    def test_rejects(self):
        cases = [
            ("A, B, 36/0", "bits, expected 108"),
            ("A, D, 37/0", "not defined"),
            ("0, A, 71/0", "crosses a word"),
            ("A, B, ODD PARITY<0:200>, 36/0", "outside the entry"),
        ]
        for text, msg in cases:
            with self.subTest(text=text):
                with self.assertRaisesRegex(gm.MicrocodeError, msg):
                    gm.parse_ramfile(text, FIELDS)

    def test_rust_names(self):
        self.assertEqual(gm.rs_ident("#"), "NUM")
        self.assertEqual(gm.rs_ident("# HIGH"), "NUM_HIGH")
        self.assertEqual(gm.rs_ident("S#"), "S_NUM")
        self.assertEqual(gm.rs_ident("FL-B"), "FL_B")
        self.assertEqual(gm.rs_ident("I.CO3"), "I_CO3")


@unittest.skipUnless(KSHACK.is_dir(), "the submodule 3rdparty/its is not checked out")
class Sources(unittest.TestCase):
    def test_the_its_microcode(self):
        data = gm.read(KSHACK)
        items = gm.parse_ramfile(data["ramfile"][0], data["ufields"])
        fields = {i[1]: i for i in items if i[0] == "field"}
        # J is the next address, bits 0 to 11 (ks10, "MICROCODE FIELDS"),
        # in the third word of the RAM file.
        self.assertEqual(fields["J"][2:], (0, 12, 84))
        self.assertEqual(sum(1 for i in items if i[0] == "parity"), 2)
        names = [f[0] for f in data["ufields"]]
        self.assertEqual(len(names), len(set(names)))
        # The dispatch ROM word has its own J, bits 12 to 23.
        self.assertIn(("J", 12, 23), [f[:3] for f in data["dfields"]])


if __name__ == "__main__":
    unittest.main()
