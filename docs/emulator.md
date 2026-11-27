# Emulator design

This document is the design of the emulator program in `emu/`. It covers
step 1 of the plan in `architecture.md` (the software processor boots ITS
on a host computer),
step 2 (the same program on the PS of the board) and the parts of step 5
that the program supplies (the devices for the core in the PL).

The document has three parts:

- The machine: what the program must emulate. Each statement has an
  evidence mark (AGENTS.md).
- The design: the decisions of this document.
- Design decisions: the decisions with their dates, and the items that
  are still open.

## Sources

The ITS sources are at the pin of `3rdparty/its` (commit 83534677).
References name the file and the label in that tree. The reference
emulators are the submodules of the same tree:

| Emulator | Path | Commit | Use |
|---|---|---|---|
| sims `pdp10-ks` (Richard Cornwell) | `tools/sims` | 9556eca0 | The ITS build of step 0 uses it (`Makefile`, variable `KS10`) |
| KLH10 | `tools/klh10` | 6d733f2a | Second reference |

KS10FPGA is the submodule `3rdparty/KS10FPGA` (commit 5090e7a1, shallow).
It is a KS10 in Verilog with the data path of the DEC schematics: ten
am2901 slices, the SCAD, the dispatches and the microsequencer
(`fpga/ks10/cpu`). It is the reference for the data path of the
microcode-level model (decision 17). Its console processor is a program
on an ARM, not the 8080 program.

Step 0 built ITS with `make EMULATOR=pdp10-ks` on the host on 2026-11-26.
The machine configuration of that build is `DB` in
`build/pdp10-ks/config.202` of the ITS tree.

## The machine

### Configuration

The configuration `DB` has these parts **[its]**
(`build/pdp10-ks/config.202`, `IFE MCOND DB`):

- A KS10 processor (`KS10P`) with 512 Ki words of memory (`TSYSM==512.`).
- One RH11 with one disk drive. The build asks for the drive type: RP06,
  RP07, RM03 or RM80. The step 0 build used RP06.
- One RH11 with one TM03 magnetic tape formatter (`TM03S`, `NMTCS==1`).
- One DZ11 with 8 lines (`DZ11NB==1`, `NDZTYS==8.`).
- The 8080 console terminal (`NKSTYS==1`).
- A CH11 Chaosnet interface (`CH11P`, address `MYCHAD==177002`).
- An IMP interface of the type "simulated simple IMP" (`SSIMP`). This type
  exists only in emulators.

### Unibus devices

The KS10 has Unibus adapter 1 (`UBAQ`, the disk) and Unibus adapter 3
(`UBAI`, the other devices) **[its]** (`src/system/ksdefs.194`, `UBAQ`,
`UBAI`). An I/O address is the adapter number in the left half and the
Unibus address in the right half.

| Device | Adapter | First register | Vector | Source **[its]** |
|---|---|---|---|---|
| RH11 for the disk | 1 | 776700 | 254 | `rh11.defs48`, `%HRCS1`, `PHVEC`; `its.1652`, `UBAQIT` |
| RH11 for the tape | 3 | 772440 | 224 | `tm03s.defs5`, `%TMBAS`, `%TMVEC`; `its.1652`, `UBAIIT` |
| DZ11 | 3 | 760010 | 340 receive, 344 transmit | `config.202`, `DZ0BA`; `its.1652`, `UBAIIT` |
| CH11 | 3 | 764140 | 270, and 230 | `ch11.defs1`, `CAICSR`; `its.1652`, `UBAIIT` |
| IMP (`SSIMP`) | 3 | not read yet | `%LHVEC` and `%LHVEC`+4 | `its.1652`, `UBAIIT` |

The bit layouts of the registers are in the files of the table.
`spec/devices.toml` holds them, with the CH11 and the IMP; `docs/devices.md`
shows them. `make check` compares each entry with the ITS sources.

Each adapter has these registers **[its]** (`ksdefs.194`, `UBAPAG`,
`UBASTA`, `UBAMNT`):

- A paging RAM of 64 entries at 763000 to 763077. It maps Unibus
  addresses to KS10 memory for DMA.
- A status register at 763100. Among other fields, it holds the PI level
  for the high request lines (BR7, BR6) and for the low request lines (BR5,
  BR4).
- A maintenance register at 763101.

Interrupts: word `EPTUIT`+n of the EPT points to the interrupt table of
adapter n **[its]** (`ksdefs.194`, `EPTUIT`). ITS fills the entry at
vector/4 of that table with a `JSR` to the handler **[its]** (`its.1652`,
macro `UBINT`). The processor asks which adapters request on the PI level
(WRU cycle), reads the vector of the first one (vector cycle), and runs
the instruction at table + vector/4 in the executive address space
**[microcode]** (`inout.50`, PI10, VECINT). A zero table pointer halts
with code 102.

An I/O reference to an address without a device gives a page fail with
bit `%PFNXI` **[its]** (`ksdefs.194`, page fail word). The memory status
register is at I/O address 100000 **[its]** (`ksdefs.194`, `KSECCS`).

### Processor registers outside the Unibus

`ksdefs.194` defines the instructions and the formats; the emulator follows
the microcode where the two differ (AGENTS.md, source order):

- APR flags, PI system, `APRID` (`RDAPR`, `WRAPR`, `RDPI`, `WRPI`,
  `APRID`).
- Paging: `WREBR`, `WRUBR`, the ITS registers DBR1 to DBR4, `LPMR`,
  `SPM`, the page fail word, the page table entry.
- Time: `RDTIM` reads a 71-bit count that increments at 4.1 MHz.
  `WRINT` sets the interval; at its end, the APR flag "interval done"
  (bit 1.6) is set **[its]** (`ksdefs.194`, `RDTIM`, `RDINT`).
- Halt: the processor stores the halt code in location 0 and the PC in
  location 1, then the halt status block at 500 **[its]** (`ksdefs.194`,
  `RDHSB`, halt codes).
- The ITS I/O instructions `IORDI` to `IOWRQ` (710 to 715) and `IORDBI`
  to `IOWRBQ` (720 to 725), and `BLTBU` and `BLTUB` (716, 717) **[its]**
  (`ksdefs.194`).

The instruction set is in `spec/instructions.toml`, and
`docs/instructions.md` shows it. It holds the dispatch ROM of the ITS
microcode for each opcode, the dispatch on the AC field, and the names
from MIDAS and KSDEFS. `make check` compares it with the ITS sources.

### The 8080 console

The KS10 has no console switches. An 8080 front end gives a command line
**[manual]** (`doc/kshack/consol.24`). The 8080 and ITS share words in low
memory **[its]** (`ksdefs.194`, 8080 communication area):

