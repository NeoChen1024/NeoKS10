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

Schematic citations give the drawing, its revision and the chips.
MP00540 has two revisions of each board: rev A1 and the later etch rev B.
The readings of the DPE and CRA sheets come from rev A1 (PDF pages: DPE1
125, DPE2 126, DPE5 129, DPE9 133, DPEA 134, CRA1 184, CRA2 185). Rev B
of the same sheets (PDF pages 112, 113, 116, 120, 121, 172, 173) has the
same connections with other chip numbers; for example DPE1 E703–E707
are E20, E71, E48, E41 and E12 in rev B, and DPE5 rev B keeps the E603
flags in other bits of its register. The comparison was made for the
connections that this document uses; for DPE9 it compared the inputs of
each gate. New readings use rev B. After the first citation of a
drawing, the text gives the board sheet only, for example "DPE5 E603".

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

The address of the next microinstruction has 12 bits, 00 to 11. Each bit
is the OR of the `J` field, the page fail signal, the dispatch mixers and,
for bit 11, the skip mixers **[manual]** (M8622-0-CRA1 rev A1, the ORs
E515 to E522 and E220 to E222, E214, E215).

**Skip.** `SKIP` has three group enables in its left three bits and a
select code in its right three bits. The enables are active low: the 4
bit enables group 40, the 2 bit group 20, the 1 bit group 10. Bit 11 of
the address is set if the selected input of an enabled group is true
**[microcode]** (ks10, "MICROCODE FIELDS -- SKIP"; every named value fits
this rule, for example `LLE=12` enables groups 40 and 20, `IOLGL=04` all
three).

| Select | Group 10 (CRA2 E114) | Group 20 (DPEA E203) | Group 40 (DPEA E202) |
|---|---|---|---|
| 0 | 0 | 0 | 0 |
| 1 | TRAP CYCLE | CARRY 02 | CARRY OUT |
| 2 | AD=0 | ADL SIGN | ADL=0 |
| 3 | SC SIGN | ADR SIGN | ADR=0 |
| 4 | EXECUTE | USER I/O | −USER |
| 5 | −I/O LATCH | JFCL SKIP | FPD |
| 6 | −CONTINUE | DP CRY1 | AC=0 |
| 7 | −1 MSEC | TXXX | INTERRUPT REQ |

**[manual]** (M8622-0-CRA2 rev A1, E114; M8620-0-DPEA rev A1, E202,
E203). The inputs 4 to 7 of group 10 go through the latch E115.

- ADL SIGN is the F SIGN pin of the leftmost 2901 (DPE1 E505). It is the
  sign of the 40-bit ALU output F, not DP bit 0, and not the output Y.
- ADL=0 and ADR=0 are the F=0 pins of the five slices of each half, so
  they test 20 bits each. AD=0 is ADL=0 AND ADR=0 (DPEA E601): all 40
  bits of F.
- TXXX is DROM TXXX EN XOR AD=0 (DPEA E319).

**Dispatch.** `DISP` has the same form: active-low enables for groups 40,
20 and 10, and a select code. Group 40 (DPEA E120 to E123) and group 10
(CRA1 E120 to E123) drive address bits 08 to 11. Group 20 (CRA1 E516,
E517, E520, E521) drives bits 00 to 07; its 74LS153 mixers get only the
low two select bits, so codes 4 to 7 act as 0 to 3.

| Select | Group 40, bits 08–11 | Group 20, bits 00–07 | Group 10, bits 08–11 |
|---|---|---|---|
| 0 | 0 | DIAG ADR | DIAG ADR |
| 1 | DP 18–21 | SBR RET | SBR RET |
| 2 | J 08–11 | 0, 0, 1, 1, DROM J 00–03 | 0, FLAG QR 37, FK2, 0 |
| 3 | AREAD 08–11 | 0, 0, A=J, A=J, AREAD 04–07 | EU1, FE2, FL2, FS2 |
| 4 | AD=0, DP 08, DP 09, DP SIGN | as 0 | MEMORY CYCLE, NICOND 09–11 |
| 5 | DP 32–35 | as 1 | 0, BYTE DISP 09–11 |
| 6 | DROM A 00–03 | as 2 | −JRST 0, INDIRECT, XR=0, 0 |
| 7 | DROM B 00–03 | as 3 | 0, 0, SCAD 00, 0 |

**[manual]** (M8622-0-CRA1 rev A1; M8620-0-DPEA rev A1). EU1, FE2, FK2,
FL2 and FS2 are backplane pins; [`unknowns.md`](unknowns.md) lists them.

