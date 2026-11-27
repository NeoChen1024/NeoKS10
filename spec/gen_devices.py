#!/usr/bin/env python3
"""Generate the device register files from spec/devices.toml.

Outputs (paths relative to the repository root):
  emu/ks10-core-soft/src/devregs.rs   one Rust module for each device
  docs/devices.md           the tables for people

Usage: gen_devices.py [--check] [--its DIR] [--root DIR]
  --check     compare the outputs with the files on disk; write nothing.
  --its DIR   also compare every entry that has an `its` name with the
              value of that symbol in DIR/src/system (the ITS tree).

Exit status 1 if a check fails. Only the Python standard library is used.
"""

import argparse
import re
import sys
import tomllib
from pathlib import Path

SOURCE = "spec/devices.toml"
OUT_RS = "emu/ks10-core-soft/src/devregs.rs"
OUT_MD = "docs/devices.md"
NAME_RE = re.compile(r"^[A-Z][A-Z0-9_]*$")
DEV_RE = re.compile(r"^[a-z][a-z0-9_]*$")


class DeviceError(Exception):
    """The device table is not valid."""


def _int(t, k, where, default=None):
    v = t.get(k, default)
    if not isinstance(v, int) or isinstance(v, bool):
        raise DeviceError(f"{where}: '{k}' must be an integer")
    return v


def load(path: Path) -> dict:
    data = tomllib.loads(path.read_text())
    validate(data)
    return data


def register_bits(reg: dict) -> int:
    """Unibus registers have 16 bits; I/O bus registers (with `addr`) 36."""
    return 36 if "addr" in reg else 16


def validate(data: dict) -> None:
    names = set()
    for dev in data.get("device", []):
        dn = dev.get("name", "")
        if not DEV_RE.match(dn) or dn in names:
            raise DeviceError(f"device name {dn!r} is not valid or not unique")
        names.add(dn)
        regs = {}
        places = set()
        for r in dev.get("registers", []):
            rn = r.get("name", "")
            where = f"{dn}.{rn}"
            if not NAME_RE.match(rn) or rn in regs:
                raise DeviceError(f"{where}: register name not valid or not unique")
            if ("offset" in r) == ("addr" in r):
                raise DeviceError(f"{where}: give one of 'offset' and 'addr'")
            place = _int(r, "offset" if "offset" in r else "addr", where)
            if "offset" in r and place % 2:
                raise DeviceError(f"{where}: a Unibus offset must be even")
            count = _int(r, "count", where, 1)
            for k in range(count):
                if (place + k) in places:
                    raise DeviceError(f"{where}: overlaps another register")
                places.add(place + k)
            regs[rn] = r
        for group in dev.get("fields", []):
            rn = group.get("register")
            if rn not in regs:
                raise DeviceError(f"{dn}: fields for unknown register {rn!r}")
            bits = register_bits(regs[rn])
            seen = set()
            for f in group.get("fields", []):
                fn = f.get("name", "")
                where = f"{dn}.{rn}.{fn}"
                if not NAME_RE.match(fn) or fn in seen:
                    raise DeviceError(f"{where}: field name not valid or not unique")
                seen.add(fn)
                lsb = _int(f, "lsb", where)
                width = _int(f, "width", where, 1)
                if lsb < 0 or width < 1 or lsb + width > bits:
                    raise DeviceError(f"{where}: field outside the {bits}-bit register")
        for group in dev.get("values", []):
            rn, fname = group.get("register"), group.get("field")
            fields = [f for g in dev.get("fields", []) if g["register"] == rn for f in g["fields"]]
            match = [f for f in fields if f["name"] == fname]
            if not match:
                raise DeviceError(f"{dn}: values for unknown field {rn}.{fname}")
            width = match[0].get("width", 1)
            seen = set()
            for v in group.get("values", []):
                vn = v.get("name", "")
                where = f"{dn}.{rn}.{fname}.{vn}"
                if not NAME_RE.match(vn) or vn in seen:
                    raise DeviceError(f"{where}: value name not valid or not unique")
                seen.add(vn)
                if not 0 <= _int(v, "value", where) < (1 << width):
                    raise DeviceError(f"{where}: value does not fit in the field")
        for c in dev.get("constants", []):
            if not NAME_RE.match(c.get("name", "")):
                raise DeviceError(f"{dn}: constant name not valid")
            _int(c, "value", f"{dn}.{c.get('name')}")
    inst = set()
    for i in data.get("instance", []):
        n = i.get("name", "")
        if not DEV_RE.match(n) or n in inst:
            raise DeviceError(f"instance name {n!r} is not valid or not unique")
        inst.add(n)
        if i.get("device") not in names:
            raise DeviceError(f"instance {n}: unknown device")
        for k in ("adapter", "base", "vector", "br"):
            _int(i, k, f"instance {n}")
        if i["vector"] % 4:
            raise DeviceError(f"instance {n}: a vector is a multiple of 4")


