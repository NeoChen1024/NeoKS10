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
