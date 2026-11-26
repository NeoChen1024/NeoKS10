# Open items

Values and causes that have no source yet. When an item gets a source,
move the value to its place and remove the item here.

## DEC KS10 diagnostics

The diagnostics are on two tapes (docs/emulator.md, "Documents and
tapes"). Not known yet:

- Which of them apply to the ITS microcode behaviour. They are for the DEC
  microcode. `DSKEA` and `DSKEC` test DEC paging, which the ITS microcode
  does not have.
- How a pass of a program with `SUBSM` ends, and what it prints then. A
  60 s run of `DSKCA` in sims showed no end of a pass.

## KS10 processor and devices

- How the microcode takes a Unibus interrupt: the request line, the PI
  level from the adapter status register, and the use of the vector
  (`src/kshack/inout`, not read yet). docs/emulator.md.
- The register address of the `SSIMP` IMP interface (`%LHVEC`,
  `src/system/lhdh.defs`, not read yet). docs/emulator.md.
- The instruction times of the KS10, for the virtual time of the emulator.
  docs/emulator.md.

## AFI: 64-bit writes after 32-bit writes stop the PS

**[measured]** 2026-11-26, Linux 6.18.64-pynq-z2, memprobe build a4249ad7 at
100 MHz (docs/mem-path-experiment.md, M2c). The sequence:

1. `32BitEn` of AFI_WRCHAN_CTRL of HP0 is 1, and the design writes with
   32-bit beats. The writes complete.
2. The PS writes 0 to `32BitEn` while no transaction is in progress.
3. The design writes with 64-bit beats on HP0. After three or four
   writes, one write gets no response.

Then the PS stops: no network and no console output. JTAG reads give
`AFI_WRDEBUG.OutWrCmds` = 1 for HP0: one write command is between the AFI
and the DDR. Only a reset of the PS ends the state. The sequence stopped
the PS twice in two attempts.

64-bit reads after step 2 completed. A change of `32BitEn` from 0 to 1 and
back to 0 without 32-bit traffic had no effect: three 64-bit B9 passes
gave 0 mismatches after it.

UG585 v1.7 does not say when `32BitEn` can change. The cause is not known.
The PS program therefore never sets `32BitEn` back to 0 (`plio::afi`), and
a design uses one data width on a port from a reset of the PS.