def field_mask(f: dict) -> int:
    return ((1 << f.get("width", 1)) - 1) << f["lsb"]


def oct_(v: int) -> str:
    return f"0o{v:o}"


def render_rust(data: dict) -> str:
    out = [
        "//! The device registers, generated by spec/gen_devices.py from",
        "//! spec/devices.toml. Do not edit.",
        "//!",
        "//! Unibus register offsets are in bytes from the base address of the",
        "//! device; field constants are masks of the register, with `_LSB` for",
        "//! the number of the low bit of fields wider than one bit.",
        "",
    ]
    for dev in data["device"]:
        out.append(f"/// {dev['title']} ({dev['source']}).")
        out.append(f"pub mod {dev['name']} {{")
        for r in dev.get("registers", []):
            its = f" ({r['its']})" if "its" in r else ""
            out.append(f"    /// {r['title']}{its}.")
            if "offset" in r:
                out.append(f"    pub const {r['name']}: u32 = {oct_(r['offset'])};")
            else:
                out.append(f"    pub const {r['name']}: u32 = {oct_(r['addr'])};")
            if r.get("count", 1) > 1:
                out.append(f"    pub const {r['name']}_COUNT: u32 = {r['count']};")
        for group in dev.get("fields", []):
            out.append(f"    /// Fields of {group['register']}.")
            out.append(f"    pub mod {group['register'].lower()} {{")
            for f in group["fields"]:
                its = f" ({f['its']})" if "its" in f else ""
                out.append(f"        /// {f['title']}{its}.")
                out.append(f"        pub const {f['name']}: u64 = {oct_(field_mask(f))};")
                if f.get("width", 1) > 1:
                    out.append(f"        pub const {f['name']}_LSB: u32 = {f['lsb']};")
            out.append("    }")
        for group in dev.get("values", []):
            out.append(f"    /// Values of {group['register']}.{group['field']}.")
            out.append(f"    pub mod {group['register'].lower()}_{group['field'].lower()} {{")
            for v in group["values"]:
                its = f" ({v['its']})" if "its" in v else ""
                out.append(f"        /// {v['name']}{its}.")
                out.append(f"        pub const {v['name']}: u64 = {oct_(v['value'])};")
            out.append("    }")
        for c in dev.get("constants", []):
            out.append(f"    /// {c['title']}.")
            out.append(f"    pub const {c['name']}: u32 = {c['value']};")
        out.append("}")
        out.append("")
    out.append("/// A device of the configuration: adapter, base address, vector and")
    out.append("/// the bus request level.")
    out.append("#[derive(Clone, Copy, Debug, PartialEq, Eq)]")
    out.append("pub struct Instance {")
    out.append("    pub adapter: u32,")
    out.append("    pub base: u32,")
    out.append("    pub vector: u32,")
    out.append("    pub br: u32,")
    out.append("}")
    out.append("")
    for i in data.get("instance", []):
        out.append(f"/// {i['sources']}.")
        out.append(
            f"pub const {i['name'].upper()}: Instance = Instance {{ adapter: {i['adapter']}, "
            f"base: {oct_(i['base'])}, vector: {oct_(i['vector'])}, br: {i['br']} }};"
        )
    out.append("")
    return "\n".join(out)


