# Conformance

This document gives what a processor of this project must do to be a
correct KS10 for ITS. It has three levels. The first core runs the ITS
microcode on the KS10 data path, so it follows the microcode in all three
levels. A later pipelined core of its own design follows levels A and B
only.

## The behaviour of the KS10

No two sources agree fully about the KS10. The microcode with the
schematics, the PRM, SIMH, KLH10 and KS10FPGA each differ in some cases
([`discrepancies.md`](discrepancies.md)). Real machines can also differ:
the modules have revisions, and the microcode has versions. Thus the
behaviour of the KS10 is not one fixed thing.

DEC defined two parts of the behaviour: what the PRM states, and what the
DEC diagnostics check. Outside these two parts, the behaviour is
implementation-defined. For the microcode-level model, this project uses
one configuration of the machine ([`architecture.md`](architecture.md),
"Target").

## Level A: exact at each instruction boundary

A processor must agree exactly at each instruction boundary in these
cases:

- Results and flags that the PRM defines.
- Each behaviour that ITS relies on: ITS paging and the page fail word,
  traps, interrupts, the I/O instructions, one-proceed (its.15,
  `.SET/1PROC=1`) and `XCTR` (ksdefs, opcode 103) **[its]**.
- The values that the DEC diagnostics check
  ([`unknowns.md`](unknowns.md) lists which diagnostics apply).
- Self-modifying code, and instructions that run in the ACs.
- The order of a page failure and a change of the flags that a program can
  see. At AREAD, before the instruction runs, the microcode checks the
  reference to E as the DROM fields READ, TEST and WRITE give. So a page
  failure comes before a change of the flags ([`emulator.md`](emulator.md),
  "Processor model").
- Indirect chains and `XCT` chains that an interrupt stops. An interrupt
  request makes the next virtual read of the chain page-fail, and the page
  fail routine takes the interrupt **[manual]** (M8621-0-DPM6 rev C, E9)
  **[microcode]** (itspag, `PAGE-FAIL`, `PF-INT`). The instruction starts
  again after the interrupt.

FDV with normalized operands is level A. It stays an open case until a
source decides it ([`impldef.md`](impldef.md), `fdv-quotient`).

## Level B: implementation-defined

The cases of level B are in [`impldef.md`](impldef.md) (the data in
`spec/impldef.toml`). Each core selects one behaviour for each case, and
the table records the selection. The table also has the cases in which a
reference emulator differs from level A, because the masks of the
differential runs come from it. Those cases have level A in the table.

## Level C: not kept

A processor does not have to keep these:

- The internal state of the microcode: the temporaries in the halt status
  block (`inout.50`, DUMP: AR, ARX, BR, BRX, FLG, T0, VMA). Their values
  come from the microcode steps before the halt. The 8080 reads only
  words 0 and 1 after a halt (`cmds.m80`, `HLTCM`, `SMVER`) **[its]**.
  `emu/ks10-core-soft` writes 0 into them.
- The instruction times.
- The microcode step at which a long instruction takes an interrupt. For
  example, BLT stops in `CLEANUP AND TRAP` (itspag, `PF-INT`).
- The work that an instruction keeps after a page failure inside it, if
  the instruction starts again with a correct result. For example,
  `emu/ks10-core-soft` sets the ACs of EXTEND back to their start values;
  the microcode keeps the work (`extend.4`, "PAGE FAIL CLEANUP").

The state that an interrupted instruction leaves must be a correct state
for its restart.

## Rules for the levels

- A move of a behaviour from level A to level B or C needs evidence: the
  PRM does not define the behaviour, and no DEC diagnostic checks it. A
  case of level B gives this evidence in `level_b`.
- The flag cases of the design decision of 2026-11-28 against this rule:
  - `trap-flags-clear` and `dmul-carry` meet it, and have level B.
  - `ash-trap-last-step` and `dmul-trap-old-overflow` do not: the PRM
    defines them (PRM 1982, p. 2-41 and p. 2-78). They have level A and
    the state open.
  - `dmovn-carry` is not checked against the rule yet.
- If the PRM and a KS10 source differ in a case of level A, the order of
  the sources decides the behaviour of the KS10
  ([`AGENTS.md`](../AGENTS.md), "Sources of truth"). The first core
  follows it. For the other cores, such a case stays open until a
  decision.
- After each change of the levels, run the full ITS build (`make
  its-build`). The target and the `its/` directory do not exist yet
  ([`AGENTS.md`](../AGENTS.md), "Layout"). Until they exist, the check
  is `make its-boot`.
- The core with the KS10 data path stays available as a bitstream. It
  separates a question of the contract from an error of the pipelined
  core.

## Comparisons

- `ks10-core-uop` is the reference of the core with the KS10 data path.
  The co-simulation compares them at each microcycle, exactly.
- `ks10-core-soft` is the reference of the pipelined core. The two are
  compared at each instruction, by this contract.
- `ks10-core-uop` and `ks10-core-soft` are compared at each instruction.
  Each difference between them is a case of level B in
  [`impldef.md`](impldef.md), or an item of level C.
- The masks of each comparison come from `spec/impldef.toml` through a
  generator. A mask written by hand is not permitted. Some masks of today
  do not fit the table yet ("Masks of the differential runs").

### Masks of the differential runs

These are the masks of 2026-11-28 and their fit with the table:

- The runs against SIMH and KLH10 (`emu/ks10-core-soft/tests/oracle.rs`)
  skip whole cases. Each skip matches a case with the compare value
  `skip`. Some skips cover more than the case:
  - all FDV cases;
  - ASH and ASHC above a count of places;
  - DIV in KLH10 without the sign of the quotient.
- The table `KNOWN` of `emu/ks10-core-uop/tests/instructions.rs` masks
  flags between the two models. Its three entries are for the cases
  `trap-flags-clear`, `ash-trap-last-step`, `dmul-trap-old-overflow`
  (one entry for the three), `dmovn-carry` and `dmul-carry`. The masks
  cover more than the cases: each ASH, ASHC and DMUL, and each DMOVN.
- These do not fit a case of the table:
  - The limits of the case generator: no jumps, I/O or EXTEND,
    normalized operands only, exponent differences of 60 or less. They
    belong to a list of coverage, not to cases.
  - The exclusion of the halt status block (500 to 520) from E. It hides a
    difference with no case yet: `emu/ks10-core-soft` writes the block at
    a HALT, and KLH10 does not.
- Some predicates need code: the IMUL product range and the DIV magnitude
  test. The table names them from a fixed vocabulary (`PREDICATES` in
  `spec/gen_impldef.py`). One Rust module will implement each name once,
  with its own tests.
- The generator does not write the masks yet. Until it does, the masks of
  `oracle.rs` and `instructions.rs` stay, and each one has its case in
  the table.

## A test on a real KS10

This is a plan; the program does not exist yet.

- One guest program runs the program of each processor case of
  [`impldef.md`](impldef.md). It prints the name of the case and the
  words that differ (ACs, the flags from `JSP`, memory) on the CTY, in
  octal.
- It needs no device other than the CTY. The cases that need paging set
  up the ITS page tables themselves.
- A report records the microcode version (APRID) and the revision of each
  module of the machine. Two machines with different results are both
  valid data. Each result goes into the table: a case gets the state
  `confirmed`, with the machine and the revisions.
