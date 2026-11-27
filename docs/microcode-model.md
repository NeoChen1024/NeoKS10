# Microcode-level model

`emu/ks10uop` is a model of the KS10 at the level of the microcode. A
model of the data path runs the microcode that the ITS build assembles.
If the sources disagree about an instruction, this model decides
(AGENTS.md, "Sources of truth"). It is also the model that the logic in
`rtl/` is compared with ([`architecture.md`](architecture.md)).

The work has three stages:

1. The data path and the microsequencer on a flat memory, without paging
   and I/O. Random instructions compare it with `emu/ks10`.
2. ITS paging, the I/O cycles to the devices of `emu/ks10`, interrupts,
   page fail. Then the diagnostics and an ITS boot.
3. A model of the 8080 runs the real console program.

## The microcode of the ITS build

The ITS build assembles the microcode with MICRO: `KSHACK; MCR 262` from
`its`, `ks10`, `simple`, `flt`, `extend`, `inout`, `itspag` and `pagef`
**[its]** (`build/ks10/processor.tcl` of the ITS tree). MICRO writes two
files:

- The listing `KSHACK; MCR 262`. A line `U aaaa, ...` gives a control RAM
  word: the address and 108 bits in nine groups of four octal digits. A
  line `D aaaa, ...` gives a dispatch ROM word, 36 bits **[microcode]**.
- The RAM file `KSHACK; MCR RAM`. The build copies it to `.; RAM RAM`, and
  `KSFEDR` puts it into the file system of the 8080 on the disk.

### The control RAM

The control RAM has 4000 words of 96 bits **[manual]** (EK-OKS10-TM-002,
section 5.4.3, page 5-40: bits 0 to 35 on the CRA board, 36 to 95 on the
CRM board). These are the bit numbers of the hardware, not of MICRO.
The 8080 loads it from the RAM file before it starts the microcode
**[its]** (`cmds.m80`: MICROP, DMEM2CRAM, NEXTCR):

1. The home block, then the page of file pointers. Pointer word 2 gives
   the first page of the RAM file.
2. Each control RAM word takes three memory words. The loader stops at
   address 4000. The RAM file thus has 3 × 4000 = 14000 words (octal):
   12 (decimal) pages of 1000 words.
3. The pages after the first come from the next sectors: the transfer
   list `QXFR` (`dr.m80`) sets no disk address.

`ks10::boot::read_microcode` does these steps on the emulated RH11 and
RP06.

`.RAMFILE` in `ks10` gives the layout of an entry **[microcode]**: 108
bits, the listed fields from left to right, two parity bits and two pads
(MICRO, `$RAMFL` and `RAMFL4`). The generator `spec/gen_microcode.py`
reads this layout and the field definitions into
`emu/ks10uop/src/fields.rs`.

**Parity.** The parity bit of `ODD PARITY<ranges>` is the XOR of the given
bits of the RAM file entry **[microcode]** (MICRO, `PNTB91`). With the
parity bit, each range set has an even number of ones. This agrees with
the comment of `.RAMFILE` ("Parity is even parity"). `EVEN PARITY` gives
the complement. The ranges are bits of the RAM file entry, not of the
microword.

**Check of the RAM file [measured]** 2026-11-27 (`make microcode`, test
`emu/ks10uop/tests/ram_file.rs`). The RAM file was read from `rp0.dsk` of
the ITS build of 2026-11-26 as the 8080 reads it:

- All 4000 entries have the right parity bits and zero pads.
- The 1980 (decimal) U words of the listing agree with the decoded
  entries in all fields of the RAM file. The other 68 (decimal) entries
  are 0.
- No listing bit is outside the fields of the RAM file.
- The SHA-256 of the RAM file is in the test.

If a field position or the parity rule is wrong, the test fails: a test
with the parity rule reversed, and a test with `AD` moved by one bit,
both failed.

### The dispatch ROM

The dispatch ROM has 512 words, one for each opcode. It is three
512 × 8-bit ROM chips on the DPEA board **[manual]** (EK-OKS10-TM-002,
section 5.4.4.1, page 5-41). The console loads only the control RAM: an
address and eight 12-bit segments for each word **[manual]** (section
5.4.6) **[its]** (`cmds.m80`, DMEM2CR). The only source of the dispatch
ROM contents is the listing.

