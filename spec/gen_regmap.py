#!/usr/bin/env python3
"""Generate the register map files from spec/regmap.toml.

Outputs (paths relative to the repository root):
  rtl/gen/regmap_pkg.sv      one SystemVerilog package for each block
  emu/plio/src/regmap.rs     one Rust module for each block
  tb/gen/regmap.h            C macros for the testbenches
  docs/regmap.md             the tables for people

Usage: gen_regmap.py [--check] [--root DIR]
  --check   compare the outputs with the files on disk; write nothing.
            Exit status 1 if a file is missing or different.

Only the Python standard library is used.
"""

import argparse
import re
import sys
import tomllib
from pathlib import Path

SOURCE = "spec/regmap.toml"
OUT_SV = "rtl/gen/regmap_pkg.sv"
OUT_RS = "emu/plio/src/regmap.rs"
OUT_MD = "docs/regmap.md"
OUT_H = "tb/gen/regmap.h"

ACCESS = ("ro", "rw", "wo", "w1c")
REG_BYTES = 4
NAME_RE = re.compile(r"^[A-Z][A-Z0-9_]*$")
# Lower-case register and field names are Rust module names.
RUST_KEYWORDS = frozenset("""
    as async await break const continue crate dyn else enum extern false fn for if impl in let
    loop match mod move mut pub ref return self static struct super trait true type unsafe use
    where while abstract become box do final gen macro override priv try typeof unsized virtual
    yield union
""".split())
BLOCK_RE = re.compile(r"^[a-z][a-z0-9_]*$")


class RegmapError(Exception):
    """The register map is not valid."""


def _req_int(table, key, where) -> int:
    if key not in table:
        raise RegmapError(f"{where}: missing '{key}'")
    value = table[key]
    if not isinstance(value, int) or isinstance(value, bool):
        raise RegmapError(f"{where}: '{key}' must be an integer")
    return value


def _req_str(table, key, where) -> str:
    if key not in table:
        raise RegmapError(f"{where}: missing '{key}'")
    value = table[key]
    if not isinstance(value, str):
        raise RegmapError(f"{where}: '{key}' must be a string")
    return value


def _check_keys(table, allowed, where):
    extra = sorted(set(table) - set(allowed))
    if extra:
        raise RegmapError(f"{where}: unknown keys {extra}")


def _pow2(n):
    return n > 0 and n & (n - 1) == 0


def _text(s):
    return " ".join(s.split())


def validate(spec):
    """Check the parsed TOML. Return a list of normalised blocks."""
    _check_keys(spec, ("format", "block"), "top level")
    if spec.get("format") != 1:
        raise RegmapError("top level: 'format' must be 1")
    blocks = []
    seen_blocks = set()
    for raw in spec.get("block", []):
        blocks.append(_validate_block(raw, seen_blocks))
    if not blocks:
        raise RegmapError("top level: no block")
    return blocks


