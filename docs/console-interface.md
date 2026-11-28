# The console and the KS10

The system in daily use has a high-level console: the operator console of
`neoks10` with the console words and the boot commands of
`emu/ks10-machine` (`console.rs`, `boot.rs`). It does not run the DEC 8080
program. A model of the 8080 that runs the real program is for tests
only, with the microcode-level model ([`architecture.md`](architecture.md)).

Design decision 2026-11-28: the high-level console keeps the simpler
behaviour of the rows below. It serves `ks10-core-soft` and the
pipelined core. An operating system is not likely to rely on the exact
behaviour of the 8080 program. The exact behaviour is a goal only with
`ks10-core-uop`: there, the model of the 8080 runs the real program.

This document lists each effect of the console that the KS10 side can
see. For each effect it gives the source in the 8080 program and what the
high-level console does:

- **same**: the same effect;
- **simpler**: a smaller effect; the row says how;
- **not done**: no effect; the row says why.

The rows "simpler" and "not done" are deliberate differences.

The 8080 sources are `src/kshack/*.m80` of the ITS tree, version "KS10
CSL.V4.2" (`hcore.m80`, `INIMS`) **[its]**. Addresses and values are
octal. Bits have DEC numbers.

## Boot from the disk

| Effect | 8080 source | High-level console |
|---|---|---|
| Home block: cylinder 0, sector 1, else sector 10; word 0 must be `SIXBIT /HOM/` | `cmds.m80`, `FILEINIT`, `CHKHOM` | same (`boot.rs`, `boot_disk`) |
| Word 103 of the home block points to the front-end directory page | `cmds.m80`, `GOODPK` | same |
| The first page of the monitor pre-boot (directory entry 4) goes to 1000 | `cmds.m80`, `LBINT`, `FILEINIT`; pointer format in `BLKRDR` | same |
| The diagnostic pre-boot (entry 6), `B2` (entry 12) and `FI` (entry 22) | `cmds.m80`, `BTCMD`, `B2CMD`, `FICMD` | not done: the boot of ITS uses entry 4 only |
| The channel program: UBA map word 763001 of the adapter takes 140001; the RH11 and RP06 registers in the order of the list | `dr.m80`, `DSKSEQ`, `QXFR` | same |
| The microcode: directory entry 2, three memory words for each control RAM word, loaded through registers 204, 205, 210 and 103–113 | `cmds.m80`, `MICROP`, `DMEM2CR`, `MEM2CR`, `CADWR`, `WFUNC` | not done: `ks10-core-soft` has no control RAM, and `ks10-core-uop` takes the microcode that `spec/micro.py` assembles. `tests/ram_file.rs` reads the RAM file in the way of the 8080 |
| Master reset: in each `FILEINIT`, and again in `BT.GO` (`SM1`) after the transfers. The last bus RESET comes after the last transfer | `cmds.m80`, `FILEINIT`, `BT.GO`, `SM1`, `MRCMD`; `hcore.m80`, `MRINT` | simpler: one reset, before the transfers. If the bus RESET clears the UBA paging RAM, the 8080 leaves map entry 1 clear and the high-level console leaves it set ([`unknowns.md`](unknowns.md)) |
| Word 36: the RH11 base with the adapter number (1,,776700) | `cmds.m80`, `PASSSRC`; ksdefs, `8RHBAS` | same |
| Word 37: the unit | `cmds.m80`, `PASSSRC`; ksdefs, `8QNUM` | same |
| Word 40: `DEN.SLV`, 0,,002000 by default, also for a disk boot | `cmds.m80`, `PASSSRC`; ksdefs, `8BOOTP` | simpler: 0 for a disk boot. Only MTBOOT reads word 40 (`mtboot.177`, `MTHAK`) |

## Boot from the tape

