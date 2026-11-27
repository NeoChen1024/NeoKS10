# NeoKS10

A KS10-compatible PDP-10 processor in the programmable logic of a
[PYNQ-Z2](https://github.com/NeoChen1024/PYNQ-Z2) board, built to run
[ITS](https://github.com/PDP-10/its), the Incompatible Timesharing System.

The processor is the KS10 data path in the FPGA, and it runs the real ITS
microcode. Memory, disks, tapes, terminals and the 8080 console are
software on the ARM cores of the same chip.

## Status

The hardware does not exist yet. The software side works:

- `neoks10`, an emulator in Rust, boots ITS 1652 from the disk of an ITS
  build, and a user can log in.
- It passes the DEC KS10 CPU diagnostics that apply to the ITS microcode
  (`make diag`).
- Random instruction tests compare it with two other emulators, SIMH
  `pdp10-ks` and KLH10. Each fault that they found is fixed or recorded
  in [`docs/discrepancies.md`](docs/discrepancies.md).
- The memory path between the FPGA and the ARM memory is measured on the
  board ([`docs/mem-path-experiment.md`](docs/mem-path-experiment.md)).

Next is a model of the KS10 data path that runs the ITS microcode. The
logic in the FPGA will follow that model. The parts, the plan and the
state of each step are in [`docs/architecture.md`](docs/architecture.md).

## Quick start

You need Rust, Python 3 and the build tools of PDP-10/its. The commands
run on a Linux host.

```sh
git clone --recursive https://github.com/NeoChen1024/NeoKS10
cd NeoKS10

# Build ITS once with the SIMH KS10 emulator. It takes a long time.
make -C 3rdparty/its EMULATOR=pdp10-ks

# Boot it in neoks10, log in and list SYS; (a scripted run).
make its-boot KS10_ITS_DISK=3rdparty/its/out/pdp10-ks/rp0.dsk
```

To use the system yourself, write a small configuration file and start
the operator console:

```sh
cd emu && cargo build --release -p neoks10
cp ../3rdparty/its/out/pdp10-ks/rp0.dsk /tmp/rp0.dsk
printf '[disk.0]\nimage = "/tmp/rp0.dsk"\n' > /tmp/its.toml
target/release/neoks10 run /tmp/its.toml
```

The terminal is the KS10 console terminal (CTY). Ctrl-\ opens the
console prompt. Type `boot disk`, then an empty line to return to the CTY.
At the `DSKDMP` prompt, type `its`, then ESC and `g`. The console commands
are in [`docs/console.md`](docs/console.md).

Other targets: `make check` (formatting, lint, unit tests and the short
differential run), `make diag` (the DEC diagnostics), `make oracle`
(builds the reference emulators).

## Documents

| Document | Contents |
|---|---|
| [`docs/architecture.md`](docs/architecture.md) | Parts, reasons, verification and plan |
| [`docs/emulator.md`](docs/emulator.md) | The design of `neoks10`, the device model, the diagnostics and the differential runs |
| [`docs/microcode-model.md`](docs/microcode-model.md) | The microcode-level model: the microcode of the ITS build, the data path |
| [`docs/mem-path-experiment.md`](docs/mem-path-experiment.md) | The PS/PL memory path: plan and measurements |
| [`docs/discrepancies.md`](docs/discrepancies.md) | Cases where two sources disagree, and the decision |
| [`docs/unknowns.md`](docs/unknowns.md) | Values that no source gives yet |
| [`docs/instructions.md`](docs/instructions.md), [`docs/devices.md`](docs/devices.md), [`docs/regmap.md`](docs/regmap.md), [`docs/console.md`](docs/console.md) | Generated references: instructions, device registers, PS/PL registers, console commands |
| [`AGENTS.md`](AGENTS.md) | The rules for work in this repository, for people and coding agents |

## Sources

The ITS KS10 microcode ([`src/kshack/`](https://github.com/PDP-10/its/tree/master/src/kshack))
defines the behaviour. The DEC KS10 manuals and schematics on
[bitsavers](https://bitsavers.org/pdf/dec/pdp10/KS10) describe the data
path that runs it. [`AGENTS.md`](AGENTS.md) gives the full order of the
sources. Manuals, diagnostics and system images are not in this
repository; [`third-party.lock`](third-party.lock) gives the URL and the
SHA-256 of each, and `make references` downloads them.

## Related projects

- [PDP-10/its](https://github.com/PDP-10/its): the ITS sources and the
  scripts that build the system in an emulator.
- [KS10FPGA](https://github.com/KS10FPGA/KS10FPGA): a KS10 that runs the
  DEC microcode in an FPGA. This project reads it as a reference for the
  data path.
- [PDP-10/klh10](https://github.com/PDP-10/klh10) and
  [larsbrinkhoff/ka10-simh](https://github.com/larsbrinkhoff/ka10-simh):
  the emulators that the differential tests compare with.
- [aap/fpdpga](https://github.com/aap/fpdpga): the PDP-6 and the KA10 in an
  FPGA.

## License

The original work in this repository is under the [MIT License](LICENSE).
Third-party material keeps its own license: the submodules in `3rdparty/`,
the reference emulators that `oracle/` patches, and the files in
`third-party.lock`.

## Credits

The maintainers of PDP-10/its keep ITS and its build alive. bitsavers.org
keeps the DEC documents available.
