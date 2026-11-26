#!/usr/bin/env python3
"""Mutation test of a SystemVerilog module (AGENTS.md: make mutate).

Each mutant changes one operator or one constant in one line of the module.
For each mutant, the script builds and runs the Verilator testbench, and runs
the formal tasks of the module if they exist. A mutant is killed if a test
fails. A mutant that does not compile is not counted.

Usage: mutate.py --module NAME [--count N] [--seed S] [--jobs J]
  NAME is a file rtl/memprobe/NAME.sv. The tests are the testbenches of
  tb/memprobe (tb_hist for memprobe_hist, then tb_core; seeds 1 to 3) and
  the proof task of tb/memprobe/NAME.sby if it exists.

Exit status: 0 if at least one mutant is killed, 1 if none, 2 on an error.
A surviving mutant needs a look: it is a gap in the tests, or a change that
has no effect.
"""

import argparse
import concurrent.futures
import os
import random
import re
import shutil
import subprocess
import sys
import tempfile
from pathlib import Path

ROOT = Path(__file__).resolve().parent.parent

# (name, pattern, replacement). Each pattern must not match a non-blocking
# assignment "<=".
OPERATORS = [
    ("== to !=", r"==", "!="),
    ("!= to ==", r"!=", "=="),
    ("&& to ||", r"&&", "||"),
    ("|| to &&", r"\|\|", "&&"),
    (">= to >", r">=", ">"),
    ("> to >=", r"(?<![-=>])>(?![=>])", ">="),
    ("+ 1 to + 2", r"\+ 1'b1", "+ 2'd2"),
    ("- 1 to + 1", r"- 1'b1", "+ 1'b1"),
    ("1'b1 to 1'b0", r"1'b1", "1'b0"),
    ("1'b0 to 1'b1", r"1'b0", "1'b1"),
    ("'0 to '1", r"'0(?![0-9])", "'1"),
    ("! removed", r"!(?!=)", ""),
    ("^ to |", r"\^", "|"),
]

TB_RTL = ["rtl/gen/regmap_pkg.sv"] + [f"rtl/memprobe/memprobe_{m}.sv"
                                      for m in ("sync", "pattern", "gen", "hist", "regs", "core")]
COSIM_SEEDS = (1, 2, 3)


def code_part(line):
    """The part of a line before a comment."""
    i = line.find("//")
    return line if i < 0 else line[:i]


def candidates(text):
    """All (line number, operator index, column) where an operator applies."""
    out = []
    for ln, line in enumerate(text.splitlines()):
        code = code_part(line)
        if not code.strip() or code.lstrip().startswith(("module", "parameter", "localparam",
                                                         "input", "output", "import", "/*")):
            continue
        for oi, (_, pat, _) in enumerate(OPERATORS):
            for m in re.finditer(pat, code):
                out.append((ln, oi, m.start()))
    return out


def apply(text, cand):
    ln, oi, col = cand
    lines = text.splitlines(keepends=True)
    _, pat, rep = OPERATORS[oi]
    line = lines[ln]
    m = re.compile(pat).match(line, col)
    lines[ln] = line[:col] + m.expand(rep) + line[m.end():]
    return "".join(lines), lines[ln].rstrip()


def run(cmd, cwd, timeout):
    try:
        p = subprocess.run(cmd, cwd=cwd, stdout=subprocess.PIPE, stderr=subprocess.STDOUT,
                           timeout=timeout, text=True)
        return p.returncode, p.stdout
    except subprocess.TimeoutExpired:
        return None, "timeout"