def _validate_block(raw, seen_blocks):
    where = f"block {raw.get('name', '?')}"
    _check_keys(raw, ("name", "description", "id", "version", "window_bytes",
                      "hp", "note", "register", "memory"), where)
    name = _req_str(raw, "name", where)
    if not BLOCK_RE.match(name):
        raise RegmapError(f"{where}: name must match {BLOCK_RE.pattern}")
    if name in seen_blocks:
        raise RegmapError(f"{where}: duplicate block name")
    seen_blocks.add(name)
    block = {
        "name": name,
        "description": _text(_req_str(raw, "description", where)),
        "id": _req_int(raw, "id", where),
        "version": _req_int(raw, "version", where),
        "window_bytes": _req_int(raw, "window_bytes", where),
        "hp": None,
        "notes": [],
        "registers": [],
        "memories": [],
    }
    for key in ("id", "version"):
        if not 0 <= block[key] < 1 << 32:
            raise RegmapError(f"{where}: '{key}' does not fit 32 bits")
    window = block["window_bytes"]
    if not _pow2(window):
        raise RegmapError(f"{where}: window_bytes must be a power of 2")

    if "hp" in raw:
        hp = raw["hp"]
        hw = f"{where} hp"
        _check_keys(hp, ("base", "size", "source"), hw)
        base = _req_int(hp, "base", hw)
        size = _req_int(hp, "size", hw)
        source = _text(_req_str(hp, "source", hw))
        if not _pow2(size):
            raise RegmapError(f"{hw}: size must be a power of 2")
        if base < 0 or base % size:
            raise RegmapError(f"{hw}: base must be a multiple of size")
        if base + size > 1 << 32:
            raise RegmapError(f"{hw}: the region ends above 4 GiB")
        block["hp"] = {"base": base, "size": size, "source": source}

    for n in raw.get("note", []):
        nw = f"{where} note {n.get('title', '?')}"
        _check_keys(n, ("title", "text"), nw)
        block["notes"].append({"title": _text(_req_str(n, "title", nw)),
                               "text": _text(_req_str(n, "text", nw))})

    names = set()
    ranges = []  # (start, end, name)

    for r in raw.get("register", []):
        reg = _validate_register(r, where, window)
        if reg["name"] in names:
            raise RegmapError(f"{where}: duplicate name {reg['name']}")
        names.add(reg["name"])
        ranges.append((reg["offset"], reg["offset"] + REG_BYTES, reg["name"]))
        block["registers"].append(reg)

    for m in raw.get("memory", []):
        mem = _validate_memory(m, where, window)
        if mem["name"] in names:
            raise RegmapError(f"{where}: duplicate name {mem['name']}")
        names.add(mem["name"])
        ranges.append((mem["offset"], mem["offset"] + mem["bytes"], mem["name"]))
        block["memories"].append(mem)

    ranges.sort()
    for (s0, e0, n0), (s1, _e1, n1) in zip(ranges, ranges[1:]):
        if s1 < e0:
            raise RegmapError(f"{where}: {n0} and {n1} overlap")

    regs = {r["name"]: r for r in block["registers"]}
    for key, reg_name in (("id", "ID"), ("version", "VERSION")):
        if reg_name in regs and regs[reg_name]["reset"] != block[key]:
            raise RegmapError(f"{where}: reset of {reg_name} is not '{key}' of the block")

    _check_symbols(block, where)
    return block


def _validate_register(r, where, window):
    rw = f"{where} register {r.get('name', '?')}"
    _check_keys(r, ("name", "offset", "access", "reset", "description", "field"), rw)
    name = _req_str(r, "name", rw)
    if not NAME_RE.match(name):
        raise RegmapError(f"{rw}: name must match {NAME_RE.pattern}")
    if name.lower() in RUST_KEYWORDS:
        raise RegmapError(f"{rw}: {name.lower()} is a Rust keyword")
    offset = _req_int(r, "offset", rw)
    if offset < 0 or offset % REG_BYTES:
        raise RegmapError(f"{rw}: offset must be a multiple of {REG_BYTES}")
    if offset + REG_BYTES > window:
        raise RegmapError(f"{rw}: offset is outside the window")
    access = _req_str(r, "access", rw)
    if access not in ACCESS:
        raise RegmapError(f"{rw}: access must be one of {ACCESS}")
    reset = _req_int(r, "reset", rw)
    if not 0 <= reset < 1 << 32:
        raise RegmapError(f"{rw}: reset does not fit 32 bits")
    reg = {
        "name": name,
        "offset": offset,
        "access": access,
        "reset": reset,
        "description": _text(_req_str(r, "description", rw)),
        "fields": [],
    }
    used = 0
    fnames = set()
    for f in r.get("field", []):
        fw = f"{rw} field {f.get('name', '?')}"
        _check_keys(f, ("name", "lsb", "width", "description", "access", "values"), fw)
        fname = _req_str(f, "name", fw)
        if not NAME_RE.match(fname):
            raise RegmapError(f"{fw}: name must match {NAME_RE.pattern}")
        if fname.lower() in RUST_KEYWORDS:
            raise RegmapError(f"{fw}: {fname.lower()} is a Rust keyword")
        if fname in fnames:
            raise RegmapError(f"{fw}: duplicate field name")
        fnames.add(fname)
        lsb = _req_int(f, "lsb", fw)
        width = _req_int(f, "width", fw)
        if lsb < 0 or width < 1 or lsb + width > 32:
            raise RegmapError(f"{fw}: bits {lsb}+{width} are outside 0..31")
        mask = ((1 << width) - 1) << lsb
        if used & mask:
            raise RegmapError(f"{fw}: overlaps another field")
        used |= mask
        faccess = f.get("access", access)
        if faccess not in ACCESS:
            raise RegmapError(f"{fw}: access must be one of {ACCESS}")
        values = []
        vnames = set()
        vnums = set()
        for vname, vnum in f.get("values", {}).items():
            if not NAME_RE.match(vname):
                raise RegmapError(f"{fw}: value name {vname} must match {NAME_RE.pattern}")
            if not isinstance(vnum, int) or isinstance(vnum, bool):
                raise RegmapError(f"{fw}: value {vname} must be an integer")
            if not 0 <= vnum < 1 << width:
                raise RegmapError(f"{fw}: value {vname} does not fit {width} bits")
            if vnum in vnums:
                raise RegmapError(f"{fw}: value {vnum} has two names")
            vnames.add(vname)
            vnums.add(vnum)
            values.append((vname, vnum))
        values.sort(key=lambda v: v[1])
        reg["fields"].append({
            "name": fname,
            "lsb": lsb,
            "width": width,
            "access": faccess,
            "description": _text(_req_str(f, "description", fw)),
            "values": values,
        })
    reg["fields"].sort(key=lambda f: f["lsb"])
    if reg["fields"] and reset & ~used:
        raise RegmapError(f"{rw}: reset has bits outside the fields")
    return reg


