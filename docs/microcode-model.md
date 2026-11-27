# Microcode-level model

`emu/ks10-core-uop` is a model of the KS10 at the level of the microcode. A
model of the data path runs the microcode that the ITS build assembles.
If the sources disagree about an instruction, this model decides
(AGENTS.md, "Sources of truth"). It is also the model that the logic in
`rtl/` is compared with ([`architecture.md`](architecture.md)).

The work has three stages:

1. The data path and the microsequencer on a flat memory, without paging
   and I/O. Random instructions compare it with `emu/ks10-core-soft`.
2. ITS paging, the I/O cycles to the devices of `emu/ks10-machine`, interrupts,
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

`ks10_machine::boot::read_microcode` does these steps on the emulated RH11 and
RP06.

`.RAMFILE` in `ks10` gives the layout of an entry **[microcode]**: 108
bits, the listed fields from left to right, two parity bits and two pads
(MICRO, `$RAMFL` and `RAMFL4`). The generator `spec/gen_microcode.py`
reads this layout and the field definitions into
`emu/ks10-core-uop/src/fields.rs`.

**Parity.** The parity bit of `ODD PARITY<ranges>` is the XOR of the given
bits of the RAM file entry **[microcode]** (MICRO, `PNTB91`). With the
parity bit, each range set has an even number of ones. This agrees with
the comment of `.RAMFILE` ("Parity is even parity"). `EVEN PARITY` gives
the complement. The ranges are bits of the RAM file entry, not of the
microword.

**Check of the RAM file [measured]** 2026-11-27 (`make microcode`, test
`emu/ks10-core-uop/tests/ram_file.rs`). The RAM file was read from `rp0.dsk` of
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
generated `emu/ks10-core-uop/src/fields.rs` has their positions. The exact
logic of each mixer and condition is in the code of `ks10-core-uop`, with its
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
  **[manual]** (5.4.1, bit 79). The CRA clock stops while `MULTI SHIFT`
  and FE SIGN are both 1 (M8616-0-CSL5 rev D, E52, E33). Thus the word
  repeats with the same address and stack. If FE SIGN is 0, the DPE/M
  clock is off (E52, E45, E54). If FE SIGN is 1, the DPE/M clock also
  needs FS (1) (E53), which is not traced
  ([`unknowns.md`](unknowns.md)). The model stops if such a word also has
  a memory function, `FMWRITE` or a special function of group 10, 20 or
  40.
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

The mixers of the two inputs are on M8621-0-DPM3 rev B **[manual]**. SCAD
bit 00 is the left bit; bit 09 is the right bit. `SCADA` is # 03–05, and
`SCADB` is # 06–07; the select inputs are the inverted number field
bits with inverting inputs.

| `SCADA` | Bits 01–07 (E75, E51, E91, E67, E83, E99, E107) |
|---|---|
| 0 `SC` | SC 01–07 |
| 1 `S#` | # 09–15 |
| 2 `PTR44` | 1, 0, 0, 1, 0, 0, DP 06 |
| 3–7 `BYTE1`–`BYTE5` | DP 00–06, 07–13, 14–20, 21–27, 28–34 |

For `SCADA` 2–7, bits 00, 08 and 09 come from E28: SC 00, 08 and 09 if #
05 is 0, else # 08, 16 and 17. For `SCADA` 0 and 1, E28 gives the same
bits as the 8-input mixers. The 2-input mixer only reads # 05, so `S#`
gives # 08–17, and `SC` gives SC.

| `SCADB` | Bits 00–01 | Bits 02–09 (E36, E60, E76, E84, E92) |
|---|---|---|
| 0 `FE` | FE 00–01 | FE 02–09 |
| 1 `EXP` | 0, 0 | DP 01–08 XOR DP 00 (E68, E100, E41) |
| 2 `SHIFT` | DP 18, DP 18 | DP 28–35 |
| 3 `SIZE` | 0, DP 06 | DP 07–11, then 0, 0, 0 |

SIZE puts the byte size in SCAD bits 01–06, which is the size times 8.
The microcode agrees: `FE_S+2` uses `S#/20` **[microcode]** (ks10,
`FE_S+2`).

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

### Memory functions in the schematics

