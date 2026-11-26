#!/usr/bin/env python3
"""Generate the instruction set files from spec/instructions.toml.

spec/instructions.toml is the only definition of the instruction set
(AGENTS.md). It holds, for each of the 512 opcodes, the entry of the
dispatch ROM (DRAM) of the ITS KS10 microcode and the name, and for the
opcodes that dispatch on the AC field, the 16 entries of that dispatch.

Outputs (paths relative to the repository root):
  emu/pdp10/src/instructions.rs   Rust tables
  docs/instructions.md            the tables for people

Usage: gen_instructions.py [--check] [--its DIR] [--import] [--root DIR]
  --check    compare the outputs with the files on disk; write nothing.
  --its DIR  also compare the table with the ITS sources in DIR (the
             submodule 3rdparty/its): the DRAM entries, the dispatch on the
             AC field, the names and the classes.
  --import   make spec/instructions.toml from the ITS sources in --its DIR.
             It does not replace an existing file without --force.

Exit status 1 if a check fails. Only the Python standard library is used.
"""

import argparse
import re
import sys
import tomllib
from pathlib import Path

import its_sources as its

SOURCE = "spec/instructions.toml"
OUT_RS = "emu/pdp10/src/instructions.rs"
OUT_MD = "docs/instructions.md"

# Keys of a DRAM entry in the table and the field names of the microcode.
FIELD_KEYS = {"A": "A", "B": "B", "J": "J", "ACDISP": "ACDISP", "I": "I", "READ": "READ",
              "TEST": "TEST", "COND_FUNC": "COND FUNC", "VMA": "VMA", "WRITE": "WRITE"}
# ROUND, MODE and FL-B are other names for bits of field B (floating point).
B_ALIASES = ("ROUND", "MODE", "FL-B")
CLASSES = ("inst", "acdisp", "luuo", "muuo", "uuo")
NAME_FROM = ("midas", "ksdefs", "ucode", "")
NAME_RE = re.compile(r"^[A-Z][A-Z0-9]*$")
AT_RE = re.compile(r"^[a-z0-9]+\.\d+:\d+$")


class SpecError(Exception):
    """The instruction table is not valid."""


# ---- The table -------------------------------------------------------------------------

def _keys(table, allowed, required, where):
    for k in table:
        if k not in allowed:
            raise SpecError(f"{where}: unknown key '{k}'")
    for k in required:
        if k not in table:
            raise SpecError(f"{where}: missing '{k}'")


def validate(spec: dict) -> dict:
    """Check the table and return it in a normal form."""
    _keys(spec, ("format", "sources", "dram", "op", "acdisp"),
          ("format", "sources", "dram", "op", "acdisp"), "top level")
    if spec["format"] != 1:
        raise SpecError("format must be 1")
    _keys(spec["sources"], ("ucode", "midas", "ksdefs"), ("ucode", "midas", "ksdefs"), "sources")

    fields = {}
    for key, f in spec["dram"].items():
        if key not in FIELD_KEYS and key not in B_ALIASES:
            raise SpecError(f"dram: unknown field '{key}'")
        _keys(f, ("bits", "default", "values"), ("bits",), f"dram.{key}")
        first, last = f["bits"]
        if not 0 <= first <= last <= 35:
            raise SpecError(f"dram.{key}: bad bits")
        values = f.get("values", {})
        width = last - first + 1
        for name, v in values.items():
            if not 0 <= v < (1 << width):
                raise SpecError(f"dram.{key}.{name}: value does not fit")
        fields[key] = {"bits": (first, last), "default": f.get("default", 0), "values": values}
    for key in FIELD_KEYS:
        if key not in fields:
            raise SpecError(f"dram: missing field '{key}'")

    ops = {}
    for i, o in enumerate(spec["op"]):
        where = f"op[{i}]"
        _keys(o, ("code", "name", "name_from", "class", "ucode", "dram", "at"),
              ("code", "name", "name_from", "class", "ucode", "dram", "at"), where)
        code = o["code"]
        if not 0 <= code <= 0o777:
            raise SpecError(f"{where}: code out of range")
        where = f"op {code:03o}"
        if code in ops:
            raise SpecError(f"{where}: defined twice")
        _check_name(o, where)
        if o["class"] not in CLASSES or o["class"] == "acdisp" and not o["dram"].get("ACDISP"):
            raise SpecError(f"{where}: bad class '{o['class']}'")
        _check_dram(o["dram"], fields, where)
        if not AT_RE.match(o["at"]):
            raise SpecError(f"{where}: bad 'at'")
        ops[code] = o
    if len(ops) != 512:
        raise SpecError(f"{len(ops)} opcodes, expected 512")

    acops = {}
    for i, a in enumerate(spec["acdisp"]):
        where = f"acdisp[{i}]"
        _keys(a, ("code", "ac", "name", "name_from", "class", "label", "ucode", "at"),
              ("code", "ac", "name", "name_from", "class", "label", "ucode", "at"), where)
        key = (a["code"], a["ac"])
        where = f"acdisp {a['code']:03o} ac {a['ac']:o}"
        if key[0] not in ops or ops[key[0]]["class"] != "acdisp":
            raise SpecError(f"{where}: the opcode does not dispatch on AC")
        if not 0 <= key[1] <= 0o17 or key in acops:
            raise SpecError(f"{where}: bad or second entry")
        _check_name(a, where)
        if a["class"] not in ("inst", "uuo"):
            raise SpecError(f"{where}: bad class")
        if not AT_RE.match(a["at"]):
            raise SpecError(f"{where}: bad 'at'")
        acops[key] = a
    for code, o in ops.items():
        if o["class"] == "acdisp":
            for ac in range(16):
                if (code, ac) not in acops:
                    raise SpecError(f"acdisp {code:03o} ac {ac:o}: missing")
    return {"sources": spec["sources"], "fields": fields, "ops": ops, "acops": acops}


