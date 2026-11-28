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
| User mode: the 8080 serves word 33 and checks word 31 only in user mode. A halt or Ctrl-\ ends user mode | `hcore.m80`, `N4`, `LIVE10`, `USER`, `END.USR`, `CLRUSE` | simpler: no user mode; word 33 is served at all times. Ctrl-\ opens the operator prompt, as Ctrl-] does ([`emulator.md`](emulator.md), decision 3) |

## Keep-alive and KLINIK

| Effect | 8080 source | High-level console |
|---|---|---|
| Keep-alive: if bit 5 of word 31 is set and bits 20–27 do not change for 35 (decimal) checks, the 8080 prints `?KA`, halts, executes the instruction in word 71 and continues with start code 1 | `hcore.m80`, `CHKKA`, `DIEING`; `condef.m80`, `KPAINI`, `KATIMX` | not done: ITS does not use word 31 (`8KALIV` has no reference in `src/system` or `src/kshack` other than its definition in ksdefs) |
| Forced reload: bit 4 of word 31 makes the 8080 print `?FRC`, load the monitor pre-boot and start it at 1000 | `hcore.m80`, `FRELOAD` | not done, for the same reason |
| KLINIK: word 34 takes input (400 + character) and the codes 001000 (active) and 001400 (carrier loss); word 35 has output | `hcore.m80`, `CHRRDY`, `NULKL`; `subrtn.m80`, `KL3435`, `WRD34` | simpler: mode 0 only; a word 35 is discarded and word 34 is never written |
| Word 30: SH writes 0,,776700 into it; ITS then enters DDT | `cmds.m80`, `SHCMD`; `its.1652`, `CLKBKS` | not done |

## Comparison

This is the plan of the comparison (design decision 2026-11-28). The
model of the board (`emu/ks10-csl`) runs the program through the boot
to the prompt of the disk program, with `ks10-core-uop` as the KS10.

### The program

Design decision 2026-11-28: the model runs version 5.2 first, from its ROM
images. Version 4.2 of the ITS tree follows, when an assembler for its
sources exists.

- Version 5.2: `KS10_CSL_V5.2.zip` has the four 2 KiB images of the board.
  The tape `ks10_8080_microcode` has the listing `KNS10.PNT`, the sources
  in one file (`KS.MAS`; `SEP.TEC` splits it into the files of version
  4.2), the build file `M80.CTL` and the notes `V5-2.NOT` (both files in
  `third-party.lock`; `make csl` extracts them).
- **[measured]** 2026-11-28, `make csl` (`emu/ks10-csl/tests/rom.rs`):
  the 7016 bytes of the listing are the bytes of the images at these
  addresses (octal): E81 at 0, E73 at 4000, E58 at 10000, E18 at 14000.
  The RAM starts at 20000 (`KNS10.PNT`, `PWRUP`: `LXI SP,RAMST+^O2000`).
- Version 4.2: the sources are in the syntax of the DEC cross assembler
  (`CROSS.EXE` on the tape, a TOPS-10 program). An assembler of this
  project has an exact check: the sources of version 5.2 must give the
  four images.
- The rows of this document cite version 4.2. `V5-2.NOT` lists changes
  in `CHRRDY`, `TTOCOM` and the KLINIK code. Before a recorded effect
  becomes expected data, its row is checked against version 5.2; a
  difference goes into [`discrepancies.md`](discrepancies.md).

### The model of the board

Design decision 2026-11-28: the 8080 of the model is the crate
`x80-core`. It has no dependencies and no `unsafe` code. It gives the
number of states of each instruction. On an interrupt it reads the
instruction from the bus, so a model of the board can give an `RST`. An opcode
without documentation stops it with an error, not with a panic. The
crate cites Intel 98-153B, but it is not a source of this project:
statements about the 8080 and the 8251 cite 98-153B (`third-party.lock`).

