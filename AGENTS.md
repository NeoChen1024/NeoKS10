# Rules for work in this repository

This file is a workflow guide. It is not a specification. Do not copy
instruction semantics, bit layouts or other numbers from this file into code.
Get them from the sources below.

`README.md` gives the goal and the state of the project.
`docs/architecture.md` gives the parts, the verification levels and the
plan. Read both first.

## Status

The project is at the design stage. The layout and the commands in this file
are a plan. When you create a directory or a command, use the name that is
given here. If a different name is necessary, change this file in the same
commit.

## Sources of truth

Use the sources in this order. A lower source cannot overrule a higher
source.

1. The ITS KS10 microcode sources: `src/kshack/` in
   <https://github.com/PDP-10/its>. The files of the assembly are `its`,
   `ks10`, `simple`, `flt`, `extend`, `inout`, `itspag` and `pagef`.
   For the devices and the console, the same rank has the ITS system
   sources (`src/system/`, for example `ksdefs` and the device drivers) and
   the 8080 front-end sources (`src/kshack/*.m80`): they give what ITS and
   the boot process expect.
   A microcode step does what the KS10 data path does with it. The field
   definitions of `ks10` and the DEC KS10 hardware documents (rank 3)
   give the data path. A model of the data path that runs the assembled
   microcode is the executable form of this rank.
2. The expected values of the DEC KS10 diagnostics: recorded data of the
   machine. A value applies only if the ITS microcode takes the path of
   the DEC microcode, and only to the class of operands of the test.
3. The manuals: the ITS notes in `doc/kshack/` of the same repository, and
   the DEC manuals on <https://bitsavers.org/pdf/dec/pdp10/>. A statement
   for the whole PDP-10 family applies to the KS10 only if no KS10 source
   differs.
4. The behaviour of other implementations: the emulators KLH10 and
   `pdp10-ks`, and KS10FPGA (`3rdparty/KS10FPGA`), which implements the
   data path of the DEC schematics. They find cases to examine. Their
   agreement does not decide a case. A data path detail from KS10FPGA
   has the mark [emulator] until a DEC schematic or manual gives it.
5. Inside this repository:
   - `spec/instructions.*` is the only definition of the instruction set.
   - `spec/devices.*` is the only definition of the I/O device registers.
   - `spec/regmap.*` is the only definition of the PS/PL interface.
   - The microcode-level model in `emu/` is the model for the logic in
     `rtl/`. The instruction-level software processor is compared with it.

The data of the board is in <https://github.com/NeoChen1024/PYNQ-Z2>. The
submodule `3rdparty/PYNQ-Z2` pins the commit that this repository uses. Do not
copy pin tables or measurements from it. Refer to it.

If two sources disagree, do not select one silently. Add the case to
`docs/discrepancies.md` with both statements, then use the higher source.

## Evidence marks

Each statement about the machine in `docs/` or in a code comment has a mark.

| Mark | Meaning |
|---|---|
| **[microcode]** | Read from the microcode source. Give the file and the label |
| **[its]** | Read from the ITS system sources or the 8080 front-end sources. Give the file and the label |
| **[manual]** | Read from a manual. Give the document and the page or section |
| **[emulator]** | Seen in another emulator. Give the emulator and its commit |
| **[measured]** | Measured on the board or in a test. Give the date and the method |
| **[estimate]** | An estimate. Give its basis |

- A value that has no source goes into `docs/unknowns.md`, not into the code
  as a fact.
- Do not write a value that only looks correct.
- A speed increase is a measurement. Give the two numbers and the method.

## Layout