- **Dispatch ROM.** The 9-bit opcode in IR selects a word of three ROMs
  (DPEA E113, E114, E115). The ROM `J` field has 8 bits; group 20 code 2
  adds 1400. `DISP/DROM` (12) thus jumps to 1400 plus the ROM `J`.
- **AREAD.** `DISP/AREAD` (13) uses code 3 in groups 40 and 20. If the
  DROM `I` bit (A=J) is 0, the address is 40 plus the DROM `A` field. If
  it is 1, the address is the same as for `DISP/DROM` (DPEA E119; CRA2
  E420; CRA1 E517).
- **AC dispatch.** If the DROM `ACDISP` bit is 1, AC 09–12 replace the
  low four bits of the ROM `J` (DPEA E118) **[manual]**. The microcode
  expects this: `JRST` "DISPATCHES TO 1 OF 16 PLACES ON AC BITS", block
  `=0000` at 1520 **[microcode]** (simple, label `JRST`).
- **Combinations.** The enables combine freely. `DISP/1` at 2724 enables
  all three groups with code 1: the return address ORed with DP 18–21
  **[microcode]** (simple, label `FIX1++`: "RETURN TO 1 OF 4 PLACES").
- **NICOND.** A priority encoder (CRA2 E216) gives NICOND 09–11, the
  number of its first active input: 0 EA1, 1 TRAP 3, 2 TRAP 2, 3 TRAP 1,
  4 EF1, 5 not RUN, 6 EK1, 7 always. Bit 08 is MEMORY CYCLE (DPM6)
  **[manual]** (CRA2 E216; CRA1 E120). TRAP CYCLE is NOT NICOND 09,
  latched at `SPEC/NICOND` (CRA2 E108).
- **Subroutine stack.** `CALL` saves the address of the current
  microinstruction; `DISP/RETURN` gives it back through `SBR RET` and pops
  it. The stack holds 15 levels **[manual]** (5.4.5). The stack moves on
  CALL, on RETURN (group 20 enabled with code 1), and on a page fail
  (CRA2 E723).
- **Page fail.** A page fail sets all 12 address bits (CRA1).

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
- The left half is the five slices of DPE1 (DP SIGN to DP 17), the right
  half the five slices of DPE2 (DP 18 to DP 37). `RSRC` drives the source
  pins of the right half only **[manual]** (M8620-0-DPE1 rev A1,
  M8620-0-DPE2 rev A1, the 2901 pins I0–I2).
- The carry runs through 2902 lookahead chips over all 40 bits (DPE2
  E709, E710, E711, E719). The carry in at bit 37 is DPE5 CARRY IN. The
  carry from bit 18 into bit 17 goes through the P2 input of E711, which
  `SPEC/INHCRY18` turns off. CARRY OUT is the carry out of bit −2 (E709).
  CARRY 02 is the carry out of bit 02 into bit 01 (E710) **[manual]**
  (DPE2).
- Each slice gives the 2902 chips a generate G and a propagate P. For an
  arithmetic function they are the usual lookahead terms. For a logic
  function the data sheet gives other terms: for R∨S, P = 1 and G =
  NOT(P3P2P1P0); for R∧S, P = 1 and G = G3+G2+G1+G0; for R⊽S, P =
  NOT(G3+G2+G1+G0) and G = NOT(G3+P3G2+P3P2G1+P3P2P1P0) **[manual]**
  (The Am2900 Family Data Book, 1978, Am2901A, Figure 8). So a logic
  function also has a defined CARRY OUT and CARRY 02.
- The 2901 shift pins: RAM0 and Q0 are outputs in RAMQD and RAMD, RAM3
  and Q3 in RAMQU, RAM3 in RAMU, and all four are inputs in the other
  destinations **[manual]** (Am2901A, Figure 4). Thus the line QR 37 is
  driven in a right shift of F (field `DEST` 6 and 7), and FL 02 in a
  left shift of F (4 and 5).
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

The 2901 has 40 bits in ten slices. The leftmost slice holds DP SIGN, DP
SIGN SMEAR, DP 00 and DP 01 (DPE1 E505). The shift lines between DP 01
and DP 02, and at the right end (bit 37), go through mixers selected by
`SHSTYLE` (DPE1 E702 to E707). `DEST` bit 2 selects the direction: 0
shifts left, 1 shifts right (DPE5 E403, E384). F names the main register
shift lines and Q those of the Q register.

