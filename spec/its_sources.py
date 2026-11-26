"""Read the instruction set data from the ITS sources (submodule 3rdparty/its).

gen_instructions.py uses this module to make spec/instructions.toml from the
sources (--import) and to check the table against them (--its). The sources:

  - The KS10 microcode of ITS, src/kshack/: the dispatch ROM (DRAM), which
    gives for each opcode how the operands are fetched (field A), how the
    result is stored (field B), the microcode entry label (field J) and
    control bits. Also the microinstructions at fixed control RAM (CRAM)
    addresses, which hold the dispatch on the AC field.
  - The initial symbol table of MIDAS (label ISYMTB): the names of opcodes
    104 to 677, in the order of the opcodes.
  - SYSTEM;KSDEFS: the names of the KS10 and ITS instructions in 700 to 777.

Only the Python standard library is used.
"""

import re
from pathlib import Path

# The files of the microcode assembly, in the order of the assembly
# (AGENTS.md, "Sources of truth").
UCODE_FILES = ("its", "ks10", "simple", "flt", "extend", "inout", "itspag", "pagef")

# I/O instruction functions: bits 10 to 12 of the instruction.
IO_FUNCTIONS = {"BLKI": 0, "DATAI": 1, "BLKO": 2, "DATAO": 3,
                "CONO": 4, "CONI": 5, "CONSZ": 6, "CONSO": 7}


class SourceError(Exception):
    """A source file does not have the expected form."""


def ucode_file(ucode_dir: Path, stem: str) -> Path:
    """The one file '<stem>.<version>' of the microcode directory."""
    found = [p for p in ucode_dir.iterdir() if re.fullmatch(re.escape(stem) + r"\.\d+", p.name)]
    if len(found) != 1:
        raise SourceError(f"{ucode_dir}: expected one file {stem}.<n>, found {len(found)}")
    return found[0]


def _active_lines(ucode_dir: Path):
    """Yield (file name, line number, mode, text, comment) for each line that
    the assembly uses. mode is 'D' after .DCODE and 'U' after .UCODE.

    Conditional assembly: .SET/X=n defines X; .IF/X assembles if X is not 0,
    .IFNOT/X if X is 0, and .ENDIF/X ends the condition on X (ks10, section
    8.0 "PSEUDO OPS"). An .IFNOT/X after an .IF/X replaces the condition on X.
    A parameter without .SET counts as 0.
    """
    sym = {}
    for stem in UCODE_FILES:
        path = ucode_file(ucode_dir, stem)
        mode = "U"
        cond = {}
        with open(path, encoding="latin-1") as f:
            for n, raw in enumerate(f, 1):
                raw = raw.rstrip("\n").replace("\f", "")
                text, _, comment = raw.partition(";")
                t = text.strip()
                m = re.match(r"\.SET/([A-Z0-9]+)=(\d+)", t)
                if m:
                    if all(cond.values()):
                        sym[m.group(1)] = int(m.group(2))
                    continue
                m = re.match(r"\.(IF|IFNOT|ENDIF)/([A-Z0-9]+)", t)
                if m:
                    kind, name = m.groups()
                    if kind == "ENDIF":
                        cond.pop(name, None)
                    else:
                        value = sym.get(name, 0)
                        cond[name] = bool(value) if kind == "IF" else not value
                    continue
                if not all(cond.values()):
                    continue
                if t.startswith(".DCODE"):
                    mode = "D"
                    continue
                if t.startswith(".UCODE"):
                    mode = "U"
                    continue
                yield path.name, n, mode, text, comment.strip()


def _norm(text: str) -> str:
    return ", ".join(x.strip() for x in text.split(",") if x.strip())