From M8621-0-DPM4 rev B and M8621-0-DPM5 rev B (MP00540 PDF pages 144
and 145) **[manual]**:

- **VMA load.** VMA 18–35 load from DP 18–35, and VMA 14–17 with VMA
  EXTENDED (`EXT ADR`) from DP 14–17, when DPE5 VMA EN is true: `MEM`
  with `LDVMA`, or `MEM` with `AREAD` and the DROM `VMA` bit, or SET SWEEP
  (DPE5 E205, E601).
- **VMA flags** (DPM4 E55, E54, E74, E82, E90). If `DP FUNC` is 1, USER,
  PREVIOUS, PHYSICAL and FETCH load from DP 00, 09, 08 and 02, and the
  I/O flags (WRU, VECTOR, I/O BYTE, VMA I/O) from DP 11, 10, 13 and 12.
  Otherwise PHYSICAL and FETCH come from the number field, the I/O flags
  are 0, PREVIOUS is DPMA PREVIOUS EN OR `SPEC/PREV`, and USER is
  (NOT `FORCE EXEC` AND NOT console EXECUTE AND USER) OR (`FETCH` AND
  USER) OR `FORCE USER` OR (PREVIOUS AND PCU).
- **AC reference.** VMA AC REF is NOT VMA PHYSICAL AND VMA 18–31 all 0
  (DPM4 E160, E191, E168).
- **Cycle type** (DPM5 E48, E66, E33). READ, WRITE, WRITE TEST and CACHE
  INHIBIT come from the number field (`READ CYCLE`, `WRITE CYCLE`, `WRITE
  TEST`, `DONT CACHE`), from DP 03, 05, 04 and 07 if `DP FUNC` is 1, or
  from the DROM if `AREAD` is 1. They load only on MEM EN: `MEM` with
  `WAIT`, or `MEM` with `BWRITE` and the DROM `COND FUNC` bit (E118).
- **Stop.** STOP MAIN MEMORY is AC REF OR PAGE FAIL OR CACHE HIT (E47):
  then no bus cycle takes place. For a read, FORCE RAMFILE makes the
  DBus take the RAM file in place of DBM MEM (E14).

The microcode uses two microinstructions for each cycle: `START READ`
or `START WRITE` set up the cycle, and `MEM WAIT` (`MEM/1, WAIT/1`)
completes it. A read gives the word through `DBM/MEM`. A write puts the
word on DP in the `MEM WAIT` word (`MEM_[]` is `AD/B, DBUS/DP,
RAMADR/VMA`) **[microcode]** (ks10, the macros after `MEM CYCLE`). In
the data phase the bus mixers of DPM8 and DPM9 take DP 00–35 **[manual]**
(M8621-0-DPM8 and DPM9 rev B, MIX inputs).

**Paging.** A virtual reference goes through the page table of DPM6
only if DPM8 PAGE EN is 1: PAGED REF is BUS REQUEST AND PAGE EN AND NOT
VMA PHYSICAL (M8621-0-DPM9 rev B, E38, E119). PAGE EN is DP 23 at
`SPEC/APR EN` (M8621-0-DPMB rev B, E123), and TRAP EN is DP 22 there. A
cache hit also needs PAGE EN (M8621-0-DPM7 rev B, E114). The power-up
sequence gives `SPEC/APR EN` with DP 0 (`[FLG]_0, SET APR ENABLES`), so
after power-up there is no paging, no cache and no trap **[manual]**
**[microcode]** (simple, "POWER UP SEQUENCE"). The address then is VMA:
bits 18–35, and bits 14–17 only with VMA EXTENDED (DPM8 E111, E119,
E127; DPM9 E153).

With PAGE EN 1, a page fail jumps to 3777 (`PAGE-FAIL`), and the ITS
microcode fills the entry from the page tables in memory that
`WORK[DBR1]` to `WORK[DBR4]` point to **[microcode]** (itspag,
`PAGE-FAIL`, `PFMAP`, `GOTDBR`). The page table (M8621-0-DPM6 rev C) has
512 entries by VMA 18–26: VALID, WRITEABLE, CACHEABLE and USER, and the
physical page 16–26 from DP 18, 21, 22, VMA USER and DP 25–35.

