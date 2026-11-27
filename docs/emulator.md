# Emulator design

This document is the design of the emulator program in `emu/`. It covers
step 1 of README.md (the software processor boots ITS on a host computer),
step 2 (the same program on the PS of the board) and the parts of step 5
that the program supplies (the devices for the core in the PL).

The document has three parts:

- The machine: what the program must emulate. Each statement has an
  evidence mark (AGENTS.md).
- The design: the decisions of this document.
- Decisions of the owner: the decisions that the owner made, and the
  items that are still open.

## Sources

The ITS sources are at the pin of `3rdparty/its` (commit 83534677).
References name the file and the label in that tree. The reference
emulators are the submodules of the same tree:

| Emulator | Path | Commit | Use |
|---|---|---|---|
| sims `pdp10-ks` (Richard Cornwell) | `tools/sims` | 9556eca0 | The ITS build of step 0 uses it (`Makefile`, variable `KS10`) |
| KLH10 | `tools/klh10` | 6d733f2a | Second reference |

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

The bit layouts of the registers are in the files of the table. The
emulator does not copy them by hand: they go into a file of `spec/`, as
the register map of the PL does (see [Program structure](#program-structure)).

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
macro `UBINT`). How the processor takes the interrupt is in the microcode
(`inout`). This document does not describe it yet.

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
| No new dependency without approval | The owner approved `toml` and `serde` for the configuration file (see [Decisions of the owner](#decisions-of-the-owner)) |
| Decoders and constants from `spec/` | The instruction table and a device register table in `spec/` generate the Rust constants, as `spec/regmap.toml` does now |
| C interface for co-simulation and the PL bridge | The `ks10` library exports C functions: create a machine, run one instruction, read the instruction record |
| Device I/O with `pread` and `pwrite` | A disk transfer reads or writes the image file directly into the guest memory, when the adapter map gives contiguous memory |
| Disk image with 8 bytes for each word | The format of step 0 (above). Images of step 0 work without a conversion |
| Tape image in the format of the ITS tools | The SIMH tape image format with core dump words (above) |

### Program structure

The crates of `emu/`:

| Crate | Contents |
|---|---|
| `pdp10` | `Word`, octal text, instruction decode generated from `spec/instructions.*` |
| `ks10` | `Machine`: the processor (interpreter), memory, paging, APR, PI, timers, 8080 console, Unibus adapters, devices, the host layer. The C interface |
| `neoks10` (the binary `neoks10`) | Command line, configuration, operator console, image tools |
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
- `--realtime` makes the emulator wait so that virtual time does not run
  ahead of the wall clock. It does not change the result of a run, only its
  speed.
- The wall clock (for the date of ITS) is a host input of the host layer.

### Command line

The program `neoks10` has subcommands.

```
neoks10 run [CONFIG] [--record FILE] [--replay FILE] [--trace FILE]
            [--cpu soft|pl] [--realtime] [--stop-after N] [--script FILE]
neoks10 image create rp06 FILE
neoks10 image info FILE
neoks10 image convert --from klh10 IN OUT
neoks10 tape list FILE
neoks10 trace diff A B
```

- `run` starts the machine and the operator console. `CONFIG` names the
  configuration file.
- `--trace` writes one record for each completed instruction, in the
  format that the co-simulation compares.
- `--script` reads operator console commands from a file, for the tests
  `make diag` and `make its-boot`.
- `image convert` exists for KLH10 images; the step 0 images need no
  conversion.

### Operator console

The terminal of `neoks10 run` is the console terminal of the KS10. An escape
character stops the guest and gives the operator prompt. The ITS build
scripts use Ctrl-\ (034) for this (`build/pdp10-ks/build.tcl`,
`emulator_escape`), and the default is the same.

The ITS build drives the emulator with these operations
(`build/pdp10-ks/build.tcl` and `build/ks10/include.tcl` of the ITS tree):
start with an initial command file, a prompt, attach a tape image to a
unit, boot from a tape unit, boot from the disk, continue, quit, and the
escape character. The console has a command for each:

| Command | Function | 8080 command |
|---|---|---|
| `boot disk [UNIT]` | The `BT` sequence | `BT` |
| `boot tape [UNIT]` | The `MT` sequence | `MT` |
| `attach DEVICE FILE [ro]` | Attach an image to a disk or tape unit | none |
| `detach DEVICE` | Detach an image | none |
| `continue` | Continue the guest | `CO` |
| `halt` | Stop the processor | `HA` |
| `shutdown` | Set `8SWIT0` | `SH` |
| `start ADDR` | Start at an address | `ST` |
| `step [N]` | Run N instructions | `SI` |
| `examine ADDR [N]`, `deposit ADDR WORD` | Physical memory, octal | `EM`, `DM` |
| `show STATE` | Processor, devices, attachments | none |
| `record mark TEXT` | A mark in the record file | none |
| `quit` | End the program | none |

The `its/` directory of this repository holds the scripts that the ITS build
needs under `build/<emulator>/` (AGENTS.md). They use these commands.

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
lines = 8
listen = "127.0.0.1:10004"

[chaosnet]
enabled = false
```

The defaults keep all network listeners on the loopback address. A
connection to a LAN needs the permission of the owner (AGENTS.md).

### Tests

- Each instruction: differential runs against the instruction records of
  the reference emulator, with a trace patch in `oracle/` (AGENTS.md).
- The DEC KS10 diagnostics (`make diag`); see [Diagnostics](#diagnostics).
- `make its-boot`: boot the step 0 disk image with a replay file of
  console input, to the ITS prompt.
- `make its-build`: build ITS from source with the scripts of `its/`.

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
  **[measured]** (2026-11-27, `emu/ks10` diag example: `WREBR` at 3753
  with E bit 22 set). It expects the KI style paging of the DEC
  microcode, which the ITS microcode does not have. The switch `INHPAG`,
  left half 000100, inhibits paging and traps (`DSKAM.TXT`, section 4.0).
  So the answers are `Y`, `000100` and `0`.
- After each pass, `SUBSM` prints `END PASS n.` (decimal) **[measured]**
  (2026-11-27, `DSKAI` to `DSKAM` in `emu/ks10`).
- `SMCPU.CMD` lists the processor diagnostics in the order in which the
  monitor runs them.

**The plan for `make diag`:**

1. Extract the `.SAV` files from the tape into `build/`. A `.SAV` file is a
   list of blocks (`-count,,address-1`, then the words) and ends with
   `JRST start`.
2. For each program of `SMCPU.CMD`: load `SUBSM.SAV` (if the program needs
   it), then the program, with `neoks10` and a console script.
3. Start at 30000. Answer the switch questions with `Y`, `000100` and
   `0` (`INHPAG`). Type each answer 100 ms of virtual time after its
   prompt: a character that comes while `SUBSM` prints is read as a
   control character and lost **[measured]** (2026-11-27, trace of the
   CTY output loop of `SUBSM`).
4. Stop after a fixed number of instructions. The result is a pass if the
   processor did not halt, `PASCNT` did not decrease, and the console
   output has no error report. A halt gives the PC, which the listing
   (`.SEQ` file) explains.

**Reference runs [emulator]** (sims `pdp10-ks` 9556eca0, `set cpu its`,
`load -c` of the extracted files, 2026-11-26):

- `DSKAA` ran for 120 s of host time without a halt. `PASCNT` was
  14162641 (octal) at the end.
- `DSKCA` with `SUBSM` printed its title, took the answer `0` and ran for
  60 s without a halt or an error message. The end of a pass was not seen
  in that time.
- The title of `DSKCA` gave `CPU#=4097`, `MCV=416` and `MCO=570`. These
  values of `APRID` differ from the ITS microcode (`docs/discrepancies.md`).

**Runs in `emu/ks10` [measured]** (2026-11-27, `cargo test --release -p
ks10 --test diag` with `KS10_DIAG_DIR`): `DSKAA` to `DSKAH` run 5 million
instructions without a halt. `DSKAI` to `DSKAM` run 3 million
instructions under `SUBSM` and print `END PASS 10.` with no error
message. A run of 60 million instructions gave more than 2600 passes of
each of `DSKAI` to `DSKAM`.

**Which programs apply.** The programs are for the DEC microcode. `DSKEA`
and `DSKEC` test the paging of the DEC microcode, which the ITS microcode
does not have; `make diag` leaves them out. For the other programs, a
difference that comes from the ITS microcode goes into
`docs/discrepancies.md`, with the instruction and the two behaviours.

### Order of the work

1. `spec/instructions.*` and its generator; the `pdp10` crate.
2. The interpreter without paging, with tests against the reference
   traces.
3. ITS paging, APR, PI, timers, the 8080 console words, the halt status
   block.
4. The adapters, the RH11 with the TM03: boot from tape (salvager,
   DSKDMP).
5. The RH11 with the RP06: boot ITS from `rp0.dsk`.
6. The DZ11, the diagnostics, the ITS build.
7. Step 6 of README.md: the CH11 and the IMP.

## Decisions of the owner

Decided on 2026-11-26:

1. The binary name is `neoks10`.
2. The configuration file is TOML. The crates `toml` and `serde` are
   approved for it.
3. The escape character is Ctrl-\ (034), as in the ITS build scripts.
4. The DZ11 listeners are on the loopback address only by default.

Open:

5. The IMP interface for step 6. The choices include the emulator-only
   `SSIMP` of the `DB` configuration, the ACC LH/DH of a real KS10
   (`KSIMP`), and an interface of this project. The decision waits for
   step 6.

## Open items

These are in `docs/unknowns.md`:

- Which DEC KS10 diagnostics apply to the ITS microcode behaviour, and how
  the diagnostic monitor loads them.
- How the microcode takes a Unibus interrupt (`inout`).
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

A related project is KS10FPGA (github.com/KS10FPGA/KS10FPGA), a KS10 with
the DEC microcode in an FPGA. Its repository has no license file
(2026-11-26), so this project can read it as a reference but not copy it.