def _validate_memory(m, where, window):
    mw = f"{where} memory {m.get('name', '?')}"
    _check_keys(m, ("name", "offset", "depth", "width", "access", "description"), mw)
    name = _req_str(m, "name", mw)
    if not NAME_RE.match(name):
        raise RegmapError(f"{mw}: name must match {NAME_RE.pattern}")
    offset = _req_int(m, "offset", mw)
    depth = _req_int(m, "depth", mw)
    width = _req_int(m, "width", mw)
    access = _req_str(m, "access", mw)
    if width != 32:
        raise RegmapError(f"{mw}: width must be 32")
    if not _pow2(depth):
        raise RegmapError(f"{mw}: depth must be a power of 2")
    nbytes = depth * REG_BYTES
    if offset < 0 or offset % nbytes:
        raise RegmapError(f"{mw}: offset must be a multiple of the size ({nbytes} bytes)")
    if offset + nbytes > window:
        raise RegmapError(f"{mw}: memory is outside the window")
    if access not in ACCESS:
        raise RegmapError(f"{mw}: access must be one of {ACCESS}")
    return {
        "name": name,
        "offset": offset,
        "depth": depth,
        "width": width,
        "bytes": nbytes,
        "access": access,
        "description": _text(_req_str(m, "description", mw)),
    }


def _check_symbols(block, where):
    """The flat names of the SystemVerilog package must be unique."""
    seen = {}

    def add(sym, origin):
        if sym in seen:
            raise RegmapError(f"{where}: generated name {sym} comes from {seen[sym]} and {origin}")
        seen[sym] = origin

    for sym in ("WINDOW_BYTES", "ID_VALUE", "VERSION_VALUE", "HP_BASE", "HP_SIZE", "HP_SIZE_LOG2"):
        add(sym, "the block")
    for r in block["registers"]:
        add(f"{r['name']}_OFFSET", r["name"])
        add(f"{r['name']}_RESET", r["name"])
        add(f"{r['name']}_MASK", r["name"])
        for f in r["fields"]:
            origin = f"{r['name']}.{f['name']}"
            for suffix in ("LSB", "WIDTH", "MASK"):
                add(f"{r['name']}_{f['name']}_{suffix}", origin)
            for vname, _ in f["values"]:
                add(f"{r['name']}_{f['name']}_{vname}", f"{origin}={vname}")
    for m in block["memories"]:
        for suffix in ("OFFSET", "DEPTH", "WIDTH"):
            add(f"{m['name']}_{suffix}", m["name"])


def load(text):
    try:
        spec = tomllib.loads(text)
    except tomllib.TOMLDecodeError as e:
        raise RegmapError(f"TOML: {e}") from e
    return validate(spec)