**The cycle in the registers.** The cycle type (DPM5 E33), MEMORY CYCLE
(DPM6 E16 R0), VMA and the VMA flags load at the clock. The word that
completes a cycle sees what the starting word loaded **[manual]**:

- MEM EN is `MEM` AND (`WAIT` OR (`BWRITE` AND DROM `COND FUNC`)) (DPM5
  E118). MEM WAIT is MEM EN OR `SPEC/MEM WAIT` OR `SPEC/MEMCLR` OR DP
  RESET (DPM5 E8).
- START CYCLE is MEM EN AND (READ EN OR WRITE EN) AND NOT RPW CYCLE AND
  SYNC (DPM5 E5). READ EN and WRITE EN are the new cycle type. RPW CYCLE
  is MEM READ AND MEM WRITE AND NOT STOP MAIN MEMORY, with the old
  values (DPM5 E32). E16 loads START CYCLE at MEM WAIT.
- MEMORY DELAY is (MEM EN OR `SPEC/MEM WAIT`) AND MEMORY CYCLE (DPM5
  E30): this word completes the cycle.
- A bus request needs START CYCLE, or a request that has no grant yet
  (DPM5 E46, E70); single-step mode adds terms (E39, E62). DP RESET is in
  STOP MAIN MEMORY and clears the held request.
- DPMA WRITE CYCLE, which puts DP on the bus, is SYNC AND NOT STOP MAIN
  MEMORY AND NOT MEM READ AND MEMORY CYCLE AND MEMORY DELAY outside
  single-step mode (DPMA E14, E21, E17). A cycle with READ and WRITE thus
  reads and sends no data. With RPW CYCLE, a MEM EN word starts no cycle
  and sends nothing.
- FORCE RAMFILE is (CACHE HIT EN OR AC REF) AND MEMORY DELAY AND MEM
  READ (DPM5 E14). It changes the DBus select DBM (3) into the RAM file
  (2) in both halves (M8620-0-DPE3 rev B, E70).
- RAMFILE WRITE is MEMORY CYCLE AND MEM WAIT AND (AC REF AND NOT MEM READ,
  or NOT AC REF AND NOT STOP MAIN MEMORY) (M8621-0-DPMA rev B, E22, E54).
  The second term writes the cache data: the RAM file gets the DBus at
  each memory cycle. The address comes from `RAMADR` of the word.
- INT OR ERR is (MEMORY CYCLE AND NOT VMA PHYSICAL AND NOT MEM WRITE AND
  (INT OR 1MS)) OR NXM ERR OR BAD DATA EN (DPM6 E15, E9). E16 latches INT
  (DPEB INTERRUPT REQ) and 1MS (DPMC 1 MSEC) at MEM WAIT. INT OR ERR
  makes a page fail: the microcode takes interrupts and the 1 ms tick
  through `PAGE-FAIL`.

**RAM file address** (M8620-0-DPE6 rev B) **[manual]**. Bits 00–02 are 0
unless `RAMADR` bit 4 is 1 (E7, E16, E8). Bits 03–05 come from E14, E15
and E13 through E3. Bits 06–09 come from E22, E23, E24 and E21.

| `RAMADR` | Address |
|---|---|
| 0 `AC#` | current block, AC |
| 1 `AC*#` | current block, the 74LS181 output (E79) |
| 2 `XR#` | previous block if XR PREV, else current block; XR |
| 4 `VMA` | if AC REF: previous block if VMA PREVIOUS, else current block; VMA 32–35. Else 1000 plus VMA 27–35 (the cache) |
| 6 `RAM` | VMA 26–35 |
| 7 `#` | DBM 26–28 and DBM 11–17: the number field bits 08–17 with `DBM/#` |

**DBM** (M8621-0-DPM1 and DPM2 rev B) **[manual]**. Input 5 (VMA) has
in the left half: 00 VMA USER, 02 FETCH, 03 MEM READ, 04 MEM WR-TEST, 05
MEM WRITE, 07 CACHE INH, 08 PHYSICAL, 09 PREVIOUS, 10 VMA I/O, 11 WRU,
12 VECTOR, 13 I/O BYTE, and 14–17 VMA 14–17. Bits 01 and 06 are 0. The
right half is VMA 18–35. Input 6 is MB.