def _check_name(o, where):
    name, src = o["name"], o["name_from"]
    if src not in NAME_FROM or (name == "") != (src == ""):
        raise SpecError(f"{where}: bad name or name_from")
    if name and not NAME_RE.match(name):
        raise SpecError(f"{where}: bad name '{name}'")


def _check_dram(d, fields, where):
    for k, v in d.items():
        if k not in FIELD_KEYS:
            raise SpecError(f"{where}: unknown DRAM field '{k}'")
        f = fields[k]
        first, last = f["bits"]
        if k == "J":
            if not isinstance(v, str) or not v:
                raise SpecError(f"{where}: J must be a label")
        elif k == "A":
            if v not in f["values"]:
                raise SpecError(f"{where}: unknown A value '{v}'")
        elif not isinstance(v, int) or not 0 <= v < (1 << (last - first + 1)):
            raise SpecError(f"{where}: bad value of {k}")
        elif v == f["default"]:
            raise SpecError(f"{where}: {k} has its default value; leave it out")
    if "J" not in d:
        raise SpecError(f"{where}: missing J")


def load(text: str) -> dict:
    try:
        return validate(tomllib.loads(text))
    except tomllib.TOMLDecodeError as e:
        raise SpecError(f"TOML: {e}") from e


# ---- The ITS sources --------------------------------------------------------------------