| Word | Name | Use |
|---|---|---|
| 30 | `8SWIT0` | Simulated switch 0, set by the `SH` command |
| 31 | `8KALIV` | Keep-alive and status |
| 32 | `8CTYIN` | Console terminal input |
| 33 | `8CTYOT` | Console terminal output |
| 34, 35 | `8KLKIN`, `8KLKOT` | KLINIK line |
| 36 | `8RHBAS` | RH11 base address for the boot |
| 37 | `8QNUM` | Unit number for the boot |
| 40 | `8BOOTP` | Magnetic tape boot format and slave number |

The console terminal works this way **[its]**:

- Output: ITS writes the character with bit 400 set into `8CTYOT`, then
  sets the APR flag that interrupts the 8080 (`ts3tty.402`, `TTKSDO`).
  The 8080 takes the character and clears the word. Then it sets the APR
  flag "8080 interrupting the KS" (`hcore.m80`, CHRRDY, TTOCOM, POKE10).
- The flag "KS interrupting the 8080" is a pulse: the microcode clears it
  just after it sets it (`inout.50`, WRAPR2).
- At its interrupt, the 8080 looks at `8KLKOT` first. Without a KLINIK
  line, the console is in mode 0, and the 8080 discards a KLINIK character
  (`hcore.m80`, NULKL; `subrtn.m80`, SETM0).
- Input: the 8080 puts the character into `8CTYIN` and sets the APR flag
  "8080 interrupting the KS" (bit 1.5). ITS takes the character, clears the
  word, and clears the flag (`its.1652`, `CLKBKS`).
- Switch 0: if `8SWIT0` is not 0, ITS enters DDT, then clears the word
  (`its.1652`, `CLKBKS`).

The boot commands of the 8080 **[its]** (`src/kshack/cmds.m80`):

- `BT` (`BTCMD`): read the home block of the disk, then the front-end
  directory. Load the microcode file, then load the boot program file to
  address 1000. Write the RH11 base and the unit into words 36 to 40
  (`PASSSRC`), and start at 1000.
- `MT` (`MTCMD`): read the first file of the tape (the microcode). Skip to
  the second file, load it to 1000, write words 36 to 40, and start at
  1000.
- `MB` (`MBCMD`, `MBINT`): the same without the microcode. Rewind, space
  forward over the first file, read one record of the second file to
  1000. The space stops at the tape mark with a frame count error; `MBINT`
  expects it and clears it (`MTARST`).

The images of step 0 agree with this **[measured]** (2026-11-26, a read of
the files with a Python script):

- `rp0.dsk` has `SIXBIT /HOM/` in word 0 of sectors 1 and 8 (`ksdefs.194`:
  sector 1, else sector 10 octal). Word 103 of the home block points to the
  front-end directory. Its entries 2 (microcode) and 4 (boot program) are
  not 0.
- `salv.tape` and `dskdmp.tape`: the first file has 6144 words (6 blocks,
  the size of the microcode file in `ksdefs.194`, `88RAM`). The second file
  is the program.

The emulator implements the microcode behaviour in software, so it skips
the microcode file.

### Device model in `emu/ks10-machine`

- The adapters: the paging RAM, the status register with the PI levels
  of the two request groups, Unibus init, and DMA through the map.
- RH11 and RP06: the registers, the commands of `rh11.defs48`, attention
  and the interrupt with IE, DMA of 18-bit words. A command ends after a
  fixed delay in virtual time; a data transfer moves its data at that end.
- RH11 and TM03 with up to 8 TU45 transports: the registers of
  `tm03s.defs5`, the commands that `nmtape.30` and the 8080 use, rewind
  with PIP, space with the frame count, read and write in the core dump
  and industry compatible formats. Read reverse moves the words of the
  record in reverse order.
