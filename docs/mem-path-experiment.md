# Experiment: the memory path between the PL and the PS

This document is a plan. It contains no result. Each number in it is an
**[estimate]** or a parameter to measure.

## Purpose

The processor is in the PL. The guest memory is in the PS DDR3. The PL has a
write-through cache with a write queue (README.md). Before the design of the
cache, these questions need answers:

1. How many reads and writes does the guest make, and where?
2. How much time does one read and one write through an HP port take?
3. How many writes for each second can the path accept continuously?
4. Does the path keep the order of a write and a later read of the same word?
5. How fast can the PS program use the same memory through a non-cached
   mapping?

The answers give the cache size, the line size, the depth of the write
queue, and the data width of the HP port. They also show if a write-through
cache is sufficient.

The experiment has two parts. Part A uses software only. Part B uses the
board, and it needs no PDP-10 logic.

## Conditions

- Part A needs a reference emulator that boots ITS, with a hook that writes
  a memory trace (`oracle/`).
- Part B needs the PS/PL interface of the board repository: the reserved
  memory region, the UIO node and the PL clocks. See `docs/pl-interface.md`
  in <https://github.com/NeoChen1024/PYNQ-Z2>.
- The reserved region is larger than the guest memory. The tests use the
  first 4 MiB of it, the size of 512 Ki words at 8 bytes for each word.

## Part A: the demand of the guest

Record a trace of each guest memory access: address, read or write, and the
number of the instruction. Record these workloads:

| Workload | Reason |
|---|---|
| ITS boot to the prompt | Start of the system, much I/O |
| Idle system, 60 s | The background load |
| An assembly with MIDAS | Typical processor work |
| A part of the ITS build | Many processes and much paging |
| One DEC diagnostic | Unusual access patterns |

From each trace, calculate:

- Reads and writes for each instruction.
- The longest sequence of writes with no read between them.
- The number of different words and of different pages in windows of 10^3,
  10^5 and 10^7 instructions.
- The miss rate of a cache model for each combination of these parameters:

| Parameter | Values |
|---|---|
| Cache size | 4 Ki, 16 Ki, 64 Ki words |
| Line size | 1, 4, 8 words |
| Associativity | Direct-mapped, 2-way |

The cache model is a small program. Keep it in `tb/` with a test of its own.

## Part B: the supply of the path

### Test design

The PL design is `rtl/memprobe/`, and a program on the PS controls it. The
register map is the block `memprobe` of `spec/regmap.toml` (`docs/regmap.md`).

- The registers on M_AXI_GP0 use FCLK0 (100 MHz). They answer also when the
  MMCM is not locked.
- An MMCM makes `clk_m` from FCLK0 for the HP side. A bitstream has one
  frequency: 50, 100 or 150 MHz (B7). `CAPS.CLK_MHZ` gives it.
- A traffic generator is an AXI3 master on an HP port. Registers set the
  operation, the address pattern, the burst length, the span, the gap
  between transactions, and the number of transactions in progress.
- The bus address is the base of the reserved region plus an offset of
  24 bits. Thus no configuration can make an access outside the region.
- Timers count `clk_m` cycles from the address handshake to the first R
  beat, to RLAST, and to B (note "Latency" of the register map).
- Three histograms of 1024 bins in block RAM, with minimum, maximum and sum.
- A data checker with the pattern of the note "Data pattern".
- Counters: transactions, stall cycles, response errors, data errors.

If the MMCM loses the lock during a run, the domain of `clk_m` goes into
reset, and the run ends without DONE. The HP port can then keep a
transaction that the design started. Thus the PS program loads the PL again
after a loss of the lock.

The first version has one transaction in progress, sequential addresses,
64-bit beats and HP0 only. `CAPS` shows the features of a bitstream. The PS
program must not start a measurement that needs a feature that `CAPS` does
not have.

Verification of the test design (`tb/memprobe/`):