def from_its(its_dir: Path, sources: dict) -> dict:
    """The table as the ITS sources give it, in the form of validate()."""
    src = its_dir / "src"
    u = its.read_ucode(src / sources["ucode"])
    midas = its.read_midas_names(src / sources["midas"])
    ks_ops, ks_acops = its.read_ksdefs_names(src / sources["ksdefs"])

    fields = {}
    for name, (first, last, default, values) in u["fields"].items():
        key = next((k for k, v in FIELD_KEYS.items() if v == name), name)
        fields[key] = {"bits": (first, last), "default": default, "values": values}
    for key in list(FIELD_KEYS) + list(B_ALIASES):
        if key not in fields:
            raise its.SourceError(f"the microcode has no DRAM field {key}")

    labels = its.cram_labels(u["cram"])

    def at_label(label):
        a = labels.get(label)
        return u["cram"][a][1] if a is not None else None

    ops = {}
    for code, (text, comment, fname, line) in u["dram"].items():
        d = _resolve(its.expand(text, u["macros"]), fields)
        body = at_label(d["J"])
        if d.get("ACDISP"):
            cls = "acdisp"
        elif d["J"] == "LUUO" or body == "LUUO":
            cls = "luuo"
        elif d["J"] == "MUUO":
            cls = "muuo"
        elif d["J"] == "UUO" or body == "UUO":
            cls = "uuo"
        else:
            cls = "inst"
        if code in ks_ops:
            name, src_name = ks_ops[code], "ksdefs"
        elif code in midas:
            name, src_name = midas[code], "midas"
        else:
            name, src_name = "", ""
        ops[code] = {"code": code, "name": name, "name_from": src_name, "class": cls,
                     "ucode": text, "dram": d, "at": f"{fname}:{line}"}

    acops = {}
    for code, o in ops.items():
        if o["class"] != "acdisp":
            continue
        base = labels.get(o["dram"]["J"])
        if base is None:
            raise its.SourceError(f"{code:03o}: label {o['dram']['J']} has no explicit address")
        for ac in range(16):
            entry = u["cram"].get(base + ac)
            if entry is None:
                raise its.SourceError(f"{code:03o} ac {ac:o}: no microinstruction at {base + ac:o}")
            labs, text, comment, fname, line = entry
            label = " ".join(labs)
            name, src_name = "", ""
            if (code, ac) in ks_acops:
                name, src_name = ks_acops[(code, ac)], "ksdefs"
            else:
                m = re.match(r"\((\d+)\)\s+([A-Z][A-Z0-9]*)\b", comment)
                if m and int(m.group(1), 8) == ac and text != "UUO":
                    name, src_name = m.group(2), "ucode"
            acops[(code, ac)] = {"code": code, "ac": ac, "name": name, "name_from": src_name,
                                 "class": "uuo" if text == "UUO" else "inst",
                                 "label": label, "ucode": text, "at": f"{fname}:{line}"}
    return {"fields": fields, "ops": ops, "acops": acops}


def _resolve(items: list, fields: dict) -> dict:
    """The DRAM fields of a list of 'FIELD/VALUE' items, as in the table."""
    raw = {}
    for item in items:
        name, _, value = item.partition("/")
        key = next((k for k, v in FIELD_KEYS.items() if v == name), name)
        if key not in fields:
            raise its.SourceError(f"unknown DRAM field in {item!r}")
        raw[key] = value
    d = {}
    b = None
    for key, value in raw.items():
        f = fields[key]
        if key == "J":
            d["J"] = value
            continue
        if key == "A":
            if value not in f["values"]:
                raise its.SourceError(f"unknown A value {value!r}")
            d["A"] = value
            continue
        v = f["values"][value] if value in f["values"] else int(value, 8)
        if key == "B" or key in B_ALIASES:
            first, last = f["bits"]
            blast = fields["B"]["bits"][1]
            shift = blast - last
            mask = ((1 << (last - first + 1)) - 1) << shift
            b = ((b or 0) & ~mask) | (v << shift)
            continue
        d[key] = v
    if b is not None:
        d["B"] = b
    for key in list(d):
        if key not in ("A", "J") and d[key] == fields[key]["default"]:
            del d[key]
    return {k: d[k] for k in FIELD_KEYS if k in d}


def compare(table: dict, ref: dict) -> list:
    """The differences between the table and the sources, as text lines."""
    out = []
    for key, f in ref["fields"].items():
        t = table["fields"].get(key)
        if t is None or tuple(t["bits"]) != f["bits"] or t["default"] != f["default"] \
                or t["values"] != f["values"]:
            out.append(f"dram field {key}: table {t}, sources {f}")
    for code in range(512):
        a, b = table["ops"][code], ref["ops"][code]
        for k in b:
            if a.get(k) != b[k]:
                out.append(f"op {code:03o} {k}: table {a.get(k)!r}, sources {b[k]!r}")
    for key in sorted(set(table["acops"]) | set(ref["acops"])):
        a, b = table["acops"].get(key), ref["acops"].get(key)
        if a is None or b is None:
            out.append(f"acdisp {key[0]:03o} ac {key[1]:o}: only in {'sources' if a is None else 'table'}")
            continue
        for k in b:
            if a.get(k) != b[k]:
                out.append(f"acdisp {key[0]:03o} ac {key[1]:o} {k}: table {a.get(k)!r}, sources {b[k]!r}")
    return out


