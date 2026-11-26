# Experiment: the memory path between the PL and the PS

The sections before "Record of results" are the plan. Each number in them
is an **[estimate]** or a parameter to measure. The results are in "Record
of results".

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

### M1: first version of the test design, 100 MHz, PS idle

**[measured]** 2026-11-26. Design `rtl/memprobe` at commit 6fd9cc11 (the
`BUILD` register gave this value). Linux 6.18.64-pynq-z2. The bitstream was
loaded over JTAG. Conditions: `clk_m` 100 MHz (FCLK0 and `clk_m` measured
100.004 MHz against the monotonic clock of Linux), HP0, 64-bit beats, one
transaction in progress, sequential addresses in the first 4 MiB of the
reserved region, no other load on the PS.

**B9, short run.** Method: `memprobe consistency --words 10000000`. The PS
wrote 10485760 words and the PL checked them: 0 mismatches. The PL wrote
10485760 words and the PS checked them: 0 mismatches. The full B9 needs
10^9 words in each direction; it is not done yet.

**B1 to B3.** Method: `memprobe latency read|write --beats N --count 10000`,
GAP 0. The latency is in `clk_m` cycles (note "Latency" of the register
map); at 100 MHz, one cycle is 10 ns. All data is in
`docs/data/mem-path-m1-latency-100mhz.csv`, and the rates are in
`docs/data/mem-path-m1-rate-100mhz.csv`.

| Measure | Beats | Minimum | Median | 99th percentile | Maximum |
|---|---|---|---|---|---|
| Read, first beat (B1) | 1 | 19 | 19 | 38 | 75 |
| Read, first beat | 16 | 19 | 19 | 36 | 74 |
| Read, last beat (B3) | 16 | 34 | 34 | 51 | 89 |
| Write response (B2) | 1 | 12 | 12 | 12 | 13 |
| Write response (B3) | 16 | 31 | 31 | 31 | 32 |

- The first read beat comes after 19 cycles (190 ns) at the median, for all
  burst lengths. Then one beat comes in each cycle: the median of the last
  beat is 19 + (beats - 1) cycles.
- The write response comes after 12, 13, 16, 21 and 31 cycles at the median
  for 1, 2, 4, 8 and 16 beats: about 1.3 cycles for each more beat. The
  spread is 1 cycle or less.
- **[estimate]** The write response comes before the data is in the DDR
  memory. Basis: the writes have AxCACHE 0011 (bufferable), and the write
  latency does not vary as the read latency does. B6 must show if a read
  can pass such a write.