| Path | Contents |
|---|---|
| `spec/` | Instruction table, register map (`regmap.toml`), and their generators |
| `emu/` | The Rust workspace: emulator, software processor, image tools. `pdp10` has the word type and the instruction tables from `spec/`. `ks10` is the machine and the software processor. `ks10uop` is the microcode-level model. `plio` is the access to the PL. `memprobe` is the program of the memory path test. `neoks10` is the program `neoks10`: configuration, operator console, control socket, scripts |
| `rtl/` | SystemVerilog: core, memory bridge, panel. `rtl/memprobe/` is the test design of the memory path. `rtl/gen/` has generated files |
| `tb/` | Verilator co-simulation, mutation scripts (`mutate.py`), SymbiYosys proofs. `tb/memprobe/` tests `rtl/memprobe/`. `tb/gen/` has generated files |
| `fpga/` | Vivado scripts and constraints for the PYNQ-Z2. `fpga/memprobe/` builds `rtl/memprobe/` |
| `oracle/` | Pinned commits and patch files for the reference emulators |
| `3rdparty/` | Git submodules: `its` is the PDP-10/its fork with the build changes, `PYNQ-Z2` is the board data, `KS10FPGA` is the FPGA KS10 that is a reference for the data path (shallow) |
| `its/` | The files that PDP-10/its needs under `build/<emulator>/` |
| `tests/` | Guest programs, saved random seeds, expected traces |
| `docs/` | Design notes, `unknowns.md`, `discrepancies.md`, `regmap.md` (generated) |
| `references/` | Third-party manuals from `third-party.lock`, made by `make references`. Git ignores it |
| `third-party.lock` | Source URL and SHA-256 of each external file |

Build output and downloaded files stay outside the repository tree, or in a
directory that `.gitignore` excludes.

`make gen` writes files from `spec/`: `rtl/gen/`, `tb/gen/`,
`emu/plio/src/regmap.rs`, `emu/pdp10/src/instructions.rs`,
`emu/ks10/src/devregs.rs`, `docs/regmap.md`, `docs/instructions.md` and
`docs/devices.md`. It also writes `emu/ks10uop/src/fields.rs` from the
field definitions of the ITS microcode. It also writes `docs/console.md` from the command table
of `emu/neoks10`. These files are in git, so that a build does not need
the generators. Do not edit them. If they are not current, `make check`
fails.

## Build and verification

| Change | Run before the commit |
|---|---|
| A file in `spec/` | `make gen`, then all rows below |
| The software processor or a device | `make check` |
| A file in `rtl/` | `make check`, `make cosim`, and `make formal` for the changed module |
| A new behaviour in `rtl/` | Also `make mutate` for the changed module |
| Before a merge | `make diag` and `make its-boot` |
| A milestone, or a possible fault in a full system | `make its-build` |
| A file in `fpga/`, or a timing claim | `make synth` (100 MHz; other clocks optional), and read the timing report |

- `make check` is: format, lint, unit tests, a check that the generated
  files are current, and a short random differential run.
- `make oracle` builds the reference emulators (SIMH `pdp10-ks` and KLH10
  `kn10-ks-its`, pinned in `oracle/*.pin`) with the trace patches of
  `oracle/` into `build/`. The short differential run of `make check`
  uses them, through the variables `KS10_ORACLE` and `KS10_ORACLE_KLH10`
  of `emu/ks10/tests/oracle.rs`. Without one, `make check` prints
  SKIPPED for it: a skip is not a pass.
- `make microcode KS10_ITS_DISK=<rp0.dsk>` extracts the listing of the
  microcode from the dump tape of the ITS build (`output.tape`, next to
  `rp0.dsk`) and compares it with the RAM file on the disk. Run it when
  `emu/ks10uop` or the microcode changes.
- `make cross` builds the programs of `emu/` for the board (armv7, musl,
  static), and runs their lint and unit tests for that target (qemu-arm).
- `make formal MODULE=<name>` runs `tb/memprobe/<name>.sby`, for example
  `gen` or `regs`. `make mutate MODULE=<file>` mutates
  `rtl/memprobe/<file>.sv`, for example `memprobe_gen`.
- Run heavy jobs (synthesis, formal proofs, the ITS build) with `nice -n 19`.
- Do not change a file that a test or a synthesis in progress reads.
- State in the commit message which rows you ran. Do not state a result that
  you did not see.

## Tests

- An expected result comes from a different implementation or from recorded
  data. It does not come from the code under test alone.
- The software processor and the logic can have the same error. Their
  agreement is not evidence that one of them is correct.
- A regression test must fail without its fix. Make sure of this before the
  commit.
- Do not change a test to make it pass, unless the test is wrong. Then give
  the source that shows the error.
- Do not add a special case for the input of a test.
- A random test that fails goes into `tests/` with its seed.
- For a new behaviour in the logic, make mutants. A test set that kills no
  mutant of the new code is not sufficient.
- A formal proof has a time limit. A timeout is not a pass and not a fail.
  Report it as a timeout.
- The reference emulators and KS10FPGA are external. Pin them by commit
  in `oracle/`. Keep the trace hook as a patch file. You can read their
  source. Do not copy it.

## Rules for the emulator (Rust)