| Effect | 8080 source | High-level console |
|---|---|---|
| The channel program: UBA map word takes 040001; TC takes `DEN.SLV`; rewind | `dr.m80`, `MTASEQ`, `QTXFR` | same |
| The first file (the microcode) goes into the control RAM | `cmds.m80`, `MTCMD`, `MEM2CR` | not done, as for the disk |
| Space forward over the first file; the frame count error is expected and cleared | `cmds.m80`, `MBINT`; `dr.m80`, `MTARST` | same |
| One record of the second file goes to 1000 | `cmds.m80`, `MBINT` | same |
| Words 36, 37 and 40: 3,,772440, the formatter, `DEN.SLV` | `cmds.m80`, `PASSSRC` | same for formatter 0 (`boot.rs`, `boot_tape`) |

## Start, halt and continue

| Effect | 8080 source | High-level console |
|---|---|---|
| Start of the microcode: registers 204 ← 5 and 100 ← 200, word 0 ← 777777,,777777, the control RAM address, the clock, then a wait for HALT LOOP | `cmds.m80`, `SMCMD`, `SM1`; `hcore.m80`, `MRINT` | `ks10-core-uop`: same, at the level of the signals (`console_reset`). `ks10-core-soft`: not done; it has no microcode |
| The enables after a boot: register 100 ← 174 (cache, 1 ms clock, parity), 205 ← 20 (TRAP EN) | `cmds.m80`, `BT.GO1`; `condef.m80`, `DEFLTE`, `TRPDEF` | `ks10-core-uop`: same, without the parity enables |
| ST: words 31, 32 and 33 ← 0 | `cmds.m80`, `STINT` | not done: a fresh memory has 0 there. After a boot over a running system, old words stay |
| ST: JRST to the address from the console instruction register, with I/O DATA CYCLE; EXECUTE and CONTINUE; then RUN and CONTINUE | `cmds.m80`, `EXINT`, `COCMD` | `ks10-core-uop`: same (`console_start`). `ks10-core-soft`: simpler; it sets the PC |
| ST and CO: word 31 takes the start code in bits 28–35 (1 keep-alive, 2 forced reload, 4 power fail, 10 automatic boot), TRAP EN in bit 12, maintenance mode in bit 13, and the enables in bits 6–11 | `cmds.m80`, `COCMD`, `SETUSE`, `WRD31` | not done: ITS and DSKDMP do not read word 31; the diagnostic monitor reads bit 13 only (KLAD sources, `SUBSMT.MAC`, `$80MM`) **[manual]** |
| A halt: the 8080 reads words 0 and 1 and prints `%HLTD/` with the halt code and `PC/` with the PC. It does not read the halt status block | `cmds.m80`, `HLTCM`, `SMVER`, `PCCOM` | simpler: `neoks10` prints the halt code and the PC in its own form |
| HA: register 212 ← 0; the microcode writes halt code 2, the PC and the halt status block | `cmds.m80`, `HACMD` | not done: `cpu halt` stops between two instructions and writes no word |
| CO: word 31, then register 212 ← 5 | `cmds.m80`, `COCMD` | simpler: `cpu continue` goes on; no word 31 |
| Automatic boot at power on: the 8080 reads the memory status register, waits for a typed character or the BOOT switch, then boots | `hcore.m80`, `PWRCHK`, `SYSUP`, `BTAUT`, `PWR.FAIL` | not done: `neoks10` boots by command or by its configuration. The code and the comment of `SYSUP` give opposite senses of the power fail bit |

## Memory and I/O

| Effect | 8080 source | High-level console |
|---|---|---|
| EM and DM: a physical memory cycle on the bus, also while the processor runs | `cmds.m80`, `EMCMD`, `DMINT` | simpler: `mem deposit` needs `cpu halt`; `mem examine` also reads virtual addresses |
| EI and DI: an I/O cycle on the bus | `cmds.m80`, `EICMD`, `DICMD` | not done at the operator console |
| EX, SM, MR and the control RAM commands | `cmds.m80` | not done at the operator console. `ks10-core-uop` models SM and ST for the boot |

## The console terminal

