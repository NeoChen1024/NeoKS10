# Architecture and plan

This document gives the parts of NeoKS10, the reasons for the division,
the levels of verification, and the order of the work. The emulator
program has its own design in [`emulator.md`](emulator.md).

## Parts

The Zynq-7000 chip of the PYNQ-Z2 has two parts. The programmable logic
(PL) holds the processor. The processing system (PS) runs Linux and the
program `neoks10`, which supplies the memory and all the devices.

| Part | Where | Contents |
|---|---|---|
| Processor | PL | The KS10 data path. It runs the microcode of the ITS build |
| Console processor | PS, software | The DEC 8080 console program, in a model of the 8080. It reaches the processor through the console registers of the PS/PL interface |
| Main memory | PS DDR3 | 512 Ki words **[its]** (`TSYSM==512.`, machine `DB` of the ITS build), 8 bytes for each 36-bit word (4 MiB) |
| Memory bridge | PL | A cache in block RAM and a write queue to the PS memory. The measurements of [`mem-path-experiment.md`](mem-path-experiment.md) decide its size and its write policy |
| Devices | PS, software | Disk (RH11, RP06), tape (RH11, TM03), terminal lines (DZ11), Chaosnet (CH11), IMP interface |
| Front panel | PL to HDMI | Lamps only. The PL makes the video signal. It has no switches and does not change the processor state |
| Operation | PS, software | The operator console of `neoks10`: a command line, a configuration file, a control socket and scripts |

The processor implements the behaviour of the ITS microcode: ITS paging
and the ITS I/O instructions. The paging of TOPS-10 and TOPS-20 is not a
goal.

The PS maps the guest memory without the CPU cache. Thus the PS and the PL
see the same data without cache maintenance.

## Reasons for the division

- **A 36-bit word does not fit a 32-bit ARM core well.** The PL has DSP
  slices and block RAM that fit a 36-bit data path. Thus the processor is
  the part that gets hardware.
- **Devices are slow and complex.** Software on the PS is sufficient for
  them. Software is easier to change and to test than logic.
- **The emulator is new code.** It does not use the code of SIMH or KLH10.
  It has its own command line, configuration file and disk image format.
  The other emulators are references for tests only.
- **The software processor comes first.** The emulator has an interpreter
  for the full instruction set. It makes the system usable without the PL.
- **The real microcode is the reference.** A model of the KS10 data path
  runs the microcode of the ITS build and the 8080 console program. If the
  sources disagree about an instruction, this model decides. The logic is
  compared with this model.

## Components

| Component | Language | Function |
|---|---|---|
| Instruction table | Data file | One definition of each instruction. Decoders, documents and coverage lists come from it |
| Device register table | Data file | One definition of each I/O device register |
| Register map | Data file | One definition of the PS/PL interface. A SystemVerilog package and Rust constants come from it |
| Emulator | Rust | Console, devices, image formats, and the software processor |
| Microcode-level model | Rust | `emu/ks10uop`: the data path, the assembled ITS microcode, and the 8080 model ([`microcode-model.md`](microcode-model.md)) |
| Processor core | SystemVerilog | The KS10 data path. It is multi-cycle. A pipeline is a later step |
| Memory bridge | SystemVerilog | Cache, write queue, AXI master to the PS memory |
| Panel | SystemVerilog | Lamp brightness and the HDMI signal |
| Test tools | Rust, Verilator, SymbiYosys | Differential tests, co-simulation, mutation, formal proofs |

## Verification

Each level is examined against the level above it. If two parts of this
project agree, that is not a proof that one of them is correct.

1. **External references.** The ITS microcode sources, the DEC manuals and
   schematics, the DEC KS10 diagnostics, the emulators SIMH `pdp10-ks` and
   KLH10, KS10FPGA, and ITS itself: the system must boot and must build
   itself from source. [`AGENTS.md`](../AGENTS.md) gives their order.
2. **The microcode-level model.** It runs the microcode and the 8080
   program on a model of the data path.
3. **The software processor.** It must agree with the microcode-level
   model and with the external references.
4. **The logic.** It must agree with the microcode-level model at each
   completed instruction in co-simulation.

| When | Tests |
|---|---|
| Each change | Unit tests and a short random differential run against SIMH and KLH10 |
| Before a merge | The diagnostics, and an ITS boot to a login |
| At a milestone, or when a fault is possible | A full ITS build from source |

## Plan

| Step | Result | State |
|---|---|---|
| 0 | A reference emulator builds and boots ITS on a host computer. This gives known-good disk images and traces | Done |
| 1 | The Rust emulator with the software processor passes the diagnostics and boots ITS on a host computer | Done ([`emulator.md`](emulator.md), "Device model" and "Diagnostics") |
| 2 | The same emulator runs on the PS of the board | It builds for the board and passes its tests in qemu-arm; no run on the board yet |
| 3 | Measurements of the PS/PL memory path: latency of the HP ports, uncached mapping, write queue | The path is measured (Part B of [`mem-path-experiment.md`](mem-path-experiment.md)); the demand of the guest (Parts A and C) is open |
| 4 | The microcode-level model, then the processor core, which agrees with it in co-simulation | The microcode of the ITS build loads and is checked; the data path model is next |
| 5 | The core runs on the board with the devices on the PS | Not started |
| 6 | Front panel on HDMI, Chaosnet, IMP interface | Not started |
