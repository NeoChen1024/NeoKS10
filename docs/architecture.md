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
| Processor | PL | The KS10 data path. It runs the microcode of the ITS build. Later, a second core: a pipelined PDP-10 of its own design ([`conformance.md`](conformance.md)) |
| Console | PS, software | A high-level model of the 8080 console: the console words, the boot, examine, deposit, halt and continue ([`console-interface.md`](console-interface.md)). It does not run the 8080 program, and it has no time base of its own. It reaches the processor through the console registers of the PS/PL interface |
| Main memory | PS DDR3 | 512 Ki words **[its]** (`TSYSM==512.`, machine `DB` of the ITS build), 8 bytes for each 36-bit word (4 MiB) |
| Memory bridge | PL | A cache in block RAM and a write queue to the PS memory. The measurements of [`mem-path-experiment.md`](mem-path-experiment.md) decide its size and its write policy |
| Devices | PS, software | Disk (RH11, RP06), tape (RH11, TM03), terminal lines (DZ11), Chaosnet (CH11), IMP interface |
| Front panel | PL to HDMI | Lamps only. The PL makes the video signal. It has no switches and does not change the processor state |
| Operation | PS, software | The operator console of `neoks10`: a command line, a configuration file, a control socket and scripts. Its escape character is Ctrl-\. The functions of the prompt `KS10>` of the 8080 program (`msg.m80`, `KSPRMT`) **[its]** are commands of the operator console |

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
- **The real microcode is the reference of the first core.** A model of
  the KS10 data path runs the microcode of the ITS build. The logic of the
  first core is compared with this model. Outside the PRM and the DEC
  diagnostics, the behaviour is implementation-defined; the model gives
  the behaviour of one configuration ("Target").
- **The real console program decides questions about the console.** The
  system in daily use has a high-level console. A model of the 8080 that
  runs the DEC console program is for tests only, with the
  microcode-level model. It records the effects that the high-level
  console must give ([`console-interface.md`](console-interface.md)).

## Components

| Component | Language | Function |
|---|---|---|
| Instruction table | Data file | One definition of each instruction. Decoders, documents and coverage lists come from it |
| Device register table | Data file | One definition of each I/O device register |
| Register map | Data file | One definition of the PS/PL interface. A SystemVerilog package and Rust constants come from it |
| Emulator | Rust | Console, devices, image formats, and the software processor |
| Microcode-level model | Rust | `emu/ks10-core-uop`: the data path and the assembled ITS microcode ([`microcode-model.md`](microcode-model.md)). For tests, a model of the 8080 that runs the DEC console program (not written yet) |
| Implementation differences | Data file | `spec/impldef.toml`: one case for each behaviour in which the implementations differ, with the choice of each core ([`impldef.md`](impldef.md)) |
| Processor core | SystemVerilog | The KS10 data path. It is multi-cycle |
| Pipelined core | SystemVerilog | Later: a PDP-10 of its own design that runs ITS. It follows levels A and B of [`conformance.md`](conformance.md), not the microcode steps |
| Memory bridge | SystemVerilog | Cache, write queue, AXI master to the PS memory |
| Panel | SystemVerilog | Lamp brightness and the HDMI signal |
| Test tools | Rust, Verilator, SymbiYosys | Differential tests, co-simulation, mutation, formal proofs |

## Target

The microcode-level model and the first core follow one configuration of
the machine:

- The microcode is that of the ITS build, `KSHACK; MCR 262`
  ([`microcode-model.md`](microcode-model.md)).
- The data path follows the drawings of MP00540 (KS10 maintenance
  schematics, March 1981). MP00540 has two revisions of several sheets,
  for example DPM5, DPM6, DPMC, MMC5, DPE9 and CRA2. The code and the
  documents cite these revisions: CSL4 rev C, CSL5 rev D, DPE1, DPE2,
  DPE5 and DPEA rev A1, DPE3, DPE6 and DPEB rev B, DPM1 to DPM5, DPM7,
  DPM9, DPMA and DPMB rev B, DPM6 rev C, DPMC rev D, CRA1 rev A1, MMC5
  rev B. They cite both revisions of DPE9 and of CRA2. A sheet that they
  cite without a revision is not in this list. The CRA1 sheet on page
  172 of MP00540 is rev B ([`unknowns.md`](unknowns.md)).

Behaviour outside the PRM and the DEC diagnostics is
implementation-defined ([`conformance.md`](conformance.md)). For the
microcode-level model, this project selects the behaviour of this
configuration.

## Verification

Each level is examined against the level above it. If two parts of this
project agree, that is not a proof that one of them is correct.

1. **External references.** The ITS microcode sources, the DEC manuals and
   schematics, the DEC KS10 diagnostics, the emulators SIMH `pdp10-ks` and
   KLH10, KS10FPGA, and ITS itself: the system must boot and must build
   itself from source. [`AGENTS.md`](../AGENTS.md) gives their order.
2. **The microcode-level model** (`ks10-core-uop`). It runs the microcode
   on a model of the data path. It is the reference of the first core: the
   co-simulation compares them at each microcycle, exactly.
3. **The software processor** (`ks10-core-soft`). It is the reference of
   the pipelined core: they are compared at each instruction, by the
   contract of [`conformance.md`](conformance.md).
4. **The two models.** They are compared at each instruction. Each
   difference is a case of level B in [`impldef.md`](impldef.md), or an
   item of level C. The masks of the comparisons come from
   `spec/impldef.toml` through a generator.
5. **The console.** The high-level console gives the same effects on the
   KS10 side as the DEC console program in the 8080 model, except the
   differences that [`console-interface.md`](console-interface.md) lists.

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
| 4 | The microcode-level model, then the processor core, which agrees with it in co-simulation | The model runs the microcode that `spec/micro.py` assembles. It passes the DEC diagnostics that apply to the ITS microcode, and boots ITS to a login. It gives a record at each instruction, through a C interface for the co-simulation. Next: a record at each microcycle for the co-simulation, the model of the 8080 for tests, and the processor core ([`microcode-model.md`](microcode-model.md)) |
| 5 | The core runs on the board with the devices on the PS | Not started |
| 6 | Front panel on HDMI, Chaosnet, IMP interface | Not started |
| 7 | The pipelined core ([`conformance.md`](conformance.md)) | Not started |

Goals for the speed (not measurements):

- Each core is faster than a real KS10. The instruction times of the
  KS10 have no source yet ([`unknowns.md`](unknowns.md)).
- The pipelined core runs at more than 4.5 MIPS on a workload of this
  project, for example an assembly of a fixed source with MIDAS under
  ITS. 4.5 MIPS is the figure of the English Wikipedia article "Foonly"
  for the Foonly F1 (revision 1373290044, read 2026-11-28). The article
  names no benchmark.