**Check of the dispatch ROM [measured]** 2026-11-27 (the same test). The
fields `A`, `B`, `ACDISP`, `I`, `READ`, `TEST`, `COND FUNC`, `VMA` and
`WRITE` of all 512 D words agree with `spec/instructions.toml`. That
table comes from the source text of the microcode, not from the listing.
A test with `B` moved by one bit failed.

The 512 D words of the ITS listing 262 are the same as those of the DEC
microcode 130 **[measured]** (2026-11-27, a comparison of the D lines with
`microcode/V130/ks10.mcr` of KS10FPGA, commit 5090e7a1). This agrees with
a ROM that ITS did not change.

### Agreement with the historical listing

The listing and the RAM file of the ITS build of 2026-11-26 agree with the
copies in KS10FPGA (`microcode/ITS/mcr.262` and `ram.262`, commit
5090e7a1) **[measured]** (2026-11-27): the same U and D words, and the same
bytes of the RAM file. KS10FPGA is a reference of rank 4, so this is
support, not proof.

## The data path

This section gives what each microword field does. The field values and
their names are in the field definitions of `ks10` **[microcode]**; the
generated `emu/ks10uop/src/fields.rs` has their positions. The exact
logic of each mixer and condition is in the code of `ks10uop`, with its
source. Bit numbers of a 36-bit word are those of DEC (bit 0 at the left).

### Cycle

One microinstruction takes one processor cycle of 2 to 5 clock ticks
(`T` field: ticks minus 2) **[manual]** (EK-OKS10-TM-002, 5.4.1, bits
70–71). The model counts cycles and ticks; it does not model the ticks
inside a cycle.

- If `MULTI SHIFT` is 1, the microinstruction repeats until FE bit 0 is
  0. If FE bit 0 is already 0, the microinstruction is a no-op
  **[manual]** (5.4.1, bit 79).
- A memory function needs two microinstructions: the first sets up and
  starts it, the second waits and takes or gives the word **[manual]**
  (5.6). A flat memory completes each function at once.

### Microsequencer

The address of the next microinstruction is the `J` field, ORed with the
outputs of the dispatch mixers and the skip condition **[manual]** (5.4.1,
bits 57–68; 5.4.4).

- **Skip.** `SKIP` has a select code in its right three bits and three
  group enables in its left three bits. A group is enabled by a 0 bit: 3
  (011) enables group 40 only, 5 (101) group 20 only, 6 (110) group 10
  only, 0 all three, 7 none. If any enabled condition is true, bit 11 of
  the address is set **[microcode]** (ks10, "MICROCODE FIELDS -- SKIP":
  the table and every named value, for example `LE=42` is AD sign or
  AD=0, and `IOLGL=04` uses all three groups) **[manual]** (5.4.1, bits
  63–68).
- **Dispatch.** `DISP` has the same form with active-low group enables.
  "DISP EN 40 & DISP EN 10 ONLY CONTROL THE LOW 4 BITS OF THE JUMP
  ADDRESS. DISP EN 20 ONLY CONTROLS THE HI 7 BITS" **[microcode]** (ks10,
  "MICROCODE FIELDS -- DISPATCH", the note under the table). The manual
  gives `J` bits 0–7 and 8–11 instead ([`discrepancies.md`](discrepancies.md)).
- **Dispatch ROM.** The 9-bit opcode in IR selects a word. `DISP/DROM`
  jumps to the DROM `J` field, which is in the range 1400–1777; the `J`
  field of the microword is 0. `DISP/AREAD` dispatches on the DROM `A`
  field to locations 40–57, or to the DROM `J` field if the DROM `I` bit
  is 1 **[manual]** (5.4.1, bits 57–62; 5.4.2; 5.4.4.1) **[microcode]**
  (ks10, "DISPATCH ROM DEFINITIONS").
- **AC dispatch.** If the DROM `ACDISP` bit is 1, the dispatch goes to one
  of 16 locations, DROM `J` plus the AC field **[microcode]** (simple,
  label `JRST`: "DISPATCHES TO 1 OF 16 PLACES ON AC BITS", block `=0000`
  at 1520). The four opcodes with `ACDISP` (254, 700, 701, 702) have a
  DROM `J` with the low four bits 0 **[measured]** 2026-11-27 (the D words
  of the ITS listing 262). Thus OR and addition give the same address.
