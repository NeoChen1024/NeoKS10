"""Tests of spec/gen_regmap.py. Run: python3 -m unittest discover -s spec"""

import sys
import unittest
from pathlib import Path

sys.path.insert(0, str(Path(__file__).resolve().parent))

import gen_regmap as g  # noqa: E402

ROOT = Path(__file__).resolve().parent.parent

# A small valid map. The tests change one item of it.
BASE = """
format = 1
[[block]]
name = "t"
description = "Test block."
id = 0x1234
version = 2
window_bytes = 0x1000
[block.hp]
base = 0x1000_0000
size = 0x0010_0000
source = "Test."
[[block.register]]
name = "ID"
offset = 0x0
access = "ro"
reset = 0x1234
description = "Id."
[[block.register]]
name = "MODE"
offset = 0x4
access = "rw"
reset = 0x21
description = "Mode."
[[block.register.field]]
name = "OP"
lsb = 0
width = 3
description = "Op."
values = { A = 0, B = 1 }
[[block.register.field]]
name = "X"
lsb = 4
width = 2
access = "ro"
description = "X."
[[block.memory]]
name = "M"
offset = 0x800
depth = 512
width = 32
access = "ro"
description = "Mem."
"""


def change(old, new):
    assert old in BASE, old
    return BASE.replace(old, new, 1)


class Valid(unittest.TestCase):
    def test_small_map(self):
        blocks = g.load(BASE)
        self.assertEqual(len(blocks), 1)
        mode = blocks[0]["registers"][1]
        self.assertEqual([f["name"] for f in mode["fields"]], ["OP", "X"])
        self.assertEqual(mode["fields"][1]["access"], "ro")

    def test_repository_map(self):
        g.load((ROOT / g.SOURCE).read_text(encoding="utf-8"))

    def test_repository_outputs_are_current(self):
        blocks = g.load((ROOT / g.SOURCE).read_text(encoding="utf-8"))
        for rel, text in g.outputs(blocks).items():
            with self.subTest(file=rel):
                self.assertEqual((ROOT / rel).read_text(encoding="utf-8"), text,
                                 f"{rel} is not current: run `make gen`")


class Rejected(unittest.TestCase):
    def rejects(self, text, words):
        with self.assertRaises(g.RegmapError) as cm:
            g.load(text)
        self.assertIn(words, str(cm.exception))

    def test_bad_toml(self):
        self.rejects(BASE + "\n[[[\n", "TOML")

    def test_format(self):
        self.rejects(change("format = 1", "format = 2"), "'format' must be 1")

    def test_unknown_key(self):
        self.rejects(change('access = "rw"', 'access = "rw"\nlength = 4'), "unknown keys")

    def test_missing_description(self):
        self.rejects(change('description = "Mode."', ""), "missing 'description'")

    def test_register_overlap(self):
        self.rejects(change("offset = 0x4", "offset = 0x0"), "overlap")

    def test_register_memory_overlap(self):
        self.rejects(change("offset = 0x4", "offset = 0x804"), "overlap")

    def test_misaligned_register(self):
        self.rejects(change("offset = 0x4", "offset = 0x6"), "multiple of 4")

    def test_register_outside_window(self):
        self.rejects(change("offset = 0x4", "offset = 0x1000"), "outside the window")

    def test_bad_access(self):
        self.rejects(change('access = "rw"', 'access = "rc"'), "access must be")

    def test_field_outside_register(self):
        self.rejects(change("lsb = 4", "lsb = 31"), "outside 0..31")

    def test_field_overlap(self):
        self.rejects(change("lsb = 4", "lsb = 2"), "overlaps another field")

    def test_value_too_large(self):
        self.rejects(change("B = 1", "B = 8"), "does not fit 3 bits")

    def test_value_twice(self):
        self.rejects(change("B = 1", "B = 0"), "has two names")

    def test_reset_outside_fields(self):
        self.rejects(change("reset = 0x21", "reset = 0x81"), "outside the fields")

    def test_reset_too_wide(self):
        self.rejects(change("reset = 0x21", "reset = 0x1_0000_0000"), "does not fit 32 bits")

    def test_id_mismatch(self):
        self.rejects(change("id = 0x1234", "id = 0x1235"), "reset of ID")

    def test_hp_misaligned(self):
        self.rejects(change("base = 0x1000_0000", "base = 0x1008_0000"), "multiple of size")

    def test_hp_size_not_pow2(self):
        self.rejects(change("size = 0x0010_0000", "size = 0x0018_0000"), "power of 2")

    def test_memory_misaligned(self):
        self.rejects(change("offset = 0x800", "offset = 0x400"), "multiple of the size")

    def test_memory_width(self):
        self.rejects(change("width = 32", "width = 64"), "width must be 32")

    def test_duplicate_name(self):
        self.rejects(change('name = "M"', 'name = "MODE"'), "duplicate name")

    def test_generated_name_collision(self):
        # Register MODE_OP gives MODE_OP_OFFSET; field OP of MODE gives MODE_OP_LSB.
        # These differ, so use a value name that repeats a suffix of a field.
        text = change("values = { A = 0, B = 1 }", "values = { A = 0, LSB = 1 }")
        self.rejects(text, "generated name MODE_OP_LSB")

    def test_register_mask_collision(self):
        # Field MASK of register MODE gives MODE_MASK_LSB, not MODE_MASK; a
        # register named MODE_OP gives MODE_OP_MASK like the field OP of MODE.
        text = BASE + '''
[[block.register]]
name = "MODE_OP"
offset = 0x8
access = "rw"
reset = 0
description = "Collides."
'''
        self.rejects(text, "generated name MODE_OP_")

    def test_rust_keyword_register(self):
        self.rejects(change('name = "MODE"', 'name = "TYPE"'), "type is a Rust keyword")

    def test_rust_keyword_field(self):
        self.rejects(change('name = "X"', 'name = "LOOP"'), "loop is a Rust keyword")

    def test_note_needs_text(self):
        self.rejects(change("window_bytes = 0x1000", 'window_bytes = 0x1000\n[[block.note]]\ntitle = "N"'),
                     "missing 'text'")

    def test_bad_name(self):
        self.rejects(change('name = "OP"', 'name = "op"'), "name must match")