| `SHSTYLE` | Right: into F 02 | Right: into Q 02 | Left: into F 01 | Left: into F 37 | Left: into Q 37 |
|---|---|---|---|---|---|
| 0 NORM | F 01 | Q 01 | F 02 | MULTI SHIFT | 0 |
| 1 ZERO | 0 | 0 | DP SIGN | 0 | 0 |
| 2 ONES | 1 | 1 | DP SIGN | 1 | 1 |
| 3 ROT | F 37 | Q 37 | DP SIGN | F 02 | Q 02 |
| 4 ASHC | F 01 | F 37 | DP SIGN | Q 02 | 0 |
| 5 LSHC | 0 | F 37 | F 02 | Q 02 | 0 |
| 6 DIV | 1 | F 37 | F 02 | Q 02 | DIVIDE SHIFT |
| 7 ROTC | Q 37 | F 37 | DP SIGN | Q 02 | F 02 |

**[manual]** (M8620-0-DPE1 rev A1, E703, E707, E706, E705, E704). In
each direction the rest is fixed (DPE1 E702):

- On a right shift, F SIGN gets ADL SIGN (the sign stays), Q SIGN gets 0,
  and Q 01 shifts into Q 02 only through the table.
- On a left shift, Q 02 shifts into Q 01.

The table agrees with Figure 5-22 **[manual]** (5.5.1) and with KS10FPGA
`alu.v` **[emulator]** (5090e7a1).

**DIVIDE and MULTI PREC.** The register E603 (DPE5) stores, at the end of
every cycle, the function bit 01, CARRY 02, CARRY OUT, F 02 (the bit
shifted out to the left) and Q 37. The next cycle uses them **[manual]**
(M8620-0-DPE5 rev A1, E603, E503, E602):

- function bit 01 = `AD` bit 01 OR (`MULTI PREC` AND flag function 01) OR
  (`DIVIDE` AND flag CARRY OUT);
- carry in = `CRY38` OR (`MULTI PREC` AND flag CARRY 02) OR (`DIVIDE` AND
  flag CARRY OUT);
- DIVIDE SHIFT = `DIVIDE` AND flag CARRY OUT;
- MULTI SHIFT = `MULTI PREC` AND flag F 02.

The flag Q 37 is the input of the MUL dispatch (CRA1 E121).

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

The flags read the left half of the DBM mixer, not the microword.
`CHANGE FLAGS` is `SPEC/FLAGS` only, and `DBM` defaults to `#`, so DBM
bit n is bit n of the number field **[microcode]** (ks10, `DBM` "D,7" and
the macro `CHANGE FLAGS`). With LOAD = DBM 15, CARRY = DBM 17, JFCL EN =
DBM 14, each with `SPEC/FLAGS`, the registers load at the clock
**[manual]** (M8620-0-DPE9 rev A1, PDF page 133):

| Flag | New value | Register enabled by |
|---|---|---|
| CRY0 | (LOAD ? DP 01 : CRY0) AND NOT JFCL 10, OR (CARRY AND CARRY OUT) | FLAGS (E305) |
| CRY1 | (LOAD ? DP 02 : CRY1) AND NOT JFCL 11, OR (CARRY AND DP CRY1) | FLAGS (E305) |
| FPD | ((LOAD ? DP 04 : FPD) OR DBM 04) AND NOT DBM 03 | FLAGS (E305) |
| USER | (USER AND DBM 05) OR (LOAD AND DP 05) | FLAGS (E210) |
| PCU | (LOAD AND DP 06 AND NOT USER) OR (NOT LOAD AND PCU) OR (USER AND DBM 09) OR (DBM 05 AND PCU AND DP 06 AND LOAD) | FLAGS (E210) |
| TRAP 2 | (DP 09 AND DBM 15) OR DBM 07 | FLAGS (E210) |
| NO DIV | DBM 02 OR (LOAD ? DP 12 : NO DIV) | FLAGS (E210) |
| OV | P OR X OR ((LOAD ? DP 00 : OV) AND NOT JFCL 09) OR DBM 00 | FLAGS, ASH TEST, EXP TEST (E209) |
| TRAP 1 | P OR X OR (LOAD AND DP 10) OR DBM 08 | as OV |
| FOV | X OR ((LOAD ? DP 03 : FOV) AND NOT JFCL 12) OR DBM 01 | as OV |
| FXU | (LOAD ? DP 11 : FXU), and a term of EXP TEST | as OV |

- P = (ASH TEST AND DP 01 XOR DP 02) OR (CARRY AND DP OVERFLOW); X = EXP
  TEST AND SCAD 01 (E205, E501).