def render_md(data: dict) -> str:
    out = [
        "# Device registers",
        "",
        "Generated by `spec/gen_devices.py` from `spec/devices.toml`. Do not",
        "edit. Numbers are octal. The source files are in `src/system/` of the",
        "ITS tree **[its]**.",
        "",
        "## Devices of the configuration",
        "",
        "| Device | Type | Adapter | Base | Vector | BR | Sources |",
        "|---|---|---|---|---|---|---|",
    ]
    for i in data.get("instance", []):
        out.append(
            f"| {i['name']} | {i['device']} | {i['adapter']} | {i['base']:o} | "
            f"{i['vector']:o} | {i['br']} | {i['sources']} |"
        )
    out.append("")
    for dev in data["device"]:
        out.append(f"## {dev['name']}: {dev['title']}")
        out.append("")
        out.append(f"Source: `{dev['source']}`.")
        out.append("")
        if dev.get("registers"):
            out.append("| Register | Place | ITS name | Function |")
            out.append("|---|---|---|---|")
            for r in dev["registers"]:
                place = f"+{r['offset']:o}" if "offset" in r else f"{r['addr']:o}"
                if r.get("count", 1) > 1:
                    place += f" ({r['count']} words)"
                out.append(f"| {r['name']} | {place} | {r.get('its', '')} | {r['title']} |")
            out.append("")
        for group in dev.get("fields", []):
            out.append(f"Fields of {group['register']}:")
            out.append("")
            out.append("| Field | Bits | Mask | ITS name | Function |")
            out.append("|---|---|---|---|---|")
            for f in group["fields"]:
                lo, w = f["lsb"], f.get("width", 1)
                bits = f"{lo}" if w == 1 else f"{lo + w - 1}-{lo}"
                out.append(
                    f"| {f['name']} | {bits} | {field_mask(f):o} | {f.get('its', '')} | {f['title']} |"
                )
            out.append("")
        for group in dev.get("values", []):
            out.append(f"Values of {group['register']}.{group['field']}:")
            out.append("")
            out.append("| Name | Value | ITS name |")
            out.append("|---|---|---|")
            for v in group["values"]:
                out.append(f"| {v['name']} | {v['value']:o} | {v.get('its', '')} |")
            out.append("")
        if dev.get("constants"):
            out.append("| Constant | Value | ITS name | Meaning |")
            out.append("|---|---|---|---|")
            for c in dev["constants"]:
                out.append(f"| {c['name']} | {c['value']} | {c.get('its', '')} | {c['title']} |")
            out.append("")
    return "\n".join(out)


# --- The symbols of the ITS sources --------------------------------------

SYM_RE = re.compile(r"^\s*(?:DEFSYM\s+)?([%$A-Z0-9.]+)\s*(==:|=:|==|=)\s*([^;]*)")


def read_symbols(path: Path) -> dict:
    """The symbol definitions of a MIDAS file: {name: expression text}.
    Only the first definition of a name counts."""
    syms = {}
    for line in path.read_text(encoding="latin-1").splitlines():
        m = SYM_RE.match(line)
        if m and m.group(1) not in syms:
            syms[m.group(1)] = m.group(3).strip()
    return syms


TOKEN_RE = re.compile(r"\s*(\.BP|[0-9]+\.?|[%$A-Z][%$A-Z0-9.]*|[+\-_])")


def evaluate(expr: str, syms: dict, depth: int = 0) -> int:
    """Value of a MIDAS expression with numbers (octal; decimal with a
    trailing point), symbols, `a_b` (a shifted left b), `.BP m` (the mask
    m), `+` and `-`."""
    if depth > 20:
        raise DeviceError(f"expression too deep: {expr}")
    if ",," in expr:
        lh, rh = expr.split(",,", 1)
        return (evaluate(lh, syms, depth + 1) << 18) | (evaluate(rh, syms, depth + 1) & 0o777777)
    pos, toks = 0, []
    while pos < len(expr):
        m = TOKEN_RE.match(expr, pos)
        if not m:
            if expr[pos:].strip() == "":
                break
            raise DeviceError(f"cannot read {expr!r}")
        toks.append(m.group(1))
        pos = m.end()

    def atom(t):
        if re.fullmatch(r"[0-9]+\.", t):
            return int(t[:-1], 10)
        if re.fullmatch(r"[0-9]+", t):
            return int(t, 8)
        if t not in syms:
            raise DeviceError(f"unknown symbol {t}")
        return evaluate(syms[t], syms, depth + 1)

    total, sign, i = 0, 1, 0
    while i < len(toks):
        t = toks[i]
        if t in "+-":
            sign = 1 if t == "+" else -1
            i += 1
            continue
        if t == ".BP":
            v = atom(toks[i + 1])
            i += 2
        else:
            v = atom(t)
            i += 1
        while i < len(toks) and toks[i] == "_":
            v <<= atom(toks[i + 1])
            i += 2
        total += sign * v
        sign = 1
    return total