# ---- Output ----------------------------------------------------------------

HEADER = (f"Generated by spec/gen_regmap.py from {SOURCE}. Do not edit. "
          f"Run `make gen` after a change of {SOURCE}.")


def _header(comment):
    return f"{comment} {HEADER}"


def _h32(v):
    return f"32'h{v:08X}"


def render_sv(blocks):
    # A package has many constants that one design does not use, and the
    # file has one package for each block.
    out = [_header("//"), "/* verilator lint_off DECLFILENAME */", "/* verilator lint_off UNUSEDPARAM */"]
    for b in blocks:
        out.append("")
        out.append(f"// {b['description']}")
        out.append(f"package {b['name']}_regmap_pkg;")
        out.append(f"  localparam [31:0] WINDOW_BYTES = {_h32(b['window_bytes'])};")
        out.append(f"  localparam [31:0] ID_VALUE = {_h32(b['id'])};")
        out.append(f"  localparam [31:0] VERSION_VALUE = {_h32(b['version'])};")
        if b["hp"]:
            hp = b["hp"]
            out.append(f"  // {hp['source']}")
            out.append(f"  localparam [31:0] HP_BASE = {_h32(hp['base'])};")
            out.append(f"  localparam [31:0] HP_SIZE = {_h32(hp['size'])};")
            out.append(f"  localparam [31:0] HP_SIZE_LOG2 = 32'd{hp['size'].bit_length() - 1};")
        for n in b["notes"]:
            out.append(f"  // {n['title']}: {n['text']}")
        for r in b["registers"]:
            out.append("")
            out.append(f"  // {r['name']} ({r['access']}): {r['description']}")
            out.append(f"  localparam [31:0] {r['name']}_OFFSET = {_h32(r['offset'])};")
            out.append(f"  localparam [31:0] {r['name']}_RESET = {_h32(r['reset'])};")
            out.append(f"  localparam [31:0] {r['name']}_MASK = {_h32(_reg_mask(r))};")
            for f in r["fields"]:
                p = f"{r['name']}_{f['name']}"
                mask = ((1 << f["width"]) - 1) << f["lsb"]
                out.append(f"  localparam [31:0] {p}_LSB = 32'd{f['lsb']};")
                out.append(f"  localparam [31:0] {p}_WIDTH = 32'd{f['width']};")
                out.append(f"  localparam [31:0] {p}_MASK = {_h32(mask)};")
                for vname, vnum in f["values"]:
                    w = f["width"]
                    out.append(f"  localparam [{w - 1}:0] {p}_{vname} = {w}'d{vnum};")
        for m in b["memories"]:
            out.append("")
            out.append(f"  // {m['name']} ({m['access']}): {m['description']}")
            out.append(f"  localparam [31:0] {m['name']}_OFFSET = {_h32(m['offset'])};")
            out.append(f"  localparam [31:0] {m['name']}_DEPTH = 32'd{m['depth']};")
            out.append(f"  localparam [31:0] {m['name']}_WIDTH = 32'd{m['width']};")
        out.append(f"endpackage : {b['name']}_regmap_pkg")
    return "\n".join(out) + "\n"


def _reg_mask(r):
    """The bits that the fields use; all bits if the register has no field."""
    if not r["fields"]:
        return (1 << 32) - 1
    m = 0
    for f in r["fields"]:
        m |= ((1 << f["width"]) - 1) << f["lsb"]
    return m