- **NICOND.** The dispatch at the start of the next instruction selects,
  in order of priority, trap 3, trap 2, trap 1, halt, or the next
  instruction. The signal NICOND 08 tells if a memory cycle is in
  progress, that is, if the fetch has started **[manual]** (5.4.4.2).
- **Subroutine stack.** `CALL` saves the address of the current
  microinstruction. `DISP/RETURN` ORs the saved address into `J` and pops
  it. The stack holds 15 levels. A page fail also pushes **[manual]**
  (5.4.5).

### Arithmetic unit

Ten Am2901 4-bit slices make a 40-bit unit with 16 registers and the Q
register **[manual]** (5.5.1). The 2901 instruction comes from the
microword:

| 2901 inputs | Field | Note |
|---|---|---|
| I5–I3, function | `AD` bits 12–14 | `DIVIDE` and `MULTI PREC` can change bit 1 of the function (add or subtract) |
| I2–I0, source, left half | `AD` bits 15–17 | |
| I2–I0, source, right half | `RSRC` | Defaults to the left source (`LSRC`) |
| I8–I6, destination | `DEST` | The 2 bit is inverted: `DEST` 0 is 2901 code 2 (RAMA) |
| A address, B address | `A`, `B` | Only B can be loaded |
| D input | DBus | 36 bits, with two copies of bit 0 at the left and two 0s at the right |

**[microcode]** (ks10, "MICROCODE FIELDS -- DATAPATH CHIP") **[manual]**
(5.4.1, bits 12–35; 5.5.1).

- A 36-bit word is in the middle of the 40 bits: two copies of the sign
  at the left, two 0 bits at the right **[manual]** (5.5.1). Thus the
  rightmost bit is a quarter of the word LSB, and "−.25" in an `AD` name
  is the −1 of a 2901 subtract without carry in **[estimate]** (the names
  in ks10 and the 2901 functions).
- The unit has two halves of 20 bits. `CLKL` and `CLKR` clock them; an
  unclocked half does not change at all. The carry into the left half can
  be inhibited (`SPEC/INHCRY18`). `CRY38` puts a carry into the right end
  **[manual]** (5.4.1, bits 45–50 and 72; 5.5.1).
- Overflow comes from DP SIGN (the 40-bit sign) against DP bit 0. Carry 1
  is overflow XOR carry 0 **[manual]** (5.5.5).
- `DEST` selects the 2901 output Y: the A register for `DEST/A`,
  otherwise the function result F **[microcode]** (ks10, `DEST` values).

### Shifts

The right three bits of `SPEC` always go to the shift connections and to
the byte select of DBM. The left three bits enable groups of special
functions. Thus one microinstruction can combine a special function with
a shift style or a byte select **[manual]** (5.4.1, bits 51–56).

If `DEST` shifts (Q_Q*2, AD*2, Q_Q*.5, AD*.5), `SHSTYLE` (the right three
bits of `SPEC`) selects the connections at the ends of the register and
of Q: NORM, ZERO, ONES, ROT, ASHC, LSHC, DIV, ROTC **[manual]** (5.5.1,
Figure 5-22) **[microcode]** (ks10, `SHSTYLE`).

- In a divide step (`DIVIDE`), the carry out of the previous step makes
  the next step shift a 1 into Q (DIVIDE SHIFT) and subtract (carry in
  and function bit 1) **[manual]** (5.5.1, Figure 5-22 note).
- If `MULTI PREC` is 1, MULTI SHIFT is the flag FL02, and a carry out of
  the lower word becomes a carry in. Otherwise MULTI SHIFT is 0
  **[manual]** (5.5.1; Figure 5-22 note, where the dash is a negation)
  **[emulator]** (KS10FPGA 5090e7a1, `alu.v`, the DPE5 flag logic).
- Figure 5-22 gives the connections of each style. The connection of
  each bit comes from the DPE1 schematic; until it is read, KS10FPGA
  `alu.v` gives it **[emulator]** (5090e7a1).

### DBus and DBM

The DBus mixer (`DBUS`) selects the PC flags with the PI level and the
right 10 VMA bits (0), DP (1), the RAM file (2), or DBM (3). If `DBUS`
selects DBM with MB, and the reference is an AC or a cache hit, the RAM
file is selected instead **[manual]** (5.5.2) **[microcode]** (ks10,
`DBUS`).

