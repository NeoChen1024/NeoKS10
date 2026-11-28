"""Tests of gen_capi.py (python3 -m unittest)."""

import tempfile
import unittest
from pathlib import Path

import gen_capi as gc

SMALL = """
prefix = "ks10"
[[status]]
name = "OK"
value = 0
doc = "ok"
[[status]]
name = "NULL"
value = -1
doc = "null"
[[record]]
name = "count"
doc = "n"
[[record]]
name = "ac"
count = 16
doc = "acs"
[[function]]
name = "uop_examine"
returns = "int"
args = [["const ks10_uop *", "m"], ["uint32_t", "address"], ["uint64_t *", "word"]]
doc = "read"
"""


def load(text):
    with tempfile.TemporaryDirectory() as d:
        p = Path(d) / "capi.toml"
        p.write_text(text)
        return gc.load(p)


class Render(unittest.TestCase):
    def test_header(self):
        h = gc.render_h(load(SMALL))
        self.assertIn("#define KS10_NULL (-1)", h)
        self.assertIn("    uint64_t ac[16];", h)
        self.assertIn("int ks10_uop_examine(const ks10_uop *m, uint32_t address, uint64_t *word);", h)

    def test_rust(self):
        rs = gc.render_rs(load(SMALL))
        self.assertIn("pub const KS10_NULL: c_int = -1;", rs)
        self.assertIn("    pub ac: [u64; 16],", rs)
        self.assertIn(
            'pub type UopExamine = unsafe extern "C" fn(*const Ks10Uop, u32, *mut u64) -> c_int;', rs
        )

    def test_rejects(self):
        cases = [
            (SMALL.replace('"uint32_t", "address"', '"long", "address"'), "type"),
            (SMALL.replace("value = -1", "value = 0"), "must differ"),
            (SMALL.replace('name = "ac"', 'name = "count"'), "record field"),
        ]
        for text, msg in cases:
            with self.subTest(msg=msg):
                with self.assertRaisesRegex(gc.CapiError, msg):
                    load(text)


if __name__ == "__main__":
    unittest.main()