# ---- Output -------------------------------------------------------------------------------

HEADER = "generated by spec/gen_instructions.py from spec/instructions.toml; do not edit"


def _toml_str(s: str) -> str:
    return '"' + s.replace("\\", "\\\\").replace('"', '\\"') + '"'


def _toml_value(v) -> str:
    return _toml_str(v) if isinstance(v, str) else f"0o{v:o}"


def render_toml(ref: dict, sources: dict) -> str:
    out = [f"""# The instruction set of the KS10 with the ITS microcode. This file is the
# only definition of the instruction set (AGENTS.md). `make gen` writes the
# Rust tables and docs/instructions.md from it. `make check` compares it with
# the ITS sources of the submodule 3rdparty/its (gen_instructions.py --its).
#
# Sources (paths below src/ of the ITS tree):
#   ucode   the KS10 microcode of ITS. "dram" and the entries come from its
#           dispatch ROM (.DCODE), "acdisp" from the microinstructions at the
#           dispatch address plus the AC field.
#   midas   the initial symbol table of MIDAS (ISYMTB): names of 104 to 677.
#   ksdefs  SYSTEM;KSDEFS: names of the KS10 and ITS instructions.
#
# op: one entry for each opcode.
#   code       the opcode (bits 0 to 8 of the instruction)
#   name       the mnemonic, or "" if the sources give none
#   name_from  the source of the name: "midas", "ksdefs", "ucode" or ""
#   class      inst    an instruction of the microcode
#              acdisp  the microcode dispatches on the AC field (see acdisp)
#              luuo    a local UUO (trap through 40 and 41)
#              muuo    a monitor UUO
#              uuo     not implemented: the microcode does a monitor UUO
#   ucode      the DRAM entry as the microcode writes it
#   dram       its fields, macros expanded; a field with its default value
#              is left out. J is the label of the microcode entry.
#   at         file and line of the DRAM entry in the microcode
#
# acdisp: one entry for each AC value of an opcode of class acdisp.
#   label, ucode, at: the microinstruction at the dispatch address; label
#              lists the labels at that address, separated by spaces.
#   class "uuo" if that microinstruction is UUO, else "inst".
#
# For opcodes 001 to 027, the DRAM entry is also the dispatch of the EXTEND
# instructions. Outside EXTEND, its label does a local UUO (class luuo).

format = 1
sources = {{ ucode = {_toml_str(sources['ucode'])}, midas = {_toml_str(sources['midas'])}, ksdefs = {_toml_str(sources['ksdefs'])} }}

op = ["""]
    for code in range(512):
        o = ref["ops"][code]
        d = ", ".join(f"{k} = {_toml_value(v)}" for k, v in o["dram"].items())
        out.append(f"  {{ code = 0o{code:03o}, name = {_toml_str(o['name'])}, "
                   f"name_from = {_toml_str(o['name_from'])}, class = {_toml_str(o['class'])}, "
                   f"ucode = {_toml_str(o['ucode'])}, dram = {{ {d} }}, at = {_toml_str(o['at'])} }},")
    out.append("]\n\nacdisp = [")
    for (code, ac), a in sorted(ref["acops"].items()):
        out.append(f"  {{ code = 0o{code:03o}, ac = 0o{ac:02o}, name = {_toml_str(a['name'])}, "
                   f"name_from = {_toml_str(a['name_from'])}, class = {_toml_str(a['class'])}, "
                   f"label = {_toml_str(a['label'])}, ucode = {_toml_str(a['ucode'])}, "
                   f"at = {_toml_str(a['at'])} }},")
    out.append("]\n\n# Fields of the DRAM word; bit numbers as in the microcode (ks10, \"DISPATCH ROM")
    out.append("# DEFINITIONS\"). ROUND, MODE and FL-B are names for bits of field B.")
    for key, f in ref["fields"].items():
        out.append(f"\n[dram.{key.replace(' ', '_')}]")
        out.append(f"bits = [{f['bits'][0]}, {f['bits'][1]}]")
        if f["default"]:
            out.append(f"default = {f['default']}")
        if f["values"]:
            vals = ", ".join(f'{_toml_key(n)} = 0o{v:o}' for n, v in f["values"].items())
            out.append(f"values = {{ {vals} }}")
    return "\n".join(out) + "\n"