- `tb_core` simulates the design with an AXI slave model, random latencies,
  injected errors and two clocks with different periods. The model computes
  the expected histograms and counters from its own record of the handshakes.
- `tb_hist` tests one histogram with a full FIFO.
- `gen.sby` and `regs.sby` prove the AXI rules of the HP master and of the
  GP0 slave. The proofs include: each access is in the span, and each GP0
  request gets its answer.

### Procedure on the board

Each step on the board needs the permission of the owner in that session.

1. Build the bitstream from a commit with no changes: `make synth`. The
   `BUILD` register then identifies the design.
2. Build the program: `make cross`. The program is `emu/memprobe`.
3. Load the bitstream with `jtag/load.sh` of the board repository, and copy
   the program to the board.
4. As root, run `memprobe info`. It checks that the PL is configured and
   that FCLK0 runs before it reads a register. Then it checks the ID, the
   clocks and the reserved region.
5. Run `memprobe consistency --words 10000000` (a short B9). Continue only
   if it gives 0 mismatches.
6. Run `memprobe latency read` and `memprobe latency write` (B1, B2), and
   with `--beats 2`, 4, 8 and 16 (B3).
7. Put the output into `docs/data/` and the results section of this file.


| Number | Measurement | Variations |
|---|---|---|
| B1 | Latency of a 1-beat read | PS idle; PS with memory load; PS with SD card and network load |
| B2 | Latency of a 1-beat write | The same |
| B3 | Latency of a burst read to the first beat and to the last beat | 1, 2, 4, 8, 16 beats |
| B4 | Continuous write rate, random addresses in the 4 MiB | 1, 2, 4, 8 transactions in progress |
| B5 | Continuous rate of mixed reads and writes | Read share 50%, 80%, 95% |
| B6 | Order of a write and an immediate read of the same word, with no wait for the write response | Same port; read and write on two ports |
| B7 | B1 to B5 again | Data width 32 bits and 64 bits; PL clock 50, 100 and 150 MHz |
| B8 | PS access to the region through the non-cached mapping: time for each word read, each word write, and the rate of `pread` from a file into the region | One core; both cores |
| B9 | Consistency: the PS writes and the PL reads, then the PL writes and the PS reads | 10^9 words each direction, expected mismatches: 0 |

Notes:

- Give the latency as minimum, median, 99th percentile and maximum, in clock
  cycles and in nanoseconds.
- B6 decides a design rule. If the path does not keep the order, the cache
  must hold a read until the write to the same word has its response.
- B8 limits the device models. Compare the `pread` rate with the read rate
  of the SD card that the board repository gives.
- B9 is the proof for the decision to map the region without the CPU cache.
- If B1 or B8 is not sufficient, examine the ACP port as an alternative.
  That is a separate experiment.

## Part C: the two parts together

Run the traces of Part A through a model of the write queue. Use the write
latency and the write rate of Part B as the service time. Calculate for
queue depths of 4, 8, 16 and 32 entries:

- The share of processor cycles that the processor stops for a full queue.
- The share of cycles that it stops for a cache miss (from B3 and the miss
  rate of Part A).

## Decisions that the results give

| Decision | Input |
|---|---|
| Cache size, line size, associativity | Part A, B3, block RAM that remains for the core |
| Depth of the write queue | Part C |
| Data width and clock of the HP port | B7 |
| One HP port, or one for reads and one for writes | B5, B6 |
| Write-through cache, or a different design | Part C: the stop time must be small. The owner sets the limit when the numbers exist |
| Non-cached mapping on the PS, or the ACP port | B8, B9 |

## Record of results

Put the results into this file, below this section, when they exist. Each
result has the mark **[measured]** with the date, the commit of the test
design, the kernel version, and the method. Small tables of data go to
`docs/data/`. Raw traces stay outside the repository.

Do not replace a planned value with a result silently. If a result
contradicts an estimate in README.md or in a design note, correct that text
in the same commit.