def check_its(data: dict, its_tree: Path) -> list:
    """Compare the table with the ITS sources. Returns a list of errors."""
    errors = []
    sysdir = its_tree / "src" / "system"
    cache = {}

    def syms(name):
        if name not in cache:
            cache[name] = read_symbols(sysdir / name)
        return cache[name]

    def value(sym, file):
        s = syms(file)
        if sym not in s:
            raise DeviceError(f"{file}: no symbol {sym}")
        return evaluate(s[sym], s)

    insts = {i["name"]: i for i in data.get("instance", [])}
    for dev in data["device"]:
        src = dev["source"]
        base = insts[dev["base_instance"]]["base"] if "base_instance" in dev else 0
        try:
            for r in dev.get("registers", []):
                if "its" in r:
                    want = r["addr"] if "addr" in r else base + r["offset"]
                    if r.get("adapter_in_its"):
                        # The ITS symbol is adapter,,address (3,,764140).
                        want |= insts[dev["base_instance"]]["adapter"] << 18
                    got = value(r["its"], src)
                    if got != want:
                        errors.append(f"{dev['name']}.{r['name']}: table {want:o}, {r['its']} = {got:o}")
            for g in dev.get("fields", []):
                for f in g["fields"]:
                    if "its" in f:
                        got = value(f["its"], src) << (18 if f.get("lh") else 0)
                        if f.get("bp"):
                            pos, size = got >> 12, (got >> 6) & 0o77
                            got = ((1 << size) - 1) << pos
                        if got != field_mask(f):
                            errors.append(
                                f"{dev['name']}.{g['register']}.{f['name']}: mask {field_mask(f):o}, "
                                f"{f['its']} = {got:o}"
                            )
            for g in dev.get("values", []):
                for v in g["values"]:
                    if "its" in v and value(v["its"], src) != v["value"]:
                        errors.append(f"{dev['name']}.{v['name']}: {v['value']:o} != {v['its']}")
            for c in dev.get("constants", []):
                if "its" in c:
                    s = syms(src)
                    got = evaluate(c["its"], s)
                    if got != c["value"]:
                        errors.append(f"{dev['name']}.{c['name']}: {c['value']} != {c['its']} = {got}")
        except DeviceError as e:
            errors.append(f"{dev['name']}: {e}")
    for i in data.get("instance", []):
        for key, field in (("its_base", "base"), ("its_vector", "vector")):
            if key in i:
                try:
                    got = value(i[key]["symbol"], i[key]["file"])
                    if got != i[field]:
                        errors.append(f"instance {i['name']}: {field} {i[field]:o} != {i[key]['symbol']} = {got:o}")
                except DeviceError as e:
                    errors.append(f"instance {i['name']}: {e}")
    return errors


def main(argv=None) -> int:
    ap = argparse.ArgumentParser(description="Generate the device register files.")
    ap.add_argument("--check", action="store_true")
    ap.add_argument("--its", type=Path)
    ap.add_argument("--root", type=Path, default=Path(__file__).resolve().parent.parent)
    args = ap.parse_args(argv)
    try:
        data = load(args.root / SOURCE)
    except (DeviceError, tomllib.TOMLDecodeError) as e:
        print(f"{SOURCE}: {e}", file=sys.stderr)
        return 1
    status = 0
    if args.its:
        for e in check_its(data, args.its):
            print(f"its: {e}", file=sys.stderr)
            status = 1
    outputs = {OUT_RS: render_rust(data), OUT_MD: render_md(data)}
    for rel, text in outputs.items():
        p = args.root / rel
        if args.check:
            if not p.exists() or p.read_text() != text:
                print(f"{rel} is not current; run make gen", file=sys.stderr)
                status = 1
        else:
            p.write_text(text)
            print(f"wrote {rel}")
    return status


if __name__ == "__main__":
    sys.exit(main())