def _toml_key(name: str) -> str:
    return name if re.fullmatch(r"[A-Za-z0-9_-]+", name) else _toml_str(name)


def _rs_ident(name: str) -> str:
    return re.sub(r"[^A-Z0-9]", "_", name.upper())


def render_rust(t: dict) -> str:
    out = [f"// {HEADER}.", "",
           "//! The instruction set: the dispatch ROM entries of the ITS KS10 microcode",
           "//! and the names (spec/instructions.toml).", ""]
    for key in ("A", "B"):
        out.append(f"/// Values of DRAM field {key}.")
        out.append(f"pub mod {key.lower()} {{")
        for name, v in t["fields"][key]["values"].items():
            out.append(f"    pub const {_rs_ident(name)}: u8 = 0o{v:o};")
        out.append("}")
        out.append("")
    out += ["/// The kind of an opcode (spec/instructions.toml, `class`).",
            "#[derive(Clone, Copy, Debug, PartialEq, Eq)]",
            "pub enum Class {",
            "    /// An instruction of the microcode.",
            "    Inst,",
            "    /// The microcode dispatches on the AC field: see [`ac_op`].",
            "    AcDisp,",
            "    /// A local UUO.",
            "    Luuo,",
            "    /// A monitor UUO.",
            "    Muuo,",
            "    /// Not implemented: the microcode does a monitor UUO.",
            "    Uuo,",
            "}",
            "",
            "/// An entry of the dispatch ROM.",
            "#[derive(Clone, Copy, Debug, PartialEq, Eq)]",
            "pub struct Dram {",
            "    /// Operand fetch (values in [`a`]).",
            "    pub a: u8,",
            "    /// Result store, or a condition (values in [`b`]).",
            "    pub b: u8,",
            "    /// Label of the microcode entry.",
            "    pub j: &'static str,",
            "    pub acdisp: bool,",
            "    pub i: bool,",
            "    pub read: bool,",
            "    pub test: bool,",
            "    pub cond_func: bool,",
            "    pub vma: bool,",
            "    pub write: bool,",
            "}",
            "",
            "/// An opcode.",
            "#[derive(Clone, Copy, Debug, PartialEq, Eq)]",
            "pub struct Op {",
            "    pub code: u16,",
            "    /// The mnemonic, or \"\" if the sources give none.",
            "    pub name: &'static str,",
            "    pub class: Class,",
            "    pub dram: Dram,",
            "}",
            "",
            "/// An entry of the dispatch on the AC field.",
            "#[derive(Clone, Copy, Debug, PartialEq, Eq)]",
            "pub struct AcOp {",
            "    pub code: u16,",
            "    pub ac: u8,",
            "    pub name: &'static str,",
            "    /// [`Class::Inst`] or [`Class::Uuo`].",
            "    pub class: Class,",
            "}",
            ""]
    cls = {"inst": "Inst", "acdisp": "AcDisp", "luuo": "Luuo", "muuo": "Muuo", "uuo": "Uuo"}
    fields = t["fields"]
    a_values = fields["A"]["values"]
    out.append("/// The 512 opcodes, in the order of the code.")
    out.append("pub static OPS: [Op; 512] = [")
    for code in range(512):
        o = t["ops"][code]
        d = o["dram"]

        def flag(k):
            return "true" if d.get(k, fields[k]["default"]) else "false"
        out.append(
            f"    Op {{ code: 0o{code:03o}, name: \"{o['name']}\", class: Class::{cls[o['class']]}, "
            f"dram: Dram {{ a: 0o{a_values[d.get('A', 'READ')]:o}, b: 0o{d.get('B', 0):o}, "
            f"j: \"{d['J']}\", acdisp: {flag('ACDISP')}, i: {flag('I')}, read: {flag('READ')}, "
            f"test: {flag('TEST')}, cond_func: {flag('COND_FUNC')}, vma: {flag('VMA')}, "
            f"write: {flag('WRITE')} }} }},")
    out.append("];")
    out.append("")
    acs = sorted(t["acops"].items())
    out.append("/// The dispatch on the AC field, 16 entries for each opcode of class")
    out.append("/// [`Class::AcDisp`], in the order of code and AC.")
    out.append(f"pub static AC_OPS: [AcOp; {len(acs)}] = [")
    for (code, ac), a in acs:
        out.append(f"    AcOp {{ code: 0o{code:03o}, ac: 0o{ac:o}, name: \"{a['name']}\", "
                   f"class: Class::{cls[a['class']]} }},")
    out.append("];")
    out.append("")
    out += ["/// The entry for an opcode and an AC value, if the opcode dispatches on AC.",
            "pub fn ac_op(code: u16, ac: u8) -> Option<&'static AcOp> {",
            "    AC_OPS.iter().find(|a| a.code == code && a.ac == ac)",
            "}"]
    return "\n".join(out) + "\n"


