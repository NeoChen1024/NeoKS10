"""Tests of spec/gen_devices.py. Run: python3 -m unittest discover -s spec"""

import tempfile
import unittest
from pathlib import Path

import gen_devices as gd

ROOT = Path(__file__).resolve().parent.parent


class Expressions(unittest.TestCase):
    # MIDAS numbers are octal unless they end with a point; a_b is a shifted
    # left b; .BP m is the mask m (rh11.defs48 uses all three forms).
    def test_evaluate(self):
        syms = {"BAS": "772440", "X": "BAS+2", "N": "812.", "M": "815.-N"}
        self.assertEqual(gd.evaluate("1_15.", syms), 1 << 15)
        self.assertEqual(gd.evaluate("7_8.", syms), 0o3400)
        self.assertEqual(gd.evaluate(".BP 77", syms), 0o77)
        self.assertEqual(gd.evaluate("X", syms), 0o772442)
        self.assertEqual(gd.evaluate("N+M", syms), 815)
        self.assertEqual(gd.evaluate("3,,764140", syms), (3 << 18) | 0o764140)

    def test_read_symbols(self):
        with tempfile.TemporaryDirectory() as d:
            p = Path(d) / "x.defs"
            p.write_text("DEFSYM\t%HRCS1=:776700\t\t;CTRL\n DEFSYM %HXSC==\t1_15.\t; SC\n")
            s = gd.read_symbols(p)
            self.assertEqual(s["%HRCS1"], "776700")
            self.assertEqual(gd.evaluate(s["%HXSC"], s), 1 << 15)


class Validation(unittest.TestCase):
    def base(self):
        return {"device": [{"name": "d", "title": "t", "source": "s",
                            "registers": [{"name": "A", "offset": 0, "title": "a"}],
                            "fields": [{"register": "A", "fields": [
                                {"name": "F", "lsb": 15, "title": "f"}]}]}]}

    def test_accepts(self):
        gd.validate(self.base())

    def test_rejects_wide_field(self):
        d = self.base()
        d["device"][0]["fields"][0]["fields"][0]["width"] = 2
        with self.assertRaises(gd.DeviceError):
            gd.validate(d)

    def test_rejects_odd_offset_and_overlap(self):
        d = self.base()
        d["device"][0]["registers"].append({"name": "B", "offset": 1, "title": "b"})
        with self.assertRaises(gd.DeviceError):
            gd.validate(d)
        d["device"][0]["registers"][1]["offset"] = 0
        with self.assertRaises(gd.DeviceError):
            gd.validate(d)


class AgainstIts(unittest.TestCase):
    def test_mismatch_is_found(self):
        data = {"device": [{"name": "d", "title": "t", "source": "x.defs",
                            "registers": [{"name": "A", "addr": 0o100, "its": "REG", "title": "a"}],
                            "fields": [{"register": "A", "fields": [
                                {"name": "F", "lsb": 1, "its": "BIT", "title": "f"}]}]}]}
        with tempfile.TemporaryDirectory() as d:
            sysdir = Path(d) / "src" / "system"
            sysdir.mkdir(parents=True)
            (sysdir / "x.defs").write_text("REG==:100\nBIT==1_2.\n")
            errs = gd.check_its(data, Path(d))
        self.assertEqual(len(errs), 1)
        self.assertIn("d.A.F", errs[0])

    def test_table_agrees(self):
        its = ROOT / "3rdparty" / "its"
        if not (its / "src" / "system").exists():
            self.skipTest("no ITS tree")
        self.assertEqual(gd.check_its(gd.load(ROOT / gd.SOURCE), its), [])


if __name__ == "__main__":
    unittest.main()