def test_mutant(module, text, sby, workdir):
    """Return 'killed: reason', 'survived' or 'invalid: reason'."""
    tree = Path(workdir)
    for rel in TB_RTL + ["tb/memprobe/tb_core.cpp", "tb/memprobe/tb_hist.cpp", "tb/gen/regmap.h",
                         "tests/memprobe-pattern.txt"]:
        (tree / rel).parent.mkdir(parents=True, exist_ok=True)
        shutil.copy(ROOT / rel, tree / rel)
    (tree / f"rtl/memprobe/{module}.sv").write_text(text)
    if sby:
        (tree / "tb/memprobe").mkdir(parents=True, exist_ok=True)
        for f in (ROOT / "tb/memprobe").glob(f"{sby.stem}*"):
            shutil.copy(f, tree / "tb/memprobe" / f.name)
        shutil.copy(ROOT / f"tb/memprobe/{sby.stem}_formal.sv", tree / "tb/memprobe")

    if module == "memprobe_hist":
        rc, out = run(["nice", "-n", "19", "verilator", "--cc", "--exe", "--build", "-j", "2",
                       "-Wno-fatal", "--top-module", "memprobe_hist", "-Mdir", "obj_hist",
                       "-o", "../tb_hist", "-CFLAGS", "-O1", "-CFLAGS", "-std=c++17",
                       "rtl/memprobe/memprobe_hist.sv", "tb/memprobe/tb_hist.cpp"], tree, 600)
        if rc != 0:
            return "invalid: does not compile"
        for s in COSIM_SEEDS:
            rc, out = run(["./tb_hist", "--seed", str(s)], tree, 120)
            if rc != 0:
                first = next((l for l in out.splitlines() if l.startswith("FAIL")), out[-200:])
                return f"killed: tb_hist seed {s}: {first.strip()}" if rc is not None else "killed: tb_hist timeout"

    rc, out = run(["nice", "-n", "19", "verilator", "--cc", "--exe", "--build", "-j", "2",
                   "-Wno-fatal", "--top-module", "memprobe_core", "-GBUILD=32'h5EED0001",
                   "-Mdir", "obj", "-o", "../tb_core", "-CFLAGS", "-O1", "-CFLAGS", "-std=c++17"]
                  + TB_RTL + ["tb/memprobe/tb_core.cpp"], tree, 600)
    if rc != 0:
        return "invalid: does not compile"
    for s in COSIM_SEEDS:
        rc, out = run(["./tb_core", "--pattern", "tests/memprobe-pattern.txt", "--seed", str(s)],
                      tree, 120)
        if rc != 0:
            first = next((l for l in out.splitlines() if l.startswith(("FAIL", "FATAL"))), out[-200:])
            return f"killed: cosim seed {s}: {first.strip()}" if rc is not None else "killed: cosim timeout"
    if sby:
        rc, out = run(["nice", "-n", "19", "sby", "-f", "-d", "formal", f"{sby.stem}.sby", "prove"],
                      tree / "tb/memprobe", 600)
        if rc is None:
            return "survived (formal timeout)"
        if rc != 0:
            if "ERROR" in out and "Assert failed" not in out and "failed assertion" not in out:
                return "invalid: formal tools fail"
            return "killed: formal"
    return "survived"


def main(argv=None):
    ap = argparse.ArgumentParser(description="Mutation test of a module of rtl/memprobe.")
    ap.add_argument("--module", required=True)
    ap.add_argument("--count", type=int, default=20)
    ap.add_argument("--seed", type=int, default=1)
    ap.add_argument("--jobs", type=int, default=max(1, (os.cpu_count() or 2) // 4))
    args = ap.parse_args(argv)

    src = ROOT / f"rtl/memprobe/{args.module}.sv"
    if not src.exists():
        print(f"no file {src}", file=sys.stderr)
        return 2
    text = src.read_text()
    short = args.module.removeprefix("memprobe_")
    sby = ROOT / f"tb/memprobe/{short}.sby"
    sby = sby if sby.exists() else None

    cands = candidates(text)
    rng = random.Random(args.seed)
    chosen = rng.sample(cands, min(args.count, len(cands)))
    print(f"{args.module}: {len(cands)} possible mutants, testing {len(chosen)} "
          f"(seed {args.seed}; cosim seeds {COSIM_SEEDS}; formal {'yes' if sby else 'no'})")

    def one(c):
        mtext, line = apply(text, c)
        with tempfile.TemporaryDirectory(prefix="mutant-") as d:
            return c, line, test_mutant(args.module, mtext, sby, d)

    killed = survived = invalid = 0
    with concurrent.futures.ThreadPoolExecutor(args.jobs) as ex:
        for c, line, result in ex.map(one, chosen):
            ln, oi, _ = c
            print(f"  line {ln + 1:4d} {OPERATORS[oi][0]:14s} {result}")
            print(f"            {line.strip()}")
            if result.startswith("killed"):
                killed += 1
            elif result.startswith("invalid"):
                invalid += 1
            else:
                survived += 1
    print(f"{args.module}: killed {killed}, survived {survived}, invalid {invalid}")
    return 0 if killed > 0 else 1


if __name__ == "__main__":
    sys.exit(main())