def render_c(blocks):
    out = [f"/* {HEADER} */", "#ifndef NEOKS10_REGMAP_H", "#define NEOKS10_REGMAP_H"]
    for b in blocks:
        p = b["name"].upper()
        out.append("")
        out.append(f"/* {b['description']} */")
        out.append(f"#define {p}_WINDOW_BYTES 0x{b['window_bytes']:08X}u")
        out.append(f"#define {p}_ID_VALUE 0x{b['id']:08X}u")
        out.append(f"#define {p}_VERSION_VALUE {b['version']}u")
        if b["hp"]:
            out.append(f"#define {p}_HP_BASE 0x{b['hp']['base']:08X}u")
            out.append(f"#define {p}_HP_SIZE 0x{b['hp']['size']:08X}u")
        for r in b["registers"]:
            q = f"{p}_{r['name']}"
            out.append(f"#define {q}_OFFSET 0x{r['offset']:03X}u")
            out.append(f"#define {q}_RESET 0x{r['reset']:08X}u")
            out.append(f"#define {q}_MASK 0x{_reg_mask(r):08X}u")
            for f in r["fields"]:
                fq = f"{q}_{f['name']}"
                mask = ((1 << f["width"]) - 1) << f["lsb"]
                out.append(f"#define {fq}_LSB {f['lsb']}u")
                out.append(f"#define {fq}_WIDTH {f['width']}u")
                out.append(f"#define {fq}_MASK 0x{mask:08X}u")
                for vname, vnum in f["values"]:
                    out.append(f"#define {fq}_{vname} {vnum}u")
        for m in b["memories"]:
            q = f"{p}_{m['name']}"
            out.append(f"#define {q}_OFFSET 0x{m['offset']:04X}u")
            out.append(f"#define {q}_DEPTH {m['depth']}u")
            out.append(f"#define {q}_WIDTH {m['width']}u")
    out.append("")
    out.append("#endif")
    return "\n".join(out) + "\n"


def _rs_hex(v):
    s = f"{v:08x}"
    return f"0x{s[:4]}_{s[4:]}"


def _rs_doc(text, indent):
    # Square brackets would be read as intra-doc links.
    text = text.replace("[", "\\[").replace("]", "\\]")
    return f"{indent}/// {text}"


def render_rust(blocks):
    out = [_header("//"), "//! Offsets, fields and values of the PL registers, from `spec/regmap.toml`."]
    for b in blocks:
        used = []
        if any(r["fields"] for r in b["registers"]):
            used.append("Field")
        if b["memories"]:
            used.append("Mem")
        if b["registers"]:
            used.append("Reg")
        out.append("")
        out.append(_rs_doc(b["description"], ""))
        for n in b["notes"]:
            out.append("///")
            out.append(_rs_doc(f"{n['title']}: {n['text']}", ""))
        out.append(f"pub mod {b['name']} {{")
        if len(used) == 1:
            out.append(f"    use crate::{used[0]};")
        elif used:
            out.append(f"    use crate::{{{', '.join(used)}}};")
        out.append("")
        out.append("    /// Size of the register window in bytes.")
        out.append(f"    pub const WINDOW_BYTES: usize = {_rs_hex(b['window_bytes'])};")
        out.append("    /// Value of the ID register.")
        out.append(f"    pub const ID_VALUE: u32 = {_rs_hex(b['id'])};")
        out.append("    /// Version of the register map.")
        out.append(f"    pub const VERSION_VALUE: u32 = {b['version']};")
        if b["hp"]:
            hp = b["hp"]
            out.append(_rs_doc(f"Bus address of the HP memory. {hp['source']}", "    "))
            out.append(f"    pub const HP_BASE: u32 = {_rs_hex(hp['base'])};")
            out.append("    /// Size of the HP memory in bytes.")
            out.append(f"    pub const HP_SIZE: u32 = {_rs_hex(hp['size'])};")
        for r in b["registers"]:
            out.append("")
            out.append(_rs_doc(f"{r['description']} Access: {r['access']}.", "    "))
            out.append(f"    pub const {r['name']}: Reg = Reg {{")
            out.append(f"        offset: {r['offset']:#05x},")
            out.append(f"        reset: {_rs_hex(r['reset'])},")
            out.append("    };")
        for r in b["registers"]:
            if not r["fields"]:
                continue
            out.append("")
            out.append(f"    /// Fields of [`{r['name']}`].")
            out.append(f"    pub mod {r['name'].lower()} {{")
            out.append("        use super::Field;")
            for f in r["fields"]:
                out.append("")
                out.append(_rs_doc(f"{f['description']} Access: {f['access']}.", "        "))
                out.append(f"        pub const {f['name']}: Field = Field {{")
                out.append(f"            lsb: {f['lsb']},")
                out.append(f"            width: {f['width']},")
                out.append("        };")
            for f in r["fields"]:
                if not f["values"]:
                    continue
                out.append("")
                out.append(f"        /// Values of [`{f['name']}`].")
                out.append(f"        pub mod {f['name'].lower()} {{")
                for vname, vnum in f["values"]:
                    out.append(f"            pub const {vname}: u32 = {vnum};")
                out.append("        }")
            out.append("    }")
        for m in b["memories"]:
            out.append("")
            out.append(_rs_doc(f"{m['description']} Access: {m['access']}.", "    "))
            out.append(f"    pub const {m['name']}: Mem = Mem {{")
            out.append(f"        offset: {m['offset']:#06x},")
            out.append(f"        depth: {m['depth']},")
            out.append("    };")
        out.append("}")
    return "\n".join(out) + "\n"


