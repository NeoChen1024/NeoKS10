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
