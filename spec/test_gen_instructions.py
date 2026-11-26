"""Tests of gen_instructions.py and its_sources.py (python3 -m unittest)."""

import re
import tempfile
import unittest
from pathlib import Path

import gen_instructions as gi
import its_sources as its

ROOT = Path(__file__).resolve().parent.parent
ITS = ROOT / "3rdparty" / "its"
TABLE = (ROOT / gi.SOURCE).read_text()


def table_with(old, new, count=1):
    assert TABLE.count(old) == count, old
    return TABLE.replace(old, new)


class Validate(unittest.TestCase):
    def test_table_loads(self):
        t = gi.load(TABLE)
        self.assertEqual(len(t["ops"]), 512)

    def test_rejects(self):
        cases = [
            ("code = 0o201,", "code = 0o200,", "defined twice"),
            ('class = "acdisp", ucode = "I, VMA/0', 'class = "inst", ucode = "I, VMA/0', None),
            ('dram = { A = "RD-PF", B = 0o15, J = "STAC", READ = 0o1 }, at = "simple.42:449"',
             'dram = { A = "XX", B = 0o15, J = "STAC", READ = 0o1 }, at = "simple.42:449"',
             "unknown A value"),
            ('J = "STAC", READ = 0o1 }, at = "simple.42:449"',
             'J = "STAC", READ = 0o1, VMA = 0o1 }, at = "simple.42:449"', "default"),
            ('name = "MOVE", name_from = "midas"', 'name = "MOVE", name_from = ""', "name"),
            ('name = "MOVE", name_from = "midas"', 'name = "move", name_from = "midas"', "bad name"),
            ("{ code = 0o254, ac = 0o17,", "{ code = 0o254, ac = 0o16,", "second entry"),
        ]
        for old, new, msg in cases:
            with self.subTest(new=new):
                with self.assertRaises(gi.SpecError) as cm:
                    gi.load(table_with(old, new))
                if msg:
                    self.assertIn(msg, str(cm.exception))


@unittest.skipUnless((ITS / "src" / "kshack").is_dir(), "submodule 3rdparty/its not present")
class AgainstIts(unittest.TestCase):
    def setUp(self):
        self.table = gi.load(TABLE)
        self.ref = gi.from_its(ITS, self.table["sources"])

    def test_table_agrees(self):
        self.assertEqual(gi.compare(self.table, self.ref), [])

    def test_changes_are_found(self):
        # A changed field, name, class or dispatch entry must be reported.
        cases = [
            ('J = "STAC", READ = 0o1 }, at = "simple.42:449"', 'J = "STAC" }, at = "simple.42:449"'),
            ('name = "MOVE", name_from = "midas"', 'name = "MOV", name_from = "midas"'),
            ('code = 0o130, name = "UFA", name_from = "midas", class = "uuo"',
             'code = 0o130, name = "UFA", name_from = "midas", class = "inst"'),
            ('code = 0o254, ac = 0o03, name = "", name_from = "", class = "uuo"',
             'code = 0o254, ac = 0o03, name = "", name_from = "", class = "inst"'),
        ]
        for old, new in cases:
            with self.subTest(new=new):
                self.assertNotEqual(gi.compare(gi.load(table_with(old, new)), self.ref), [])

    def test_import_gives_the_table(self):
        self.assertEqual(gi.render_toml(self.ref, self.table["sources"]), TABLE)


class Sources(unittest.TestCase):
    def test_conditional_assembly(self):
        # .IFNOT/X after .IF/X replaces the condition on X; .ENDIF/X ends it.
        with tempfile.TemporaryDirectory() as d:
            for stem in its.UCODE_FILES:
                (Path(d) / f"{stem}.1").write_text("")
            (Path(d) / "its.1").write_text(".SET/ITS=1\n")
            (Path(d) / "simple.1").write_text(
                ".DCODE\n.IF/ITS\n200:\tI,\tJ/YES\n.IFNOT/ITS\n200:\tI,\tJ/NO\n.ENDIF/ITS\n"
                ".IF/FULL\n201:\tI,\tJ/NO\n.ENDIF/FULL\n201:\tI,\tJ/YES2\n")
            lines = [t.strip() for _, _, mode, t, _ in its._active_lines(Path(d)) if t.strip()]
            self.assertEqual(lines, ["200:\tI,\tJ/YES", "201:\tI,\tJ/YES2"])

    def test_ksdefs_io_names(self):
        # KSDEFS: RDEBR=:CONI PAG, is 701240,, (its comment): opcode 701, AC 5.
        with tempfile.TemporaryDirectory() as d:
            p = Path(d) / "ksdefs.1"
            p.write_text("PAG==:10\nRDEBR=:CONI PAG,\t;= 701240,,\nUMOVE=:704^9\n")
            ops, acops = its.read_ksdefs_names(p)
            self.assertEqual(ops, {0o704: "UMOVE"})
            self.assertEqual(acops, {(0o701, 5): "RDEBR"})


class Outputs(unittest.TestCase):
    def test_rust_has_all_opcodes(self):
        rs = gi.render_rust(gi.load(TABLE))
        self.assertEqual(len(re.findall(r"(?<![A-Za-z])Op \{ code: 0o", rs)), 512)


if __name__ == "__main__":
    unittest.main()