def read_ucode(ucode_dir: Path) -> dict:
    """Read the microcode. The result has:

    fields  {field: (first bit, last bit, default, {value name: value})} of
            the DRAM (ks10, "DISPATCH ROM DEFINITIONS")
    macros  {name: text} of the DRAM macros
    dram    {opcode: (text, comment, file, line)}
    cram    {address: (labels, text, comment, file, line)}: the
            microinstructions at an explicit address ("nnnn:"); labels is a
            tuple of the labels at that address, in the order of the source
    """
    fields, macros, dram, cram = {}, {}, {}, {}
    field = None
    addr = None
    caddr = None
    labels = []
    for fname, n, mode, text, comment in _active_lines(ucode_dir):
        t = text.strip()
        if not t:
            continue
        if mode == "D":
            m = re.match(r"([A-Z][A-Z0-9 -]*)/=<(\d+)(?::(\d+))?>(?:D,(\d+))?", text)
            if m:
                field = m.group(1).strip()
                first = int(m.group(2))
                last = int(m.group(3)) if m.group(3) else first
                default = int(m.group(4), 8) if m.group(4) else 0
                fields[field] = (first, last, default, {})
                continue
            m = re.match(r"\s+([A-Z0-9-]+)=([0-7]+)\s*$", text)
            if m and field:
                fields[field][3][m.group(1)] = int(m.group(2), 8)
                continue
            m = re.match(r'([^\s"][^"]*?)\s+"([^"]*)"\s*$', text)
            if m:
                macros[m.group(1).strip()] = m.group(2)
                continue
            m = re.match(r"([0-7]{3}):\s*(.*)", t)
            if m:
                addr = int(m.group(1), 8)
                t = m.group(2).strip()
                if not t:
                    continue
            if addr is None:
                raise SourceError(f"{fname}:{n}: DRAM entry without an address")
            if addr in dram:
                raise SourceError(f"{fname}:{n}: second DRAM entry for {addr:03o}")
            dram[addr] = (_norm(t), comment, fname, n)
            addr += 1
        else:
            m = re.match(r"([0-7]{4}):\s*(.*)", t)
            if m:
                caddr = int(m.group(1), 8)
                labels = []
                t = m.group(2).strip()
                if not t:
                    continue
            m = re.match(r"([A-Z][A-Z0-9.%$-]*):\s*(.*)", t)
            if m:
                if caddr is not None:
                    labels.append(m.group(1))
                t = m.group(2).strip()
                if not t:
                    continue
            if caddr is not None:
                cram[caddr] = (tuple(labels), _norm(t), comment, fname, n)
                caddr = None
                labels = []
    if len(dram) != 512:
        raise SourceError(f"{ucode_dir}: {len(dram)} DRAM entries, expected 512")
    return {"fields": fields, "macros": macros, "dram": dram, "cram": cram}


def expand(text: str, macros: dict) -> list:
    """The DRAM text as a list of 'FIELD/VALUE' items, macros expanded."""
    out = []
    for tok in [x.strip() for x in text.split(",") if x.strip()]:
        if tok in macros:
            out += expand(macros[tok], macros)
        elif "/" in tok:
            out.append(tok)
        else:
            raise SourceError(f"unknown DRAM macro {tok!r}")
    return out


def cram_labels(cram: dict) -> dict:
    """{label: address} for the labels at explicit addresses."""
    return {lab: a for a, entry in cram.items() for lab in entry[0]}


def read_midas_names(midas: Path) -> dict:
    """{opcode: name} from the table ISYMTB of MIDAS: one line for each
    opcode from 104 (ISMTBB) to 677; a line '0' is an opcode without a name."""
    names = {}
    state = 0
    code = None
    with open(midas, encoding="latin-1") as f:
        for raw in f:
            t = raw.split(";", 1)[0].strip()
            if state == 0:
                m = re.match(r"ISMTBB:\s+(\S+)", t)
                if m:
                    if m.group(1) != "JSYS":
                        raise SourceError(f"{midas}: ISMTBB is not JSYS")
                    code = 0o104
                if t.startswith("ISYMTB:"):
                    if code is None:
                        raise SourceError(f"{midas}: ISYMTB before ISMTBB")
                    state = 1
                continue
            if t.startswith("EISYM1:"):
                break
            assert code is not None
            if not t:
                continue
            m = re.match(r"SQUOZE\s+10,([A-Z0-9.%$]+)$", t)
            if m:
                names[code] = m.group(1)
            elif t != "0":
                raise SourceError(f"{midas}: unexpected line in ISYMTB: {t!r}")
            code += 1
    if code != 0o700:
        raise SourceError(f"{midas}: ISYMTB ends at {code:o}, expected 700")
    return names


def read_ksdefs_names(ksdefs: Path) -> tuple:
    """The instruction names of KSDEFS.

    Returns ({opcode: name} for 'NAME=:nnn^9', {(opcode, ac): name} for
    'NAME==:IOFUNC DEV,'). The device symbols are the ones that KSDEFS
    defines with 'SYM==:n' and the numbers.
    """
    devices = {}
    ops, acops = {}, {}
    with open(ksdefs, encoding="latin-1") as f:
        text = [raw.split(";", 1)[0].strip() for raw in f]
    for t in text:
        m = re.match(r"([A-Z.][A-Z0-9.]*)==:([0-7]+)$", t)
        if m:
            devices[m.group(1)] = int(m.group(2), 8)
    for t in text:
        m = re.match(r"([A-Z0-9]+)=:([0-7]{3})\^9$", t)
        if m:
            ops[int(m.group(2), 8)] = m.group(1)
            continue
        m = re.match(r"([A-Z0-9]+)==?:(BLKI|DATAI|BLKO|DATAO|CONO|CONI|CONSZ|CONSO)\s+([A-Z0-9.]+),$", t)
        if m:
            dev = m.group(3)
            dev = int(dev, 8) if re.fullmatch(r"[0-7]+", dev) else devices.get(dev)
            if dev is None:
                raise SourceError(f"{ksdefs}: unknown device in {t!r}")
            word = 0o700000 | (dev << 6) | (IO_FUNCTIONS[m.group(2)] << 5)
            acops[(word >> 9, (word >> 5) & 0o17)] = m.group(1)
    return ops, acops