- DP OVERFLOW = DP SIGN XOR DP 00; DP CRY1 = DP OVERFLOW XOR CARRY OUT
  (E504). Both use DP, so they are right only if DP is the ALU output.
- JFCL 09–12 = the flag (OV, CRY0, CRY1, FOV) AND the AC bit AND JFCL EN
  (E107, E109). JFCL SKIP is their OR (E108).
- One flip-flop is PCU and USER IOT (E210 R1).
- TRAP 1 has no "keep" input: a flag change without a set clears it.
  KS10FPGA `pcflags.v` agrees **[emulator]** (5090e7a1).

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
memory of 512 Ki words, physical addresses, no cache. A test runs one
instruction from a state and compares the result with `emu/ks10` for the
same state. The model is `emu/ks10uop/src/alu.rs` and `machine.rs`.

A function is either modelled or a stop with "unmodelled". The model
does not guess a value. A shift line that nothing drives is undefined; a
step that reads one stops.

**State of 2026-11-27.** Modelled:

- the microsequencer: skip, dispatch, the dispatch ROM, AREAD, the AC
  dispatch, the stack;
- the arithmetic unit with the shifts, DIVIDE, MULTI PREC and E603;
- the DBus (DP, RAM file, DBM) and DBM (DP, DP swapped, number field);
- the RAM file with `RAMADR` 0, 1 (`ACALU` `B` and `AC+N`), 6 and 7;
- SCAD with `SCADA` SC or `S#`, `SCADB` FE; SC and FE;
- IR and AC; the program flags with `SPEC/FLAGS` and `SPEC/ASHOV`;
  `SPEC/LDACBLK`; the VMA load.

These functions keep DP and change nothing else, because stage 1 has no
interrupts, no APR conditions and no page fail: `SPEC/LDPI`, `SPEC/APR
EN`, `SPEC/MEMCLR`.

The rest stops (the `unmodelled` branches of `machine.rs`):

- memory and I/O cycles (`READ CYCLE`, `WRITE TEST`, `WRITE CYCLE`,
  `AREAD`, `DP FUNC`, `WAIT`, `BWRITE`);
- DBus input 0 (flags, PI, VMA); DBM inputs 0, 1, 2, 5, 6, and DP with a
  byte insertion; DP on the DBus while DP depends on the DBus;
- `RAMADR` 2 and 4, and `ACALU` other than `B` and `AC+N`;
- `SCADA` 2–7 and `SCADB` 1–3;
- `SPEC` group 40 codes 2 and 5 (`EXPTST`), group 20 codes other than 5
  and 7 (so `LOADXR`, `LDINST` and `PXCT OFF`), and all of group 10;
- the skip inputs INTERRUPT REQ, EXECUTE, −I/O LATCH, −CONTINUE and
  −1 MSEC;
- the dispatches DIAG ADR, PAGE FAIL, NICOND and BYTE, and `DISP/MUL`
  with `J` bit 10 clear;
- `MULTI SHIFT`, and `CALL` with a return in one word;
- a read of a shift line that nothing drove in the previous cycle.

**Power-up [measured]** 2026-11-27 (`emu/ks10uop/tests/power_up.rs`, the
ITS listing 262). The power-up sequence runs from 0 to `PWRON` in 637
cycles, from two initial states (zero, and a pattern in the registers,
the RAM file and the flags). The table of the powers of ten in the
workspace equals 10^n for n from 0 to 21, as the test computes them, and
the microcode check of 10^21 passes. MASK, XWD1, ONE, PC, USER and the
trap flags have the values that the comments of the sequence give.

The run executes only these codes, so the test verifies nothing else:

| Field | Codes (octal) |
|---|---|
| `AD` | 1 5 24 33 34 35 37 41 42 44 45 47 65 74 |
| `DEST` | 0 1 3 5 7 |
| `SHSTYLE` | NORM, left and right |
| `SKIP` | 32 56 63 70 |
| `DISP` | 41 70 |
| `DBUS`, `DBM` | 1 2 3; 4 7 |
| `RAMADR` | 1 6 7 |
| `SCAD`/`SCADA`/`SCADB` | 6/0/0 7/1/0 |
| `SPEC` | 0 25 27 43 46 47 |

A model with the skip on SC after its load fails the test. A model that
shifts 0 into the sign on a right shift passes it, and so does a model
with the MUL dispatch on the QR 37 of the same cycle: power-up shifts
only positive values and does not multiply with `DISP/MUL`.

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