- **[manual]** EK-OKS10-TM-002 5.8.1 (p. 5-70 to 5-75): the board has an
  8080A with an 8224 clock generator and an 8228 bus driver, two 8251
  USARTs, 1 KiB of RAM (two 2114) and 8 KiB of ROM (four 2716). The
  crystal is 14.746 MHz, and the 8080A clock is that frequency divided by
  9: 1.638 MHz, so a state is 610 ns. Table 5-8 gives the ROM chips at 0,
  4000, 10000 and 14000 (octal), the RAM at 20000 to 21777, and the
  decode of the register reads and writes.
- The model has the ROM, the RAM and the two USARTs at ports 200 to 203.
  The USARTs stay in reset until the first write to port 100 or 101
  (MP00540 CSL5 rev D, E145; CSL9 rev D, E65).
- The KS10 side (`emu/ks10-csl/src/ks10.rs`) has the bits of each register
  from the drawings ([`csl-registers.md`](csl-registers.md)): MP00540
  CSL3 rev D, CSL4 rev C, CSL5 rev D, CSL6 rev C, CSL7 and CSL8 rev B,
  CSLA rev C. CSL5 RESET clears the registers 204, 205, 210 and RUN,
  EXECUTE and CONTINUE; a write while it holds is lost. At power-up,
  `MRINT` writes 204 in the reset. The multiplexers of the reads
  invert, so the program complements each read (`CMA`). The program
  writes the KS10 word into the 4x4 memories (ports 102 to 115): an odd
  port writes the word of the command/address cycle, an even port the
  word of the data cycle.
- It reaches the KS10 through the trait `Ks10`, at the level of the bus
  functions of EK-OKS10-TM-002 Table 5-10. `Detached` is a board without
  a KS10. The microcode-level model implements the trait: its memory,
  its Unibus adapters and devices, its device time, and its inputs RUN,
  EXECUTE, CONTINUE, TRAP EN, CACHE EN and 1 MSEC EN.
- A bus function completes at the write to register 210. The model has
  the memory read and write, the I/O register read and write, and the
  examine bus. The other functions, SINGLE CLK alone and the maintenance
  clock stop the run. If the KS10 clock runs, the function completes
  between two processor cycles.
- Register 206 bit 0 is CLK RUN (CSL5; RESETB clears it). While it is
  set, the KS10 runs with the 8080: after each 8080 instruction, the
  processor runs for the same time. An 8080 state is 9 / 14.7456 MHz
  (EK-OKS10-TM-002 5.8.1). A T clock is 150 ns, and a processor cycle is
  2 + T T clocks (5.2.1). So a state is 9375/2304 T clocks. The two
  clocks have separate crystals (MP00540 CSL1, CSL2), and the model runs
  both at the nominal rate.
- At the start of the clock, a clear microinstruction register runs one
  cycle with the zero word. Its dispatch selects the diagnostic address,
  so the microcode starts there (EK-OKS10-TM-002 5.4.6; SM1 loads 0).
- The CRAM load goes through the CRA (EK-OKS10-TM-002 5.3.8): 210/144
  latches bits 24 to 35 of the COM/ADR word (CRA R CLK). Register 204
  gives CRA/M RESET, DP RESET, CRAM ADR LOAD and CRAM WRITE, and register
  205 the diagnostic function. The model sends the edges of 204 to the
  KS10. It has no STACK RESET and no diagnostic read.
- INT 10 (register 116 bit 0) sets CSL4 E171. The request reaches the
  KS10 through E170 and E163, which clock on CSL5 ENABLE (0) H
  (MP00540 CSL4 rev C). The model holds it while the KS10 clock is
  stopped ([`unknowns.md`](unknowns.md)), and gives it to the KS10 when
  the clock runs: it presets APR flag 31.
- The doorbell of the KS10 (APR flag 25, DPMB CSL INTERRUPT) presets
  CSL4 10 INT (E161), which the program reads on port 301 bit 7.
- The data word of ports 102 to 112 is the console instruction register
  (CSL4 E90, E106), and register 114 bit 1 gives I/O DATA CYCLE.
