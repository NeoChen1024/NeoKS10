# Discrepancies

Cases in which two sources disagree (AGENTS.md, "Sources of truth"). Each
case gives both statements and the source that the project uses.

## APRID of the ITS microcode

- **[microcode]** `src/kshack/inout.50`, label `APRID`, with the values of
  `its.15` (`UCV=262.`, `HWOPT=0`, `HWSER=1729.`) and `ks10.47` (`OPT=020`,
  field "MICROCODE OPTIONS"): left half 020406 (options 020, version 262
  decimal = 406 octal), right half 003301 (hardware options 0, serial number
  1729 decimal = 3301 octal).
- **[emulator]** sims `pdp10-ks` (commit 9556eca0, `set cpu its`): the title
  of the diagnostic `DSKCA` gave `CPU#=4097, MCV=416, MCO=570` on 2026-11-26
  (docs/emulator.md, section "Diagnostics").

The project uses the microcode: `APRID` gives 020406,,003301.

## Program interrupt requests on levels that are off

- **[manual]** PRM 1982, 4.1, WRPI bit 24: request interrupts on the
  selected levels "and force the processor to recognize them even on
  levels that are off". Bit 26: turned-off levels do not recognize
  requests "unless made by a WRPI with a 1 in bit 24".
- **[microcode]** `src/kshack/inout.50`, LOADPI: the program requests go
  to the PI hardware only on levels that are on ("ONLY REQUEST CHANS THAT
  ARE ON").

The project uses the microcode: a program request counts on an active
level only (`emu/ks10/src/apr.rs`, `Pi::accept`).

## Bits 0 and 2 of the RDUBR word

- **[its]** `src/system/ksdefs.194`, RDUBR: "RDUBR always returns a word
  with 4.9 and 4.7 set".
- **[microcode]** `src/kshack/inout.50`, WRUBR and RDUBR of the ITS
  version: WRUBR puts bit 0 into UBR with the AC blocks and bit 2 with the
  base address. RDUBR returns UBR AND 507703,,777777. So the bits are 0
  until a WRUBR sets them. ITS always sets both bits, so both sources
  agree for ITS.

The project uses the microcode (`emu/ks10/src/pag.rs`, `wrubr`).

## The AC of MOVSRJ at the end

- **[manual]** PRM 1982, 2.12, MOVSRJ: "AC bits 9-35 contain the number of
  source bytes skipped over (if any)".
- **[microcode]** `src/kshack/extend.4`, MVSKP and MOVST3: after the
  extra source bytes are skipped, AC+3 is 0, and the end stores AC+3 (0)
  with the flags into AC.

The project uses the microcode: AC bits 9-35 are 0 at the end
(`emu/ks10/src/extend.rs`, `movs`).

## FAD with a large exponent difference

- **[manual]** PRM 1982, 2.3, footnote to FAD: if the difference of the
  exponents is more than 72 (64 in the KI10), the hardware uses 0 for the
  smaller operand; from 54 to 72, a negative operand becomes all ones.
- **[microcode]** `src/kshack/flt.5`, FAD to FAS4: the smaller operand is
  shifted right in BR!Q (ASHC) by the full difference, with no limit. A
  negative operand always becomes all ones.
- **[emulator]** sims `pdp10-ks` 9556eca0 (`kx10_cpu.c`, FAD, for KL and
  KS): 0 for a difference of 65 or more.

The project uses the microcode (`emu/ks10/src/float.rs`, `fad`).

## TM03: a space that stops at a tape mark

- **[emulator]** KLH10 6d733f2, `dvtm03.c`, `tm_cmddon`: after Space
  Forward or Space Reverse, FC has the count; there is no error bit.
- **[emulator]** sims 9556eca0, `kx10_tu.c`, `FNC_SPACEF`: if the space
  stops at a tape mark before FC counts to 0, the formatter sets the frame
  count error (FCE).
- **[its]** `src/kshack/cmds.m80`, `MBINT`: "we expect there to be a frame
  count error from the space forward". `src/system/nmtape.30`, `MGSPCF`,
  accepts FCE after a space.

The project uses the 8080 source: a space that stops at a tape mark
before the count ends sets FCE (`emu/ks10/src/tm03.rs`).

## TM03: a read of a tape mark

- **[emulator]** KLH10 6d733f2, `dvtm03.c`, `tm_flsbuf`: the read ends
  with FC 0 and TM; no error bit.
- **[emulator]** sims 9556eca0, `kx10_tu.c`, `FNC_READ`: the read sets
  FCE and the controller transfer error.
- **[its]** `src/system/nmtape.30`, `MGRD1A`: "the TM03, which doesn't
  give a FC error on READ". ITS checks TM on the normal path and on the
  error path (`MGRERR`, `MGRER2`).

The ITS source does not decide the case. The project uses KLH10, which
agrees with the ITS comment: a read of a tape mark gives TM without an
error.

## ASH and ASHC: a left shift of many places

- **[microcode]** `src/kshack/simple.42`, ASHL and ASHC: the shift goes
  one place for each step of SC, for the full count (up to 255 places).
  Overflow is set if a bit that leaves bit 1 differs from the sign.
- **[emulator]** sims 9556eca0, `kx10_cpu.c`, ASH (240): the result is
  `AR << SC` in a 64-bit integer. For ASH, if 36 + SC > 64, the overflow
  test loses bits; if SC ≥ 64, the C shift is undefined. ASHC (244) does
  the same with 35 + SC > 64. **[measured]** 2026-11-27, `oracle.rs`:
  ASH 13,160614 (140 places) of 717711,,575334 gives 515753,,340000 in
  SIMH. That word is the shift by 140 mod 64 = 12 places.

The project uses the microcode. The differential run does not compare
ASH left shifts of more than 28 places, or ASHC left shifts of more than
29 places (`emu/ks10/tests/oracle.rs`, `skip`).

## Shift count -256

- **[microcode]** `src/kshack/ks10.47`, SCADB SHIFT: the count is bits 18
  and 28 to 35 of E. So E with bit 18 set and bits 28 to 35 clear is -256
  places.
- **[emulator]** sims 9556eca0, `kx10_cpu.c`, ASH, LSH, ASHC and LSHC:
  `SC = ((0377 ^ AB) + 1) & 0377` is 0 for this E, so nothing shifts. ROT
  and ROTC use 0400 for this case.

The project uses the microcode (`emu/ks10/src/shift.rs`, `count`). The
differential run does not compare this count.

## CIRC

- **[microcode]** `src/kshack/its.15` sets `CIRC=1`, and
  `src/kshack/simple.42`, CIRC, assembles the instruction (opcode 247).
- **[emulator]** sims 9556eca0, `kx10_cpu.c`: the CIRC code of case 0247
  is in a part of the file that the KS10 build does not compile. In
  `pdp10-ks`, opcode 247 changes nothing. **[measured]** 2026-11-27: CIRC
  3,3 with AC 3 = 1 left AC 3 and AC 4 unchanged.

The project uses the microcode. KLH10 6d733f2a has CIRC (`KLH10_I_CIRC`
in the configuration of the ITS tree). **[measured]** 2026-11-27: 318
CIRC cases of the differential run agree with KLH10.

## FDV: the quotient

- **[manual]** DSKCB (DECSYSTEM 2020 advanced instruction diagnostic 2),
  `DSKCB.SEQ` page 32, test P7700: FDV of 377400,,777777 by
  577400,,777777 must give 777375,,773771. The divisor is not normalized
  (its magnitude is 200377,,000001; bit 9 is 0).
- **[measured]** 2026-11-27: sims 9556eca0 `pdp10-ks` gives
  777375,,773772 for this test and prints "C(AC) FAILED" at 032610. It
  passes the other FDV tests of DSKCB. `emu/ks10` passes DSKCB (`make
  diag`).
- **[measured]** 2026-11-27, `oracle.rs` without `skip`: with normalized
  operands, 744 of 2075 FDV cases (all forms) differ in bit 35 of the
  quotient. No DEC vector with normalized operands separates the two
  emulators. Which one does what the microcode does is open
  (`docs/unknowns.md`).
- **[measured]** 2026-11-27: KLH10 6d733f2a agrees with `emu/ks10` in
  all 2123 FDV cases of 100000 (both emulators are rank 4; this does not
  decide the case).
- **[microcode]** `src/kshack/flt.5`, DCODE of FDVB and FDVRB (FL-BOTH):
  the quotient goes to AC and to E. **[measured]** 2026-11-27: FDVRB with
  a dividend of 0 stores 0 in the AC, but SIMH leaves E unchanged.

The project uses the diagnostic for P7700 and the microcode for the store.
The differential run does not compare FDV.

## IMUL: a negative product that does not fit

- **[microcode]** `src/kshack/simple.42`, IMUL to IMUL3: if the high word
  of the product is neither 0 nor all sign bits, Overflow is set.
- **[emulator]** sims 9556eca0, `kx10_cpu.c`, IMUL (220): Overflow is set
  if the high part of the magnitude is more than 1 for a negative
  product. So a negative product of magnitude 2^35 + 1 to 2^36 - 1 does
  not set Overflow. **[measured]** 2026-11-27: IMULB 5,2 of 17 by
  750330,,434634 (product -47466848220) sets no flag in SIMH.

The project uses the microcode. The differential run does not compare
these products.

## IDIV of -2^35 by 1 or -1

- **[microcode]** `src/kshack/simple.42`, IDIV and DIV1: the only test
  before DIVSUB is the test for a divisor of 0.
- **[manual]** PRM 1982, page 2-14: on the KS10, -2^35 divided by -1
  gives -2^35 without an error.
- **[emulator]** sims 9556eca0, `kx10_cpu.c`, IDIV (230): a dividend of
  -2^35 and a divisor of magnitude 1 set No Divide.

The project uses the microcode and the PRM. The differential run does not
compare these cases.

## DIV of -2^70 by 0

- **[microcode]** `src/kshack/simple.42`, DIV1: a divisor of 0 sets No
  Divide ("DIVIDE BY ZERO"), and nothing is stored.
- **[emulator]** sims 9556eca0, `kx10_cpu.c`, DIV (234): the magnitude of
  -2^70 (AC 400000,,0, bits 1 to 35 of AC + 1 clear) is again 400000,,0,
  so the test for no divide passes for a divisor of 0. For a divisor that
  is not 0, the test gives No Divide. **[measured]** 2026-11-27: DIVB
  14,2044 with AC 14 = 400000,,0, AC 15 = 0 and a divisor of 0 sets no
  flag and stores 400000,,1.

The project uses the microcode. The differential run does not compare
this case.

## HLLE in KLH10

- **[microcode]** `src/kshack/simple.42`, HLLE: `READ [AR], SKIP DP0`,
  then `RIGHT_0` or `RIGHT_-1`. The left half does not change.
- **[emulator]** KLH10 6d733f2a, `inhalf.c` (HLLE) and `kn10ops.h`
  (`op10m_trz`): for a positive source, `XWDVAL(w) &= ~(H10ONES)`.
  **[measured]** 2026-11-27: HLLES 11,777773 of 040267,,515675 gives
  000267,,000000; HLLE of 100000,,123456 gives 0, and of 300000,,123456
  gives 0. So bits 1 to 3 are cleared. **[estimate]** The cause: the mask
  `H10ONES` has the type `uint19`, so its complement has 32 bits and
  clears bits 32 to 35 of the 64-bit integer word.

The project uses the microcode. The differential run does not compare
HLLE, HLLEM and HLLES of a positive source with a bit of 1 to 3 set.

## DIV in KLH10: a high word equal to the divisor

- **[microcode]** `src/kshack/simple.42`, DIVB, DIVC and NODIV: the
  magnitude of the high word minus the magnitude of the divisor; if the
  result is not negative, No Divide. So equal magnitudes give No Divide.
- **[manual]** PRM 1982, page 2-14, DIV: if the high word of the
  magnitude of the dividend is greater than or equal to the magnitude of
  the divisor, No Divide.
- **[emulator]** KLH10 6d733f2a, `kn10ops.c`, `ddivstep`: for equal
  magnitudes and a negative quotient, the result is 400000,,0 without an
  error, as the comment says a KL10 does. **[measured]** 2026-11-27: DIV
  7,2036 with AC 7,10 = 777777,,777750 0 and a divisor of 30 gives
  400000,,0 and no flag. DIVM 3,2041 with AC 3,4 = 1 400000,,0 (2^35)
  and a divisor of -1 stores 400000,,0 and sets no flag.

The project uses the microcode and the PRM. The differential run does
not compare DIV when the high word of the magnitude of the dividend
equals the magnitude of the divisor.

## SCAD: seven or eight functions

- **[manual]** EK-OKS10-TM-002, 5.5.4: the SCAD ALUs "are limited to the
  seven functions listed in the table at the upper left [of DPM3] because
  selection is made by only three bits".
- **[microcode]** `src/kshack/ks10.47`, field `SCAD`: eight codes, 0 to 7
  (`A*2`, `A.OR.B`, `A-B-1`, `A-B`, `A+B`, `A.AND.B`, `A-1`, `A`).

The project uses the microcode: eight codes. The DPM3 table can show if
two codes give the same function.

## Dispatch: the address bits of each group

- **[microcode]** `src/kshack/ks10.47`, "MICROCODE FIELDS -- DISPATCH":
  DISP EN 40 and DISP EN 10 control the low 4 bits of the jump address,
  DISP EN 20 controls "THE HI 7 BITS", and all 11 bits need two groups.
- **[manual]** EK-OKS10-TM-002, 5.4.1, bits 57–62: the dispatch is ORed
  with `J` bits 0–7 or 8–11 or both.
- **[manual]** M8622-0-CRA1 rev A1: group 20 drives address bits 00–07
  (E516, E517, E520, E521). For codes 2 and 3, bits 00 and 01 are 0.

The project uses the schematic: group 20 drives bits 00–07. The
microcode text counts 7 bits because bit 00 is outside the 4000 words of
the control RAM.

## ASH: TRAP 1 when an early step loses significance

- **[microcode]** **[manual]** The ITS microcode shifts ASH left one place
  per step and tests each step with `ASH AROV` (`SPEC/ASHOV`) (simple,
  `ASHL`, macro `ASH36 LEFT`). At each `SPEC/ASHOV`, DPE9 E78 loads OV and
  TRAP 1. OV has a hold term: OV AND NOT JFCL 09 (E68). TRAP 1 has none:
  P OR X OR (LOAD AND DP 10) OR (DBM 08 AND `SPEC/FLAGS`) (M8620-0-DPE9
  rev B, E85, E92, E78). A step without overflow thus clears TRAP 1.
  **[measured]** 2026-11-27, the microcode-level model
  (`emu/ks10uop/tests/instructions.rs`, case "ASH overflow"): ASH 1,2 with
  AC 1 = 200000,,000001 gives AC 1 = 4 and OV 1. The first step sets OV
  and TRAP 1; the second step does not overflow and clears TRAP 1.
- `emu/ks10` (`shift.rs`, `execute`, 240) tests the whole shift once and
  sets Overflow and Trap 1 together. **[emulator]** sims 9556eca0,
  `kx10_cpu.c`, ASH (240), and KLH10 6d733f2, `kn10ops.c`, `x_ashflg`, do
  the same.
- **[emulator]** KS10FPGA 5090e7a1, `pcflags.v`: at ASH TEST, both OV and
  TRAP 1 take the overflow of the step, with no hold.

Not decided. The microcode-level model follows its reading of rank 1, and
`emu/ks10` keeps Trap 1. A DEC diagnostic that tests Trap 1 after ASH
(rank 2), or a second reading of DPE9, decides the case. The test of
single instructions expects this difference and no other.