class Output(unittest.TestCase):
    """Expected text is written by hand from BASE."""

    def setUp(self):
        self.blocks = g.load(BASE)

    def test_sv(self):
        sv = g.render_sv(self.blocks)
        for line in (
            "package t_regmap_pkg;",
            "  localparam [31:0] ID_VALUE = 32'h00001234;",
            "  localparam [31:0] HP_BASE = 32'h10000000;",
            "  localparam [31:0] HP_SIZE_LOG2 = 32'd20;",
            "  localparam [31:0] MODE_OFFSET = 32'h00000004;",
            "  localparam [31:0] MODE_RESET = 32'h00000021;",
            "  localparam [31:0] MODE_MASK = 32'h00000037;",
            "  localparam [31:0] ID_MASK = 32'hFFFFFFFF;",
            "  localparam [31:0] MODE_X_LSB = 32'd4;",
            "  localparam [31:0] MODE_X_MASK = 32'h00000030;",
            "  localparam [2:0] MODE_OP_B = 3'd1;",
            "  localparam [31:0] M_DEPTH = 32'd512;",
            "endpackage : t_regmap_pkg",
        ):
            self.assertIn(line + "\n", sv)

    def test_rust(self):
        rs = g.render_rust(self.blocks)
        for text in (
            "pub mod t {\n    use crate::{Field, Mem, Reg};\n",
            "    pub const MODE: Reg = Reg {\n        offset: 0x004,\n        reset: 0x0000_0021,\n    };\n",
            "    pub mod mode {\n        use super::Field;\n",
            "        pub const X: Field = Field {\n            lsb: 4,\n            width: 2,\n        };\n",
            "        pub mod op {\n            pub const A: u32 = 0;\n            pub const B: u32 = 1;\n        }\n",
            "    pub const M: Mem = Mem {\n        offset: 0x0800,\n        depth: 512,\n    };\n",
        ):
            self.assertIn(text, rs)

    def test_c(self):
        h = g.render_c(self.blocks)
        for line in (
            "#define T_ID_VALUE 0x00001234u",
            "#define T_HP_BASE 0x10000000u",
            "#define T_MODE_OFFSET 0x004u",
            "#define T_MODE_MASK 0x00000037u",
            "#define T_MODE_X_LSB 4u",
            "#define T_MODE_X_MASK 0x00000030u",
            "#define T_MODE_OP_B 1u",
            "#define T_M_OFFSET 0x0800u",
            "#define T_M_DEPTH 512u",
        ):
            self.assertIn(line + "\n", h)

    def test_md(self):
        md = g.render_md(self.blocks)
        self.assertIn("| 0x004 | MODE | rw | 0x00000021 | Mode. |\n", md)
        self.assertIn("| 2:0 | OP | rw | Op. Values: A = 0, B = 1. |\n", md)
        self.assertIn("| 5:4 | X | ro | X. |\n", md)


class Main(unittest.TestCase):
    def test_check_detects_stale_file(self):
        import contextlib
        import io
        import tempfile
        with tempfile.TemporaryDirectory() as d, contextlib.redirect_stdout(io.StringIO()), \
                contextlib.redirect_stderr(io.StringIO()):
            root = Path(d)
            (root / "spec").mkdir()
            (root / g.SOURCE).write_text(BASE, encoding="utf-8")
            self.assertEqual(g.main(["--root", d, "--check"]), 1)
            self.assertEqual(g.main(["--root", d]), 0)
            self.assertEqual(g.main(["--root", d, "--check"]), 0)
            (root / g.OUT_MD).write_text("changed\n", encoding="utf-8")
            self.assertEqual(g.main(["--root", d, "--check"]), 1)


if __name__ == "__main__":
    unittest.main()
