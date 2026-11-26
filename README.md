# NeoKS10

A KS10-compatible PDP-10 processor in the programmable logic of a PYNQ-Z2.
The peripherals are software on the ARM cores of the same chip. The target
operating system is ITS.

> `NeoKS10` is a working name. The owner has not selected the final name.

This is a design-stage project. No part is built, and no number in this
repository is a measurement unless it has the mark **[measured]**. Interfaces,
file formats and the layout can change.

## What it is

The Zynq-7000 chip of the PYNQ-Z2 has two parts. The programmable logic (PL)
holds the processor. The processing system (PS) runs Linux and an emulator
program, which supplies the memory and all the devices.

| Part | Where | Contents |
|---|---|---|
| Processor | PL | The KS10 instruction set with the behaviour of the ITS microcode: ITS paging, ITS I/O instructions |
| Main memory | PS DDR3 | 512 Ki words, 8 bytes for each 36-bit word (4 MiB) |
| Memory cache | PL block RAM | Write-through, with a write queue to the PS memory |
| Devices | PS, software | Disk (RH11, RP06), tape (RH11), terminal lines (DZ11), Chaosnet (CH11), IMP interface, console |
| Front panel | PL to HDMI | Lamps only. The PL makes the video signal. No switches |
| Operation | PS, software | A console program with a command line and a configuration file |

The processor implements the ITS microcode behaviour only. The paging of
TOPS-10 and of TOPS-20 is not a goal.

## Why these decisions

- **36-bit words do not fit a 32-bit ARM core well.** The PL has DSP slices
  and block RAM that fit a 36-bit data path. Thus the processor is the part
  that gets hardware.
- **Devices are slow and complex.** Software on the PS is sufficient for
  them, and software is easier to change and to test than logic.
- **The emulator is new code.** It does not use the code of SIMH or of KLH10.
  It has its own command line, configuration file and disk image format.
  The other emulators are references for tests only.
- **The software processor comes first.** The emulator includes an
  interpreter for the full instruction set. It makes the system usable
  without the PL, and it is the model that the logic is compared with.

## Components

| Component | Language | Function |
|---|---|---|
| Instruction table | Data file | One definition of each instruction. Decoders, documents and coverage lists are generated from it |
| Register map | Data file | One definition of the PS/PL interface. A SystemVerilog package and Rust constants are generated from it |
| Emulator | Rust | Console, devices, image formats, and the software processor |
| Processor core | SystemVerilog | First a multi-cycle state machine. A pipeline is a later step |
| Memory bridge | SystemVerilog | Cache, write queue, AXI master to the PS memory |
| Panel | SystemVerilog | Lamp brightness and the HDMI signal |
| Test tools | Rust, Verilator, SymbiYosys | Differential tests, co-simulation, mutation, formal proofs |

## Verification

Each level is examined against the level above it. Agreement between two
parts of this project is not a proof that one of them is correct.

1. **External references.** The ITS microcode sources, the DEC manuals, the
   DEC KS10 diagnostics, other emulators, and ITS itself: the system must
   boot and must build itself from source.
2. **The software processor.** It must agree with the external references.
3. **The logic.** It must agree with the software processor at each
   completed instruction in co-simulation.

| When | Tests |
|---|---|
| Each change | Unit tests and a short random differential run |
| Before a merge | The diagnostics, and an ITS boot to the prompt |
| At a milestone, or when a fault is possible | A full ITS build from source |

## Plan

| Step | Result |
|---|---|
| 0 | A reference emulator builds and boots ITS on a host computer. This gives known-good disk images and traces |
| 1 | The Rust emulator with the software processor passes the diagnostics and boots ITS on a host computer |
| 2 | The same emulator runs on the PS of the board |
| 3 | Measurements of the PS/PL memory path: latency of the HP port, uncached mapping, write queue |
| 4 | The processor core agrees with the software processor in co-simulation |
| 5 | The core runs on the board with the devices on the PS |
| 6 | Front panel on HDMI, Chaosnet, IMP interface |

## Sources

The specification is not in this repository. These sources are, in order of
priority:

| Source | Location |
|---|---|
| ITS KS10 microcode sources | [`src/kshack/`](https://github.com/PDP-10/its/tree/master/src/kshack) in PDP-10/its |
| ITS notes on the KS10 | [`doc/kshack/`](https://github.com/PDP-10/its/tree/master/doc/kshack) in PDP-10/its |
| DECsystem-10/DECSYSTEM-20 Processor Reference Manual (June 1982) | [bitsavers](https://bitsavers.org/pdf/dec/pdp10/1982_ProcRefMan.pdf) |
| KS10-Based DECSYSTEM-2020 Technical Manual | [bitsavers](https://bitsavers.org/pdf/dec/pdp10/KS10/EK-OKS10-TM-002_tech_Sep79.pdf) |
| KS10 maintenance schematics | [bitsavers](https://bitsavers.org/pdf/dec/pdp10/KS10/MP00540_KS10_MaintSch_Mar81.pdf) |
| More KS10 documents | [bitsavers, `pdp10/KS10`](https://bitsavers.org/pdf/dec/pdp10/KS10) |
| Board data | [NeoChen1024/PYNQ-Z2](https://github.com/NeoChen1024/PYNQ-Z2) |

Manuals, diagnostics, system images and the output of vendor tools are not
in this repository. `third-party.lock` gives the source and the SHA-256 of
each such file.

The location of the DEC KS10 diagnostics is an open item in
[`docs/unknowns.md`](docs/unknowns.md).

## Related projects

- [PDP-10/its](https://github.com/PDP-10/its): the ITS sources and the
  scripts that build the system in an emulator.
- [KS10FPGA](https://github.com/KS10FPGA/KS10FPGA): a KS10 that runs the DEC
  microcode in an FPGA.
- [aap/fpdpga](https://github.com/aap/fpdpga): the PDP-6 and the KA10 in an
  FPGA.
- [PDP-10/klh10](https://github.com/PDP-10/klh10) and
  [larsbrinkhoff/ka10-simh](https://github.com/larsbrinkhoff/ka10-simh): the
  emulators that this project uses as references.

## Contributing

[`AGENTS.md`](AGENTS.md) has the rules for work in this repository. They
apply to persons and to coding agents.

## License

Not selected.

## Credits

The ITS maintainers of the PDP-10/its project keep the system and its build
alive. bitsavers.org keeps the DEC documents available.
