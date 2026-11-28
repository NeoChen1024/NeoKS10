# Open items

Values and causes that have no source yet. Each item has one of two
kinds:

- **Answer not found:** the machine has an answer, and no source of this
  project gives it yet. When an item gets a source, move the value to its
  place and remove the item here.
- **Implementation-defined:** the PRM and the DEC diagnostics do not
  define it, and this project selects a behaviour
  ([`conformance.md`](conformance.md)). When the selection is made, the
  item moves to `spec/impldef.toml`.

## DEC KS10 diagnostics

The diagnostics are on two tapes (docs/emulator.md, "Documents and
tapes"). Not known yet:

- **Answer not found.** Which of them apply to the ITS microcode behaviour. They are for the DEC
  microcode. `DSKEA` and `DSKEC` test DEC paging, which the ITS microcode
  does not have.

## KS10 processor and devices

- **Answer not found.** The register address of the `SSIMP` IMP interface (`%LHVEC`,
  `src/system/lhdh.defs`, not read yet). docs/emulator.md.
- **Answer not found.** The instruction times of the KS10, for the virtual time of the emulator.
  docs/emulator.md.
- **Answer not found.** The time of a reference to nonexistent memory. NXM ERR sets three
  bus cycles after COM/ADR (EK-OKS10-TM-002 5.6.5; M8621-0-DPMC rev
  D, E4). The microcode-level model adds no time for the wait.
- **Implementation-defined.** Device times in `emu/ks10-machine`, all estimates in instructions at 1 µs each:
  the RP06 seek and transfer time (`rh11.rs`, DELAY = 100), the DZ11
  character time (`dz11.rs`, 9600 baud), the CH11 transmission time
  (`ch11.rs`), the TM03 command time and rewind time (`tm03.rs`, DELAY =
  100, REWIND_TIME = 1000).
- **Answer not found.** What the TM03 does at the end of the recorded data of a tape image. A
  real drive reads blank tape. `emu/ks10-machine` stops the command with
  "operation incomplete" (OPI) and attention.
- **Answer not found.** How the TM03 packs a record in reverse order if its frame count is not
  a multiple of the frames of a word. `emu/ks10-machine` reverses the order of
  the words only. ITS does not use Read Reverse (`nmtape.30`).
- **Answer not found.** The serial number of the TM03 and the TM03 check character. `emu/ks10-machine`
  gives 0.
- **Answer not found.** The parity bits of the UBA paging RAM as read (%UPPAR, %UPPVL), the
  serial number of the RP06, the CH11 check word, and how the IMP
  interface counts its address register in 32-bit mode. `emu/ks10-machine` gives
  0 or leaves the register as it is.
- **Answer not found.** The time of the SHA-256 of a disk image on the board (`--record`,
  `--replay`). On the x86-64 host, 317 MB take about 0.25 s
  **[measured]**; the Cortex-A9 has no SHA instructions.
- **Answer not found.** The line speed of the CTY, and the time of the 8080 for one character.
  `emu/ks10-machine/src/console.rs` uses 1000 instructions (an estimate: 9600 baud
  at 1 µs for each instruction). The 8080 program sets the mode of its
  UART from switches but gives no baud rate (`hcore.m80`, `PWRUP`). For
  each output character it waits for the transmitter before it clears
  word 33 (`CTYPCHR`) **[its]**.
- **Answer not found.** Whether the bus RESET (register 100 ← 200)
  clears the UBA paging RAM. The 8080 writes map entry 1 for the boot,
  then resets the bus again before the start (`dr.m80`, `DSKSEQ`;
  `cmds.m80`, `BT.GO`, `MRCMD`) **[its]**. The high-level console resets
  once, before the boot ([`console-interface.md`](console-interface.md)).

## Instructions that the differential run does not check

The differential run (docs/emulator.md, "Differential runs") does not
compare these with a reference emulator, or the two reference emulators
disagree:

- **Implementation-defined.** Floating point instructions with unnormalized operands, and FAD with an
  exponent difference of 61 or more. The KS10 microcode handles them in
  its own way (`flt.5`). SIMH follows the KL10; KLH10 is not yet run
  with such operands. The cases `float-unnormalized` and
  `fad-large-exponent-difference` of [`impldef.md`](impldef.md) hold
  them; the level of each is open.

## The microcode-level model

[`microcode-model.md`](microcode-model.md), "The data path", gives what
the manual and the field definitions give. These connections need the
schematics (MP00540):

- **Answer not found.** Backplane inputs of the microsequencer that the CRA sheets do not
  trace: NICOND inputs 0, 4 and 6 (EA1, EF1, EK1), the MUL dispatch input
  FK2, and the page fail dispatch inputs EU1, FE2, FL2, FS2. The ITS
  microcode depends on EA1, EF1 and EK1 being inactive, and does not read
  the others **[measured]** 2026-11-27 (the ITS listing 262): the NICOND
  block (60–77) has its words at 61, 62, 63, 65, 67, 71, 72, 73, 75 and
  77, and the assembler put other words at 60, 64, 66, 70, 74 and 76;
  every `DISP/MUL` word has `J` bit 10 set; no word uses `DISP/PAGE
  FAIL`. The model takes EA1, EF1 and EK1 as inactive.
- **Answer not found.** The I/O cycles of the model in microcycles
  ([`microcode-model.md`](microcode-model.md), "I/O cycles"): how long a
  Unibus adapter asserts I/O BUSY before its data cycle, and after a
  write. The model uses 4 microcycles **[estimate]**. This value and the
  other timing estimates here are verification targets: the DEC
  diagnostics and the co-simulation with `rtl/` can check them, and a
  comparison with `emu/ks10-core-soft` cannot
  ([`microcode-model.md`](microcode-model.md), "Time and the boot of
  ITS").
- **Answer not found.** What a Unibus adapter does on a read or a write of an address with no
  device. The model gives no I/O BUSY and no data cycle, so IOWAIT ends
  in the page fail "nonexistent I/O register" (inout, IOW5; ksdefs,
  `%PFNXI`), as `emu/ks10-core-soft` does.
- **Answer not found.** The correction code bits of the memory status register after a
  correct word. MMC4 E104 loads the syndrome at each data strobe. The
  emulator gives 0 **[estimate]**: the syndrome of a correct word.
- **Answer not found.** The time from the command/address of a status read to its data cycle.
  The microcode-level model takes 2 microcycles, as for the console.
- **Answer not found.** Which registers DP RESET and CRAM RESET clear (hcore.m80, MRINT). DP
  RESET clears the APR flags (DPMA E23, E87; DPMB). The model also sets
  the CRAM address to 0 and leaves the rest; the power-up sequence sets
  the state that the microcode uses.
- **Answer not found.** The inputs 0 of DBM bits 33–35 (DPM2 E187, E165, E188, pin 12) have
  no connection on the drawing. The model takes them as 1, as an open
  TTL input **[estimate]**. The ITS microcode masks these bits in each
  use of `DBM/APR FLAGS` (inout, `RDAPR`, `APRSZ`, `APRSO`; `WRAPR` and
  `TOCK` load only bits 24–31) **[microcode]**.
- **Answer not found.** The tick at which CSL5 E32 takes PAGE FAIL DELAY. VMA JUST LOADED is 1
  for one tick after a word that loads VMA (DPMC E10). The model takes it
  as 0 in the word that completes the cycle, so a page fail takes place
  in that word also if the word before it loaded VMA **[estimate]**. The
  same holds for CACHE HIT (DPM7 E41), which needs VMA JUST LOADED 0.
- **Implementation-defined.** The content of the page table and the cache directory at power on,
  and its parity (DPM6, DPM7, DPM9). The model keeps no parity bits and
  marks an entry that nothing wrote. The ITS boot loader sweeps both
  tables with `WRUBR` at 1004, its fifth instruction **[measured]**
  2026-11-27 (`emu/ks10-core-uop/tests/its_boot.rs`).
- **Answer not found.** MP00540 has DPE9 at rev B and rev A1, and CRA2
  at rev A1 and rev B. The code cites both revisions of each
  ([`architecture.md`](architecture.md), "Target"): DPE9 rev A1 for the
  flags in `emu/ks10-core-uop`, rev B for PCU in `emu/ks10-core-soft`.
  Whether the cited gates differ between the revisions is not checked.
  The code cites CRA1 rev A1; the CRA1 sheet on page 172 of MP00540 is
  rev B, and a sheet of rev A1 was not looked for.
- **Answer not found.** The D input of CSL5 E53 (FS (1)). While `MULTI SHIFT` and FE SIGN are
  both 1, the DPE/M clock needs FS (1) (CSL5 rev D, E52, E45). The model
  clocks the data path at each repeat, as the manual describes
  (EK-OKS10-TM-002 5.4.1, bit 79).

## CSL board

The model of the board in `emu/ks10-csl` needs these answers. Until
then, the model takes each value as a parameter, or stops.

- **Answer not found.** The level of the bits 4 to 7 of the ports 302 and
  303, and of port 100 bit 5. No element of the CSL drawings drives the
  first, and the multiplexer input of the second has no connection
  (MP00540 CSLA rev C, E157, E112). The model reads the bits 4 to 7 as 1,
  and the input of port 100 bit 5 as high.
- **Answer not found.** The items of [`csl-registers.md`](csl-registers.md) that
  the drawings do not settle (U1 to U9 there).
- **Answer not found.** The baud rate of a board. DIP switches set it
  (MP00540 CSL2 rev E, E10 and the 34702 E4; CSL9 rev D, E11), and the
  8080 cannot read it. The model takes the time of one character as a
  parameter.

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
