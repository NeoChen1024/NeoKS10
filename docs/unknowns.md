# Open items

Values and causes that have no source yet. When an item gets a source,
move the value to its place and remove the item here.

## DEC KS10 diagnostics

The diagnostics are on two tapes (docs/emulator.md, "Documents and
tapes"). Not known yet:

- Which of them apply to the ITS microcode behaviour. They are for the DEC
  microcode. `DSKEA` and `DSKEC` test DEC paging, which the ITS microcode
  does not have.

## KS10 processor and devices

- How the microcode takes a Unibus interrupt: the request line, the PI
  level from the adapter status register, and the use of the vector
  (`src/kshack/inout`, not read yet). docs/emulator.md.
- The register address of the `SSIMP` IMP interface (`%LHVEC`,
  `src/system/lhdh.defs`, not read yet). docs/emulator.md.
- The instruction times of the KS10, for the virtual time of the emulator.
  docs/emulator.md.
- The halt status block (`inout.50`, DUMP) has the 2901 registers AR,
  ARX, BR, BRX, FLG, T0 and the VMA. Their values at a halt come from the
  microcode steps before the halt. `emu/ks10` writes 0 for them.
- Whether a reference to nonexistent memory sets the APR flag "no
  memory" (KSDEFS, RDAPR 1.9) in addition to the page failure.
- The halt turns off the PI system in the hardware (`inout.50`, WRTHSB:
  "TURN OFF PI SYSTEM"). `emu/ks10` does not model a continue after a
  halt.
- Device times in `emu/ks10`, all estimates in instructions at 1 µs each:
  the RP06 seek and transfer time (`rh11.rs`, DELAY = 100), the DZ11
  character time (`dz11.rs`, 9600 baud), the CH11 transmission time
  (`ch11.rs`), the TM03 command time and rewind time (`tm03.rs`, DELAY =
  100, REWIND_TIME = 1000).
- What the TM03 does at the end of the recorded data of a tape image. A
  real drive reads blank tape. `emu/ks10` stops the command with
  "operation incomplete" (OPI) and attention.
- How the TM03 packs a record in reverse order if its frame count is not
  a multiple of the frames of a word. `emu/ks10` reverses the order of
  the words only. ITS does not use Read Reverse (`nmtape.30`).
- The serial number of the TM03 and the TM03 check character. `emu/ks10`
  gives 0.
- The parity bits of the UBA paging RAM as read (%UPPAR, %UPPVL), the
  serial number of the RP06, the CH11 check word, and how the IMP
  interface counts its address register in 32-bit mode. `emu/ks10` gives
  0 or leaves the register as it is.
- The line speed of the CTY, and the time of the 8080 for one character.
  `emu/ks10/src/console.rs` uses 1000 instructions (an estimate: 9600 baud
  at 1 µs for each instruction).

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