- With 16-beat writes, the rate was 369.9 MiB/s. This is equal to the
  write rate that the board repository gives for its test design, which
  also has one burst at a time (`docs/pl-interface.md` there, section "HP
  ports"). With 16-beat reads, the rate was 328.2 MiB/s.
- The stall counters count the cycles in which the next transaction waits
  for the one in progress. With one transaction in progress, this is
  nearly all cycles of a run.

### M1: full B9, loads on the PS, and B7

**[measured]** 2026-11-26, Linux 6.18.64-pynq-z2, the same conditions as
above except where a paragraph gives other conditions.

**B9, full run.** Design commit 6fd9cc11, 100 MHz. Method: `memprobe
consistency --words 1000000000`. The PS wrote 1000341504 words and the PL
checked them: 0 mismatches. The PL wrote 1000341504 words and the PS
checked them: 0 mismatches. This supports the decision to map the region
without the CPU cache (B9 in the table of measurements).

**B1 and B2 with loads on the PS.** Design commit 6fd9cc11, 100 MHz, 1-beat
transactions. Method: `memprobe latency read|write --count 100000`, with
the loads below. Data: `docs/data/mem-path-m1-load-100mhz.csv`.

- PS memory load: `--load-threads 2`. Two threads copied between buffers of
  16 MiB: 322 MiB/s during the read run, 245 MiB/s during the write run.
- SD card and network load: `dd` read the card at 22.6 MB/s. `iperf3
  --bidir` from a host on the LAN gave 260 Mbit/s to the board and
  341 Mbit/s from the board.

| Condition | Read median | Read p99 | Read maximum | Write median | Write p99 | Write maximum |
|---|---|---|---|---|---|---|
| PS idle | 19 | 37 | 89 | 12 | 12 | 12 |
| PS memory load | 24 | 64 | 100 | 12 | 12 | 13 |
| SD card and network | 19 | 56 | 102 | 12 | 12 | 32 |

The values are in cycles of 10 ns. The read latency increases with a load
on the PS; the maximum stays near 1 us. The write response does not change
at the median and p99. This agrees with the estimate above that the write
response does not wait for the DDR memory.

**B7, clock of the HP side.** Design commit 09728152 for 50 MHz and
150 MHz (the same `rtl/` as 6fd9cc11), 6fd9cc11 for 100 MHz. PS idle,
10000 transactions for each burst length. The short B9 (10485760 words in
each direction) gave 0 mismatches at each clock. Data:
`docs/data/mem-path-m1-latency-<clock>mhz.csv` and
`docs/data/mem-path-m1-rate-<clock>mhz.csv`.

| `clk_m` | Read, first beat, median | Read p99 (1 beat) | Write response, 1 beat | 16-beat reads | 16-beat writes |
|---|---|---|---|---|---|
| 50 MHz | 13 cycles, 260 ns | 25 cycles, 500 ns | 9 cycles, 180 ns | 199.4 MiB/s | 218.0 MiB/s |
| 100 MHz | 19 cycles, 190 ns | 38 cycles, 380 ns | 12 cycles, 120 ns | 328.2 MiB/s | 369.9 MiB/s |
| 150 MHz | 25 cycles, 167 ns | 53 cycles, 353 ns | 16 cycles, 107 ns | 417.8 MiB/s | 469.5 MiB/s |

- A higher clock gives a shorter latency in ns, but more cycles. The
  rates with one burst at a time increase with the clock.
- **[estimate]** A part of the latency is a number of `clk_m` cycles, and a
  part is a time in the PS. Basis: the read latency in ns changes less than
  the clock period does.
- The 150 MHz build met timing with a setup slack of 0.359 ns only. A
  change of the design can make it fail.
- The 32-bit width of B7 needs the next version of the design.

### B8: access of the PS to the region

**[measured]** 2026-11-26, Linux 6.18.64-pynq-z2. The region is mapped
through `/dev/mem` with `O_SYNC`; the PL was idle. Method: `memprobe
ps-access --threads N --passes 8`, and `memprobe pread --file /dev/mmcblk0
--chunk N --threads N`. Each thread has its own part of the first 4 MiB.
Data: `docs/data/mem-path-b8.csv`.

| Access | 1 thread | 2 threads |
|---|---|---|
| 32-bit read | 132.9 ns, 28.7 MiB/s | 135.6 ns, 56.3 MiB/s |
| 64-bit read | 132.6 ns, 57.5 MiB/s | 140.8 ns, 108.4 MiB/s |
| 32-bit write | 26.5 ns, 144.0 MiB/s | 26.5 ns, 287.6 MiB/s |
| 64-bit write | 28.3 ns, 269.5 MiB/s | 28.4 ns, 538.2 MiB/s |
| `pread` 1 KiB, page cache | 5.5 us, 176.6 MiB/s | 6.3 us, 312.0 MiB/s |
| `pread` 64 KiB, page cache | 146 us, 427.2 MiB/s | 232 us, 537.8 MiB/s |
| `pread` 1 KiB, SD card | 45 us, 21.7 MiB/s | not measured |
| `pread` 64 KiB, SD card | 2.9 ms, 21.7 MiB/s | not measured |

The times are for one access or one `pread` of one thread; the rates are
for all threads together.

- A read of one word takes about 133 ns, for 32 bits and for 64 bits. The
  time of a guest word read by the PS is thus that of one 64-bit read. Two
  threads nearly double the rate.
- A write takes about 27 ns: the CPU does not wait for the memory.
- A `pread` of data in the page cache gives 177 MiB/s with 1 KiB blocks
  (one PDP-10 disk sector of 128 words at 8 bytes) and 427 MiB/s with
  64 KiB blocks.
- From the SD card, `pread` gives 21.7 MiB/s for both block sizes. This is
  near the read rate of the card in `docs/sd-card.md` of the board
  repository: the card limits it, not the copy into the region.
- The data of a `pread` into the region was checked at four offsets of the
  partition table of the card, with `devmem2`: equal.

Not done yet: B4 to B6. They need more than one transaction in progress,
mixed operations, HP1 and the order test: the next version of the design.