- DZ11: the silo, the transmit scanner, the line parameters, both
  interrupts. CH11: transmit, receive, loop back. IMP: the registers
  only, with "IMP not ready". The host side of the DZ11 lines is in
  `neoks10` ([DZ11 lines](#dz11-lines)). The host side of the Chaosnet
  and the IMP comes later.
- The 8080 `BT` command runs the channel command list `DSKSEQ`
  (`dr.m80`) on the emulated RH11. The `MB` command runs `MTASEQ` and
  `QTXFR` on the emulated TM03 (`emu/ks10-machine/src/boot.rs`). `MT` gives the
  same result in the emulator, because the emulator does not load
  microcode.

**[measured]** 2026-11-27 (`cargo test --release -p ks10-core-soft --test its` with
`KS10_ITS_DISK` naming the step 0 `rp0.dsk`): `BT`, then `its` and ESC G
at the DSKDMP prompt, start ITS. The Salvager runs, ITS prints `DB ITS
1652 IN OPERATION`, the system job logs in, and a login on the console
lists `SYS;`. This takes about 140 million steps.

**[measured]** 2026-11-27 (`cargo test --release -p ks10-core-soft --test tape`
with the same `KS10_ITS_DISK`; the tapes come from the same directory):

- `MB` on transport 2 with `dskdmp.tape` prints `ITS MTBOOT.176`. ESC G
  starts DSKDMP, and DSKDMP starts ITS from the disk.
- DUMP `LIST` under ITS reads all 16 files of `minsys.tape` to `E-O-T`.
  The SIMH build wrote this tape.
- DUMP writes a copy of `SYS; TS PDSET` to an empty tape. `ICHECK` finds
  no difference to the disk, and `LIST` shows the file.
- A Python script compared the data words of that file with the file
  `SYSBIN; PDSET BIN` on `output.tape` that SIMH wrote at the end of the
  build. All 1439 data words agree; only the file headers differ.

### Image formats in step 0

- Disk: 8 bytes for each word, little-endian, bits 63:36 are 0. Sectors
  of 128 words. `rp0.dsk` has 317132800 bytes: 815 cylinders, 19 tracks,
  20 sectors **[measured]** (2026-11-26, the file size and the home block
  words above) and **[its]** (`rp06.defs1`, `NCYLS`, `XCYLS`, `NHEDS`,
  `NSECS`, `SECBLK`).
- Tape: the SIMH tape image format (a 32-bit little-endian length before
  and after each record, 0 for a tape mark). Each word takes 5 bytes in
  the "core dump" format (`tools/dasm/README` of the ITS tree). The records
  of `salv.tape` have 2560 bytes, 512 words **[measured]** (2026-11-26).

## Design

### Rules of AGENTS.md and how the design meets them

| Rule | Design |
|---|---|
| A guest error is not a crash | All guest input goes through checked paths. A device error is a device status or a console message. The emulator has no `unwrap` on guest data |
| Deterministic; all time input through one layer | Virtual time is the count of completed instructions. Each input from the host (console, DZ11 line, network, wall clock) goes through the host layer. The layer stamps it with the instruction count at which the guest sees it. `--record` writes the stamped inputs; `--replay` feeds them at the same counts |
| One `Machine` owns all state | `Machine` holds the processor, the memory, the 8080 console state, the two adapters and their devices as fields. Devices get a context (memory, interrupt lines, time) for each call; they keep no references |
| A word is its own type | A type `Word` (36 bits in a `u64`) in a crate `pdp10`, with the octal text and the field helpers |
| `unsafe` only in `plio::mmio` | The memory of step 5 is a `plio` mapping. All other code is safe |
| Each dependency is a decision | `toml` and `serde` read the configuration file, and `reedline` is the line editor of the operator console (see [Design decisions](#design-decisions)) |
| Decoders and constants from `spec/` | The instruction table and a device register table in `spec/` generate the Rust constants, as `spec/regmap.toml` does now |
| C interface for co-simulation and the PL bridge | The `ks10-core-soft` library exports C functions: create a machine, run one instruction, read the instruction record |
| Device I/O with `pread` and `pwrite` | A disk transfer reads or writes the image file directly into the guest memory, when the adapter map gives contiguous memory |
| Disk image with 8 bytes for each word | The format of step 0 (above). Images of step 0 work without a conversion |
| Tape image in the format of the ITS tools | The SIMH tape image format with core dump words (above) |

### Program structure

The crates of `emu/`:

| Crate | Contents |
|---|---|
| `pdp10` | `Word`, octal text, instruction decode generated from `spec/instructions.*` |
| `ks10-machine` | The machine around the processor: the Unibus adapters and the devices, the device registers from `spec/devices.*`, the 8080 console words, the 8080 boot commands, the SAV loader, the size of the memory |
| `ks10-core-soft` | `Machine`: the software processor (interpreter), memory, paging, APR, PI, timers, the host layer. The C interface |
| `ks10-core-uop` | The microcode-level model ([`microcode-model.md`](microcode-model.md)) |
| `neoks10` (the binary `neoks10`) | Command line, configuration, operator console, control socket, scripts, image tools |
| `plio` | Access to the PL (exists) |

Memory has two kinds, behind one interface:

- Host memory (`Vec<u64>`), for step 1 and step 2. On the board, an
  uncached read of the reserved region takes about 133 ns
  (`mem-path-experiment.md`, B8). The software processor thus uses cached
  memory.
- The reserved region of the board, for step 5. The core in the PL reads
  it through the HP port. The devices on the PS use `pread` and `pwrite`
  into it.

Device register tables: a new file `spec/devices.toml` holds the register
offsets and the bit fields of the RH11, RP06, TM03, DZ11, CH11 and the
adapter registers, with their sources. A generator writes the Rust
constants and `docs/devices.md`.

Processor back ends: `soft` is the interpreter. `pl` (step 5) runs the core
in the PL through the register map, with the same devices. A run with `pl`
records its inputs with the instruction count of the core. A replay of
that record on `soft` then repeats the run in software.

### Time

- Virtual time advances by a fixed amount for each instruction. The amount
  is a configuration value; its default is an **[estimate]**: 1 µs. Basis:
  none yet. The instruction times of the KS10 technical manual will replace
  it. The amount sets the rate of `RDTIM` and of the interval timer.
- Device delays (seek, tape motion, line speed of the DZ11) are in virtual
  time. They do not change the results of the guest program, only its
  timing.
- `--pace` selects how the run follows the wall clock. In `fast`, `idle`
  and `realtime`, the wait does not change the result of a run, only its
  speed. While the machine thread waits, a command from the terminal or
  the socket comes in at once.
- `normal` is the mode for daily use. The ticks of the 1 ms clock
  (`Machine::tick`) come from the wall clock, as in SIMH. The guest works
  at full speed; while ITS is idle, the host sleeps to the next tick, and
  the idle passes count at 1 for each `time_per_instruction_ns`. The run
  depends on the host and cannot be repeated, so `normal` refuses
  `--record` and `--replay`.
- If the host stops (a sleep of the host, a stop of the process), the
  ticks of the lost time come back. They come one interval of the timer
  at a time, so that ITS gets each interval interrupt. **[measured]**
  2026-11-27: after a `SIGSTOP` of 10 s during the boot, the time base
  was back at the wall clock within 1 s (11028 ms after 11.00 s).

| `--pace` | Wait | Guest clock | Host CPU | Repeatable |
|---|---|---|---|---|
| `normal` | To the next tick, while ITS is idle | The wall clock | Low while ITS is idle | No |
| `fast` | None | Far ahead of the wall clock, most of all while ITS is idle | One core | Yes |
| `idle` | The time of the idle passes | Ahead while ITS works, at wall speed while it is idle | Low while ITS is idle | Yes |
| `realtime` | Until the wall clock reaches the virtual time | At wall speed; work is limited to 1 instruction for each `time_per_instruction_ns` | Low | Yes |

- `run` with a terminal, a control socket or `[dz11]` uses `normal`, if
  it has no `--script` and no `--replay`; with `--record` it uses `idle`. Other
  runs use `fast`.
- SIMH sleeps while the guest is idle and runs at full speed while it
  works **[emulator]**. Its clock comes from the wall clock (`kx10_cpu.c`:
  `sim_activate_after`, `sim_rtcn_calb`). `normal` does the same. `idle`
  is the repeatable form: its guest clock is virtual and runs ahead while
  ITS works.
- The wall clock (for the date of ITS) is a host input of the host layer.

**The idle loop.** If no job can run, ITS runs the null job **[its]**
(its.1652, NULJOB). The null job puts `AOJA 0,17` into AC 17 and jumps
there in user mode. The loop adds 1 to AC 0 and jumps to itself until an
interrupt comes. The `DB` configuration of the `pdp10-ks` build does not
set `KLH10P`, so its null job has no `IOWRI` to the KLH10 HOST device
**[its]** (build/pdp10-ks/config.202, `DB` and the defaults).

`Machine::step_idle` does one step. If that step was a pass of the loop,
it also does the next passes at once, up to the first instruction count
at which the timer, the 8080 or a device can do something. A pass only
adds 1 to the AC and to the count, so the state after the passes is the
same as after the same number of steps. It is not a guess about the
guest, and the virtual time does not change.

- A pass that sets a flag (the AC at 377777,,777777 or 777777,,777777)
  is a step.
- After a step that printed on the CTY, no passes follow. Thus a script
  and `expect` see the text at the same count as with steps.
- `Machine::step` has no such shortcut. It stays the model for the RTL.
- `--no-idle` does steps only. It is for a comparison.

Other emulators **[emulator]**: SIMH (`pdp10-ks`, commit 9556eca0,
`kx10_cpu.c`) takes an `AOJ` in the ACs as idle and waits for the next
clock tick of the host. KLH10 (commit 6d733f2a, `dvhost.c`) has a HOST
device: an ITS with `KLH10P` writes to it in the null job, and the
emulator waits. Both take the wall clock into the virtual time;
`neoks10` does not.

**[measured]** 2026-11-27, x86-64 host, release build:

- `cpu halt` and `cpu show` on an idle ITS after the login: PC 17 in user
  mode, AC 17 `344000,,000017` (`AOJA 0,17`), AC 0 `254200,,031500`, and
  `254200,,000000` (`JRST 4,`) in ACs 1 to 16.
- The ITS test `idle_skip_boots_its_the_same` (`emu/ks10-core-soft/tests/idle.rs`)
  boots ITS twice and lists `SYS;`. 118406612 of 138001000 instructions
  were idle passes. The state of the two machines was the same at each
  check, and the two disk images were the same at the end.
- The boot script of [Scripts](#scripts), with `--pace fast`: 6.13 s wall
  time with `--no-idle`, 1.01 s with the idle loop. The record files of
  the two runs and of the run before this change were the same, and so
  was the CTY output.
- The same script without `quit`, with 30 s of idle ITS after the
  listing, with `--pace realtime`: 167.5 s wall time in both runs (the virtual
  time). The CPU time in user mode was 8.52 s with `--no-idle` and 1.50 s
  with the idle loop, 5.1 % and 0.9 % of one core. Method: GNU `time`.
- The same script with `--pace idle`: 146.1 s wall time for 167.5 s of
  virtual time, 1.43 s CPU time in user mode.
- The script with 1000 s of idle ITS after the listing, `--pace fast`:
  2.57 s wall time and 2.52 s CPU time for 1137.5 s of virtual time.
  The 1000 s of idle time took about 1.6 s: the guest clock ran about
  600 times faster than the wall clock, on one full core.
- `--pace normal` through the control socket (`ctl expect`, `ctl send`):
  `IN OPERATION` after 1.9 s, the `DELRNM` prompt after 123.7 s. Then 20.03
  s of wall time with an idle ITS: the time base advanced 20036 ms, the
  instruction count 19.45 million, and the CPU time in user mode was
  0.16 s (0.8 % of one core). Method: `cpu show` (`clock_ms`) and
  `/proc/PID/stat`.
- Most of the boot time of ITS is a wait: the system job tries to get
  the time from the network (`NETIME`), and no host answers. The wait is
  idle time. With `--pace normal`, `idle` or `realtime`, the boot thus
  takes about 2 minutes of wall time.

### Command line

The program `neoks10` has subcommands.

```
neoks10 run [CONFIG] [--control PATH | --no-control] [--record FILE]
            [--replay FILE] [--script FILE]
            [--pace normal|fast|idle|realtime]
            [--no-idle] [--trace FILE] [--cpu soft|pl]
neoks10 console [--control PATH]
neoks10 ctl [--control PATH] [--json] COMMAND...
neoks10 help [--markdown]
neoks10 image create rp06 FILE
neoks10 image info FILE
neoks10 image convert --from klh10 IN OUT
neoks10 tape list FILE
neoks10 trace diff A B
```

- `run` starts the machine. Its terminal is the console terminal (CTY) of
  the KS10 and the operator console. `CONFIG` names the configuration file.
- `console` opens the operator console from another terminal, through the
  control socket of a running `run`.
- `ctl` sends one command to the control socket and prints the reply. It
  is for scripts and coding agents.
- `--trace` writes one record for each completed instruction, in the
  format that the co-simulation compares.
- `--script` reads operator commands from a file (see [Scripts](#scripts)).
- `help --markdown` writes `docs/console.md`; `make gen` runs it.
- `image convert` exists for KLH10 images; the step 0 images need no
  conversion.
- `run` ends with `quit`. With `--no-control` and no terminal on stdin,
  it also ends when the processor stops.

The first version (2026-11-27) has `run`, `console`, `ctl` and `help`.
`--trace`, `--cpu` and the subcommands `image`, `tape` and `trace` come
later.
`docs/console.md` lists the commands that exist.

### Operator console

**The machine runs while the operator works.** Commands do not stop the
guest unless the operator gives `halt`. On a multi-user system, a tape
change thus does not stop the other users. A command that needs a
stopped processor (`deposit`, `step`, the registers of the processor)
tells the operator to give `halt` first.

**Two modes on the terminal of `run`:**

- CTY mode: the terminal is in raw mode. Each key goes to the CTY, and
  the CTY output goes to the terminal.
- Command mode: the escape character Ctrl-\ (034) opens the prompt. The
  guest goes on. CTY output waits in a buffer and appears when the prompt
  closes. An empty line or `cty` returns to CTY mode.

The line editor is `reedline` (decision 5). Its terminal
layer `crossterm` also sets the raw mode of CTY mode, so `neoks10` has
no `unsafe` code.

**Syntax: a noun, a unit, a verb, then arguments.**

```
tape 0 mount minsys.tape --ro      tape 0 rewind      tape 0 unmount
disk 0 attach rp0.dsk              boot disk 0        boot tape 2
dz line 7 listen 127.0.0.1:10011   mem examine 1000..1017
mem examine exec:774000            mem deposit 1000 254000,,1000
cpu show      cpu halt      cpu continue      cpu step 10
show devices  record mark TEXT     quit
```

There are no SIMH names and no abbreviations; completion replaces them.
Numbers for guest addresses and words are octal (AGENTS.md).

**One command table.** A table in Rust gives each command, the type of
each argument, a one-line help text and the source of the completion. The
parser, the completion, `help` and `docs/console.md` come from this
table. `docs/console.md` is generated and in git, like the files of
`make gen`.

**Completion and hints.** Because the arguments have types, the
completion knows the candidates at the cursor:

- Nouns and verbs, each with its help text in the completion menu.
- The units that exist, for example the tape transports of the
  configuration, with the attached image.
- File paths, filtered by the type of the argument (`.tape`, `.dsk`).
- Enumerated values, for example `soft` and `pl`.
- Addresses: octal numbers, and the address spaces `exec:`, `user:` and
  `phys:`.

A hint after the cursor shows the rest of the most recent matching line
of the history. An unknown word gives the nearest known word: `unknown
noun "tpe"; did you mean "tape"?`.

**The control socket.** `run` listens on a Unix domain socket, by default
`neoks10.sock` in the directory of the configuration file
(`--control PATH` changes it). The socket file has mode 0600. There is no
TCP control port. The protocol has one command for each line, in the
syntax above. Each reply is one line of JSON:
`{"ok":true,"result":...,"text":"..."}` or `{"ok":false,"error":"..."}`.
`result` is for programs, `text` is for a person. `serde_json` writes
and reads the lines (decision 9). Some commands exist for
clients that do not own the terminal:

| Command | Function |
|---|---|
| `cty send TEXT`, `send TEXT` | Type TEXT on the CTY (escapes `\r`, `\e`, `\xHH`) |
| `cty read [--from N]` | The CTY output from byte N, and the next N |
| `expect TEXT [--steps N]` | Wait until TEXT is in the CTY output, at most N instructions |

`neoks10 console` and `neoks10 ctl` use the socket. Several clients can
connect at the same time. Each connection has its own position in the
CTY output: an `expect` looks at the output after the last match of that
connection. `ctl --json` prints the reply line as it comes.

A Unix socket path has at most about 107 bytes. `neoks10` uses the path
relative to the current directory if that is shorter.

**Threads and determinism.** One thread owns `Machine` and runs the
instructions. The terminal thread and the socket threads send commands
to it through `std::sync::mpsc` channels and get the replies the same
way. The machine thread does a command between two instructions. A
command that changes the guest (a key, a tape mount) is an input of the
host layer: it gets the instruction count, and `--record` writes it. A
replay thus gives the same run without the terminal or the socket.

- The record file has one line of JSON for each input. Image paths are
  relative to the directory of the record file, so that the record has
  no host directory and works at another place. The last line is the
  `quit`, where a replay ends.
- A replay needs the images in the state at the start of the recorded
  run, so it must run on fresh copies. The record has the SHA-256 of
  each image at its `attach`. If the image of a replay has another
  SHA-256, the replay ends with exit status 1.
- `--record` and `--replay` read each image once to get its SHA-256,
  in the machine thread. A `disk attach` during a recorded run thus
  stops the machine for that time. **[measured]** 2026-11-27, x86-64
  host: the boot script with `--record` took 1.26 s, without it 1.01 s
  (317 MB image). The SHA-256 agreed with `sha256sum`.
- An input from the terminal or the socket gets the count at which the
  machine thread takes it. A second run of the same client can thus give
  other counts; only the replay of its record repeats the run.

**[measured]** 2026-11-27: two runs of a script that boots ITS, logs in
and lists `SYS;` gave identical record files (7 inputs, `quit` at
instruction 137519396) and identical CTY output. A replay of the record
on a fresh copy of the disk gave the same CTY output.

### Scripts

A script file has operator commands, one for each line. A line that
starts with `#` is a comment. Two commands are for scripts:

- `expect TEXT [--steps N]`: wait for TEXT in the CTY output. N is a
  number of instructions, not a wall-clock time (default 1000000000).
  If the text does not come, the script stops with an error.
- `send TEXT`: type TEXT on the CTY.

A script starts with `run --script FILE`, or with `script run FILE` at
the prompt or on the socket while the machine runs. `script show` gives
its step, and `script stop` ends it. One script runs at a time. A script
that starts later looks only at the CTY output after its start.

The machine thread runs the script, so its timing depends only on the
guest:

- While an `expect` waits, the thread looks at the CTY output after each
  instruction that prints. The match has the count of that instruction.
- A `send` comes 100000 instructions (`ks10_machine::console::ANSWER_DELAY`)
  after the previous step of the script. The DEC diagnostic monitor
  loses characters that come while it prints (`ks10-machine`, `console.rs`).
- Other commands run at once, at the count of the previous step.
- If a step of a `run --script` script fails, `run` ends with exit
  status 1 and names the file and the line. If a step of a `script run`
  script fails, only the script stops, with a message: the other users
  of the system go on.
- `cpu halt` ends the `expect` commands that wait: the instruction count
  does not move while the processor is stopped.

The ITS build of `its/` and `make its-boot` use scripts. The waits count
instructions, so a script gives the same result on a slow host.

The ITS build drives the emulator with these operations
(`build/pdp10-ks/build.tcl` and `build/ks10/include.tcl` of the ITS tree):
start with an initial command file, a prompt, attach a tape image to a
unit, boot from a tape unit, boot from the disk, continue, quit, and the
escape character. The `its/` directory of this repository holds the
scripts that the ITS build needs under `build/<emulator>/` (AGENTS.md).
They use the commands above.

### DZ11 lines

Each of the 8 lines of the DZ11 has its own TCP listener. A user who
connects to the same port again thus gets the same ITS terminal.

- `[dz11] listen = "HOST:PORT"` in the configuration gives the address
  of line 0. Line k listens on PORT + k. Without `[dz11]`, no line
  listens.
- `dz line N listen HOST:PORT` gives one line a listener while the
  machine runs; `dz line N close` stops it. `dz line N disconnect` ends
  the connection. `dz show` lists the lines.
- A line takes one connection at a time. A second client gets
  "line N is busy" and the connection closes.
- The protocol is Telnet (RFC 854) in character mode. At the start,
  `neoks10` sends WILL ECHO, WILL SUPPRESS-GO-AHEAD and DO
  SUPPRESS-GO-AHEAD; ITS echoes. Received commands are removed. CR LF
  and CR NUL become CR. A data byte 255 goes out as IAC IAC.
- `neoks10` holds at most about 64 KiB of output for a client that does
  not read (`HOLD_HIGH`; the socket buffer of the host comes on top).
  Above it, the line holds its output: the transmit scanner passes over
  the line, as if its transmitter were busy, and ITS waits. At 32 KiB
  (`HOLD_LOW`) the line goes on. No output is lost. The hold and its end
  are inputs of the host layer (`dz_hold` in a record).
- In `--pace normal`, a line sends about 1 character for each 1000
  instructions (`CHAR_TIME`), at the full speed of the guest: about
  20000 characters a second on the x86-64 host, not 960 as at 9600 baud
  (see [To do](#to-do)). **[estimate]** Basis: the ITS boot with
  `--no-idle` did 138 million instructions in 6.1 s.
- A connection sets the carrier of the line, and its end clears it.
  ITS does not read the modem status of the DZ11 **[its]** (its.1652
  and ts3tty.402 have no reference to `%DZRMS`). For ITS the lines are
  thus hardwired terminals: after a disconnection, the job of the line
  stays, and the next connection gets it.
- The machine thread decides about each connection. A connection, its
  input and its end are inputs of the host layer, so `--record` writes
  them and a replay repeats them. A replay takes no connections.
- A run that takes requests (a terminal, the control socket or
  `[dz11]`) ends only with `quit`.
- The default addresses are on the loopback interface (decision 4). A
  listener on another interface needs an explicit address in the
  configuration file or in a console command.

**[measured]** 2026-11-27, with `[dz11] listen = "127.0.0.1:10004"`:

- A client on line 0 typed Ctrl-Z, `:login` and `:listf sys;`, and got
  the listing. ITS sent 7-bit characters. After a new connection to the
  same port, a CR gave `Flushed` and the DDT prompt of the same job.
- The `telnet` program of the host, in a pseudo terminal: each typed
  character came back once, and Enter gave one new line. The session
  logged in and copied a file (`:copy`).
- That run with `--pace fast --record`, and its replay on a fresh copy
  of the disk: the same CTY output and the same disk image.

### Configuration file

The configuration file names the machine and the attachments. An example
in TOML:

```toml
[machine]
memory_kwords = 512
time_per_instruction_ns = 1000

[disk.0]
type = "rp06"
image = "rp0.dsk"

[tape.0]
image = "minsys.tape"
read_only = true

[dz11]
listen = "127.0.0.1:10004"

[chaosnet]
enabled = false
```

The defaults keep all network listeners on the loopback address. A
listener on a LAN needs an explicit address.

### Tests

- Each instruction: random differential runs against the reference
  emulator; see [Differential runs](#differential-runs).
- The DEC KS10 diagnostics (`make diag`); see [Diagnostics](#diagnostics).
- `make its-boot KS10_ITS_DISK=PATH`: PATH is `rp0.dsk` of the ITS build
  (`pdp10-ks`); `dskdmp.tape` and `minsys.tape` are in the same
  directory. The target works on copies in `build/its-boot/`.
  1. The tests of `emu/ks10-core-soft` that need the images: the boot and the
     listing of `SYS;`, the idle loop, the tapes.
  2. `neoks10` runs `tests/its-boot/boot.script` (boot, login, `:listf
     sys;`) with `--record`. The output must have `FREE BLOCKS`.
  3. A replay of the record on a fresh copy must give the same CTY
     output.
  The record is not in git: it belongs to the image of one ITS build
  (the record has its SHA-256).
  **[measured]** 2026-11-27: 25 s on the x86-64 host. A script with an
  `expect` that fails makes the target fail.
- `make its-build`: build ITS from source with the scripts of `its/`.

### Differential runs

The reference emulators are submodules of `3rdparty/its`, at the
commits of the pin files in `oracle/`:

| Emulator | Pin | Patch | Program | Variable |
|---|---|---|---|---|
| SIMH `pdp10-ks` | `sims.pin` | `sims-trace.patch` | `build/oracle/sims/BIN/pdp10-ks` | `KS10_ORACLE` |
| KLH10 `kn10-ks-its` | `klh10.pin` | `klh10-trace.patch` | `build/oracle/klh10/tmp/bld-ks-its/kn10-ks-its` | `KS10_ORACLE_KLH10` |

`make oracle` copies each one into `build/oracle/`, applies its patch and
builds it; KLH10 gets the configuration of the ITS tree. A patch adds one
record for each instruction fetch outside an interrupt or a UUO cycle: the
PC word and the 16 ACs. `KS10_TRACE` names the file. `KS10_TRACE_FROM`
is the PC of the first record, because SIMH runs a loop at 100 at
start-up to measure its speed. KLH10 takes its debug loop (`apr_walk`)
when `KS10_TRACE` is set.

`emu/ks10-core-soft/tests/oracle.rs` runs the cases against each emulator that a
variable names. `make check` runs 2000 cases against each emulator that
`make oracle` built, and says SKIPPED for a missing one.

1. A seed gives one case: one instruction at 1000, two HALTs, the 16 ACs,
   and the words at E and E + 1. E is a direct address outside the code.
2. Both emulators run the case from executive mode, with flags 0, no
   paging and no interrupts. A reference emulator runs 500 cases in one
   process. E and E + 1 are not in the halt status block (500 to 520),
   which the HALT of `emu/ks10-core-soft` writes.
3. The test compares the records before the instruction and before the
   HALT, the record count, and the words at E and E + 1.
4. A case that found a difference goes into `tests/oracle/cases` as
   words, so that it does not depend on the generator.

The instructions come from `spec/` (`pdp10::instructions`). These are not
in the cases:

- Jumps, XCT, EXTEND, the stack and byte instructions, BLT, the
  instructions of the previous context and the I/O instructions.
- For SIMH (`skip_simh`): CIRC, which SIMH does not have on the KS10;
  FDV and its forms, because SIMH fails a DSKCB test and differs in bit
  35 for many normalized operands; ASH and ASHC left shifts of many
  places, the shift count -256, IMUL products of magnitude 2^35 + 1 to
  2^36 - 1 with a negative sign, IDIV of -2^35 by 1 or -1, DIV of -2^70
  by 0.
- For KLH10 (`skip_klh10`): HLLE of a positive source with a bit of 1 to
  3 set; DIV with a high word of the magnitude equal to the divisor.
- Each of these has its entry in `docs/discrepancies.md`, and `skip`
  decides from the inputs only. The test counts the cases that no named
  emulator compares; with both emulators, the count must be 0.
- `tests/oracle/cases` also has directed cases for operands that the
  generator does not reach: ASHC of -1 by 70 and 71 places, and the
  shift count -256.
- Unnormalized floating point operands. The operands of FAD, FSB, FMP,
  FIX, FSC and of the double forms are normalized, with exponents that
  differ by 60 or less (`docs/unknowns.md`).

Paging, traps and interrupts are not compared. The diagnostics, the ITS
boot and the unit tests check them.

**[measured]** 2026-11-27, x86-64 host, 100000 seeds: SIMH 97257 cases
and KLH10 99745 cases, 0 differences, 242 s for both (KLH10 takes 19 s).
2000 cases take 5 s. `KS10_ORACLE_NOSKIP` also runs the cases of `skip`:
then 807 of 100001 cases differ from SIMH, 744 of them FDV. The runs
found two faults of `emu/ks10-core-soft`: IMUL of -2^35 by -2^35 gave 0, not
400000,,0 (`muldiv.rs`; SIMH), and ASH of a negative number by more than
35 places set no Overflow (`shift.rs`; KLH10). Without the Overflow flag
of an addition, 11 of 1958 cases differ from SIMH and 13 of 1995 from
KLH10.

### Diagnostics

The diagnostics come from `ks_diag_gs.tap.bz2` (third-party.lock). They
stay in `references/` and in build output, never in git: their files
carry the license text of DEC.

**How a diagnostic runs.** On a real KS10, the diagnostic monitor `SMMON`
loads the programs from a TOPS-10 or TOPS-20 disk pack, or `SMMAG` loads
them from the diagnostic magnetic tape `DSXLA` **[manual]** (`SMMON.TXT`
and `DSXLA.INP` on the tapes). This project has neither medium. The
programs can also run alone **[manual]** (`DSKAA.TXT`, sections 3.1 and
3.2; `DSKCA.TXT`, section 3.1):

- `DSKAA` to `DSKAH` need only the console. They start at 30000. An
  error is a `HALT`, and word 30047 (`PASCNT`) counts the passes. The
  program runs until it is stopped.
- The other programs, `DSKAI` and later, need the subroutine program
  `SUBSM` in memory. It prints the messages on the console terminal
  through the 8080 words. `SUBSM.SAV` ends with `JRST 2000`; the program
  is loaded after it and starts at 30000.
- At the start, `SUBSM` asks `TTY SWITCH CONTROL ? - 0,S OR Y <CR> -`.
  The answer `0` selects all switches 0, which is the full test
  (`DSKCA.TXT`, sections 3.2 and 3.3). The answer `Y` asks for the left
  and right half of the switches in octal (`DSKAM.TXT`, section 3.2).
- With all switches 0, `SUBSM` enables the pager with `WREBR`
  **[measured]** (2026-11-27, `emu/ks10-core-soft` diag example: `WREBR` at 3753
  with E bit 22 set). It expects the KI style paging of the DEC
  microcode, which the ITS microcode does not have. The switch `INHPAG`,
  left half 000100, inhibits paging and traps (`DSKAM.TXT`, section 4.0).
  So the answers are `Y`, `000100` and `0`.
- After each pass, `SUBSM` prints `END PASS n.` (decimal) **[measured]**
  (2026-11-27, `DSKAI` to `DSKAM` in `emu/ks10-core-soft`).
- `SMCPU.CMD` lists the processor diagnostics in the order in which the
  monitor runs them.

**`make diag`.** It needs the tape from `make references` and the
submodule `tools/tapeutils` of `3rdparty/its` (`git -C 3rdparty/its
submodule update --init tools/tapeutils`). The files stay in
`build/diag/`.

1. The target builds `t10backup` of tapeutils and extracts the files of
   the tape. A `.SAV` file is a list of blocks (`-count,,address-1`, then
   the words) and ends with `JRST start`.
2. `emu/ks10-core-soft/tests/diag.rs` reads the run file `SMCPU.CMD`. Each line has
   the name, the passes, the right half of the switches and the
   iterations in octal **[manual]** (`SMMON.TXT`, section 5.0). Each
   program runs for the iterations of its line, in its own thread.
3. `DSKAA` to `DSKAH` run alone from 30000. Such a program passes if it
   does not halt and `PASCNT` reaches the iterations.
4. The other programs run under `SUBSM`, from 30000. The answers to the
   switch questions are `Y`, the left half (`000100`, `INHPAG`; `000110`
   for `DSKBA`) and the right half of the run file. `DSKBA` needs
   `OPRSEL` (000010) too: without it, the program limits memory to 32K,
   and with `INHPAG` it gets too little memory. Each answer comes 100
   ms of virtual time after its prompt: a character that comes while
   `SUBSM` prints is read as a control character and lost **[measured]**
   (2026-11-27, trace of the CTY output loop of `SUBSM`). Such a program
   passes if it does not halt, prints `END PASS n.` for n = the
   iterations, and prints no `ERROR`.
5. A program of the run file without a decision in `diag.rs` fails the
   test. The programs that do not apply are listed below.

**Reference runs [emulator]** (sims `pdp10-ks` 9556eca0, `set cpu its`,
`load -c` of the extracted files, 2026-11-26):

- `DSKAA` ran for 120 s of host time without a halt. `PASCNT` was
  14162641 (octal) at the end.
- `DSKCA` with `SUBSM` printed its title, took the answer `0` and ran for
  60 s without a halt or an error message. The end of a pass was not seen
  in that time.
- The title of `DSKCA` gave `CPU#=4097`, `MCV=416` and `MCO=570`. These
  values of `APRID` differ from the ITS microcode (`docs/discrepancies.md`).

**`make diag` [measured]** (2026-11-27, x86-64 host, 3.7 s for all):

- `DSKAA` to `DSKAG` reached 64 passes (`SMCPU.CMD`: iterations 100
  octal) in 40000 to 130000 instructions, `DSKAH` 8 passes in 230000.
- `DSKAI` to `DSKAM` and `DSKCA` to `DSKCE` printed `END PASS 64.` after
  2.5 to 2.7 million instructions, `DSKCF` after 5.8 million.
- `DSKCG` ran tests 1 to 8 (MUUO traps, CMPS, MOVS, CVTDB, CVTBD, EDIT)
  with no error message, and stopped at test 9 (see below).
- `DSKBA` printed `END PASS 1.` after 85 million instructions.
- A test of the test: without the flag `AROV` of an overflow, `DSKAD`
  halted at 032756 and `DSKCF` printed an error message.

**Which programs apply.** The programs are for the DEC microcode:

- `DSKEA` and `DSKEC` test the paging of the DEC microcode, which the ITS
  microcode does not have.
- `DSKEB` tests the cache. Its AC block test passes; its cache tests
  measure hits, and this emulator has no cache.
- `DSKCG` test 9 (page fail recovery) sets up a DEC page table (DSKCGM.MAC,
  PAGSET).
- `SUBSM` clears UBR with `DATAO PAG,[LLDUSB,,400000]` (SUBSM.SEQ, line
  1959). In the DEC format, bits 23 to 35 are a page number, so the UPT is
  at 0 (PARAM.KSM, "DATAO PAG 010 ASSIGMENTS"). The ITS microcode takes
  bits 16 to 35 as the address, so the UPT is at 400000. The `DSKCG` test
  writes the ITS form, 100000,,0, into the literal at 7103.

`make diag` leaves out the programs and tests that do not apply. For the
other programs, a difference that comes from the ITS microcode goes into
`docs/discrepancies.md`, with the instruction and the two behaviours.

### Processor model

The software processor in `emu/ks10-core-soft` follows the ITS microcode. These
parts are modelled as hardware state, not as microcode steps:

- The page table of the pager (512 entries, one for each DEC page). The
  microcode refills an entry from the ITS page tables and clears the age
  bit (`itspag.98`). Sweeps and `CLRPT` clear entries.
- AREAD: before an instruction runs, the reference to E is checked as
  the DROM fields READ, TEST and WRITE give. So a page failure comes
  before a change of the flags.
- The trap cycle, the one-proceed trap, the page fail trap, the halt
  status block, the 1 ms clock, and the PC sample table.
- The I/O instructions reach the I/O address space. With no devices, each
  reference is a page failure with "no IO register".

Differences from the microcode:

- EXTEND runs to the end in one step. After a page failure inside it,
  the ACs go back to their values at the start. The microcode keeps the
  work done (`extend.4`, "PAGE FAIL CLEANUP"). The second try gives the
  same result, because the strings must not overlap (PRM 2.12).
- The halt status block has 0 in the words of microcode temporaries
  (`docs/unknowns.md`).

### Order of the work

1. `spec/instructions.*` and its generator; the `pdp10` crate.
2. The interpreter without paging, with tests against the reference
   traces.
3. ITS paging, APR, PI, timers, the 8080 console words, the halt status
   block. Done in `emu/ks10-core-soft`, except the Unibus interrupts, which come
   with the adapters.
4. The adapters, the RH11 with the TM03: boot from tape (salvager,
   DSKDMP). Done.
5. The RH11 with the RP06: boot ITS from `rp0.dsk`. Done.
6. `neoks10`: the configuration, the operator console, the control
   socket, scripts, the host side of the DZ11 lines. Done. Then the
   diagnostics, the ITS build.
7. Step 6 of the plan in `architecture.md`: the CH11 and the IMP.

## Design decisions

Decided on 2026-11-26:

1. The binary name is `neoks10`.
2. The configuration file is TOML. The crates `toml` and `serde` read
   it.
3. The escape character is Ctrl-\ (034), as in the ITS build scripts.
4. The DZ11 listeners are on the loopback address only by default.

Decided on 2026-11-27:

5. The operator console uses the crate `reedline`. `neoks10`
   also uses its terminal layer `crossterm` directly, at the version that
   `reedline` uses, so the build has no other crate. A test
   (2026-11-27) built a small program with `reedline` 0.53.0 for
   armv7-unknown-linux-musleabihf: 43 crates, no asynchronous runtime, a
   static binary of 1.4 MiB. In a pseudo terminal under `qemu-arm`, Tab
   completed `ta` to `tape`, as on the host **[measured]**.
6. The machine runs while the operator console is open.
7. The console has no SIMH command names.
8. The control socket is in the first version.
9. The control socket uses the crate `serde_json`. A well-known crate is
   better than a new implementation, within reason.
10. The interactive console can start a script (`script run FILE`).
11. `crossterm` is a direct dependency (decision 5).
12. The crate `sha2` checks the images of a replay. With
   `default-features = false` it brings `digest`, `block-buffer`,
   `crypto-common`, `hybrid-array`, `typenum` and `cpufeatures`.
13. The emulator detects the idle loop of ITS, as SIMH does. See
   [Time](#time).
14. `--pace normal` is the mode for daily use and the default of a run
   with a terminal, a control socket or DZ11 listeners. Its clock comes
   from the wall clock, as in SIMH. Only the modes for tests must be
   repeatable, and the record and replay layer must exist for them
   (AGENTS.md).
15. The DZ11 lines use Telnet, with one listener port for each line.
16. The order of the sources (AGENTS.md, "Sources of truth"): the
   microcode with the data path first, then the expected values of the
   DEC diagnostics, the manuals, and the emulators.
17. A microcode-level model of the KS10 comes next (choice B of
   2026-11-27): a model of the data path runs the microcode that the ITS
   build assembles (`KSHACK; MCR 262`, copied to `RAM RAM`), and the
   real 8080 console program. The first core in the PL has the same
   goal. The 8080 program runs in a software model of the 8080 on the PS
   (ARM); the PL does not need an 8080. It reaches the PL processor
   through the console registers of the PS/PL interface.
18. KLH10 is a second reference emulator, next to SIMH.
19. KS10FPGA is a reference for the data path, as the submodule
   `3rdparty/KS10FPGA`.

Open:

20. The IMP interface for step 6. The choices include the emulator-only
   `SSIMP` of the `DB` configuration, the ACC LH/DH of a real KS10
   (`KSIMP`), and an interface of this project. The decision waits for
   step 6.

## To do

- The timing of the devices in `--pace normal`. The seek and transfer
  times of the RH11/RP06, the tape motion of the TM03 and the character
  time of the DZ11 are in instructions. Because the guest works at full
  speed in `normal`, they are shorter in wall time than on a KS10. A
  model in wall time can replace them in `normal`; the modes for tests
  keep the times in instructions.

- The microcode-level model (decision 17), `emu/ks10-core-uop`
  ([`microcode-model.md`](microcode-model.md)). The microcode of the ITS
  build loads and is checked. Next is its first stage: the data path and
  the microcode without paging and I/O, on a flat memory. It decides the
  cases of `docs/unknowns.md` that no DEC vector decides; the first one is
  FDV with normalized operands.

## Open items

These are in `docs/unknowns.md`:

- Which DEC KS10 diagnostics apply to the ITS microcode behaviour, and how
  the diagnostic monitor loads them.
- The register address of the `SSIMP` interface (`%LHVEC`, `lhdh.defs`).

## Documents and tapes

`third-party.lock` lists the files from bitsavers.org and
pdp-10.trailing-edge.com, and `make references` copies them into
`references/`. On 2026-11-26 the list has these documents:

- The KS10 technical manual (EK-0KS10-TM-001 and EK-OKS10-TM-002), the KS10
  reference (`ksrefRev2.pdf`, `ksrefRev4.txt`), and the KS10 maintenance
  guide, volumes 1 and 2.
- The 8080 console program, version 5.2 (`KS10_CSL_V5.2.zip`).
- The DECsystem-10/DECSYSTEM-20 Processor Reference Manual (1982).
- The RH11 option description.
- The DZ11 user's guide and technical manual.
- The RP04/RP05/RP06 maintenance manual (EK-RP056-MM-01).
- The TM03 user's guide and technical manual, and the TU45 maintenance
  manual.

The KS10 schematics and the KS10 microcode listings are on bitsavers too
(`pdf/dec/pdp10/KS10/`), but not in the list.

The tapes (TOPS-10 BACKUP format; `t10backup` of `tools/tapeutils` in the
ITS tree lists and extracts them):

- `KLAD_Sources.tap` (bitsavers, `dec_distribs/klad/`): the sources and the
  `.SAV` files of the diagnostics for the KL10 and the KS10. The KS10
  programs are `DSKAA` to `DSKFA`; their `.TXT` files name them
  "DECSYSTEM 2020" or "KS10" diagnostics (2026-11-26, a list of the tape):
  - `DSKAA` to `DSKAM`: basic instruction diagnostics 1 to 13;
  - `DSKBA`: basic instruction reliability;
  - `DSKCA` to `DSKCG`: advanced instruction diagnostics 1 to 7;
  - `DSKDA`: processor and memory reliability;
  - `DSKEA`: paging (DEC); `DSKEB`: cache; `DSKEC`: KL paging;
  - `DSKFA`: instruction timing.
  The tape also has the diagnostic monitor `SMMON`, `SMDDT`, `SUBRTN` and
  the KS10 parameter files (`PARAM.KSM`, `FIXED.KSM`, `STOR.KSM`).
- `ks_diag_gs.tap.bz2` (trailing-edge, "KS Diagnostics GS"): the `.SAV`
  files of the same programs, and the device diagnostics, for example
  `DSDZA` (DZ11), `DSRPA`, `DSRHA`, `DSTUA`, `DSUBA` and `DSMMA` to `DSMMD`.
  It also has `SMMON`, `SMTAPE` and the command files `SMCPU.CMD` and
  `SMFLT.CMD`.
- `BB-JF16A-SB_1986.tap.gz` (bitsavers, `dec_distribs/microcode/`): the KS10
  microcode version 2.0(130) of DEC, sources.
- `ks10_8080_microcode.tap.bz2` (trailing-edge, "KS-10 8080 Microcode
  V5.2"): the 8080 console program, with the listing `KNS10.PNT`.

KS10FPGA (`3rdparty/KS10FPGA`, see [Sources](#sources)) has no license
file at the top. Its Verilog files say LGPL 2.1 in their headers, and its
microcode files have the DEC hobbyist license. This project reads it as a
reference and does not copy it (AGENTS.md).