**PXCT** (M8621-0-DPMA rev B) **[manual]**. `SPEC/PXCT EN` (group 10,
code 6) loads E71: PXCT ON gets NOT `SPEC/PXCT OFF` (group 20, code 6),
and B09–B12 get DP 09–12. `SPEC/PXCT OFF` is code 36: both groups. PREVIOUS
EN is the input of E63 that the `PXCT` field selects: 0 is 0, 1 is B09,
2 and 3 are B10, 4 and 5 are B11, 6 and 7 are B12. E63 is enabled if
PXCT ON AND (`PXCT` bit 11 OR B09) (E62). XR PREV loads from PREVIOUS EN
at `SPEC/LOADXR` (DPEA E311).

**Console** (M8616-0-CSL4 rev C) **[manual]**. The 8080 sets CONTINUE
(bit 0), EXECUTE (bit 1) and RUN (bit 2) with a write to register 212
(E171, E149, E135). `SPEC/#` (group 10, code 0) with the number field
clears them: bit 12 CONTINUE, bit 13 EXECUTE, bit 14 RUN. Bit 15 sets
HALT LOOP and bit 16 clears it (E161). The skip inputs of group 10 are
EXECUTE (4), NOT I/O LATCH (5), NOT CONTINUE (6) and NOT 1 MSEC (7),
through the latch CRA2 E98 (M8622-0-CRA2 rev B).

**NICOND and traps** **[manual]**. TRAP 1, 2 and 3 come from a decoder
of the trap flags. It is enabled only if CSL4 TRAP EN AND DPEB TRAP EN (M8620-0-DPE9
rev B, E125). DPEB TRAP EN is DP 22 at `SPEC/APR EN` (M8620-0-DPEB rev B,
E173). `SPEC/NICOND` loads TRAP CYCLE (CRA2 E159).

**Interrupt request** (M8620-0-DPEB rev B) **[manual]**. `SPEC/LDPI`
loads the complement of DP: program requests from DP 11–17, levels in
progress from DP 21–27, PI ON from DP 28, and active levels from DP
29–35 (E126, E140, E175). A level requests if it has a program request,
or a bus request and is active (E168, E154, E161). Two priority encoders
and a comparator give INTERRUPT EN if PI ON and the highest request is
above the highest level in progress (E147, E134, E148). E167 latches it
as INTERRUPT REQ at each clock. An APR request (DPMB APR INT REQ) drives
a bus request at the level of DP 33–35 (E173, E166).

**1 ms timer** (M8621-0-DPMC rev D) **[manual]**. At each 1 ms step of
the counter, E1 takes CSL4 1 MSEC EN. `SPEC/CLRCLK` clears E1 after the
clock, through E3 R2. E3 R3 copies E1 at each clock and gives DPMC 1 MSEC.

## Stage 1

Stage 1 runs single instructions that need no I/O, in the state after
power-up: PAGE EN, TRAP EN and the APR enables are 0, so there is no
paging, no cache and no trap. The model is `emu/ks10-core-uop/src/alu.rs` and
`machine.rs`.

A function is either modelled or a stop with "unmodelled". The model
does not guess a value. A shift line that nothing drives is undefined; a
step that reads one stops.

**Initial state.** `Machine::new` takes a fill value. The 2901
registers, the RAM file and main memory get it. The other registers and
flags without a reset get 0 if the fill is 0, and all ones if not. Each
test runs with both fills: a correct run does not depend on them, except
where the hardware also depends on its state at power on.

**Modelled** (2026-11-27):

- the microsequencer: skip, dispatch, the dispatch ROM, AREAD, the AC
  dispatch, the stack, NICOND;
- the arithmetic unit with the shifts, DIVIDE, MULTI PREC and E603. If
  DP depends on the DBus and the DBus selects DP, the model computes DP
  with D = 0 and with D = all ones. If the two agree, the 2901 function
  does not depend on D, and the model uses the value;
- the DBus with FORCE RAMFILE; DBM inputs 3, 4, 5, 6 and 7;
- the RAM file with `RAMADR` 0, 1 (`ACALU` `B` and `AC+N`), 2, 4, 6 and 7,
  and RAMFILE WRITE;
- memory cycles on 512 Ki words of main memory: VMA and the VMA flags,
  the cycle type, MEMORY CYCLE, AC references, MB;
