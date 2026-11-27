# Microcode-level model

`emu/ks10uop` is a model of the KS10 at the level of the microcode. A
model of the data path runs the microcode that the ITS build assembles.
If the sources disagree about an instruction, this model decides
(AGENTS.md, "Sources of truth"). It is also the model that the logic in
`rtl/` is compared with ([`architecture.md`](architecture.md)).

The work has three stages:

1. The data path and the microsequencer on a flat memory, without paging
   and I/O. Random instructions compare it with `emu/ks10`.
2. ITS paging, the I/O cycles to the devices of `emu/ks10`, interrupts,
   page fail. Then the diagnostics and an ITS boot.
3. A model of the 8080 runs the real console program.

## The microcode of the ITS build

The ITS build assembles the microcode with MICRO: `KSHACK; MCR 262` from
`its`, `ks10`, `simple`, `flt`, `extend`, `inout`, `itspag` and `pagef`
**[its]** (`build/ks10/processor.tcl` of the ITS tree). MICRO writes two
files:

- The listing `KSHACK; MCR 262`. A line `U aaaa, ...` gives a control RAM
  word: the address and 108 bits in nine groups of four octal digits. A
  line `D aaaa, ...` gives a dispatch ROM word, 36 bits **[microcode]**.
- The RAM file `KSHACK; MCR RAM`. The build copies it to `.; RAM RAM`, and
  `KSFEDR` puts it into the file system of the 8080 on the disk.

### The control RAM

The control RAM has 4000 words of 96 bits **[manual]** (EK-OKS10-TM-002,
section 5.4.3, page 5-40: bits 0 to 35 on the CRA board, 36 to 95 on the
CRM board). These are the bit numbers of the hardware, not of MICRO.
The 8080 loads it from the RAM file before it starts the microcode
**[its]** (`cmds.m80`: MICROP, DMEM2CRAM, NEXTCR):

1. The home block, then the page of file pointers. Pointer word 2 gives
   the first page of the RAM file.
2. Each control RAM word takes three memory words. The loader stops at
   address 4000. The RAM file thus has 3 × 4000 = 14000 words (octal):
   12 (decimal) pages of 1000 words.
3. The pages after the first come from the next sectors: the transfer
   list `QXFR` (`dr.m80`) sets no disk address.

`ks10::boot::read_microcode` does these steps on the emulated RH11 and
RP06.

`.RAMFILE` in `ks10` gives the layout of an entry **[microcode]**: 108
bits, the listed fields from left to right, two parity bits and two pads
(MICRO, `$RAMFL` and `RAMFL4`). The generator `spec/gen_microcode.py`
reads this layout and the field definitions into
`emu/ks10uop/src/fields.rs`.

**Parity.** The parity bit of `ODD PARITY<ranges>` is the XOR of the given
bits of the RAM file entry **[microcode]** (MICRO, `PNTB91`). With the
parity bit, each range set has an even number of ones. This agrees with
the comment of `.RAMFILE` ("Parity is even parity"). `EVEN PARITY` gives
the complement. The ranges are bits of the RAM file entry, not of the
microword.

**Check of the RAM file [measured]** 2026-11-27 (`make microcode`, test
`emu/ks10uop/tests/ram_file.rs`). The RAM file was read from `rp0.dsk` of
the ITS build of 2026-11-26 as the 8080 reads it:

- All 4000 entries have the right parity bits and zero pads.
- The 1980 (decimal) U words of the listing agree with the decoded
  entries in all fields of the RAM file. The other 68 (decimal) entries
  are 0.
- No listing bit is outside the fields of the RAM file.
- The SHA-256 of the RAM file is in the test.

If a field position or the parity rule is wrong, the test fails: a test
with the parity rule reversed, and a test with `AD` moved by one bit,
both failed.

### The dispatch ROM

The dispatch ROM has 512 words, one for each opcode. It is three
512 × 8-bit ROM chips on the DPEA board **[manual]** (EK-OKS10-TM-002,
section 5.4.4.1, page 5-41). The console loads only the control RAM: an
address and eight 12-bit segments for each word **[manual]** (section
5.4.6) **[its]** (`cmds.m80`, DMEM2CR). The only source of the dispatch
ROM contents is the listing.

**Check of the dispatch ROM [measured]** 2026-11-27 (the same test). The
fields `A`, `B`, `ACDISP`, `I`, `READ`, `TEST`, `COND FUNC`, `VMA` and
`WRITE` of all 512 D words agree with `spec/instructions.toml`. That
table comes from the source text of the microcode, not from the listing.
A test with `B` moved by one bit failed.

The 512 D words of the ITS listing 262 are the same as those of the DEC
microcode 130 **[measured]** (2026-11-27, a comparison of the D lines with
`microcode/V130/ks10.mcr` of KS10FPGA, commit 5090e7a1). This agrees with
a ROM that ITS did not change.

### Agreement with the historical listing

The listing and the RAM file of the ITS build of 2026-11-26 agree with the
copies in KS10FPGA (`microcode/ITS/mcr.262` and `ram.262`, commit
5090e7a1) **[measured]** (2026-11-27): the same U and D words, and the same
bytes of the RAM file. KS10FPGA is a reference of rank 4, so this is
support, not proof.

## Field tables

`spec/gen_microcode.py` reads from the ITS microcode sources:

- the fields of the microword (`.UCODE` mode), bit 0 at the left end of
  108 bits (`.WIDTH/108`);
- the fields of the dispatch ROM word (`.DCODE` mode);
- the layout of the RAM file (`.RAMFILE`).

`.RAMFILE` is on line 467 of `ks10.47`, before the field definitions.
MICRO handles it in its second pass, when all fields are known (MICRO,
`$RAMFL`). Thus the generator resolves the names with the fields of the
whole microword. The dispatch ROM has its own `J` (bits 12 to 23); the
microword `J` is bits 0 to 11.