- **Design decision, 2026-11-28: device time with the KS10 clock
  stopped.** At power-up the 8080 starts the disk transfers of the boot
  and waits for them (`DSKSEQ`, `WAIT`), and the KS10 clock is stopped.
  The devices of `ks10-core-uop` count time in instructions, so they need
  time from the board while the clock is stopped. Before each access to a
  KS10 port, the board runs the devices to the 8080 time of the stopped
  clock (`Ks10::idle_to`). While the clock runs, the instructions give
  the time, as without the board. One millisecond is
  1638.4 states, the 8080 clock (EK-OKS10-TM-002 5.8.1), and
  `INSTRUCTIONS_PER_MS` units of device time. The rule adds no new
  estimate. A transfer that never completed would send the program to
  its timeout path, and its error would look like a result.
- The USART model has the asynchronous mode, the transmitter, the
  receiver and the status (98-153B, p. 5-136 to 5-142). A received
  character sets RxRDY, and the read of the data clears it. If RxRDY is
  still set, the new character replaces the old one and sets the overrun
  flag. The line has no parity or framing errors.
- A character typed on the CTY arrives one character time after the line
  is free (`Csl::type_cty`). The 8080 program deposits each character in
  word 32 and does not wait for the KS10 to take the one before
  (`hcore.m80`, `TENCHR`). So a burst of input faster than the KS10
  takes it loses characters, as on the board.
- The tests type with a typist (`emu/ks10-csl/src/typist.rs`), with
  the rules of the high-level console (`Script`). An answer starts after
  its prompt, when the CTY output has been quiet for `QUIET`, a time of
  the KS10. In an answer, a character goes when the line is free and the
  KS10 has cleared word 32; the line gives the speed. These waits are a
  choice of the tests. The board has no flow control.
- A character in the transmit buffer starts when the character before
  ends (98-153B p. 5-139). At the end of a run, the model moves both
  USARTs to the time of the board, so the CTY output is current also if
  the program does not read the status.
- Each write to a port of the KS10 goes into a log. A port, an address or
  a function without a model stops the run with its reason. The 8080
  does not decode A14 and A15, and only A13 enables the RAM (CSL2 rev E,
  E36, E65): the ROM and the RAM repeat in the address space. The program
  does not use those addresses, and the model stops at them.
- At an interrupt, the 8228 gives `RST 7`: its INTA pin is on +12 V
  through 1 kOhm (CSL2 rev E, E50, R32; Intel 98-153B p. 5-8). INT is
  RxRDY of either USART (CSL2 E23). The 8080 samples INT before each
  instruction.
- Time counts in 8080 states. The switches of port 300 (E173), the
  carrier inputs and the time of one character are parameters of the
  model ([`unknowns.md`](unknowns.md), "CSL board").
- **[measured]** 2026-11-28, `make csl` (`emu/ks10-csl/tests/power_up.rs`):
  from power-up, the program prints `KS10 CSL.V5.2` on the CTY and makes
  the 22 writes that the listing gives, from `PWRUP` to `EMCONT`. The ROM
  checksum of `PWRUP` passes. With one changed byte in chip E73, the
  program prints `?CHK2` first. Without a KS10, the run stops at the
  memory read of `EMCONT`: with the switches 0110 and a character time
  of 0, after 450916 states.
- **[measured]** 2026-11-28, `make csl` (`emu/ks10-csl/tests/ks10.rs`): with
  `ks10-core-uop`, `EM1` reads memory location 0 and `EBCMD` examines the
  bus. `SYSUP` waits, prints `BT AUTO`, and `BTAUT` starts the boot,
  35396146 states (21.6 s) after power-up.
- **[measured]** 2026-11-28, `make csl` without a disk: `DSKSEQ` writes
  the UBA page map and the unit number, and `TWAIT 400` finds DPR clear
  in RPDS. The program prints `?BT 001062` (`A.BTERR`, with the address
  017062 of the failed command) and the prompt. Then the null job reads
  the REMOTE DIAG switches as PROTECT, hangs up the KLINIK line, writes
  `CARRLOSS` into word 34 and sets INT 10 (`HANGUP`, `WRD34`).