- SCAD with `SCADA` SC or `S#`, `SCADB` FE; SC and FE;
- IR, AC, XR with XR PREV; the program flags with `SPEC/FLAGS` and
  `SPEC/ASHOV`; `SPEC/LDACBLK`, `SPEC/LDPI`, `SPEC/APR EN`, `SPEC/PREV`,
  `SPEC/PXCT EN`, `SPEC/PXCT OFF`, `SPEC/MEMCLR`, `SPEC/MEM WAIT`,
  `SPEC/NICOND`, `SPEC/CLRCLK`, `SPEC/#`;
- the console inputs RUN, CONTINUE, EXECUTE, TRAP EN and 1 MSEC EN, and
  HALT LOOP; INTERRUPT REQ from program requests; the 1 ms flag.

Timing within a cycle is not modelled: SYNC is 1 at the clock, and the
skip latch CRA2 E98 has the values of the cycle. The cache directory
(DPM7) is not modelled; without PAGE EN there is no cache hit.

The rest stops (the `unmodelled` branches of `machine.rs`):

- paging: PAGE EN with a virtual VMA and a memory cycle;
- a page fail from INT OR ERR; an APR interrupt request (APR enables
  24–31 not 0) where INTERRUPT REQ is read;
- I/O cycles (VMA I/O, WRU, VECTOR), and a bus cycle above 512 Ki
  words;
- DBus input 0; DBM inputs 0, 1 and 2, and DP with a byte insertion; DP
  on the DBus where DP depends on D;
- `RAMADR` 3 and 5, and `ACALU` other than `B` and `AC+N`;
- `SPEC` group 40 codes 2 and 5 (`EXPTST`); group 20 codes 2, 3 and 4
  (`APR FLAGS`, `CLRCSH`, `SWEEP`); group 10 codes 2, 3 and 4 (I/O
  latch, I/O busy, `LDPAGE`);
- the skip input NOT I/O LATCH;
- the dispatches DIAG ADR, PAGE FAIL and BYTE, and `DISP/MUL` with `J`
  bit 10 clear;
- `MULTI SHIFT` with a memory function, `FMWRITE` or a special function
  of group 10, 20 or 40, and `CALL` with a return in one word;
- a read of a shift line that nothing drove in the previous cycle.

**NICOND inputs EA1, EF1 and EK1.** The model takes them as inactive.
The schematics do not trace them ([`unknowns.md`](unknowns.md)). The ITS
microcode depends on them being inactive: the assembler put other words
at the NICOND addresses for codes 0, 4 and 6. KS10FPGA has no such
inputs in its NICOND dispatch **[emulator]** (KS10FPGA 5090e7a1,
`disp_ni.v`).

**Power-up [measured]** 2026-11-27 (`emu/ks10-core-uop/tests/power_up.rs`, the
ITS listing 262). The power-up sequence runs from 0 to `PWRON` in 637
cycles, from both fills. The table of the powers of ten in the
workspace equals 10^n for n from 0 to 21, as the test computes them, and
the microcode check of 10^21 passes. MASK, XWD1, ONE, PC, USER and the
trap flags have the values that the comments of the sequence give. In
642 cycles the machine is in `HALTLP` with HALT LOOP set. Location 1 has
PC 0. With the zero fill, location 0 has the halt code 0 (`HALT/POWER`).