def _md(s: str) -> str:
    return s.replace("|", "\\|")


def render_md(t: dict) -> str:
    out = [f"<!-- {HEADER} -->", "", "# Instruction set", "",
           "The opcodes of the KS10 with the ITS microcode, from `spec/instructions.toml`.",
           "The DRAM columns are the dispatch ROM entry of the microcode, with macros",
           "expanded; `J` is the label of the microcode entry. A field with its default",
           "value is not shown.", "",
           "| Code | Name | Class | Microcode | DRAM | At |", "|---|---|---|---|---|---|"]
    for code in range(512):
        o = t["ops"][code]
        d = ", ".join(f"{k}={v if isinstance(v, str) else format(v, 'o')}" for k, v in o["dram"].items())
        out.append(f"| {code:03o} | {o['name']} | {o['class']} | `{_md(o['ucode'])}` | {_md(d)} | {o['at']} |")
    out += ["", "## Dispatch on the AC field", "",
            "| Code | AC | Name | Class | Label | Microcode | At |", "|---|---|---|---|---|---|---|"]
    for (code, ac), a in sorted(t["acops"].items()):
        out.append(f"| {code:03o} | {ac:o} | {a['name']} | {a['class']} | {a['label']} | "
                   f"`{_md(a['ucode'])}` | {a['at']} |")
    return "\n".join(out) + "\n"


def outputs(t: dict) -> dict:
    return {OUT_RS: render_rust(t), OUT_MD: render_md(t)}


def main(argv=None) -> int:
    ap = argparse.ArgumentParser(description="Generate the instruction set files.")
    ap.add_argument("--check", action="store_true", help="compare only; write nothing")
    ap.add_argument("--its", type=Path, help="ITS tree to compare with (or to import from)")
    ap.add_argument("--import", dest="do_import", action="store_true")
    ap.add_argument("--force", action="store_true")
    ap.add_argument("--root", type=Path, default=Path(__file__).resolve().parent.parent)
    args = ap.parse_args(argv)
    src = args.root / SOURCE

    if args.do_import:
        if args.its is None:
            print("--import needs --its DIR", file=sys.stderr)
            return 2
        if src.exists() and not args.force:
            print(f"{SOURCE} exists; use --force to replace it", file=sys.stderr)
            return 2
        sources = {"ucode": "kshack", "midas": "midas/midas.458", "ksdefs": "system/ksdefs.194"}
        ref = from_its(args.its, sources)
        text = render_toml(ref, sources)
        load(text)   # the import must give a valid table
        src.write_text(text)
        print(f"wrote {SOURCE}")
        return 0

    try:
        table = load(src.read_text())
    except SpecError as e:
        print(f"{SOURCE}: {e}", file=sys.stderr)
        return 1
    status = 0
    if args.its is not None:
        try:
            diffs = compare(table, from_its(args.its, table["sources"]))
        except its.SourceError as e:
            print(f"ITS sources: {e}", file=sys.stderr)
            return 1
        for d in diffs:
            print(f"{SOURCE}: {d}", file=sys.stderr)
        if diffs:
            status = 1
    for rel, text in outputs(table).items():
        path = args.root / rel
        if args.check:
            if not path.exists() or path.read_text() != text:
                print(f"{rel} is not current; run make gen", file=sys.stderr)
                status = 1
        else:
            path.parent.mkdir(parents=True, exist_ok=True)
            if not path.exists() or path.read_text() != text:
                path.write_text(text)
                print(f"wrote {rel}")
    return status


if __name__ == "__main__":
    sys.exit(main())
