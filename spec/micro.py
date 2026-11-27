#!/usr/bin/env python3
"""MICRO, the microcode assembler of the ITS KS10, for the ITS sources.

This program assembles the ITS KS10 microcode (src/kshack/ of the
submodule 3rdparty/its) as the ITS build does with MICRO:

    :kshack;micro kshack;mcr 262=kshack;its,ks10,simple,flt,extend,inout,itspag,pagef

(3rdparty/its, build/ks10/processor.tcl). It follows the rules of the
program MICRO (3rdparty/its, src/kshack/micro.117). The comments give the
label of micro.117 for each rule. The program has only the functions that
these eight files use; another construct is an error, not a guess.

The result is the control RAM (108-bit words), the dispatch ROM (36-bit
words), and the symbols of the fields, with the address tags (field J).
spec/gen_microcode.py writes them into emu/ks10-core-uop/src/ucode.rs.

Usage:
  micro.py --its DIR [--compare LISTING [--no-lines]]
    --its DIR          the ITS tree (the submodule 3rdparty/its).
    --compare LISTING  compare with a listing of MICRO: each U and D word
                       with its address and its line, and the cross
                       reference (the fields and symbols in their order,
                       with the line of each definition). Exit status 1
                       on a difference.
    --no-lines         do not compare the line numbers, for a listing of
                       other source text with the same words.

Only the Python standard library is used.
"""

import argparse
import re
import sys
from pathlib import Path

import its_sources as its