With the fill of all ones, location 0 keeps its content. The cycle type
in DPM5 E33 has no clear input and loads only at MEM EN. The loop for
the powers of ten leaves VMA at an address that is not an AC. If E33
holds READ and WRITE at power on, RPW CYCLE is 1 at `PWRON`: that word
starts no cycle, and its `MEM WRITE` sends no data (see "The cycle in
the registers"). E33 then holds the write type, and the write of PC to
location 1 takes place. The hardware has the same dependence on its
state at power on.

### Single instructions

Design decision 2026-11-27: the test puts the ACs into block 0 of the
RAM file and PC into the 2901 register. The console starts a program
with EXECUTE of a JRST, which reads the console register with an I/O
cycle (`cmds.m80`, `STCMD`, `EXINT`; inout, `CONT`); stage 1 has no I/O.
The test then does what the console command `SI` does: it sets CONTINUE
without RUN ("I/O WRT 212/1") **[its]** (`cmds.m80`, `SICMD`; `condef.m80`,
`CPUCTL`). One 1 ms step with 1 MSEC EN 0 clears the 1 ms flag first. The microcode then does one
instruction, finds RUN 0 at NICOND (code 5), and halts with the halt code
2 (`HALT/CSL`) in location 0 and PC in location 1 (inout, `HALTLP`,
`CONT`, `HALTED`, `PWRON`, `STOPC`). It also writes the halt status block
at 500 (`WRTHSB`, `DUMP`). The test checks PC, ONE, MASK, XWD1 and the
halt code there.

**Result [measured]** 2026-11-27 (`emu/ks10-core-uop/tests/instructions.rs`,
the ITS listing 262). The test compares the ACs, PC, the program flags
and the words at 2000 and 2001 with `emu/ks10-core-soft`. All 42 cases agree,
from both fills: moves to and from memory and ACs, ADD and SUB with
overflow and carries, ADDB, AND, IOR, SETZM, AOS (memory, AC
destination, AC operand), HRRZ, HLL, CAIE, SKIPL, an indexed and an
indirect MOVE, IMUL, MUL, IDIV, DIV, and the shifts: ASH left and right,
by 0 and with overflow in the first or the last step; ASHC both ways; LSH
both ways and by 40 places; ROT both ways. 48 random left shifts with ASH
and ASHC from a fixed seed also agree. TRAP 1 after ASH is the overflow
of the last step ([`discrepancies.md`](discrepancies.md), "ASH: TRAP 1
when an early step loses significance"). Without that rule in
`emu/ks10-core-soft`, 15 of the 90 cases differ, from both fills.

The run executes these codes (both fills, with the halt path):

| Field | Codes (octal) |
|---|---|
| `AD` | 1 5 11 21 22 24 25 32 33 34 35 36 37 40 41 42 44 45 46 47 51 57 65 70 74 |
| `DEST` | 0–7 |
| `SHSTYLE` | NORM, ROT and ASHC left and right; DIV left |
| `SKIP` | 32 33 36 42 52 53 56 62 63 64 66 70 |
| `DISP` | 1 12 13 34 37 41 62 64 66 70 |
| `DBUS`, `DBM` | 1 2 3; 4 5 6 7 |
| `RAMADR` | 0 1 2 4 6 7 |
| `SCAD`/`SCADA`/`SCADB` | 3/1/0 4/1/0 4/1/2 6/0/0 7/1/0 |
| `SPEC` | 0 3 4 6 10 15 21 25 27 36 43 44 46 47 61 |

Modelled, but no case exercises it: PXCT ON and PREVIOUS EN 1, so XR
PREV 1 and the previous block of `RAMADR` 4; `SPEC/PREV`, `SPEC/LDPXCT`,
`SPEC/MEM WAIT` and `SPEC/CLRCLK`; the VMA flags from `DP FUNC`; the USER
terms in user mode; INTERRUPT REQ 1 and `SKIP/INT`; the NICOND trap
codes; `SCADA` 2–7, and `SCADB` 1 and 3. Some of these rest on readings at overview scale (DPEB E126,
E140, E175; DPE9 E125; CRA2 E147; DPMC E1). They need a closer reading
before a case reaches them.

`RAMADR/#` takes the address through DBM. Of the 173 words that use the
RAM file with `RAMADR/#`, all have `DBM/#` **[measured]** 2026-11-27 (a
scan of the ITS listing 262).

Unit tests in `machine.rs` run single memory cycles with hand-written
words: a write, a cycle with READ and WRITE, and RPW CYCLE. Each fails
without its rule.

Mutants of the memory model: a write of MB in place of DP stops each
case; no FORCE RAMFILE gives four differences. An AC write at the end of
a read survives: FORCE RAMFILE gives the DBus the same RAM file word,
so the write does not change it. The MUL dispatch on QR 37 of the same
cycle now gives differences in IMUL and MUL. The right shift with 0 into
the sign gives differences in IMUL, MUL, ASH and ASHC.

Mutants of the shifts: `SCADB` `SHIFT` with 0 in bits 00 and 01 gives
differences in ASH right, ASHC and LSH. A last `MULTI SHIFT` word that
changes the data path gives differences in ASH right, LSH and ROT.

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
