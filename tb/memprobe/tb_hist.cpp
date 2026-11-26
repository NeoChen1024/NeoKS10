// Verilator testbench of memprobe_hist (rtl/memprobe/memprobe_hist.sv): one
// latency histogram with its banks, FIFOs, statistics and read port. The
// Makefile builds it with 4 banks (the design) and with 1 bank, so that a
// sample in each cycle also fills a FIFO.
//
// The expected values come from a model in this file:
//   - At one sample in 4 cycles or less, the FIFO never fills: each bin must
//     be exact.
//   - At one sample per cycle: with 4 banks, each bin must be exact. With 1
//     bank the FIFO fills: the statistics (MIN, MAX, SUM) must still be exact,
//     each bin must be at most the model value, and the missing samples must be
//     equal to the drop counter and the drop pulses.
//   - CLEAR sets everything to 0 (MIN to 0xFFFFFFFF).
// The read port has its own clock.
//
// Usage: tb_hist --banks N [--seed N]   (N: the BANKS_LOG2 of the build, as 2^N)

#include <cstdint>
#include <cstdio>
#include <cstdlib>
#include <cstring>
#include <vector>

#include "Vmemprobe_hist.h"
#include "verilated.h"

static int g_failures = 0;
static const char* g_step = "start";

#define CHECK(cond, ...)                                                                 \
    do {                                                                                 \
        if (!(cond)) {                                                                   \
            std::fprintf(stderr, "FAIL [%s] %s:%d: %s: ", g_step, __FILE__, __LINE__, #cond); \
            std::fprintf(stderr, __VA_ARGS__);                                           \
            std::fprintf(stderr, "\n");                                                  \
            if (++g_failures > 20) std::exit(1);                                         \
        }                                                                                \
    } while (0)

struct Rng {
    uint64_t s;
    explicit Rng(uint64_t seed) : s(seed * 0x9E3779B97F4A7C15ull + 0x7654321) {}
    uint64_t next() {
        s ^= s >> 12;
        s ^= s << 25;
        s ^= s >> 27;
        return s * 0x2545F4914F6CDD1Dull;
    }
    uint32_t below(uint32_t n) { return uint32_t(next() % n); }
};

static constexpr int BINS = 1024;

struct Model {
    std::vector<uint64_t> bins = std::vector<uint64_t>(BINS, 0);
    uint32_t min = 0xFFFFFFFF, max = 0;
    uint64_t sum = 0, samples = 0;
    void add(uint32_t v) {
        bins[v >= BINS - 1 ? BINS - 1 : v]++;
        if (v < min) min = v;
        if (v > max) max = v;
        sum += v;
        samples++;
    }
};

struct Tb {
    VerilatedContext ctx;
    Vmemprobe_hist* d = new Vmemprobe_hist(&ctx);
    Rng rng;
    Model m;
    uint64_t drop_pulses = 0;

    explicit Tb(uint64_t seed) : rng(seed) {}
    ~Tb() {
        d->final();
        delete d;
    }

    void tick() {
        d->clk = 1;
        d->eval();
        if (d->drop) drop_pulses++;
        d->clk = 0;
        d->eval();
    }
    uint32_t value() {
        switch (rng.below(8)) {
        case 0: return rng.below(4);
        case 1: return 1020 + rng.below(8);           // around the last bin
        case 2: return uint32_t(rng.next());          // any value
        default: return rng.below(1100);
        }
    }
    // A sample in this cycle, or none.
    void cycle(bool sample) {
        d->smp_valid = sample;
        if (sample) {
            d->smp_value = value();
            m.add(d->smp_value);
        }
        tick();
        d->smp_valid = 0;
    }
    void wait_idle() {
        for (int i = 0; i < 5000 && d->busy; i++) tick();
        CHECK(!d->busy, "busy does not end");
    }
    // rd_data is three cycles of rd_clk after rd_addr.
    uint32_t read_bin(unsigned b) {
        d->rd_addr = b;
        for (int i = 0; i < 3; i++) {
            d->rd_clk = 1;
            d->eval();
            d->rd_clk = 0;
            d->eval();
        }
        return d->rd_data;
    }
    void check_stats() {
        CHECK(d->min == m.min, "MIN %u, expected %u", d->min, m.min);
        CHECK(d->max == m.max, "MAX %u, expected %u", d->max, m.max);
        CHECK(d->sum == m.sum, "SUM %llu, expected %llu", (unsigned long long)d->sum, (unsigned long long)m.sum);
    }
    void check_exact() {
        check_stats();
        CHECK(d->drops == 0, "drops %u", d->drops);
        for (unsigned b = 0; b < BINS; b++) {
            uint32_t v = read_bin(b);
            CHECK(v == m.bins[b], "bin %u: %u, expected %llu", b, v, (unsigned long long)m.bins[b]);
        }
    }
    void check_with_drops() {
        check_stats();
        uint64_t missing = 0;
        for (unsigned b = 0; b < BINS; b++) {
            uint32_t v = read_bin(b);
            CHECK(v <= m.bins[b], "bin %u: %u, more than %llu", b, v, (unsigned long long)m.bins[b]);
            missing += m.bins[b] - v;
        }
        CHECK(missing == d->drops, "missing samples %llu, drops %u", (unsigned long long)missing, d->drops);
        CHECK(drop_pulses == d->drops, "drop pulses %llu, drops %u", (unsigned long long)drop_pulses, d->drops);
    }
    void clear() {
        d->clear = 1;
        tick();
        d->clear = 0;
        CHECK(d->busy, "not busy during CLEAR");
        wait_idle();
        m = Model();
        drop_pulses = 0;
        check_exact();
    }
};

int main(int argc, char** argv) {
    uint64_t seed = 1;
    unsigned banks = 0;
    for (int i = 1; i < argc; i++) {
        if (!std::strcmp(argv[i], "--seed") && i + 1 < argc) seed = std::strtoull(argv[++i], nullptr, 0);
        else if (!std::strcmp(argv[i], "--banks") && i + 1 < argc) banks = unsigned(std::strtoul(argv[++i], nullptr, 0));
        else { std::fprintf(stderr, "usage: tb_hist --banks N [--seed N]\n"); return 2; }
    }
    if (banks != 1 && banks != 4) { std::fprintf(stderr, "--banks must be 1 or 4\n"); return 2; }
    Tb t(seed);

    g_step = "reset";
    t.d->rst = 1;
    for (int i = 0; i < 4; i++) t.tick();
    t.d->rst = 0;
    t.tick();
    t.check_exact();

    g_step = "low rate";
    for (int i = 0; i < 3000; i++) {
        t.cycle(true);
        for (unsigned k = 3 + t.rng.below(6); k > 0; k--) t.cycle(false);
    }
    t.wait_idle();
    t.check_exact();

    g_step = "same bin in a row";
    for (int i = 0; i < 200; i++) {
        t.d->smp_valid = 1;
        t.d->smp_value = 7;
        t.m.add(7);
        t.tick();
        t.d->smp_valid = 0;
        t.cycle(false);
        t.cycle(false);
        t.cycle(false);
    }
    t.wait_idle();
    t.check_exact();

    g_step = "clear";
    t.clear();

    g_step = "one sample each cycle";
    for (int burst = 0; burst < 20; burst++) {
        for (unsigned k = 5 + t.rng.below(60); k > 0; k--) t.cycle(true);
        for (unsigned k = t.rng.below(40); k > 0; k--) t.cycle(false);
    }
    t.wait_idle();
    if (banks == 4) {
        t.check_exact();
    } else {
        CHECK(t.d->drops > 0, "no drop at one sample each cycle");
        t.check_with_drops();
    }

    g_step = "clear after drops";
    t.clear();

    if (g_failures) {
        std::printf("seed %llu: %d failures\n", (unsigned long long)seed, g_failures);
        return 1;
    }
    std::printf("seed %llu, %u banks: pass\n", (unsigned long long)seed, banks);
    return 0;
}