- **[measured]** 2026-11-28, `make csl KS10_ITS_DISK=<rp0.dsk>` with the
  disk of the ITS build: the program reads block 1 of cylinder 0, finds
  `HOM` at 1000, reads the page of pointers from word 1103 and the first
  page of the microcode from word 1002. With an empty control RAM in
  `ks10-core-uop`, `MEM2CRAM` loads all 2048 words. The result equals
  the control RAM that `spec/micro.py` assembles. `LBINT` reads the
  pointer to the monitor pre-boot (word 1004). `SM1` deposits all ones in
  location 0 and loads the CRAM address 0. `CSCMD` starts the KS10 clock
  45026145 states (27.5 s) after power-up. The microcode reaches the halt
  loop, `PASSSRC` deposits 36, 37 and 40, and `STINT` starts the pre-boot
  at 1000. The disk program prints its prompt `DSKDMP` through the 8080
  program after 49000253 states (29.9 s). The text after `BT AUTO` is
  the text of the high-level console on the same disk. The boot writes
  to the disk, so the test boots from a copy.
- **[measured]** 2026-11-28, `make csl KS10_ITS_DISK=<rp0.dsk>`, with a
  character time of 1707 states (9600 baud, a choice of the test): the
  typist types `its` at the prompt. It types ESC G after the CRLF that
  the disk program prints when it reads commands again after the load
  (`dskdmp`, `READ`) **[its]**. The 8080 program passes the input to the
  disk program, and ITS prints `IN OPERATION` after 101910387 states
  (62.2 s) and 12000354 instructions. The text equals the text of the high-level console,
  except for the place of the bell before `IN OPERATION`. ITS sends ^G
  without a cursor motion (`ts3tty`, `TYOBEL`) **[its]**, and the place
  changes with the character time: with 1707 and 3000 states the space
  comes first, and with 17067 states the bell comes first.
- **[measured]** 2026-11-28, the same test: if the characters arrive
  without waits, characters are lost. One character for each 1707
  states gave the echo `is` for `its`. ESC that arrived during the load
  of ITS, 10000 units of KS10 time after the echo of CR, was lost
  without an echo. On the high-level console, ESC in word 32 before the
  load starts is read after the load. The cause of the loss is not known
  ([`unknowns.md`](unknowns.md)).
- **[measured]** 2026-11-28, `emu/ks10-csl/tests/states.rs`: for each of
  the 244 opcodes of the 8080, `x80-core` takes the number of states of
  98-153B p. 4-15, with the condition true and false. The other 12
  opcodes stop it. The
  ROM checksum of `PWRUP` passes. With one changed byte in chip E73, the
  program prints `?CHK2` first.

### The ports of the 8080

**[measured]** 2026-11-28, a read of `KNS10.PNT` with a script: the
`IN` and `OUT` instructions of version 5.2 use these ports (octal, names
of `CONDEF.M80`). The model of the board needs a source for each one: the
CSL drawings of MP00540 or EK-OKS10-TM-002. EK-OKS10-TM-002 Table 5-9
gives the control flip-flops of each register write, and Table 5-10 the
flip-flops of each bus operation. The CSL drawings give the bits.

| Ports | IN | OUT |
|---|---|---|
| 0 to 3 | bus data `D2835` to `D0411` | |
| 100 to 103 | `SMPAR`, `REFERR`, `R.BUSY`, `D0003` | `RESET`, `LIGHTS`/`DTR`, bus data `W2835` to `W0003` and addresses `A2835` to `A0003` |
| 104 to 116 | | bus data and addresses, `DTARB`, `BUSARB`, `INT2KS` |
| 200 to 203 | UARTs: `CTYDAT`, `CTYCTL`, `REMDAT`, `REMCTL` | the same |
| 204 to 212 | | `CRMCTL`, `DIAG`, `CLKCTL`, `BUSCTL`, `CPUCTL` |
| 300 to 303 | `RUNFP`/`TTYSW`, `SMSTS`/`BOOTSW`, `CARRIER`, 303 | |

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
- With `ks10-core-uop`, the model of the 8080 gives each effect of the
  rows "simpler" and "not done" too. A test of the model compares them
  with the recorded data.