# micro.117: NCHARS, NWORDS, MAXPC, MAXDSP, MICMXB, NARGWD.
NCHARS = 20
NWORDS = (NCHARS + 4) // 5
MAX_NAME = (NWORDS - 1) * 5 + 4  # TOK3: the last byte of NAME stays 0
MAXPC = 2047
MAXDSP = 512
MICMXB = 144
MAX_ARGS = 16 * 5 - 1  # TKA1: NARGWD words of 7-bit characters
USAGE_BITS = (MAXPC // 36 + 1) * 36

UCODE, DISP = 0, 1

# CHRTAB: the token type of a character for the token builder (TOKTAB
# columns), and its terminator type for the top level state table.
TERM, BLANK, DOT, PLUS, MINUS, OCT, DEC, SYM = range(8)
NOTERM, COL, COM, EOL, EQL, QOT, SLSH = range(7)
LF = "\n"


def _chrtab():
    """(type, terminator, translation) for each 7-bit code (CHRTAB)."""
    tab = []
    for c in range(128):
        if c in (0o0, 0o3, 0o15, 0o177):
            entry = (TERM, NOTERM, 0)  # NUL, EOF, CR, DEL: ignored
        elif c == 0o11:
            entry = (TERM, NOTERM, 0o40)  # TAB is a space
        elif c in (0o12, 0o13, 0o14):
            entry = (TERM, EOL, 0o12)  # LF, VT, FF end a line
        elif c == 0o10 or chr(c) in "$%&'<>?@\\^`{|}~":
            entry = (TERM, NOTERM, c)
        elif c == 0o40:
            entry = (BLANK, NOTERM, c)
        elif chr(c) == '"':
            entry = (TERM, QOT, c)
        elif chr(c) == ",":
            entry = (TERM, COM, c)
        elif chr(c) == "/":
            entry = (TERM, SLSH, c)
        elif chr(c) == ":":
            entry = (TERM, COL, c)
        elif chr(c) == ";":
            entry = (TERM, EOL, c)
        elif chr(c) == "=":
            entry = (TERM, EQL, c)
        elif chr(c) == "+":
            entry = (PLUS, NOTERM, c)
        elif chr(c) == "-":
            entry = (MINUS, NOTERM, c)
        elif chr(c) == ".":
            entry = (DOT, NOTERM, c)
        elif "0" <= chr(c) <= "7":
            entry = (OCT, NOTERM, c)
        elif chr(c) in "89":
            entry = (DEC, NOTERM, c)
        elif "a" <= chr(c) <= "z":
            entry = (SYM, NOTERM, c - 0o40)
        else:
            entry = (SYM, NOTERM, c)
        tab.append(entry)
    return tab


CHRTAB = _chrtab()

# TOKTAB: for each state of the token builder, by token type of the
# character: new state * 8 + action. Actions: 0 ignore, 1 include, 2 minus
# sign, 3 digit, 4 blank token, 5 symbol, 6 decimal number, 7 octal number.
TOKTAB = [
    [0o04, 0o00, 0o11, 0o21, 0o22, 0o23, 0o33, 0o11],
    [0o05, 0o61, 0o11, 0o11, 0o11, 0o11, 0o11, 0o11],
    [0o07, 0o71, 0o41, 0o11, 0o11, 0o23, 0o33, 0o11],
    [0o06, 0o101, 0o41, 0o11, 0o11, 0o33, 0o33, 0o11],
    [0o06, 0o111, 0o11, 0o11, 0o11, 0o11, 0o11, 0o11],
    [0o54, 0o50, 0o11, 0o31, 0o32, 0o33, 0o33, 0o11],
    [0o05, 0o60, 0o11, 0o11, 0o11, 0o11, 0o11, 0o11],
    [0o07, 0o70, 0o41, 0o11, 0o11, 0o23, 0o33, 0o11],
    [0o06, 0o100, 0o41, 0o11, 0o11, 0o33, 0o33, 0o11],
    [0o06, 0o110, 0o11, 0o11, 0o11, 0o11, 0o11, 0o11],
]

TKB, TKN, TKS = "blank", "number", "symbol"

# The top level state table (ITEM): (state, token, terminator) to
# (action, new state). States: 0 start of a line or microword, 1 FIELD/ at
# the start, 2 in a microword after a comma, 3 FIELD/ in a microword,
# 4 assembly suppressed.
STATES = {
    (0, TKB, EQL): ("lblk", 0),
    (0, TKS, COL): ("tag", 0),
    (0, TKN, COL): ("lset", 0),
    (0, TKS, EQL): ("defs", 0),
    (0, TKS, SLSH): ("cfld", 1),
    (0, TKS, QOT): ("mdef", 0),
    (0, TKS, EOL): ("sudo", 0),
    (0, TKS, COM): ("mac", 0),
    (0, TKB, EOL): ("nop", 0),
    (1, TKB, EQL): ("deff", 0),
    (1, TKS, COM): ("fsym", 2),
    (1, TKS, EOL): ("fsym", 0),
    (1, TKN, COM): ("fnum", 2),
    (1, TKN, EOL): ("fnum", 0),
    (2, TKS, SLSH): ("fld", 3),
    (2, TKS, COM): ("mac", 2),
    (2, TKS, EOL): ("mac", 2),
    (2, TKB, EOL): ("nop", 2),
    (3, TKS, COM): ("fsym", 2),
    (3, TKS, EOL): ("fsym", 0),
    (3, TKN, COM): ("fnum", 2),
    (3, TKN, EOL): ("fnum", 0),
    (4, TKB, EQL): ("cmnt", 4),
    (4, TKS, COL): ("nop", 4),
    (4, TKN, COL): ("nop", 4),
    (4, TKS, EQL): ("cmnt", 4),
    (4, TKS, SLSH): ("cfld", 4),
    (4, TKS, QOT): ("cmnt", 4),
    (4, TKS, EOL): ("cmnt", 4),
    (4, TKS, COM): ("cmnt", 4),
    (4, TKB, EOL): ("cmnt", 4),
}

# DEFTAB: the special function of a field.
FN_NONE, FN_PCINC, FN_DEFAULT, FN_TIME, FN_PARITY, FN_OTHER = range(6)
FN_CHARS = {"+": FN_PCINC, "D": FN_DEFAULT, "T": FN_TIME, "P": FN_PARITY, "F": FN_OTHER}

# LOCTAB operations (%LCTAG, %LCABS, %LCUCD, %LCEQL).
LC_TAG, LC_ABS, LC_UCD, LC_EQL = range(4)


class MicroError(Exception):
    """An error of the assembly, with the place in the source."""


def name_words(name: str) -> tuple:
    """The name as NWORDS signed 36-bit words of 7-bit ASCII, five
    characters to a word from the left (SYMTXT), for the order of the
    symbol table (SRCFLD, SRCSY1: CAMLE and CAME compare signed words)."""
    codes = [ord(c) for c in name] + [0] * (NWORDS * 5 - len(name))
    words = []
    for k in range(NWORDS):
        w = 0
        for c in codes[5 * k : 5 * k + 5]:
            w = (w << 7) | c
        w <<= 1
        if w & (1 << 35):
            w -= 1 << 36
        words.append(w)
    return tuple(words)


class Node:
    """A field or a symbol (SYMLNK, SYMTXT, SYMVAL)."""

    def __init__(self, name, ram):
        self.name = name
        self.key = name_words(name)
        self.ram = ram  # DEFTYP
        self.next_field = None  # RH of SYMLNK
        self.next_sym = None  # LH of SYMLNK
        self.value = 0  # DEFVAL; for F, the other field
        self.fnc = FN_NONE  # DEFFNC
        self.pos = 0  # DEFPOS: the right bit of a field
        self.size = 0  # DEFSIZ
        self.tm1 = 0  # DEFTM1
        self.tm2 = 0  # DEFTM2
        self.text = None  # SYMMAC: the text of a macro
        self.line = None  # the line of the definition (CREF)


class Frame:
    """A text that the input reads again: a macro or a macro argument
    (SAVE, RESTOR)."""

    def __init__(self, text, saved, args):
        self.text = text
        self.pos = 0
        self.saved = saved  # the character before the text, or None
        self.args = args  # the macro arguments of the frame below


class Token:
    def __init__(self, kind, name, number, args):
        self.kind = kind
        self.name = name
        self.number = number
        self.args = args


class Assembler:
    def __init__(self, files):
        """files: [(name, text)] in the order of the command line."""
        self.files = files
        self.fields = None  # FLDPNT: the head of the field list
        self.loctab = []
        self.switch_field = None  # SWTPNT
        self.macro_fields = [None, None]  # MACPNT
        self.j_fields = [None, None]  # JPNT
        self.maxps1 = [0, 0]
        self.maxpos = [0, 0]
        self.ramfile_fields = []  # (source bit, width, dest bit)
        self.ramfile_parity = []  # (dest bit, first, count, complement)
        self.ramfile_words = 0
        self.supsym = None

    # ---- errors ----------------------------------------------------

    def where(self):
        return f"line {self.line} (pass {2 if self.pass2 else 1})"

    def error(self, msg):
        raise MicroError(f"{self.where()}: {msg}")

    # ---- input (GETCHR, GETCHM, SAVE, RESTOR) ------------------------

    def begin_input(self):
        self.file_index = 0
        self.file_pos = 0
        self.frames = []
        self.args = []
        self.reread = False
        self.lastc = None
        self.endfil = False

    def getchr(self):
        if self.reread:
            self.reread = False
            return self.lastc
        while True:
            if self.frames:
                f = self.frames[-1]
                if f.pos < len(f.text):
                    c = f.text[f.pos]
                    f.pos += 1
                    self.lastc = c
                    return c
                # RESTOR: the arguments and the character before the text.
                self.frames.pop()
                self.args = f.args
                if f.saved:
                    self.lastc = f.saved
                    return f.saved
                continue
            if self.endfil:
                self.error("read past the end of the input")
            name, text = self.files[self.file_index]
            if self.file_pos < len(text):
                raw = text[self.file_pos]
                self.file_pos += 1
                code = ord(raw)
                if code > 127:
                    self.error(f"character {code:o} in {name}")
                c = CHRTAB[code][2]
                if c == 0:
                    continue
                self.lastc = chr(c)
                return self.lastc
            # GETC7, NXTFIL: a new file starts with a form feed, the end
            # of the input is a line feed. Both end a line.
            self.file_index += 1
            self.file_pos = 0
            if self.file_index >= len(self.files):
                self.endfil = True
            self.lastc = LF
            return LF

    def push(self, text, saved, args):
        """SAVE: read text, then return the character saved."""
        self.frames.append(Frame(text, saved, self.args))
        self.args = args

    def getchm(self, token_args):
        """GETCHM: @n in a macro reads argument n."""
        while True:
            c = self.getchr()
            if c != "@" or not self.frames:
                return c
            d = self.getchr()
            if not ("1" <= d <= "9"):
                self.error("MACRO ARGUMENT NUMBER OUT OF RANGE")
            k = ord(d) - ord("1")
            if k >= len(self.args):
                self.error(f"macro argument @{d} not given")
            # SAVE takes the arguments of the token being built.
            self.push(self.args[k], None, list(token_args))

    def scnend(self, c):
        """SCNEND: skip to the end of the line."""
        while c != LF:
            c = self.getchr()
        return c

    # ---- tokens (TOKEN, TOKN10) --------------------------------------

    def token(self, decimal=False):
        state = 5 if decimal else 0
        name = ""
        oct_value = dec_value = 0
        minus = False
        args = []
        while True:
            c = self.getchm(args)
            entry = TOKTAB[state][CHRTAB[ord(c)][0]]
            state, action = entry >> 3, entry & 7
            if action == 0:
                continue
            if action == 2:
                minus = True
            elif action == 3:
                d = ord(c) - ord("0")
                oct_value = oct_value * 8 + d
                dec_value = dec_value * 10 + d
            elif action >= 4:
                self.c = c
                if action == 4:
                    return Token(TKB, "", 0, args)
                if action == 5:
                    # TOK6: a trailing space is not part of the name.
                    if name.endswith(" "):
                        name = name[:-1]
                    return Token(TKS, name, 0, args)
                n = dec_value if action == 6 else oct_value
                return Token(TKN, name, -n if minus else n, args)
            # TOK3: include the character.
            if len(name) < MAX_NAME:
                name += c
            if c == "[":
                # TKA1: the macro argument up to "]".
                arg = ""
                while True:
                    a = self.getchm(args)
                    if a == "]":
                        break
                    arg += a
                    if sum(len(x) + 1 for x in args) + len(arg) >= MAX_ARGS:
                        self.error("MACRO ARGUMENTS TOO LONG")
                args.append(arg)
                if len(name) < MAX_NAME:
                    name += "]"

    # ---- symbol table (SRCFLD, SRCSY1, MAKFLD, MAKSYM) ---------------

    def find_field(self, name, ram):
        """SRCFLD: (node, None) if found, else (None, previous node)."""
        key = name_words(name)
        prev, n = None, self.fields
        while n is not None:
            if key > n.key:
                prev, n = n, n.next_field
                continue
            if key != n.key:
                return None, prev
            if n.ram == ram:
                return n, None
            prev, n = n, n.next_field
        return None, prev

    def make_field(self, name, ram):
        """MAKFLD: (node, new)."""
        f, prev = self.find_field(name, ram)
        if f is not None:
            if not self.pass2:
                self.error(f"MULTIPLE DEFINITION of field {name}")
            return f, False
        if not name:
            self.error("CAN'T DEFINE A NULL FIELD")
        if self.pass2:
            self.error(f"FIELD UNDEFINED ON PASS 2: {name}")
        f = Node(name, ram)
        if not name.endswith("%"):
            f.line = self.line
        if prev is None:
            f.next_field, self.fields = self.fields, f
        else:
            f.next_field, prev.next_field = prev.next_field, f
        return f, True

    def find_symbol(self, field, name):
        """SRCSY1: (node, None) if found, else (None, previous node)."""
        key = name_words(name)
        prev, n = field, field.next_sym
        while n is not None:
            if key > n.key:
                prev, n = n, n.next_sym
                continue
            if key != n.key:
                return None, prev
            return n, None
        return None, prev

    def make_symbol(self, field, name):
        """MAKSYM: (node, new)."""
        s, prev = self.find_symbol(field, name)
        if s is not None:
            if not self.pass2:
                self.error(f"MULTIPLE DEFINITION of {name} in {field.name}")
            return s, False
        if not name:
            self.error("CAN'T DEFINE NULL SYMBOL")
        if self.pass2:
            self.error(f"SYMBOL UNDEFINED ON PASS2: {name}")
        s = Node(name, self.ram)
        s.line = self.line
        s.next_sym, prev.next_sym = prev.next_sym, s
        return s, True

    def defchk(self, node, attr, value, bits):
        """DEFCHK: store a value on pass 1, compare it on pass 2."""
        if value < 0 or value >= 1 << bits:
            self.error(f"NUMBER TOO BIG FOR FIELD: {value}")
        if self.pass2:
            if getattr(node, attr) != value:
                self.error(f"PASS1 AND PASS2 DEFINITIONS DIFFER: {node.name}")
        else:
            setattr(node, attr, value)

    # ---- LOCTAB (ADDLOC) ---------------------------------------------

    def addloc(self, op, data):
        """ADDLOC: pass 1 adds an entry, pass 2 compares it. Returns the
        entry."""
        if not self.pass2:
            entry = {"op": op, "data": data, "pc": None}
            self.loctab.append(entry)
            return entry
        if self.locpnt >= len(self.loctab):
            self.error("PASS 2 READ PAST END OF LOCTAB")
        entry = self.loctab[self.locpnt]
        self.locpnt += 1
        if entry["op"] != op or entry["data"] != data:
            self.error("LOCTAB PHASE ERROR")
        return entry

    # ---- the assembly ------------------------------------------------

    def assemble(self):
        self.cram = {}
        self.drom = {}
        self.lines = {}
        for p in (False, True):
            self.pass2 = p
            if p:
                self.assign()
            self.run_pass()
        if self.pass2 and self.locpnt != len(self.loctab):
            self.error("PASS 2 DIDN'T READ ALL OF LOCTAB")

    def run_pass(self):
        """BEGPAS and STATLP."""
        self.begin_input()
        self.state = 0
        self.pc = [None, None]
        self.locpnt = 0
        self.maxpos = list(self.maxps1)
        self.ram = UCODE
        self.line = 1
        self.usage = set()
        self.dused = set()
        self.supp = False
        self.nobin = False
        self.binf = False
        self.field = ""
        self.c = None
        while True:
            if self.state == 0:
                self.binf = False
                self.value = 0
                self.valset = 0
            tok = self.token()
            key = (self.state, tok.kind, CHRTAB[ord(self.c)][1])
            if key not in STATES:
                self.error(f"ILLEGAL FORMAT ({tok.kind} {tok.name!r} before {self.c!r})")
            action, self.state = STATES[key]
            getattr(self, "do_" + action)(tok)
            if self.state == 0:
                self.pntbin()
            if self.c == ";":
                self.c = self.scnend(self.c)
            if self.c != LF:
                continue
            self.line += 1
            if self.endfil:
                break

    # ---- actions (STDISP) --------------------------------------------

    def do_nop(self, tok):
        pass

    def do_cmnt(self, tok):
        self.c = self.scnend(self.c)

    def do_lblk(self, tok):
        """LOCBLK: =pattern of 0, 1 and *."""
        pat = ast = n = 0
        c = self.getchr()
        while c == " ":
            c = self.getchr()
        while c in "01*":
            pat = (pat << 1) | (c == "1")
            ast = (ast << 1) | (c == "*")
            n += 1
            c = self.getchr()
            while c == " ":
                c = self.getchr()
        self.c = c
        if c != LF:
            self.reread = True
        if self.ram != UCODE:
            # LOCB3: the dispatch RAM moves its PC to the pattern.
            size = 1 << n
            cur = self.pc[DISP]
            low = cur & (size - 1)
            d = pat - low
            if d < 0:
                d += size
            self.pc[DISP] = cur + d
            return
        self.addloc(LC_EQL, (pat, ast, n))

    def do_tag(self, tok):
        """TAG: an address tag in the J field."""
        j = self.j_fields[self.ram]
        if j is None:
            j, _ = self.find_field("J", self.ram)
            if j is None:
                self.error("CAN'T FIND J FIELD")
            self.j_fields[self.ram] = j
        sym, _ = self.make_symbol(j, tok.name)
        if self.ram != UCODE:
            self.defchk(sym, "value", self.pc[DISP], 18)
        else:
            entry = self.addloc(LC_TAG, tok.name)
            if self.pass2:
                self.defchk(sym, "value", entry["pc"], 18)
            else:
                entry["sym"] = sym
        self.field = ""

    def do_lset(self, tok):
        """LOCSET: number: sets the PC."""
        n = tok.number
        if n < 0:
            self.error("LOCATION NEGATIVE")
        if n > (MAXPC if self.ram == UCODE else MAXDSP):
            self.error("LOCATION TOO LARGE")
        self.pc[self.ram] = n
        if self.ram == UCODE:
            self.addloc(LC_ABS, n)
        self.field = ""

    def do_cfld(self, tok):
        """CFSPC: a pseudo field or a field."""
        handler = PSEUDO_FIELDS.get(tok.name)
        if handler is not None:
            getattr(self, handler)(tok)
            return
        if self.supp:
            self.c = self.scnend(self.c)
            return
        self.field = tok.name

    def do_fld(self, tok):
        """FLDSPC."""
        self.field = tok.name

    def switch(self, flag):
        """SWT: .DEFAULT (1), .SET (0), .CHANGE (-1)."""
        if self.supp:
            self.c = self.scnend(self.c)
            return
        saved = self.ram
        self.ram = UCODE
        if self.switch_field is None:
            self.switch_field, _ = self.make_field("SWITCH%", UCODE)
        tok = self.token()
        if tok.kind != TKS:
            self.error("NO SYMBOL IN CONDITIONAL ASSEMBLY DEFINITION")
        sym, _ = self.find_symbol(self.switch_field, tok.name)
        skip = False
        if sym is not None:
            if flag > 0:
                skip = True
            elif not self.pass2 and flag == 0:
                self.error("SWITCH SET TWICE")
        else:
            if flag < 0:
                self.error("SWITCH CHANGED WITHOUT SET OR DEFAULT")
            sym, _ = self.make_symbol(self.switch_field, tok.name)
        if not skip:
            if self.c != "=":
                self.error("FORMAT ERROR ON SWITCH SPECIFICATION")
            val = self.token()
            if val.kind != TKN:
                self.error("SWITCH VALUE MUST BE NUMERIC")
            sym.value = val.number
        self.ram = saved
        self.swt100()

    def swt100(self):
        self.state = 0
        self.field = ""
        self.c = self.scnend(self.c)

    def p_default(self, tok):
        self.switch(1)

    def p_set(self, tok):
        self.switch(0)

    def p_change(self, tok):
        self.switch(-1)

    def conditional(self, flag):
        """$IF (0), $IFNOT (-1), $ENDIF (1)."""
        tok = self.token()
        if tok.kind != TKS:
            self.error("SWITCH MUST BE SYMBOLIC")
        sym = None
        if self.switch_field is not None:
            sym, _ = self.find_symbol(self.switch_field, tok.name)
        if sym is None:
            self.error(f"SWITCH NOT DEFINED: {tok.name}")
        if self.supp and sym is not self.supsym:
            self.c = self.scnend(self.c)
            return
        self.supsym = sym
        if flag > 0:
            self.supp = False
        else:
            self.supp = sym.value == 0
            if flag < 0:
                self.supp = not self.supp
        self.state = 4 if self.supp else 0
        self.c = self.scnend(self.c)

    def p_if(self, tok):
        self.conditional(0)

    def p_ifnot(self, tok):
        self.conditional(-1)

    def p_endif(self, tok):
        self.conditional(1)

    def p_width(self, tok):
        n = self.token(decimal=True)
        if n.kind != TKN:
            self.error(".WIDTH IS SUPPOSED TO BE FOLLOWED BY A DECIMAL NUMBER")
        self.maxpos[self.ram] = n.number - 1
        self.swt100()

    def p_ramfile(self, tok):
        """$RAMFL: the layout of the RAM file."""
        if self.ram != UCODE:
            self.error(".RAMFILE ONLY WORKS FOR UCODE, FOR NOW")
        fields, parity, bitno = [], [], 0
        while True:
            t = self.token(decimal=True)
            if t.kind == TKN:
                # RAMFL2: padding, n/0 or 0.
                n = 1
                if self.c == "/":
                    v = self.token()
                    if v.kind != TKN:
                        self.error("ILLEGAL FORMAT")
                    n, value = t.number, v.number
                else:
                    value = t.number
                if value != 0:
                    self.error("YOU CAN ONLY PAD WITH ZERO")
                if self.pass2:
                    bitno += n
            elif t.kind == TKS and self.c == "<":
                bitno = self.ramfile_parity_spec(t, parity, bitno)
            elif t.kind == TKS and not self.pass2:
                pass  # RAMFL1: the fields are looked up on pass 2 only
            elif t.kind == TKS:
                f, _ = self.find_field(t.name, UCODE)
                if f is None:
                    self.error(f"FIELD NOT DEFINED: {t.name}")
                bitno = self.ramfile_field(f, fields, bitno)
            else:
                # RAMFL7: a line end, or a comment, after a comma.
                if self.c == ";":
                    while self.c != LF:
                        self.c = self.getchr()
                elif self.c != LF:
                    self.error("ILLEGAL FORMAT in .RAMFILE")
                c = self.getchr()
                while c == " ":
                    c = self.getchr()
                self.reread = True
                continue
            # RAMFL8
            if self.c == ",":
                continue
            if self.c in (";", LF):
                break
            self.error("ILLEGAL CHARACTER IN .RAMFILE")
        if self.pass2:
            # RAMFL9: pass 1 counts no field bits.
            if bitno % 36:
                self.error("BITS PER LOCATION IN RAM FILE MUST BE A MULTIPLE OF 36")
            self.ramfile_fields = fields
            self.ramfile_parity = parity
            self.ramfile_words = bitno // 36
        self.swt100()

    def ramfile_field(self, f, fields, bitno):
        """RAMFL4, RAMFL5: the field in its bytes of 36-bit words."""
        first = f.pos - f.size + 1
        bit = first
        while bit <= f.pos:
            end = min(f.pos, bit // 36 * 36 + 35)
            width = end - bit + 1
            if (bitno % 36) + width > 36:
                self.error("DESTINATION BYTE CROSSES WORD BOUNDARY--WON'T WORK")
            fields.append((bit, width, bitno))
            bitno += width
            bit = end + 1
        return bitno

    def ramfile_parity_spec(self, t, parity, bitno):
        """RAMFP1: ODD PARITY<a:b,...> or EVEN PARITY<...>, one bit."""
        if t.name == "ODD PARITY":
            complement = False
        elif t.name == "EVEN PARITY":
            complement = True
        else:
            self.error("UNRECOGNIZED PARITY SPECIFICATION")
        while True:
            a = self.token(decimal=True)
            if a.kind != TKN or self.c != ":":
                self.error("ILLEGAL FORMAT IN FIRST BIT OF PARITY COMPUTATION")
            b = self.token(decimal=True)
            if b.kind != TKN:
                self.error("ILLEGAL FORMAT IN LAST BIT OF PARITY COMPUTATION")
            parity.append((bitno, a.number, b.number - a.number + 1, complement))
            complement = False
            if self.c != ",":
                break
        if self.c != ">":
            self.error("MISSING RIGHT ANGLE BRACKET AFTER PARITY SPEC")
        self.c = self.getchr()
        return bitno + 1

    def do_deff(self, tok):
        """DEFFLD: FIELD/=<first:last>, with D,value or F,field or +."""
        f, new = self.make_field(self.field, self.ram)
        if not new:
            self.c = self.scnend(self.c)
            return
        t = self.token()
        if t.kind != TKB or self.c != "<":
            self.error("only the field definitions with <first:last> are known")
        left = self.token(decimal=True)
        if left.kind != TKN:
            self.error("LEFT BIT POSITION MISSING IN FIELD DEFINITION")
        right = left
        if self.c != ">":
            if self.c != ":":
                self.error("COLON MISSING BETWEEN LEFT AND RIGHT BIT POSITIONS")
            right = self.token(decimal=True)
            if right.kind != TKN or self.c != ">":
                self.error("RIGHT ANGLE-BRACKET MISSING AFTER FIELD BYTE SPEC")
        self.maxps1[self.ram] = max(self.maxps1[self.ram], right.number)
        self.defchk(f, "pos", right.number, 8)
        self.defchk(f, "size", right.number - left.number + 1, 5)
        c = self.getchr()
        self.c = c
        if c in FN_CHARS:
            f.fnc = FN_CHARS[c]
            if f.fnc == FN_DEFAULT:
                self.expect_comma()
                v = self.token()
                if v.kind != TKN:
                    self.error("VALUE REQUIRED FOR FIELD DEFINITION")
                self.defchk(f, "value", v.number, 18)
            elif f.fnc == FN_OTHER:
                self.expect_comma()
                v = self.token()
                if v.kind != TKS:
                    self.error("FIELD NAME MISSING AFTER F MODE CHAR")
                other, _ = self.find_field(v.name, self.ram)
                if other is None:
                    self.error("FIELD TO DEFAULT TO NOT DEFINED YET")
                f.value = other
            elif f.fnc != FN_PCINC:
                self.error("the field functions T and P are not used")
        else:
            # NGETA3: read the character again, and C is 0.
            self.reread = True
            self.c = "\0"
        self.check_definition(f)
        self.c = self.scnend(self.c)

    def expect_comma(self):
        if self.getchr() != ",":
            self.error("COMMA MISSING BETWEEN MODE CHAR AND ITS ARGUMENT")

    def check_definition(self, f):
        """DEFSLF: the field fits the microword, the default fits it."""
        if f.pos > MICMXB:
            self.error("POSITION TOO LARGE FOR MICRO WORD")
        if f.pos + 1 < f.size:
            self.error("SIZE TOO LARGE FOR POSITION")
        if f.fnc != FN_OTHER and isinstance(f.value, int) and f.value >= 1 << f.size:
            self.error("DEFAULT VALUE TOO LARGE FOR FIELD")

    def do_defs(self, tok):
        """DEFSYM: SYMBOL=value[,time1,time2] in the current field."""
        f, _ = self.find_field(self.field, self.ram)
        if f is None:
            self.error(f"UNDEFINED FIELD IN SYMBOL DEFINITION: {self.field!r}")
        sym, _ = self.make_symbol(f, tok.name)
        v = self.token()
        if v.kind != TKN:
            self.error("VALUE REQUIRED IN SYMBOL DEFINITION")
        self.defchk(sym, "value", v.number, 18)
        if sym.value >= 1 << f.size:
            self.error("DEFAULT VALUE TOO LARGE FOR FIELD")
        if self.c == ",":
            t = self.token(decimal=True)
            self.defchk(sym, "tm1", t.number, 4)
            if self.c == ",":
                t = self.token(decimal=True)
                self.defchk(sym, "tm2", t.number, 4)
        self.c = self.scnend(self.c)

    def do_mdef(self, tok):
        """DEFMAC: NAME "text", or .TITLE and .TOC."""
        if tok.name in (".TITLE", ".TOC"):
            self.c = self.scnend(self.c)
            return
        mf = self.macro_fields[self.ram]
        if mf is None:
            mf, _ = self.make_field("MACRO%", self.ram)
            self.macro_fields[self.ram] = mf
            self.field = ""
        sym, new = self.make_symbol(mf, tok.name)
        if new:
            # COPMAC: the text up to the quote, without @n.
            text = ""
            c = self.getchr()
            while c != '"':
                if c == LF:
                    self.error("MISSING TERMINAL QUOTE")
                text += c
                c = self.getchr()
            sym.text = text
            self.c = c
        self.c = self.scnend(self.c)

    def do_sudo(self, tok):
        """PSEUDO: a pseudo op, or a macro."""
        if tok.name == ".UCODE":
            self.ram = UCODE
        elif tok.name == ".DCODE":
            self.ram = DISP
        elif tok.name == ".BIN":
            self.nobin = False
        elif tok.name == ".NOBIN":
            self.nobin = True
        elif tok.name == ".SEQADR":
            self.error(".SEQADR is not used by the ITS sources")
        else:
            self.do_mac(tok)

    def do_mac(self, tok):
        """BEGMAC: read the text of the macro in place of its name."""
        self.field = ""
        mf = self.macro_fields[self.ram]
        sym = None
        if mf is not None:
            sym, _ = self.find_symbol(mf, tok.name)
        if sym is None:
            self.error(f"MACRO NAME NOT DEFINED: {tok.name}")
        self.push(sym.text, self.c, list(tok.args))
        # A macro must not end the line before its text.
        self.c = ","

    def do_fnum(self, tok):
        """FLDNUM: FIELD/number."""
        self.strtwd()
        if not self.pass2:
            self.field = ""
            return
        f = self.word_field()
        n = tok.number
        mask = (1 << f.size) - 1
        if abs(n) & ~mask:
            self.error(f"NUMBER TOO BIG FOR FIELD {f.name}: {n}")
        self.bits(f, n & mask)
        self.field = ""

    def do_fsym(self, tok):
        """FLDSYM: FIELD/symbol, or a symbol of the field of the same name
        in the other RAM."""
        self.strtwd()
        if not self.pass2:
            self.field = ""
            return
        f = self.word_field()
        sym, _ = self.find_symbol(f, tok.name)
        if sym is None:
            other, _ = self.find_field(f.name, 1 - self.ram)
            if other is not None:
                sym, _ = self.find_symbol(other, tok.name)
        if sym is None:
            self.error(f"SYMBOL NOT DEFINED: {f.name}/{tok.name}")
        if sym.fnc != FN_NONE:
            self.error("a symbol with a special function")
        self.bits(f, sym.value)
        self.field = ""

    def word_field(self):
        f, _ = self.find_field(self.field, self.ram)
        if f is None:
            self.error(f"FIELD NOT DEFINED: {self.field!r}")
        return f

    def strtwd(self):
        """STRTWD: the first field of a microword."""
        if self.binf:
            return
        self.binf = True
        if self.nobin:
            self.error("BINARY WORD GENERATED WHILE IN .NOBIN MODE")
        if self.ram == UCODE:
            entry = self.addloc(LC_UCD, self.line)
            if self.pass2:
                self.pc[UCODE] = entry["pc"]

    # ---- the bits of a microword (BITS1, FLDTST, DEFALT) -------------

    def field_mask(self, f):
        return ((1 << f.size) - 1) << (MICMXB - 1 - f.pos)

    def bits(self, f, n):
        """BITS1: put n into the field. A field can overlap fields that
        are set, if the bits agree (SHSTYLE in SPEC, for example)."""
        mask = self.field_mask(f)
        shift = MICMXB - 1 - f.pos
        v = n << shift
        # FLDTST: a bit of the field is set. The bits that are set must
        # agree; then all bits of the field are set (BITS3).
        if (v ^ self.value) & self.valset & mask:
            self.error(f"MICRO FIELD SET WITH CONFLICTING VALUES: {f.name}")
        if v & ~mask:
            self.error(f"VALUE TOO LARGE FOR FIELD {f.name}")
        self.value |= v
        self.valset |= mask

    def defaults(self):
        """DEFALT: the defaults of the fields not set, in the order of the
        field list."""
        f = self.fields
        while f is not None:
            if f.ram == self.ram and not (self.valset & self.field_mask(f)):
                if f.fnc == FN_DEFAULT:
                    self.bits(f, f.value)
                elif f.fnc == FN_PCINC:
                    self.bits(f, self.pcinc())
                elif f.fnc == FN_OTHER:
                    o = f.value
                    shift = MICMXB - 1 - o.pos
                    self.bits(f, (self.value >> shift) & ((1 << o.size) - 1))
                elif f.fnc != FN_NONE:
                    self.error("the field functions T and P are not used")
            f = f.next_field

    def pcinc(self):
        """PCINC: the address of the next microword in LOCTAB."""
        for entry in self.loctab[self.locpnt :]:
            if entry["op"] == LC_UCD:
                return entry["pc"]
        self.error("NO SUCCESSOR INSTRUCTION TO THIS ONE FOUND")

    def pntbin(self):
        """PNTBIN: the end of a microword."""
        if not self.binf:
            return
        if not self.pass2:
            if self.ram != UCODE:
                self.pc[DISP] += 1
            return
        self.defaults()
        pc = self.pc[self.ram]
        # PNTB3: 12-bit groups from the left, up to MAXPOS(RAM).
        width = (self.maxpos[self.ram] // 12 + 1) * 12
        word = self.value >> (MICMXB - width)
        if self.ram == UCODE:
            if pc in self.usage:
                self.error(f"MICRO WORD USED TWICE: {pc:o}")
            self.usage.add(pc)
            self.cram[pc] = word
            self.lines[("U", pc)] = self.line
        else:
            if pc in self.dused:
                self.error(f"MICRO WORD USED TWICE: D {pc:o}")
            self.dused.add(pc)
            self.drom[pc] = word
            self.lines[("D", pc)] = self.line
            self.pc[DISP] += 1

    # ---- location assignment (ASGPCS, FREWRD) ------------------------

    def assign(self):
        """START2: three rounds over LOCTAB: absolute locations, location
        blocks, unconstrained words."""
        self.used = bytearray(USAGE_BITS + 36)
        for blkf, loosf in ((False, False), (True, False), (False, True)):
            self.asgpcs(blkf, loosf)
        for entry in self.loctab:
            if entry["op"] == LC_UCD and entry["pc"] is None:
                raise MicroError(f"line {entry['data']}: INSTRUCTION FAILED TO GET LOCATED")
            if entry["op"] == LC_TAG:
                if entry["pc"] is None:
                    raise MicroError(f"tag {entry['data']}: TAG FAILED TO GET LOCATED")
                entry["sym"].value = entry["pc"]

    def asgpcs(self, blkf, loosf):
        locf = False
        loccnt = 0
        n = 0
        word = 0  # C1: the loose search starts at this USAGE word
        self.frecnt = self.locpat = self.locast = 0
        for entry in self.loctab:
            op = entry["op"]
            if op == LC_ABS:
                n, locf, loccnt = entry["data"], True, 0
            elif op == LC_TAG:
                got, n, locf, loccnt, word = self.asgpc(entry, n, locf, loccnt, loosf, word)
                if got:
                    entry["pc"] = n
            elif op == LC_UCD:
                got, n, locf, loccnt, word = self.asgpc(entry, n, locf, loccnt, loosf, word)
                if got:
                    entry["pc"] = n
                    if self.used[n]:
                        raise MicroError(f"MICRO WORD USED TWICE AT ASGPC4: {n:o}")
                    self.used[n] = 1
                # ASGP4A: the next PC in the block.
                if loccnt <= 0:
                    locf = False
                    continue
                end = loccnt + n
                keep = n & self.locast
                n = (((n | self.locast) + 1) | self.locpat) & ~self.locast | keep
                loccnt = end - n
                if loccnt <= 0:
                    locf = False
            elif op == LC_EQL:
                n, locf, loccnt = self.asgpc6(entry, n, locf, loccnt, blkf)

    def asgpc(self, entry, n, locf, loccnt, loosf, word):
        """ASGPC: (located now, pc, locf, loccnt, word)."""
        if entry["pc"] is not None:
            return False, n, locf, loccnt, word
        if locf:
            return True, n, locf, loccnt, word
        if not loosf:
            return False, n, locf, loccnt, word
        # ASGFR0: the first free word at or after USAGE word C1.
        while word * 36 < USAGE_BITS:
            for b in range(36):
                if not self.used[word * 36 + b]:
                    return True, word * 36 + b, True, 0, word
            word += 1
        raise MicroError("CRAM FULL--CANNOT ASSIGN UNCONSTRAINED PCS")

    def asgpc6(self, entry, n, locf, loccnt, blkf):
        """ASGPC6: a location block =pattern."""
        pat, ast, nb = entry["data"]
        if nb == 0:
            return n, False, 0
        size = 1 << nb
        if loccnt > 0 and size <= self.frecnt:
            # The pattern in the current block: advance the PC to it.
            t3 = self.frecnt - loccnt
            t3 &= ~ast
            t3 &= size - 1
            t3 -= pat
            loccnt += t3
            return n - t3, locf, loccnt
        # ASGPC7: a new block.
        self.locpat, self.locast, self.frecnt = pat, ast, size
        loccnt = size - pat
        if not blkf:
            return n, False, loccnt
        pc = self.frewrd()
        if pc is None:
            raise MicroError("NO SUCH REQUIRED MICRO WORD ADDRESS PATTERN")
        return pc, True, loccnt

    def frewrd(self):
        """FREWRD: the first free word with the pattern, where the words
        of the block are free. Only 32 words of a block are tested."""
        sel = (self.locpat | self.locast) & 31
        offsets = [j for j in range(32) if j & sel == 0]
        if self.frecnt < 32:
            offsets = [j for j in offsets if j < self.frecnt]
        for pc in range(USAGE_BITS):
            if self.used[pc]:
                continue
            if (pc & (self.frecnt - 1) & ~self.locast) != self.locpat:
                continue
            if all(not self.used[pc + j] if pc + j < len(self.used) else True for j in offsets):
                return pc
        return None

    # ---- results -----------------------------------------------------

    def field_list(self):
        """[(field, [symbol])] in the order of the field list (FLDPNT)."""
        out = []
        f = self.fields
        while f is not None:
            syms = []
            s = f.next_sym
            while s is not None:
                syms.append(s)
                s = s.next_sym
            out.append((f, syms))
            f = f.next_field
        return out


PSEUDO_FIELDS = {
    ".DEFAULT": "p_default",
    ".SET": "p_set",
    ".CHANGE": "p_change",
    ".IF": "p_if",
    ".IFNOT": "p_ifnot",
    ".ENDIF": "p_endif",
    ".WIDTH": "p_width",
    ".RAMFILE": "p_ramfile",
}


def read_sources(its_dir: Path) -> list:
    ucode_dir = its_dir / "src" / "kshack"
    out = []
    for stem in its.UCODE_FILES:
        p = its.ucode_file(ucode_dir, stem)
        out.append((p.name, p.read_bytes().decode("latin-1")))
    return out


def assemble(its_dir: Path) -> Assembler:
    a = Assembler(read_sources(its_dir))
    a.assemble()
    return a


# ---- comparison with a listing of MICRO --------------------------------

WORD_RE = re.compile(r"^([UD]) ([0-7]+), ([0-7,]+)\t+; *([0-9]+)\t")


def listing_words(text: str) -> dict:
    """{("U"|"D", address): (value, line)} of a MICRO listing (PNTBIN)."""
    out = {}
    for line in text.split("\n"):
        m = WORD_RE.match(line)
        if not m:
            continue
        v = 0
        for g in m.group(3).split(","):
            v = (v << 12) | int(g, 8)
        out[(m.group(1), int(m.group(2), 8))] = (v, int(m.group(4)))
    return out


def listing_cref(text: str) -> list:
    """[(ram, field, symbol or None, line)] of the cross reference listing
    (CREF): the fields and their symbols in the order of the listing, with
    the line of the definition ("n #"), or the first line for a switch."""
    lines = text.split("\n")
    try:
        k = next(i for i, l in enumerate(lines) if l.endswith("CROSS REFERENCE LISTING"))
    except StopIteration:
        raise MicroError("no cross reference listing") from None
    entries, field = [], None
    for line in lines[k:]:
        line = line.lstrip("\f")
        if "LOCATION / LINE NUMBER INDEX" in line:
            break
        if not line or line.startswith(";"):
            continue
        if line.startswith("\t\t"):
            # A continuation of the line numbers.
            if not entries:
                raise MicroError(f"cross reference line {line!r}")
            entries[-1][3] += "\t" + line
            continue
        m = re.match(r"^(?:\(([UD])\) |\t)([^\t]+)\t*(.*)$", line)
        if not m:
            raise MicroError(f"cross reference line {line!r}")
        if m.group(1):
            field = (UCODE if m.group(1) == "U" else DISP, m.group(2))
            entries.append([field[0], field[1], None, m.group(3)])
        elif field is None:
            raise MicroError(f"cross reference symbol before a field: {line!r}")
        else:
            entries.append([field[0], field[1], m.group(2), m.group(3)])
    out = []
    for ram, fname, sname, refs in entries:
        d = re.search(r"([0-9]+) #", refs)
        n = re.search(r"([0-9]+)", refs)
        at = int(d.group(1)) if d else int(n.group(1)) if n else None
        out.append((ram, fname, sname, at))
    return out


def compare(a: Assembler, text: str, lines: bool = True) -> list:
    """The differences from a listing: the words with their addresses and
    lines, and the cross reference (the order of the fields and symbols
    and the line of each definition). The words also check the value of
    each tag that a J field names. If lines is false, the line numbers
    are not compared: the listing comes from other source text with the
    same words."""
    text = text.replace("\r", "")
    want = listing_words(text)
    if not lines:
        want = {k: (v, None) for k, (v, _) in want.items()}
    def line(key):
        return a.lines[key] if lines else None

    got = {("U", pc): (v, line(("U", pc))) for pc, v in a.cram.items()}
    got.update({("D", pc): (v, line(("D", pc))) for pc, v in a.drom.items()})
    diffs = []
    for key in sorted(set(want) | set(got)):
        if want.get(key) != got.get(key):
            diffs.append(f"{key[0]} {key[1]:04o}: listing {want.get(key)}, here {got.get(key)}")
    mine = []
    for f, syms in a.field_list():
        mine.append((f.ram, f.name, None, f.line))
        mine.extend((f.ram, f.name, s.name, s.line) for s in syms)
    cref = listing_cref(text)
    if not lines:
        cref = [x[:3] for x in cref]
        mine = [x[:3] for x in mine]
    for k, (x, y) in enumerate(zip(cref, mine)):
        if x != y:
            diffs.append(f"cross reference entry {k}: listing {x}, here {y}")
    if len(cref) != len(mine):
        diffs.append(f"cross reference: {len(cref)} entries in the listing, {len(mine)} here")
    return diffs


def main(argv=None) -> int:
    ap = argparse.ArgumentParser(description=__doc__.splitlines()[0])
    ap.add_argument("--its", required=True, type=Path)
    ap.add_argument("--compare", type=Path)
    ap.add_argument("--no-lines", action="store_true")
    args = ap.parse_args(argv)
    try:
        a = assemble(args.its)
    except MicroError as e:
        print(f"micro: {e}", file=sys.stderr)
        return 1
    print(f"micro: {len(a.cram)} U words, {len(a.drom)} D words")
    if args.compare:
        diffs = compare(a, args.compare.read_bytes().decode("latin-1"), not args.no_lines)
        for d in diffs[:20]:
            print(f"micro: {d}")
        print(f"micro: {len(diffs)} differences from {args.compare}")
        return 1 if diffs else 0
    return 0


if __name__ == "__main__":
    sys.exit(main())
