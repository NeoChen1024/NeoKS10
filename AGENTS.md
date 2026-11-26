# Rules for work in this repository

This file is a workflow guide. It is not a specification. Do not copy
instruction semantics, bit layouts or other numbers from this file into code.
Get them from the sources below.

`README.md` gives the goal and the architecture. Read it first.

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
2. The manuals: the ITS notes in `doc/kshack/` of the same repository, and
   the DEC manuals on <https://bitsavers.org/pdf/dec/pdp10/>.
3. The behaviour of other emulators (KLH10, the `pdp10-ks` simulator).
4. Inside this repository:
   - `spec/instructions.*` is the only definition of the instruction set.
   - `spec/regmap.*` is the only definition of the PS/PL interface.
   - The software processor in `emu/` is the model for the logic in `rtl/`.

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
| `spec/` | Instruction table, register map, and their generators |
| `emu/` | The Rust workspace: emulator, software processor, image tools |
| `rtl/` | SystemVerilog: core, memory bridge, panel |
| `tb/` | Verilator co-simulation, mutation scripts, SymbiYosys proofs |
| `fpga/` | Vivado scripts and constraints for the PYNQ-Z2 |
| `oracle/` | Pinned commits and patch files for the reference emulators |
| `3rdparty/` | Git submodules: `its` is the PDP-10/its fork with the build changes, `PYNQ-Z2` is the board data |
| `its/` | The files that PDP-10/its needs under `build/<emulator>/` |
| `tests/` | Guest programs, saved random seeds, expected traces |
| `docs/` | Design notes, `unknowns.md`, `discrepancies.md` |
| `third-party.lock` | Source URL and SHA-256 of each external file |

Build output and downloaded files stay outside the repository tree, or in a
directory that `.gitignore` excludes.

## Build and verification

| Change | Run before the commit |
|---|---|
| A file in `spec/` | `make gen`, then all rows below |
| The software processor or a device | `make check` |
| A file in `rtl/` | `make check`, `make cosim`, and `make formal` for the changed module |
| A new behaviour in `rtl/` | Also `make mutate` for the changed module |
| Before a merge | `make diag` and `make its-boot` |
| A milestone, or a possible fault in a full system | `make its-build` |
| A file in `fpga/`, or a timing claim | `make synth`, and read the timing report |

- `make check` is: format, lint, unit tests, and a short random differential
  run.
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
- The reference emulators are external. Pin them by commit in `oracle/`.
  Keep the trace hook as a patch file. You can read their source. Do not
  copy it.

## Rules for the emulator (Rust)

- A guest program can never cause a panic or a crash of the emulator. A
  guest error is a guest trap or a console message.
- The emulator is deterministic. All input that changes with time (clock,
  terminal, network) goes through one layer that can record and replay it.
- One `Machine` structure owns all state. Do not use `Rc<RefCell<_>>`.
- A 36-bit word is its own type. Do not pass guest words as a bare `u64`.
- `unsafe` is permitted in one module only: the module for `mmap` and for
  the PL registers.
- Do not add a dependency without the approval of the owner. Do not use an
  asynchronous runtime.
- Decoders and constants come from `spec/`. Do not write a second copy by
  hand.
- The software processor has a C interface for the co-simulation and for
  the PL bridge.
- Device I/O reads and writes the guest memory directly with `pread` and
  `pwrite`. Do not use `O_DIRECT`.
- The disk image keeps 8 bytes for each word. The tape image keeps the
  format that the ITS tools write. A converter gives the formats of other
  emulators.

## Rules for the logic (SystemVerilog)

- The first core is a multi-cycle state machine. Do not add a pipeline
  before the first core runs ITS.
- The core gives a record at each completed instruction. The co-simulation
  compares this record with the software processor.
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
