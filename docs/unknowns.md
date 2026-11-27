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
  microcode steps before the halt. `emu/ks10-core-soft` writes 0 for them.
- Whether a reference to nonexistent memory sets the APR flag "no
  memory" (KSDEFS, RDAPR 1.9) in addition to the page failure.
- The halt turns off the PI system in the hardware (`inout.50`, WRTHSB:
  "TURN OFF PI SYSTEM"). `emu/ks10-core-soft` does not model a continue after a
  halt.
- Device times in `emu/ks10-machine`, all estimates in instructions at 1 µs each:
  the RP06 seek and transfer time (`rh11.rs`, DELAY = 100), the DZ11
  character time (`dz11.rs`, 9600 baud), the CH11 transmission time
  (`ch11.rs`), the TM03 command time and rewind time (`tm03.rs`, DELAY =
  100, REWIND_TIME = 1000).
- What the TM03 does at the end of the recorded data of a tape image. A
  real drive reads blank tape. `emu/ks10-machine` stops the command with
  "operation incomplete" (OPI) and attention.
- How the TM03 packs a record in reverse order if its frame count is not
  a multiple of the frames of a word. `emu/ks10-machine` reverses the order of
  the words only. ITS does not use Read Reverse (`nmtape.30`).
- The serial number of the TM03 and the TM03 check character. `emu/ks10-machine`
  gives 0.
- The parity bits of the UBA paging RAM as read (%UPPAR, %UPPVL), the
  serial number of the RP06, the CH11 check word, and how the IMP
  interface counts its address register in 32-bit mode. `emu/ks10-machine` gives
  0 or leaves the register as it is.
- The time of the SHA-256 of a disk image on the board (`--record`,
  `--replay`). On the x86-64 host, 317 MB take about 0.25 s
  **[measured]**; the Cortex-A9 has no SHA instructions.
- The line speed of the CTY, and the time of the 8080 for one character.
  `emu/ks10-machine/src/console.rs` uses 1000 instructions (an estimate: 9600 baud
  at 1 µs for each instruction).

## Instructions that the differential run does not check

The differential run (docs/emulator.md, "Differential runs") does not
compare these with a reference emulator, or the two reference emulators
disagree:

- Floating point instructions with unnormalized operands, and FAD with an
  exponent difference of 61 or more. The KS10 microcode handles them in
  its own way (`flt.5`). SIMH follows the KL10; KLH10 is not yet run
  with such operands.
- FDV and its forms with normalized operands: `emu/ks10-core-soft` and KLH10 agree,
  and SIMH differs in bit 35 of 744 of 2075 quotients
  (docs/discrepancies.md, "FDV: the quotient"). Emulators do not decide
  the case. Which side follows `flt.5` (FDV6 to SNORM) is not known until
  the microcode-level model runs it.

## The microcode-level model

[`microcode-model.md`](microcode-model.md), "The data path", gives what
the manual and the field definitions give. These connections need the
schematics (MP00540):

- Backplane inputs of the microsequencer that the CRA sheets do not
  trace: NICOND inputs 0, 4 and 6 (EA1, EF1, EK1), the MUL dispatch input
  FK2, and the page fail dispatch inputs EU1, FE2, FL2, FS2. The ITS
  microcode depends on EA1, EF1 and EK1 being inactive, and does not read
  the others **[measured]** 2026-11-27 (the ITS listing 262): the NICOND
  block (60–77) has its words at 61, 62, 63, 65, 67, 71, 72, 73, 75 and
  77, and the assembler put other words at 60, 64, 66, 70, 74 and 76;
  every `DISP/MUL` word has `J` bit 10 set; no word uses `DISP/PAGE
  FAIL`. The model takes EA1, EF1 and EK1 as inactive.
- The byte dispatch inputs BYTE DISP 09–11 (DPE3).
- The FXU term of `SPEC/EXPTST` (DPE9 E206, input 1).
- The D input of CSL5 E53 (FS (1)). While `MULTI SHIFT` and FE SIGN are
  both 1, the DPE/M clock needs FS (1) (CSL5 rev D, E52, E45). The model
  clocks the data path at each repeat, as the manual describes
  (EK-OKS10-TM-002 5.4.1, bit 79).

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