- A guest program can never cause a panic or a crash of the emulator. A
  guest error is a guest trap or a console message.
- The modes for tests are deterministic: the `ks10` library, and
  `neoks10 run --pace fast`, `idle` or `realtime`. In them, all input that
  changes with time (clock, terminal, network) goes through one layer that
  can record and replay it. This layer must exist and work.
- The mode for daily use (`--pace normal`) does not need to be
  repeatable. Tests do not use it.
- One `Machine` structure owns all state. Do not use `Rc<RefCell<_>>`.
- A 36-bit word is its own type. Do not pass guest words as a bare `u64`.
- `unsafe` is permitted in one module only: the module for `mmap` and for
  the PL registers.
- Do not add a dependency without the approval of the owner. Do not use an
  asynchronous runtime.
- Decoders and constants come from `spec/`. Do not write a second copy by
  hand.
- The microcode-level model and the software processor have a C interface
  for the co-simulation and for the PL bridge.
- Device I/O reads and writes the guest memory directly with `pread` and
  `pwrite`. Do not use `O_DIRECT`.
- The disk image keeps 8 bytes for each word. The tape image keeps the
  format that the ITS tools write. A converter gives the formats of other
  emulators.

## Rules for the logic (SystemVerilog)

- The first core runs the assembled ITS microcode on the KS10 data path,
  with the real 8080 console program. The 8080 program runs in a model of
  the 8080 on the PS, not in the PL. The core is multi-cycle. Do not add a
  pipeline before the first core runs ITS.
- The core gives a record at each completed instruction. The co-simulation
  compares this record with the microcode-level model.
- The register map comes from `spec/regmap.*`.
- The PS maps the guest memory without the CPU cache. If you change this,
  you must show how the PS and the PL see the same data.
- A read from the PL cache must not pass a queued write to the same address.
- The front panel is a display only. It must not change the state of the
  processor.

## Conventions

- Bits of a guest word have the DEC numbers: bit 0 is the most significant
  bit, and bit 35 is the least significant bit.
- Guest addresses and guest values are octal in logs, messages and
  documents. Use the helper functions for bit fields and for octal text.
- Memory sizes use IEC units (Ki, Mi, Gi). Mass storage uses SI units.
  State if a size is in words or in bytes.
- Code, comments, messages and documents are in English.

## Board and outside effects

- Load the PL through JTAG or from Linux. Do not write the configuration
  flash unless the owner tells you to do so.
- A command that has an effect outside the board needs the permission of the
  owner in that session. Examples: a connection to a real LAN, a connection
  to the public Chaosnet, a write to a storage medium.
- Automatic tests do not use an outside network.

## Publication and privacy

- You can make a local commit without the permission of the owner. Run the
  checks of "Build and verification" first.
- A push needs the permission of the owner in that session.

Do not put these items into a file of the repository or into a commit
message: host names, user names, home directory paths, IP addresses, MAC
addresses, serial numbers.

## Third-party material

- Manuals, diagnostics, system images, and the output of vendor tools stay
  out of the repository.
- For each such file, `third-party.lock` has the URL and the SHA-256. Use a
  bitsavers.org link when one exists.
- Do not invent a URL. If you did not open the link, do not write it.

## Writing

Commit subjects have the form `type(scope): summary`, in the imperative
mood, with no period at the end. The body tells why, and which tests ran.

When a design changes, remove the text of the old design. Do not keep it as
history in the documents; the history is in git.

### Document language

Documents use simplified technical English. Apply these rules to
approximately 80% of the sentences:

- One statement in each sentence.
- A maximum of 25 words in a descriptive sentence, and 20 words in a step.
- Active voice and present tense.
- Put the condition first: "If the queue is full, the core stops."
- A maximum of 3 nouns together, and 6 sentences in a paragraph.
- No metaphors, no idioms, no Latin abbreviations.
- Numbers have units.
- Technical names stay as they are.

### Voice of the documents

`README.md`, `docs/` and the comments in code are public. They are in the
voice of the author of the project.

- Do not write "the owner", "the agent", "the session", or "approved by".
  These words belong to the work rules in this file.
- Write a decision as a design decision with its date. Give the reason,
  not the person who decided.
- Do not repeat a rule of this file in a document. If a document needs
  the rule, link to this file.
- `README.md` is for visitors. Keep it short: the goal, the state, a quick
  start, and links. Details go to `docs/`.