| Effect | 8080 source | High-level console |
|---|---|---|
| Input: word 32 ← 400 + the character, then register 116 ← 1, twice. The 8080 does not read word 32 first: a character that the KS10 did not take is overwritten | `hcore.m80`, `TENCHR`, `INTCH` | simpler: the next character comes only after the KS10 took the previous one (word 32 is 0) and after `INPUT_GAP` instructions (`console.rs`, `poll`). See [`emulator.md`](emulator.md), "Input without loss" |
| Input: the 8080 clears bit 7 (parity) of the character | `hcore.m80`, `INTCH` | not done: the byte goes as it is. A terminal sends ASCII; `cty send \xHH` can set bit 7 |
| Output: the KS10 writes 400 + the character into word 33 and sets APR flag 2.2. The 8080 looks at word 35 first, then word 33; byte 20–27 must be 1. It waits for its UART, writes 0 into word 33 and sets APR flag 1.5 | `hcore.m80`, `LIVE10`, `CHRRDY`, `TTOCOM`, `POKE10`; ksdefs, `80INT` | same words and flags; simpler time: the output comes `OUTPUT_DELAY` instructions later (`console.rs`) |
| User mode: the 8080 serves word 33 and checks word 31 only in user mode. A halt or Ctrl-\ ends user mode | `hcore.m80`, `N4`, `LIVE10`, `USER`, `END.USR`, `CLRUSE` | simpler: no user mode; word 33 is served at all times. Ctrl-\ opens the operator prompt |

## Keep-alive and KLINIK

| Effect | 8080 source | High-level console |
|---|---|---|
| Keep-alive: if bit 5 of word 31 is set and bits 20–27 do not change for 35 (decimal) checks, the 8080 prints `?KA`, halts, executes the instruction in word 71 and continues with start code 1 | `hcore.m80`, `CHKKA`, `DIEING`; `condef.m80`, `KPAINI`, `KATIMX` | not done: ITS does not use word 31 (`8KALIV` has no reference in `src/system` or `src/kshack` other than its definition in ksdefs) |
| Forced reload: bit 4 of word 31 makes the 8080 print `?FRC`, load the monitor pre-boot and start it at 1000 | `hcore.m80`, `FRELOAD` | not done, for the same reason |
| KLINIK: word 34 takes input (400 + character) and the codes 001000 (active) and 001400 (carrier loss); word 35 has output | `hcore.m80`, `CHRRDY`, `NULKL`; `subrtn.m80`, `KL3435`, `WRD34` | simpler: mode 0 only; a word 35 is discarded and word 34 is never written |
| Word 30: SH writes 0,,776700 into it; ITS then enters DDT | `cmds.m80`, `SHCMD`; `its.1652`, `CLKBKS` | not done |

## Comparison

This is the plan of the comparison (design decision 2026-11-28). The
model of the 8080 does not exist yet.

- The model of the 8080 runs the DEC console program with
  `ks10-core-uop`. It records each effect on the KS10 side, for each
  operation:
  - the memory words and their addresses;
  - the order of the writes to the console registers;
  - the boot parameters in words 31 and 36 to 40;
  - each CTY word with the APR flag that goes with it;
  - the reads after a halt, and the printed text.
- The operations: power on, load of the microcode, boot from the disk,
  boot from the tape, halt, continue, examine, deposit, CTY input, CTY
  output, keep-alive.
- The recorded effects go into `tests/` as data. They are the expected
  values. They do not come from the high-level console.
- A test runs each operation on the high-level console and compares its
  effects with the recorded data. It leaves out the effects of the rows
  "simpler" and "not done" above, and only these.
- Open before the model: which program to run. The ITS tree has the
  sources of version 4.2; `references/` has version 5.2 with its listing
  ([`emulator.md`](emulator.md), "Documents and tapes"). The ITS build
  uses the program that the machine has, so the sources of the ITS tree
  are the first choice if they assemble.
- With `ks10-core-uop`, the model of the 8080 gives each effect of the
  rows "simpler" and "not done" too. A test of the model compares them
  with the recorded data.