def _md_cell(text):
    return text.replace("|", "\\|")


def render_md(blocks):
    out = [f"<!-- {HEADER} -->", "", "# Register map", ""]
    out.append(f"This file shows `{SOURCE}`. All registers are 32 bits. Offsets are bytes from")
    out.append("the start of the register window on M_AXI_GP0.")
    for b in blocks:
        out.append("")
        out.append(f"## Block `{b['name']}`")
        out.append("")
        out.append(b["description"])
        out.append("")
        out.append(f"- ID: 0x{b['id']:08X}. Version: {b['version']}.")
        out.append(f"- Window: {b['window_bytes']:#x} bytes.")
        if b["hp"]:
            hp = b["hp"]
            out.append(f"- HP memory: 0x{hp['base']:08X}, {hp['size']:#x} bytes. {hp['source']}")
        for n in b["notes"]:
            out.append("")
            out.append(f"**{n['title']}.** {n['text']}")
        out.append("")
        out.append("| Offset | Name | Access | Reset | Description |")
        out.append("|---|---|---|---|---|")
        for r in b["registers"]:
            out.append(f"| 0x{r['offset']:03X} | {r['name']} | {r['access']} | 0x{r['reset']:08X} | "
                       f"{_md_cell(r['description'])} |")
        for m in b["memories"]:
            out.append(f"| 0x{m['offset']:03X} | {m['name']} | {m['access']} | | "
                       f"{m['depth']} words of {m['width']} bits. {_md_cell(m['description'])} |")
        for r in b["registers"]:
            if not r["fields"]:
                continue
            out.append("")
            out.append(f"### {r['name']}")
            out.append("")
            out.append("| Bits | Name | Access | Description |")
            out.append("|---|---|---|---|")
            for f in r["fields"]:
                hi = f["lsb"] + f["width"] - 1
                bits = f"{hi}:{f['lsb']}" if f["width"] > 1 else f"{f['lsb']}"
                desc = f["description"]
                if f["values"]:
                    desc += " Values: " + ", ".join(f"{n} = {v}" for n, v in f["values"]) + "."
                out.append(f"| {bits} | {f['name']} | {f['access']} | {_md_cell(desc)} |")
    return "\n".join(out) + "\n"


def outputs(blocks):
    return {OUT_SV: render_sv(blocks), OUT_RS: render_rust(blocks), OUT_MD: render_md(blocks),
            OUT_H: render_c(blocks)}


def main(argv=None):
    ap = argparse.ArgumentParser(description="Generate the register map files.")
    ap.add_argument("--check", action="store_true", help="compare only; write nothing")
    ap.add_argument("--root", type=Path, default=Path(__file__).resolve().parent.parent)
    args = ap.parse_args(argv)
    try:
        blocks = load((args.root / SOURCE).read_text(encoding="utf-8"))
    except RegmapError as e:
        print(f"{SOURCE}: {e}", file=sys.stderr)
        return 2
    stale = []
    for rel, text in outputs(blocks).items():
        path = args.root / rel
        old = path.read_text(encoding="utf-8") if path.exists() else None
        if old == text:
            continue
        if args.check:
            stale.append(rel)
        else:
            path.parent.mkdir(parents=True, exist_ok=True)
            path.write_text(text, encoding="utf-8")
            print(f"wrote {rel}")
    if stale:
        print("not up to date (run `make gen`): " + ", ".join(stale), file=sys.stderr)
        return 1
    return 0


if __name__ == "__main__":
    sys.exit(main())