The DBM mixer (`DBM`) selects: APR flags and the page fail dispatch (0),
five copies of SCAD bits 1–7 (1), the exponent and the 1 ms counter (2),
DP (3), DP with halves swapped (4), VMA with its flags (5), MB (6), or the
number field in both halves (7) **[manual]** (5.5.2) **[microcode]**
(ks10, `DBM`). With `DBM/DP`, the `BYTE` field (`BYTE1` to `BYTE5`)
inserts SCAD bits 1–7 into byte 1 to 5 of DP (the table in ks10,
"MICROCODE FIELDS -- SPEC").

### RAM file

The RAM file has 1024 words with parity. `RAMADR` selects the address:
AC (0), a function of AC and the number field (1), the index
register (2), VMA (4), the right 10 VMA bits (6), or the number field
(7) **[manual]** (5.5.3) **[microcode]** (ks10, `RAMADR`). For code 1, a
74LS181 on DPE6 combines AC with the low four bits of the number field;
`ACALU` gives its function, for example `AC+N` **[microcode]** (ks10,
"# REDEFINED WHEN USED AS AC CONTROL"). The ACs are
in blocks; the current and previous block numbers are in flip-flops on
DPE5. A VMA reference is an AC reference if the in-section address is
0–17 and the reference is virtual **[manual]** (5.6.1, p5-53). `FMWRITE` writes
the DBus into the addressed word.

### 10-bit logic

SCAD is a 10-bit adder with eight function codes (`SCAD`), an A input
(`SCADA`: SC, the number field, 44, or a byte of DP) and a B input
(`SCADB`: FE, the exponent of DP, the shift count, the byte size).
`LOADSC` and `LOADFE` load SC and FE from SCAD **[manual]** (5.5.4)
**[microcode]** (ks10, `SCAD`, `SCADA`, `SCADB`). The manual says "seven
functions" ([`discrepancies.md`](discrepancies.md)).

### Program flags

`SPEC/FLAGS` with the number field sets and clears the flags (`SETOV`,
`SETFOV`, `SETNDV`, `CLRFPD`, `SETFPD`, `HOLD USER`, `TRAP2`, `TRAP1`,
`LD PCU`, `JFCLFLG`, `LD FLAGS`, `ADFLGS`). `SPEC/ASHOV` and
`SPEC/EXPTST` test ASH and exponent overflow **[manual]** (5.5.5)
**[microcode]** (ks10, "# REDEFINED WHEN USED AS PC FLAG CONTROL").

### Memory functions

`MEM` 1 starts or completes a memory or I/O function. The number field
gives its type (`READ CYCLE`, `WRITE TEST`, `WRITE CYCLE`, `FETCH`,
`PHYSICAL`, `FORCE USER`, `FORCE EXEC`, `PXCT`), and whether to load VMA
(`LDVMA`) and to start or wait (`WAIT`). `AREAD` and `BWRITE` take the
type from the dispatch ROM. `DP FUNC` takes it from DP bits 0–13 instead
of the number field **[manual]** (5.6, 5.6.1) **[microcode]** (ks10,
"# REDEFINED WHEN USED AS MEMORY CYCLE CONTROL").

## Stage 1

Stage 1 runs the instructions that need no paging and no I/O: a flat
memory of 512 Ki words, physical addresses, no cache. It has:

- the microsequencer, the dispatch ROM, the skip and dispatch conditions;
- the arithmetic unit, the shifts, DIVIDE and MULTI PREC;
- the DBus and DBM mixers, the RAM file with AC blocks;
- the 10-bit logic, the program flags, IR and XR;
- memory functions on the flat memory, with AC references.

It stops at a page fail, an I/O function, an interrupt, or a halt. A test
runs one instruction from a state and compares the result with
`emu/ks10` for the same state.

## Field tables

`spec/gen_microcode.py` reads from the ITS microcode sources:

- the fields of the microword (`.UCODE` mode), bit 0 at the left end of
  108 bits (`.WIDTH/108`);
- the fields of the dispatch ROM word (`.DCODE` mode);
- the layout of the RAM file (`.RAMFILE`).

`.RAMFILE` is on line 467 of `ks10.47`, before the field definitions.
MICRO handles it in its second pass, when all fields are known (MICRO,
`$RAMFL`). Thus the generator resolves the names with the fields of the
whole microword. The dispatch ROM has its own `J` (bits 12 to 23); the
microword `J` is bits 0 to 11.
