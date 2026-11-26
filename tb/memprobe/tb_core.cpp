// Verilator testbench of memprobe_core (rtl/memprobe/), the memory path test design.
//
// The testbench has its own models, and computes the expected results from
// them, not from the design:
//   - Hp: an AXI3 slave for S_AXI_HP0 with a 16 MiB memory, random ready
//     signals, random latencies, gaps between R beats, and injected SLVERR. It
//     checks the AXI rules and records the edge of each handshake. From these
//     edges it computes the histograms, statistics and counters that the
//     design must give (spec/regmap.toml, notes "Latency" and "Data pattern").
//   - Gp: an AXI3 master for M_AXI_GP0 that the test steps use. Each request
//     must have its answer in a limited time.
//   - pattern(): the data pattern, checked against the recorded values in
//     tests/memprobe-pattern.txt before the tests start.
// The two clocks have different periods and a random phase.
//
// Usage: tb_core --pattern FILE [--seed N]
// Exit status 0 if all checks pass.

#include <algorithm>
#include <cstdint>
#include <cstdio>
#include <cstdlib>
#include <cstring>
#include <deque>
#include <fstream>
#include <functional>
#include <string>
#include <utility>
#include <vector>

#include "Vmemprobe_core.h"
#include "verilated.h"

#include "../gen/regmap.h"

// ---- Utilities -------------------------------------------------------------------------

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

#define FATAL(...)                                                                       \
    do {                                                                                 \
        std::fprintf(stderr, "FATAL [%s] %s:%d: ", g_step, __FILE__, __LINE__);          \
        std::fprintf(stderr, __VA_ARGS__);                                               \
        std::fprintf(stderr, "\n");                                                      \
        std::exit(1);                                                                    \
    } while (0)

struct Rng {
    uint64_t s;
    explicit Rng(uint64_t seed) : s(seed * 0x9E3779B97F4A7C15ull + 0x1234567) {}
    uint64_t next() {
        s ^= s >> 12;
        s ^= s << 25;
        s ^= s >> 27;
        return s * 0x2545F4914F6CDD1Dull;
    }
    uint32_t below(uint32_t n) { return n ? uint32_t(next() % n) : 0; }
    bool chance(double p) { return double(next() >> 11) * (1.0 / 9007199254740992.0) < p; }
};

static uint32_t pattern(uint32_t seed, uint32_t off) {
    uint32_t w = off >> 2;
    return seed ^ w ^ (w << 11) ^ (w << 22);
}

static uint64_t pattern64(uint32_t seed, uint32_t off) {
    return uint64_t(pattern(seed, off)) | (uint64_t(pattern(seed, off + 4)) << 32);
}

static void check_pattern_file(const char* path) {
    std::ifstream f(path);
    if (!f) FATAL("cannot open %s", path);
    std::string line;
    int n = 0;
    while (std::getline(f, line)) {
        if (line.empty() || line[0] == '#') continue;
        unsigned seed, off, val;
        if (std::sscanf(line.c_str(), "%x %x %x", &seed, &off, &val) != 3) FATAL("bad line: %s", line.c_str());
        CHECK(pattern(seed, off) == val, "pattern(%08x, %08x) = %08x, file %08x", seed, off,
              pattern(seed, off), val);
        n++;
    }
    CHECK(n > 0, "no values in %s", path);
}

static constexpr uint32_t HP_BASE = MEMPROBE_HP_BASE;
static constexpr uint32_t HP_SIZE = MEMPROBE_HP_SIZE;
static constexpr int BINS = MEMPROBE_HIST_RD_FIRST_DEPTH;
enum { H_RD_FIRST, H_RD_LAST, H_WR_RESP };
static const uint32_t HIST_OFFSET[3] = {MEMPROBE_HIST_RD_FIRST_OFFSET, MEMPROBE_HIST_RD_LAST_OFFSET,
                                        MEMPROBE_HIST_WR_RESP_OFFSET};
static const uint32_t STAT_OFFSET[3] = {MEMPROBE_RD_FIRST_MIN_OFFSET, MEMPROBE_RD_LAST_MIN_OFFSET,
                                        MEMPROBE_WR_RESP_MIN_OFFSET};

static uint32_t field(uint32_t v, unsigned lsb, uint32_t mask) { return (v & mask) >> lsb; }
#define FIELD(v, reg, f) field((v), MEMPROBE_##reg##_##f##_LSB, MEMPROBE_##reg##_##f##_MASK)
#define BIT(reg, f) (1u << MEMPROBE_##reg##_##f##_LSB)

// ---- Expected statistics -----------------------------------------------------------------

struct Hist {
    std::vector<uint32_t> bins = std::vector<uint32_t>(BINS, 0);
    uint32_t min = 0xFFFFFFFF, max = 0;
    uint64_t sum = 0;
    void add(uint32_t v) {
        bins[v >= uint32_t(BINS - 1) ? BINS - 1 : v]++;
        if (v < min) min = v;
        if (v > max) max = v;
        sum += v;
    }
};

// ---- HP slave model (domain of clk_m) ---------------------------------------------------

struct HpConfig {
    double p_arready = 0.7, p_awready = 0.7, p_wready = 0.8, p_rbubble = 0.2;
    double p_rerr = 0.0, p_berr = 0.0;
    uint32_t lat_min = 3, lat_span = 20;   // latency of the first beat or of B
    double p_long = 0.0;                   // chance of a latency of 1000 to 1199 edges
    double p_switch = 0.3;                 // chance to go to the R beats of another ID
};

// The model takes transactions with any IDs, answers them in any order across
// IDs, and interleaves the R beats of different IDs (AXI3 allows both). It
// checks the rules of spec/regmap.toml (note "Transactions in progress") and
// computes the expected results from its own record of the handshakes.
struct Hp {
    Vmemprobe_core* d;
    Rng& rng;
    HpConfig cfg;
    std::vector<uint8_t> mem = std::vector<uint8_t>(HP_SIZE);
    uint64_t n = 0;   // number of the next edge of clk_m

    struct Rd { unsigned id; uint32_t off; unsigned len; uint64_t t_hs, ready_at; unsigned beat; };
    struct Wr { unsigned id; uint32_t off; unsigned len; uint64_t t_hs; };
    struct WBeat { uint64_t data; bool last; unsigned id; };
    struct B { unsigned id; uint64_t t_aw, ready_at; };
    std::vector<Rd> rds;              // after AR, until RLAST
    std::deque<Wr> aws;               // after AW, waiting for W beats, in AW order
    std::deque<WBeat> wbeats;         // W beats not yet matched to an AW
    std::vector<B> bs;                // complete writes, until B
    int r_cur = -1, b_cur = -1;       // ID of the R beat or B on the bus, or -1

    // Expected results since the last clear().
    Hist hist[3];
    uint64_t reads = 0, writes = 0, stalls = 0;
    uint32_t resp_errors = 0, data_errors = 0, first_err_addr = 0;
    uint64_t first_err_data = 0;
    bool err_seen = false;
    // The run; the test sets it.
    unsigned gap = 0, outstanding = 1;
    uint64_t total = 0;
    uint32_t span_lo = 0, span_hi = HP_SIZE;   // offsets
    bool check_on = false;
    uint32_t check_seed = 0;
    bool exact_stalls = true;
    // State of the run.
    uint64_t issued = 0, last_addr_hs = 0;
    unsigned inflight = 0;
    bool any_addr_hs = false;
    // Coverage: the most transactions in progress, and R beats of a read
    // between the beats of another read.
    unsigned max_inflight = 0;
    // RUN_CYCLES: the design is busy from START to the end of its pipeline.
    // The model knows the edges from the first address handshake to the last
    // completion of each run; the sum of them is a lower bound.
    uint64_t run_first = 0, run_last = 0, active_sum = 0, runs = 0;
    uint64_t interleaved = 0;
    int last_r_id = -1;

    // Previous cycle, for the checks of stability.
    bool ar_wait = false, aw_wait = false, w_wait = false, r_wait = false, b_wait = false;
    uint32_t ar_addr = 0, aw_addr = 0;
    unsigned ar_len = 0, aw_len = 0, ar_id = 0, aw_id = 0, w_id = 0;
    uint64_t w_data = 0;
    bool w_last = false;

    Hp(Vmemprobe_core* d_, Rng& r) : d(d_), rng(r) {}

    uint64_t rd64(uint32_t off) const {
        uint64_t v;
        std::memcpy(&v, &mem[off], 8);
        return v;
    }
    void wr64(uint32_t off, uint64_t v) { std::memcpy(&mem[off], &v, 8); }

    uint64_t latency() {
        if (cfg.p_long > 0 && rng.chance(cfg.p_long)) return 1000 + rng.below(200);
        return cfg.lat_min + rng.below(cfg.lat_span + 1);
    }

    void clear() {
        for (auto& h : hist) h = Hist();
        reads = writes = stalls = 0;
        active_sum = runs = 0;
        resp_errors = data_errors = first_err_addr = 0;
        first_err_data = 0;
        err_seen = false;
    }

    void new_run(unsigned gap_, uint64_t total_, unsigned outstanding_, uint32_t base, uint32_t span,
                 bool check, uint32_t seed) {
        gap = gap_;
        total = total_;
        outstanding = outstanding_;
        span_lo = base;
        span_hi = base + span;
        check_on = check;
        check_seed = seed;
        issued = 0;
        any_addr_hs = false;
        exact_stalls = true;
    }

    // After a loss of the lock: the reset of clk_m ended all transactions.
    void drop_all() {
        rds.clear();
        aws.clear();
        wbeats.clear();
        bs.clear();
        r_cur = b_cur = -1;
        inflight = 0;
        ar_wait = aw_wait = w_wait = r_wait = b_wait = false;
    }

    bool idle() const { return rds.empty() && aws.empty() && wbeats.empty() && bs.empty(); }

    bool id_in_flight(unsigned id) const {
        for (auto& r : rds) if (r.id == id) return true;
        for (auto& w : aws) if (w.id == id) return true;
        for (auto& b : bs) if (b.id == id) return true;
        return false;
    }

    void check_addr(uint32_t addr, unsigned len, const char* ch) {
        uint32_t bytes = (len + 1) * 8;
        CHECK(addr >= HP_BASE && addr - HP_BASE + bytes <= HP_SIZE, "%s address %08x outside the HP memory", ch, addr);
        uint32_t off = addr - HP_BASE;
        CHECK(off >= span_lo && off + bytes <= span_hi, "%s offset %06x outside the span %06x..%06x", ch, off,
              span_lo, span_hi);
        CHECK(len == 0 || len == 1 || len == 3 || len == 7 || len == 15, "%s len %u", ch, len);
        CHECK(addr % bytes == 0, "%s address %08x not aligned to the burst", ch, addr);
        CHECK((addr & 0xFFF) + bytes <= 0x1000, "%s burst at %08x crosses 4 KiB", ch, addr);
    }

    // A new ARVALID or AWVALID.
    void check_issue(const char* ch) {
        if (any_addr_hs)
            CHECK(n - last_addr_hs >= gap, "%s issue %llu edges after the last address handshake, GAP %u", ch,
                  (unsigned long long)(n - last_addr_hs), gap);
        CHECK(inflight + 1 <= outstanding, "%s issue with %u transactions in progress, OUTSTANDING %u", ch,
              inflight, outstanding);
    }

    void addr_hs(unsigned id, const char* ch) {
        CHECK(!id_in_flight(id), "%s ID %u is in use", ch, id);
        if (!any_addr_hs) run_first = n;
        last_addr_hs = n;
        any_addr_hs = true;
        inflight++;
        issued++;
        max_inflight = std::max(max_inflight, inflight);
    }

    void check_beat(const Rd& r, uint64_t data) {
        if (!check_on) return;
        uint32_t off = r.off + 8 * r.beat;
        uint64_t expect = pattern64(check_seed, off);
        bool lo = uint32_t(data) != uint32_t(expect), hi = (data >> 32) != (expect >> 32);
        data_errors += unsigned(lo) + unsigned(hi);
        if ((lo || hi) && !err_seen) {
            err_seen = true;
            first_err_addr = off + (lo ? 0 : 4);
            first_err_data = data;
        }
    }

    // Before the edge: read the outputs of the design and of the model.
    void pre_edge() {
        // A reset of clk_m ends all transactions of the design; no rule applies.
        if (d->rst_m) {
            ar_wait = aw_wait = w_wait = r_wait = b_wait = false;
            return;
        }
        const bool arv = d->hp_arvalid, awv = d->hp_awvalid, wv = d->hp_wvalid;
        const bool arr = d->hp_arready, awr = d->hp_awready, wr = d->hp_wready;
        const bool rv = d->hp_rvalid, rr = d->hp_rready, bv = d->hp_bvalid, br = d->hp_bready;

        // A VALID without READY stays, with the same payload.
        if (ar_wait)
            CHECK(arv && d->hp_araddr == ar_addr && d->hp_arlen == ar_len && d->hp_arid == ar_id,
                  "AR changed before ARREADY");
        if (aw_wait)
            CHECK(awv && d->hp_awaddr == aw_addr && d->hp_awlen == aw_len && d->hp_awid == aw_id,
                  "AW changed before AWREADY");
        if (w_wait)
            CHECK(wv && d->hp_wdata == w_data && d->hp_wlast == w_last && d->hp_wid == w_id,
                  "W changed before WREADY");
        if (arv && !ar_wait) check_issue("AR");
        if (awv && !aw_wait) check_issue("AW");
        CHECK(!(arv && awv), "ARVALID and AWVALID together");

        // Expected stall (spec/regmap.toml, STALLS_LO).
        bool gap_ok = !any_addr_hs || n - last_addr_hs >= gap;
        bool stall = issued < total && gap_ok
                  && ((arv && !arr) || (awv && !awr) || (!arv && !awv && inflight >= outstanding));
        if (stall) stalls++;

        ar_wait = arv && !arr;
        aw_wait = awv && !awr;
        w_wait = wv && !wr;
        r_wait = rv && !rr;
        b_wait = bv && !br;
        ar_addr = d->hp_araddr;
        ar_len = d->hp_arlen;
        ar_id = d->hp_arid;
        aw_addr = d->hp_awaddr;
        aw_len = d->hp_awlen;
        aw_id = d->hp_awid;
        w_data = d->hp_wdata;
        w_last = d->hp_wlast;
        w_id = d->hp_wid;

        if (arv && arr) {
            check_addr(d->hp_araddr, d->hp_arlen, "AR");
            addr_hs(d->hp_arid, "AR");
            rds.push_back({d->hp_arid, d->hp_araddr - HP_BASE, d->hp_arlen, n, n + latency(), 0});
        }
        if (awv && awr) {
            check_addr(d->hp_awaddr, d->hp_awlen, "AW");
            addr_hs(d->hp_awid, "AW");
            aws.push_back({d->hp_awid, d->hp_awaddr - HP_BASE, d->hp_awlen, n});
        }
        if (wv && wr) wbeats.push_back({d->hp_wdata, bool(d->hp_wlast), d->hp_wid});
        CHECK(wbeats.size() <= 16, "W beats without AW");

        // A write is complete when its AW and all its W beats are here.
        while (!aws.empty() && wbeats.size() >= aws.front().len + 1) {
            Wr w = aws.front();
            aws.pop_front();
            for (unsigned i = 0; i <= w.len; i++) {
                WBeat b = wbeats.front();
                wbeats.pop_front();
                CHECK(b.last == (i == w.len), "WLAST %d at beat %u of %u", int(b.last), i, w.len + 1);
                CHECK(b.id == w.id, "WID %u, AWID %u", b.id, w.id);
                wr64(w.off + 8 * i, b.data);
            }
            bs.push_back({w.id, w.t_hs, n + latency()});
        }

        if (rv && rr) {
            auto it = std::find_if(rds.begin(), rds.end(), [&](const Rd& r) { return int(r.id) == r_cur; });
            CHECK(it != rds.end() && d->hp_rid == unsigned(r_cur), "R without AR");
            if (it != rds.end()) {
                Rd& r = *it;
                if (last_r_id >= 0 && last_r_id != int(r.id)
                    && std::any_of(rds.begin(), rds.end(), [&](const Rd& x) { return int(x.id) == last_r_id; }))
                    interleaved++;
                last_r_id = d->hp_rlast ? -1 : int(r.id);
                if (r.beat == 0) hist[H_RD_FIRST].add(uint32_t(n - r.t_hs));
                if (d->hp_rresp != 0) resp_errors++;
                check_beat(r, d->hp_rdata);
                if (d->hp_rlast) {
                    hist[H_RD_LAST].add(uint32_t(n - r.t_hs));
                    reads++;
                    inflight--;
                    run_last = n;
                    rds.erase(it);
                } else {
                    r.beat++;
                }
            }
        }
        if (bv && br) {
            auto it = std::find_if(bs.begin(), bs.end(), [&](const B& b) { return int(b.id) == b_cur; });
            CHECK(it != bs.end() && d->hp_bid == unsigned(b_cur), "B without write");
            if (it != bs.end()) {
                hist[H_WR_RESP].add(uint32_t(n - it->t_aw));
                if (d->hp_bresp != 0) resp_errors++;
                writes++;
                inflight--;
                run_last = n;
                bs.erase(it);
            }
        }
    }

    // After the edge: the outputs of the model for the next cycle.
    void post_edge() {
        n++;
        // While clk_m is in reset, the model takes no transaction. (After a
        // loss of the MMCM lock during a run, the real HP port can keep a
        // transaction; the PS program then loads the PL again.)
        const bool in_reset = d->rst_m;
        d->hp_arready = !in_reset && rng.chance(cfg.p_arready);
        d->hp_awready = !in_reset && rng.chance(cfg.p_awready);
        d->hp_wready = !in_reset && rng.chance(cfg.p_wready);

        // A beat that waits for RREADY stays. Otherwise present a beat of the
        // current read, or of another read that is ready, or none.
        if (!r_wait) {
            d->hp_rvalid = 0;
            std::vector<int> ready;
            for (auto& r : rds)
                if (n >= r.ready_at) ready.push_back(int(r.id));
            bool cur_ok = std::find(ready.begin(), ready.end(), r_cur) != ready.end();
            if (!ready.empty() && !rng.chance(cfg.p_rbubble)) {
                if (!cur_ok || rng.chance(cfg.p_switch)) r_cur = ready[rng.below(uint32_t(ready.size()))];
                auto it = std::find_if(rds.begin(), rds.end(), [&](const Rd& r) { return int(r.id) == r_cur; });
                d->hp_rvalid = 1;
                d->hp_rid = it->id;
                d->hp_rdata = rd64(it->off + 8 * it->beat);
                d->hp_rlast = (it->beat == it->len);
                d->hp_rresp = rng.chance(cfg.p_rerr) ? 2 : 0;
            }
        }
        if (!b_wait) {
            d->hp_bvalid = 0;
            std::vector<int> ready;
            for (auto& b : bs)
                if (n >= b.ready_at) ready.push_back(int(b.id));
            if (!ready.empty()) {
                b_cur = ready[rng.below(uint32_t(ready.size()))];
                d->hp_bvalid = 1;
                d->hp_bid = unsigned(b_cur);
                d->hp_bresp = rng.chance(cfg.p_berr) ? 2 : 0;
            }
        }
    }
};

// ---- GP0 master model (domain of clk_s) --------------------------------------------------

struct Gp {
    Vmemprobe_core* d;
    Rng& rng;
    enum Kind { NONE, READ, WRITE } kind = NONE;
    uint32_t addr = 0;
    unsigned len = 0, beat = 0, wbeat = 0;
    uint16_t id = 0;
    uint8_t strb = 0xF;
    std::vector<uint32_t> wdata, rdata;
    bool a_done = false, done = false;
    bool r_wait = false, b_wait = false;
    uint32_t r_data = 0;
    bool r_last = false;
    uint64_t n = 0, last_progress = 0;
    double p_ready = 0.8;

    Gp(Vmemprobe_core* d_, Rng& r) : d(d_), rng(r) {}

    void begin(Kind k, uint32_t a, unsigned l, std::vector<uint32_t> w = {}, uint8_t s = 0xF) {
        kind = k;
        addr = a;
        len = l;
        wdata = std::move(w);
        strb = s;
        rdata.clear();
        beat = wbeat = 0;
        a_done = done = false;
        id = uint16_t(rng.below(4096));
        last_progress = n;
    }

    void pre_edge() {
        const bool rv = d->gp_rvalid, rr = d->gp_rready, bv = d->gp_bvalid, br = d->gp_bready;
        if (r_wait) CHECK(rv && d->gp_rdata == r_data && d->gp_rlast == r_last, "GP0 R changed before RREADY");
        if (b_wait) CHECK(bv, "GP0 BVALID fell before BREADY");
        r_wait = rv && !rr;
        b_wait = bv && !br;
        r_data = d->gp_rdata;
        r_last = d->gp_rlast;
        CHECK(d->gp_rresp == 0 || !rv, "GP0 RRESP %u", unsigned(d->gp_rresp));
        CHECK(d->gp_bresp == 0 || !bv, "GP0 BRESP %u", unsigned(d->gp_bresp));

        if (d->gp_arvalid && d->gp_arready) { a_done = true; last_progress = n; }
        if (d->gp_awvalid && d->gp_awready) { a_done = true; last_progress = n; }
        if (d->gp_wvalid && d->gp_wready) { wbeat++; last_progress = n; }
        if (rv && rr) {
            CHECK(kind == READ && a_done, "GP0 R without a read");
            CHECK(d->gp_rid == id, "GP0 RID %03x, ARID %03x", unsigned(d->gp_rid), unsigned(id));
            CHECK(bool(d->gp_rlast) == (beat == len), "GP0 RLAST at beat %u of %u", beat, len + 1);
            rdata.push_back(d->gp_rdata);
            last_progress = n;
            if (beat++ == len) { done = true; kind = NONE; }
        }
        if (bv && br) {
            CHECK(kind == WRITE && wbeat == len + 1, "GP0 B before the end of the write");
            CHECK(d->gp_bid == id, "GP0 BID %03x, AWID %03x", unsigned(d->gp_bid), unsigned(id));
            done = true;
            kind = NONE;
        }
    }

    void post_edge() {
        n++;
        d->gp_arvalid = (kind == READ && !a_done);
        d->gp_araddr = addr;
        d->gp_arlen = len;
        d->gp_arid = id;
        d->gp_awvalid = (kind == WRITE && !a_done);
        d->gp_awaddr = addr;
        d->gp_awlen = len;
        d->gp_awid = id;
        // W may go before the AW handshake (AXI3 allows it).
        d->gp_wvalid = (kind == WRITE && wbeat <= len);
        d->gp_wdata = (kind == WRITE && wbeat <= len) ? wdata[wbeat] : 0;
        d->gp_wstrb = strb;
        d->gp_wlast = (wbeat == len);
        d->gp_rready = rng.chance(p_ready);
        d->gp_bready = rng.chance(p_ready);
    }
};

// ---- Simulation --------------------------------------------------------------------------

struct Sim {
    VerilatedContext ctx;
    Vmemprobe_core* d;
    Rng rng;
    Hp hp;
    Gp gp;
    uint64_t per_s, per_m;               // ps
    uint64_t t_s_rise, t_s_fall, t_m_rise, t_m_fall;
    bool clk_m_on = true;

    explicit Sim(uint64_t seed)
        : d(new Vmemprobe_core(&ctx)), rng(seed), hp(d, rng), gp(d, rng) {
        per_s = 10000;
        static const uint64_t m_periods[] = {10000, 9973, 10391, 20000, 6667, 12345};
        per_m = m_periods[seed % 6];
        t_s_rise = 1000;
        t_m_rise = 1000 + rng.below(uint32_t(per_m));
        t_s_fall = t_s_rise + per_s / 2;
        t_m_fall = t_m_rise + per_m / 2;
        for (auto& b : hp.mem) b = uint8_t(rng.next());
        d->clk_s = 0;
        d->clk_m = 0;
        d->eval();
    }
    ~Sim() {
        d->final();
        delete d;
    }

    // Process the next clock event.
    void step() {
        uint64_t t = std::min({t_s_rise, t_s_fall, clk_m_on ? t_m_rise : UINT64_MAX,
                               clk_m_on ? t_m_fall : UINT64_MAX});
        ctx.time(t);
        if (t == t_s_rise) {
            gp.pre_edge();
            d->clk_s = 1;
            d->eval();
            gp.post_edge();
            d->eval();
            t_s_rise += per_s;
        } else if (t == t_s_fall) {
            d->clk_s = 0;
            d->eval();
            t_s_fall += per_s;
        } else if (t == t_m_rise) {
            hp.pre_edge();
            d->clk_m = 1;
            d->eval();
            hp.post_edge();
            d->eval();
            t_m_rise += per_m;
        } else {
            d->clk_m = 0;
            d->eval();
            t_m_fall += per_m;
        }
    }

    void cycles_s(uint64_t k) {
        uint64_t end = gp.n + k;
        while (gp.n < end) step();
    }

    // A GP0 request; the answer must come in a limited time.
    void gp_run() {
        while (!gp.done) {
            step();
            if (gp.n - gp.last_progress > 64) FATAL("no answer on GP0 at %05x", gp.addr);
        }
    }
    std::vector<uint32_t> rd_burst(uint32_t off, unsigned beats) {
        gp.begin(Gp::READ, off, beats - 1);
        gp_run();
        return gp.rdata;
    }
    uint32_t rd(uint32_t off) { return rd_burst(off, 1)[0]; }
    uint64_t rd64(uint32_t lo) { return uint64_t(rd(lo)) | (uint64_t(rd(lo + 4)) << 32); }
    void wr_burst(uint32_t off, std::vector<uint32_t> v, uint8_t strb = 0xF) {
        unsigned l = unsigned(v.size()) - 1;
        gp.begin(Gp::WRITE, off, l, std::move(v), strb);
        gp_run();
    }
    void wr(uint32_t off, uint32_t v, uint8_t strb = 0xF) { wr_burst(off, {v}, strb); }

    uint32_t status() { return rd(MEMPROBE_STATUS_OFFSET); }

    // Wait until STATUS.BUSY is 0.
    void wait_idle(uint64_t max_polls = 200000) {
        for (uint64_t i = 0; i < max_polls; i++)
            if (!(status() & BIT(STATUS, BUSY))) return;
        FATAL("BUSY did not end");
    }

    void reset() {
        d->rst_s = 1;
        d->rst_m = 1;
        d->mmcm_locked = 0;
        cycles_s(8);
        d->rst_s = 0;
        cycles_s(4);
    }
    // memprobe_top ends the reset of clk_m 2 edges of clk_m after the lock. The
    // design must also work with 1 edge of reset (memprobe_sync).
    void lock(unsigned reset_edges = 2) {
        d->mmcm_locked = 1;
        uint64_t end = hp.n + reset_edges;
        while (hp.n < end) step();
        d->rst_m = 0;
        cycles_s(4);
    }
    // The MMCM loses the lock: clk_m stops (low) and its domain is in reset.
    void unlock() {
        d->mmcm_locked = 0;
        d->rst_m = 1;
        d->hp_arready = d->hp_awready = d->hp_wready = 0;
        d->clk_m = 0;
        d->eval();
        clk_m_on = false;
    }
    void restart_clk_m() {
        clk_m_on = true;
        t_m_rise = t_s_rise + rng.below(uint32_t(per_m));
        t_m_fall = t_m_rise + per_m / 2;
    }
};

// Write 1 to a w1c bit of STATUS; it must be 0 then.
static void clear_status(Sim& s, uint32_t bit) {
    s.wr(MEMPROBE_STATUS_OFFSET, bit);
    CHECK(!(s.status() & bit), "STATUS bit %08x not cleared by a write of 1", bit);
}

// ---- Test steps --------------------------------------------------------------------------

struct Run {
    uint32_t op = MEMPROBE_MODE_OP_READ, len = 15, base = 0, span_log2 = 22, count = 1, gap = 0;
    uint32_t seed = 1, outstanding = 1, pattern = MEMPROBE_MODE_PATTERN_SEQ, read_share = 0x8000;
    bool verify = false;
    uint32_t mode() const {
        return (op << MEMPROBE_MODE_OP_LSB) | (pattern << MEMPROBE_MODE_PATTERN_LSB)
             | (len << MEMPROBE_MODE_LEN_LSB) | (MEMPROBE_MODE_SIZE_B8 << MEMPROBE_MODE_SIZE_LSB)
             | (verify ? BIT(MODE, VERIFY) : 0u);
    }
    bool checks() const {
        return op == MEMPROBE_MODE_OP_CHECK
            || (verify && (op == MEMPROBE_MODE_OP_READ || op == MEMPROBE_MODE_OP_MIXED));
    }
    uint64_t total() const {
        if (op == MEMPROBE_MODE_OP_FILL || op == MEMPROBE_MODE_OP_CHECK)
            return (uint64_t(1) << span_log2) / ((len + 1) * 8);
        return count;
    }
};

static void configure(Sim& s, const Run& r) {
    s.wr(MEMPROBE_MODE_OFFSET, r.mode());
    s.wr(MEMPROBE_BASE_OFFSET, r.base);
    s.wr(MEMPROBE_SPAN_LOG2_OFFSET, r.span_log2);
    s.wr(MEMPROBE_COUNT_OFFSET, r.count);
    s.wr(MEMPROBE_GAP_OFFSET, r.gap);
    s.wr(MEMPROBE_OUTSTANDING_OFFSET, r.outstanding);
    s.wr(MEMPROBE_READ_SHARE_OFFSET, r.read_share);
    s.wr(MEMPROBE_SEED_OFFSET, r.seed);
}

static void start(Sim& s, const Run& r) {
    configure(s, r);
    s.hp.new_run(r.gap, r.total(), r.outstanding, r.base, 1u << r.span_log2, r.checks(), r.seed);
    s.wr(MEMPROBE_CTRL_OFFSET, BIT(CTRL, START));
}

static void run(Sim& s, const Run& r) {
    start(s, r);
    s.wait_idle();
    uint32_t st = s.status();
    CHECK(st & BIT(STATUS, DONE), "no DONE, STATUS %08x", st);
    CHECK(!(st & BIT(STATUS, CFG_ERR)), "CFG_ERR, STATUS %08x", st);
    // A histogram gets at most one sample in each cycle, and takes one sample
    // in each cycle with its four banks: no sample is dropped.
    CHECK(!(st & BIT(STATUS, HIST_DROP)), "HIST_DROP, STATUS %08x", st);
    CHECK(s.hp.idle(), "transactions in progress after the run");
    CHECK(s.hp.issued == r.total(), "issued %llu, expected %llu", (unsigned long long)s.hp.issued,
          (unsigned long long)r.total());
    if (s.hp.issued > 0) s.hp.active_sum += s.hp.run_last - s.hp.run_first;
    s.hp.runs++;
    clear_status(s, BIT(STATUS, DONE));
}

static void check_initial_results(Sim& s, bool with_status);

static void clear(Sim& s) {
    s.wr(MEMPROBE_CTRL_OFFSET, BIT(CTRL, CLEAR));
    s.wait_idle();
    s.hp.clear();
    CHECK(!(s.status() & BIT(STATUS, DONE)), "CLEAR set DONE");
    check_initial_results(s, false);
}

static void check_results(Sim& s) {
    Hp& hp = s.hp;
    CHECK(s.rd64(MEMPROBE_READS_LO_OFFSET) == hp.reads, "READS %llu, expected %llu",
          (unsigned long long)s.rd64(MEMPROBE_READS_LO_OFFSET), (unsigned long long)hp.reads);
    CHECK(s.rd64(MEMPROBE_WRITES_LO_OFFSET) == hp.writes, "WRITES %llu, expected %llu",
          (unsigned long long)s.rd64(MEMPROBE_WRITES_LO_OFFSET), (unsigned long long)hp.writes);
    CHECK(s.rd(MEMPROBE_RESP_ERRORS_OFFSET) == hp.resp_errors, "RESP_ERRORS %u, expected %u",
          s.rd(MEMPROBE_RESP_ERRORS_OFFSET), hp.resp_errors);
    CHECK(s.rd(MEMPROBE_HIST_DROPS_OFFSET) == 0, "HIST_DROPS %u", s.rd(MEMPROBE_HIST_DROPS_OFFSET));
    CHECK(s.rd(MEMPROBE_DATA_ERRORS_OFFSET) == hp.data_errors, "DATA_ERRORS %u, expected %u",
          s.rd(MEMPROBE_DATA_ERRORS_OFFSET), hp.data_errors);
    CHECK(s.rd(MEMPROBE_FIRST_ERR_ADDR_OFFSET) == hp.first_err_addr, "FIRST_ERR_ADDR %06x, expected %06x",
          s.rd(MEMPROBE_FIRST_ERR_ADDR_OFFSET), hp.first_err_addr);
    CHECK(s.rd64(MEMPROBE_FIRST_ERR_DATA_LO_OFFSET) == hp.first_err_data, "FIRST_ERR_DATA");
    if (hp.exact_stalls) {
        // At most 40 edges more for each run: the command, the first VALID,
        // and the pipeline after the last completion.
        uint64_t rc = s.rd64(MEMPROBE_RUN_CYCLES_LO_OFFSET);
        CHECK(rc >= hp.active_sum && rc <= hp.active_sum + 40 * hp.runs,
              "RUN_CYCLES %llu, active %llu in %llu runs", (unsigned long long)rc,
              (unsigned long long)hp.active_sum, (unsigned long long)hp.runs);
    }
    if (hp.exact_stalls)
        CHECK(s.rd64(MEMPROBE_STALLS_LO_OFFSET) == hp.stalls, "STALLS %llu, expected %llu",
              (unsigned long long)s.rd64(MEMPROBE_STALLS_LO_OFFSET), (unsigned long long)hp.stalls);
    for (int h = 0; h < 3; h++) {
        std::vector<uint32_t> st = s.rd_burst(STAT_OFFSET[h], 4);
        CHECK(st[0] == hp.hist[h].min, "hist %d MIN %u, expected %u", h, st[0], hp.hist[h].min);
        CHECK(st[1] == hp.hist[h].max, "hist %d MAX %u, expected %u", h, st[1], hp.hist[h].max);
        uint64_t sum = uint64_t(st[2]) | (uint64_t(st[3]) << 32);
        CHECK(sum == hp.hist[h].sum, "hist %d SUM %llu, expected %llu", h, (unsigned long long)sum,
              (unsigned long long)hp.hist[h].sum);
        for (int b = 0; b < BINS; b += 16) {
            std::vector<uint32_t> v = s.rd_burst(HIST_OFFSET[h] + 4 * b, 16);
            for (int i = 0; i < 16; i++)
                CHECK(v[i] == hp.hist[h].bins[b + i], "hist %d bin %d: %u, expected %u", h, b + i, v[i],
                      hp.hist[h].bins[b + i]);
        }
    }
}

static Run random_run(Rng& rng, uint32_t op) {
    static const uint32_t lens[] = {0, 1, 3, 7, 15};
    Run r;
    r.op = op;
    r.len = lens[rng.below(5)];
    unsigned burst_log2 = 3 + __builtin_popcount(r.len);
    r.span_log2 = burst_log2 + 2 + rng.below(8);       // 4 to 512 bursts
    uint32_t spans = HP_SIZE >> r.span_log2;
    r.base = rng.below(spans) << r.span_log2;
    r.count = 20 + rng.below(100);
    r.gap = rng.chance(0.5) ? 0 : rng.below(40);
    r.seed = uint32_t(rng.next()) | 1;
    r.outstanding = rng.chance(0.3) ? 1 : 1 + rng.below(16);
    if (op == MEMPROBE_MODE_OP_READ || op == MEMPROBE_MODE_OP_WRITE || op == MEMPROBE_MODE_OP_MIXED) {
        static const uint32_t pats[] = {MEMPROBE_MODE_PATTERN_SEQ, MEMPROBE_MODE_PATTERN_RANDOM,
                                        MEMPROBE_MODE_PATTERN_FIXED};
        r.pattern = pats[rng.below(3)];
    }
    r.read_share = rng.below(0x10001);
    return r;
}

static void test_identity(Sim& s) {
    g_step = "identity";
    CHECK(s.rd(MEMPROBE_ID_OFFSET) == MEMPROBE_ID_VALUE, "ID %08x", s.rd(MEMPROBE_ID_OFFSET));
    CHECK(s.rd(MEMPROBE_VERSION_OFFSET) == MEMPROBE_VERSION_VALUE, "VERSION");
    CHECK(s.rd(MEMPROBE_BUILD_OFFSET) == 0x5EED0001u, "BUILD %08x", s.rd(MEMPROBE_BUILD_OFFSET));
    uint32_t caps = s.rd(MEMPROBE_CAPS_OFFSET);
    CHECK(FIELD(caps, CAPS, CLK_MHZ) == 100, "CAPS.CLK_MHZ %u", FIELD(caps, CAPS, CLK_MHZ));
    CHECK(FIELD(caps, CAPS, MAX_OUTSTANDING) == 16, "CAPS.MAX_OUTSTANDING %u", FIELD(caps, CAPS, MAX_OUTSTANDING));
    CHECK(FIELD(caps, CAPS, PORTS) == 1, "CAPS.PORTS %u", FIELD(caps, CAPS, PORTS));
    uint32_t flags = BIT(CAPS, MIXED) | BIT(CAPS, PATTERNS) | BIT(CAPS, VERIFY);
    CHECK((caps & ~(MEMPROBE_CAPS_CLK_MHZ_MASK | MEMPROBE_CAPS_MAX_OUTSTANDING_MASK | MEMPROBE_CAPS_PORTS_MASK))
              == flags, "CAPS %08x", caps);
    // A burst reads consecutive registers.
    std::vector<uint32_t> v = s.rd_burst(MEMPROBE_ID_OFFSET, 4);
    CHECK(v[0] == MEMPROBE_ID_VALUE && v[1] == MEMPROBE_VERSION_VALUE && v[3] == caps, "burst read of ID..CAPS");
    // Unknown offsets read 0, and a write to them does nothing.
    for (uint32_t off : {0x0C8u, 0x1FCu, 0x5000u, 0xFFFFCu, 0x40000u}) {
        s.wr(off, 0xFFFFFFFF);
        CHECK(s.rd(off) == 0, "offset %05x reads %08x", off, s.rd(off));
    }
    CHECK(s.rd(MEMPROBE_CTRL_OFFSET) == 0, "CTRL reads not 0");
}

// The results have the reset values of spec/regmap.toml: after a reset, and
// after CLEAR.
static void check_initial_results(Sim& s, bool with_status) {
#define R(name) {MEMPROBE_##name##_OFFSET, MEMPROBE_##name##_RESET, #name}
    struct { uint32_t off, reset; const char* name; } regs[] = {
        R(STATUS), R(RUN_CYCLES_LO), R(RUN_CYCLES_HI), R(READS_LO), R(READS_HI), R(WRITES_LO),
        R(WRITES_HI), R(STALLS_LO), R(STALLS_HI), R(DATA_ERRORS), R(FIRST_ERR_ADDR),
        R(FIRST_ERR_DATA_LO), R(FIRST_ERR_DATA_HI), R(ORDER_MISMATCHES), R(HIST_DROPS),
        R(RESP_ERRORS), R(RD_FIRST_MIN), R(RD_FIRST_MAX), R(RD_FIRST_SUM_LO), R(RD_FIRST_SUM_HI),
        R(RD_LAST_MIN), R(RD_LAST_MAX), R(RD_LAST_SUM_LO), R(RD_LAST_SUM_HI), R(WR_RESP_MIN),
        R(WR_RESP_MAX), R(WR_RESP_SUM_LO), R(WR_RESP_SUM_HI),
    };
#undef R
    for (auto& r : regs) {
        if (r.off == MEMPROBE_STATUS_OFFSET && !with_status) continue;
        uint32_t v = s.rd(r.off);
        if (r.off == MEMPROBE_STATUS_OFFSET) v &= ~BIT(STATUS, MMCM_LOCKED);
        CHECK(v == r.reset, "%s: %08x, expected %08x", r.name, v, r.reset);
    }
    for (int h = 0; h < 3; h++)
        for (int b = 0; b < BINS; b += 16)
            for (uint32_t v : s.rd_burst(HIST_OFFSET[h] + 4 * b, 16)) CHECK(v == 0, "hist %d bin not 0", h);
}

static void test_reset_values(Sim& s) {
    g_step = "reset values";
    check_initial_results(s, true);
}

static void test_registers(Sim& s) {
    g_step = "registers";
    struct { uint32_t off, reset, mask; } regs[] = {
        {MEMPROBE_MODE_OFFSET, MEMPROBE_MODE_RESET, MEMPROBE_MODE_MASK},
        {MEMPROBE_BASE_OFFSET, MEMPROBE_BASE_RESET, MEMPROBE_BASE_MASK},
        {MEMPROBE_SPAN_LOG2_OFFSET, MEMPROBE_SPAN_LOG2_RESET, MEMPROBE_SPAN_LOG2_MASK},
        {MEMPROBE_COUNT_OFFSET, MEMPROBE_COUNT_RESET, MEMPROBE_COUNT_MASK},
        {MEMPROBE_GAP_OFFSET, MEMPROBE_GAP_RESET, MEMPROBE_GAP_MASK},
        {MEMPROBE_OUTSTANDING_OFFSET, MEMPROBE_OUTSTANDING_RESET, MEMPROBE_OUTSTANDING_MASK},
        {MEMPROBE_READ_SHARE_OFFSET, MEMPROBE_READ_SHARE_RESET, MEMPROBE_READ_SHARE_MASK},
        {MEMPROBE_SEED_OFFSET, MEMPROBE_SEED_RESET, MEMPROBE_SEED_MASK},
        {MEMPROBE_IRQ_EN_OFFSET, MEMPROBE_IRQ_EN_RESET, MEMPROBE_IRQ_EN_MASK},
    };
    for (auto& r : regs) {
        CHECK(s.rd(r.off) == r.reset, "reset of %03x: %08x, expected %08x", r.off, s.rd(r.off), r.reset);
        s.wr(r.off, 0xFFFFFFFF);
        CHECK(s.rd(r.off) == r.mask, "%03x after all ones: %08x, expected %08x", r.off, s.rd(r.off), r.mask);
        s.wr(r.off, 0x12345678, 0x2);   // only byte 1
        uint32_t expect = ((r.mask & 0xFFFF00FF) | 0x5600) & r.mask;
        CHECK(s.rd(r.off) == expect, "%03x after byte write: %08x, expected %08x", r.off, s.rd(r.off), expect);
        s.wr(r.off, r.reset);
    }
    // A burst writes consecutive registers.
    s.wr_burst(MEMPROBE_COUNT_OFFSET, {7, 9});
    CHECK(s.rd(MEMPROBE_COUNT_OFFSET) == 7 && s.rd(MEMPROBE_GAP_OFFSET) == 9, "burst write");
    s.wr_burst(MEMPROBE_COUNT_OFFSET, {MEMPROBE_COUNT_RESET, MEMPROBE_GAP_RESET});
    CHECK(s.rd(MEMPROBE_RD_FIRST_MIN_OFFSET) == MEMPROBE_RD_FIRST_MIN_RESET, "RD_FIRST_MIN reset");
}

static void expect_refused(Sim& s, const Run& r, const char* why) {
    uint64_t ar = s.hp.issued;
    configure(s, r);
    s.wr(MEMPROBE_CTRL_OFFSET, BIT(CTRL, START));
    s.cycles_s(20);
    uint32_t st = s.status();
    CHECK(st & BIT(STATUS, CFG_ERR), "%s: START accepted, STATUS %08x", why, st);
    CHECK(!(st & BIT(STATUS, BUSY)), "%s: BUSY", why);
    CHECK(s.hp.issued == ar && s.hp.idle(), "%s: HP traffic", why);
    clear_status(s, BIT(STATUS, CFG_ERR));
    CHECK(!(s.status() & BIT(STATUS, CFG_ERR)), "%s: CFG_ERR not cleared", why);
}

static void test_refusals(Sim& s) {
    g_step = "refusals";
    Run ok;
    ok.op = MEMPROBE_MODE_OP_READ;
    ok.span_log2 = 16;
    ok.base = 0x10000;
    auto with = [&](std::function<void(Run&)> f) { Run r = ok; f(r); return r; };
    expect_refused(s, with([](Run& r) { r.len = 2; }), "LEN 2");
    expect_refused(s, with([](Run& r) { r.len = 14; }), "LEN 14");
    expect_refused(s, with([](Run& r) { r.op = MEMPROBE_MODE_OP_ORDER; }), "ORDER");
    expect_refused(s, with([](Run& r) { r.op = 6; }), "OP 6");
    expect_refused(s, with([](Run& r) { r.seed = 0; }), "SEED 0");
    expect_refused(s, with([](Run& r) { r.outstanding = 0; }), "OUTSTANDING 0");
    expect_refused(s, with([](Run& r) { r.outstanding = 17; }), "OUTSTANDING 17");
    expect_refused(s, with([](Run& r) { r.op = MEMPROBE_MODE_OP_FILL; r.pattern = MEMPROBE_MODE_PATTERN_RANDOM; }),
                   "FILL with RANDOM");
    expect_refused(s, with([](Run& r) { r.op = MEMPROBE_MODE_OP_CHECK; r.pattern = MEMPROBE_MODE_PATTERN_FIXED; }),
                   "CHECK with FIXED");
    expect_refused(s, with([](Run& r) { r.pattern = 3; }), "PATTERN 3");
    expect_refused(s, with([](Run& r) { r.span_log2 = 25; }), "span 32 MiB");
    expect_refused(s, with([](Run& r) { r.span_log2 = 6; r.len = 15; r.base = 0; }), "span < burst");
    expect_refused(s, with([](Run& r) { r.base = 0x18000; }), "BASE not a multiple of the span");
    expect_refused(s, with([](Run& r) { r.base = HP_SIZE; r.span_log2 = 12; }), "BASE at the end");
    expect_refused(s, with([](Run& r) { r.base = 0xFFFFF000; r.span_log2 = 12; }), "BASE far outside");
    // Ports that this design does not have (CAPS.PORTS = HP0), and ORDER_REVERSE
    // without ORDER.
    for (uint32_t extra : {1u << MEMPROBE_MODE_RD_PORT_LSB, 2u << MEMPROBE_MODE_RD_PORT_LSB,
                           3u << MEMPROBE_MODE_RD_PORT_LSB, 1u << MEMPROBE_MODE_WR_PORT_LSB,
                           3u << MEMPROBE_MODE_WR_PORT_LSB, BIT(MODE, ORDER_REVERSE)}) {
        configure(s, ok);
        s.wr(MEMPROBE_MODE_OFFSET, ok.mode() | extra);
        s.wr(MEMPROBE_CTRL_OFFSET, BIT(CTRL, START));
        s.cycles_s(20);
        CHECK(s.status() & BIT(STATUS, CFG_ERR), "MODE %08x accepted", ok.mode() | extra);
        clear_status(s, BIT(STATUS, CFG_ERR));
    }
    configure(s, ok);
    s.wr(MEMPROBE_MODE_OFFSET, (ok.mode() & ~MEMPROBE_MODE_SIZE_MASK)
                               | (MEMPROBE_MODE_SIZE_B4 << MEMPROBE_MODE_SIZE_LSB));
    s.wr(MEMPROBE_CTRL_OFFSET, BIT(CTRL, START));
    s.cycles_s(20);
    CHECK(s.status() & BIT(STATUS, CFG_ERR), "SIZE B4 accepted");
    clear_status(s, BIT(STATUS, CFG_ERR));
    // Two commands in one write.
    configure(s, ok);
    s.wr(MEMPROBE_CTRL_OFFSET, BIT(CTRL, START) | BIT(CTRL, CLEAR));
    s.cycles_s(20);
    CHECK(s.status() & BIT(STATUS, CFG_ERR), "START with CLEAR accepted");
    CHECK(s.hp.idle() && !(s.status() & BIT(STATUS, BUSY)), "START with CLEAR started");
    clear_status(s, BIT(STATUS, CFG_ERR));
}

static void test_fill_check(Sim& s) {
    g_step = "fill and check";
    Run r = random_run(s.rng, MEMPROBE_MODE_OP_FILL);
    std::vector<uint8_t> before = s.hp.mem;
    clear(s);
    run(s, r);
    check_results(s);
    uint32_t span = 1u << r.span_log2;
    for (uint32_t off = 0; off < HP_SIZE; off += 4) {
        uint32_t v;
        std::memcpy(&v, &s.hp.mem[off], 4);
        if (off >= r.base && off < r.base + span) {
            if (v != pattern(r.seed, off)) {
                CHECK(false, "FILL: word at %06x is %08x, expected %08x", off, v, pattern(r.seed, off));
                break;
            }
        } else if (std::memcmp(&s.hp.mem[off], &before[off], 4) != 0) {
            CHECK(false, "FILL changed %06x outside the span", off);
            break;
        }
    }

    g_step = "check without errors";
    r.op = MEMPROBE_MODE_OP_CHECK;
    clear(s);
    run(s, r);
    check_results(s);
    CHECK(s.rd(MEMPROBE_DATA_ERRORS_OFFSET) == 0, "DATA_ERRORS %u", s.rd(MEMPROBE_DATA_ERRORS_OFFSET));
    CHECK(!(s.status() & BIT(STATUS, DATA_ERR)), "DATA_ERR");

    g_step = "check with errors";
    unsigned k = 1 + s.rng.below(10);
    for (unsigned i = 0; i < k; i++) {
        uint32_t off = r.base + 4 * s.rng.below(span / 4);
        s.hp.mem[off + s.rng.below(4)] ^= uint8_t(1u << s.rng.below(8));
    }
    // The number of errors does not depend on the order of the reads. The
    // first error is the first in the order of the R handshakes: the model
    // gives it (check_results).
    uint32_t errors = 0;
    for (uint32_t off = r.base; off < r.base + span; off += 4) {
        uint32_t v;
        std::memcpy(&v, &s.hp.mem[off], 4);
        if (v != pattern(r.seed, off)) errors++;
    }
    clear(s);
    run(s, r);
    check_results(s);
    CHECK(s.rd(MEMPROBE_DATA_ERRORS_OFFSET) == errors, "DATA_ERRORS %u, expected %u",
          s.rd(MEMPROBE_DATA_ERRORS_OFFSET), errors);
    CHECK(s.hp.err_seen, "the model saw no error");
    CHECK(s.status() & BIT(STATUS, DATA_ERR), "no DATA_ERR");
    clear_status(s, BIT(STATUS, DATA_ERR));
    // A second run adds to the counters.
    s.hp.new_run(r.gap, r.total(), r.outstanding, r.base, 1u << r.span_log2, r.checks(), r.seed);
    run(s, r);
    CHECK(s.rd(MEMPROBE_DATA_ERRORS_OFFSET) == 2 * errors, "DATA_ERRORS after two runs");
    check_results(s);
    clear_status(s, BIT(STATUS, DATA_ERR));
}

static void test_read_write(Sim& s, bool errors, bool long_lat) {
    g_step = long_lat ? "read and write, long latency" : errors ? "read and write, SLVERR" : "read and write";
    HpConfig saved = s.hp.cfg;
    if (errors) s.hp.cfg.p_rerr = s.hp.cfg.p_berr = 0.1;
    if (long_lat) s.hp.cfg.p_long = 0.1;
    clear(s);
    Run r = random_run(s.rng, MEMPROBE_MODE_OP_READ);
    if (long_lat) r.count = 15;
    run(s, r);
    Run w = random_run(s.rng, MEMPROBE_MODE_OP_WRITE);
    if (long_lat) w.count = 15;
    run(s, w);
    check_results(s);
    // RUN_CYCLES covers the transactions of the two runs.
    uint64_t rc = s.rd64(MEMPROBE_RUN_CYCLES_LO_OFFSET);
    CHECK(rc > 0, "RUN_CYCLES 0");
    uint32_t st = s.status();
    CHECK(bool(st & BIT(STATUS, RESP_ERR)) == (s.hp.resp_errors != 0), "RESP_ERR %d, errors %u",
          int(bool(st & BIT(STATUS, RESP_ERR))), s.hp.resp_errors);
    clear_status(s, BIT(STATUS, RESP_ERR));
    s.hp.cfg = saved;
}

static void test_abort_and_busy(Sim& s) {
    g_step = "abort";
    clear(s);
    Run r = random_run(s.rng, MEMPROBE_MODE_OP_READ);
    r.count = 1000000;
    start(s, r);
    s.cycles_s(200 + s.rng.below(200));
    CHECK(s.status() & BIT(STATUS, BUSY), "not BUSY during a run");
    // Configuration writes during a run are ignored.
    s.wr(MEMPROBE_COUNT_OFFSET, 5);
    CHECK(s.rd(MEMPROBE_COUNT_OFFSET) == r.count, "COUNT changed during a run");
    s.wr(MEMPROBE_CTRL_OFFSET, BIT(CTRL, START));
    CHECK(s.status() & BIT(STATUS, CFG_ERR), "START during a run accepted");
    clear_status(s, BIT(STATUS, CFG_ERR));
    s.hp.exact_stalls = false;   // the moment of ABORT is not known to the model
    s.wr(MEMPROBE_CTRL_OFFSET, BIT(CTRL, ABORT));
    s.wait_idle();
    CHECK(s.status() & BIT(STATUS, DONE), "no DONE after ABORT");
    CHECK(s.hp.idle(), "transactions in progress after ABORT");
    CHECK(s.hp.reads > 0 && s.hp.reads < r.count, "reads %llu", (unsigned long long)s.hp.reads);
    check_results(s);
    clear_status(s, BIT(STATUS, DONE));
}

static void test_irq(Sim& s) {
    g_step = "interrupt";
    clear(s);
    CHECK(!s.d->irq, "irq without cause");
    s.wr(MEMPROBE_IRQ_EN_OFFSET, BIT(IRQ_EN, DONE));
    Run r = random_run(s.rng, MEMPROBE_MODE_OP_WRITE);
    r.count = 3;
    start(s, r);
    s.wait_idle();
    s.cycles_s(2);
    CHECK(s.d->irq, "no irq after DONE");
    clear_status(s, BIT(STATUS, DONE));
    s.cycles_s(2);
    CHECK(!s.d->irq, "irq after DONE was cleared");
    s.wr(MEMPROBE_IRQ_EN_OFFSET, 0);
}

static void test_clocks(Sim& s) {
    g_step = "clock counters";
    uint32_t f0 = s.rd(MEMPROBE_FCLK_COUNT_OFFSET), m0 = s.rd(MEMPROBE_CLKM_COUNT_OFFSET);
    s.cycles_s(5000);
    uint32_t f1 = s.rd(MEMPROBE_FCLK_COUNT_OFFSET), m1 = s.rd(MEMPROBE_CLKM_COUNT_OFFSET);
    double ratio = double(m1 - m0) / double(f1 - f0);
    double expect = double(s.per_s) / double(s.per_m);
    CHECK(ratio > expect * 0.99 && ratio < expect * 1.01, "clk_m/clk_s %.4f, expected %.4f", ratio, expect);
}

// Many transactions in progress, all address patterns, reads and writes mixed.
// The span is filled first, so that VERIFY can check each read: the writes
// write the same pattern.
static void test_concurrent(Sim& s, int runs) {
    g_step = "concurrent";
    for (int i = 0; i < runs; i++) {
        static const uint32_t ops[] = {MEMPROBE_MODE_OP_READ, MEMPROBE_MODE_OP_WRITE, MEMPROBE_MODE_OP_MIXED};
        Run r = random_run(s.rng, ops[s.rng.below(3)]);
        r.count = 50 + s.rng.below(300);
        if (s.rng.chance(0.5)) r.gap = 0;
        r.verify = r.op != MEMPROBE_MODE_OP_WRITE;
        Run f = r;
        f.op = MEMPROBE_MODE_OP_FILL;
        f.pattern = MEMPROBE_MODE_PATTERN_SEQ;
        f.verify = false;
        f.outstanding = 1 + s.rng.below(16);
        clear(s);
        run(s, f);
        clear(s);
        run(s, r);
        check_results(s);
        CHECK(s.rd(MEMPROBE_DATA_ERRORS_OFFSET) == 0, "run %d: DATA_ERRORS %u with VERIFY", i,
              s.rd(MEMPROBE_DATA_ERRORS_OFFSET));
    }

    CHECK(s.hp.max_inflight >= 8, "at most %u transactions were in progress", s.hp.max_inflight);
    CHECK(s.hp.interleaved > 0, "no R beats were interleaved");
    std::printf("concurrent: up to %u in progress, %llu interleaved R beats\n", s.hp.max_inflight,
                (unsigned long long)s.hp.interleaved);

    g_step = "concurrent with errors";
    Run r = random_run(s.rng, MEMPROBE_MODE_OP_READ);
    r.pattern = MEMPROBE_MODE_PATTERN_RANDOM;
    r.outstanding = 16;
    r.count = 400;
    r.gap = 0;
    r.verify = true;
    Run f = r;
    f.op = MEMPROBE_MODE_OP_FILL;
    f.pattern = MEMPROBE_MODE_PATTERN_SEQ;
    f.verify = false;
    clear(s);
    run(s, f);
    uint32_t span = 1u << r.span_log2;
    for (int k = 0; k < 20; k++) s.hp.mem[r.base + s.rng.below(span)] ^= uint8_t(1u << s.rng.below(8));
    clear(s);
    run(s, r);
    check_results(s);
    CHECK(s.hp.data_errors > 0, "no data error after the changes of the memory");
    clear_status(s, BIT(STATUS, DATA_ERR));

    // The same with MIXED: the writes write the pattern, so a changed word
    // can be repaired before a read; the model counts what the reads saw.
    g_step = "concurrent mixed with errors";
    Run m = r;
    m.op = MEMPROBE_MODE_OP_MIXED;
    m.read_share = 0xF000;
    clear(s);
    run(s, f);
    for (int k = 0; k < 40; k++) s.hp.mem[r.base + s.rng.below(span)] ^= uint8_t(1u << s.rng.below(8));
    clear(s);
    run(s, m);
    check_results(s);
    CHECK(s.hp.data_errors > 0, "no data error in MIXED after the changes of the memory");
    clear_status(s, BIT(STATUS, DATA_ERR));
}

static void test_unlock(Sim& s) {
    g_step = "MMCM not locked";
    Run r = random_run(s.rng, MEMPROBE_MODE_OP_READ);
    r.count = 1000000;
    start(s, r);
    s.cycles_s(100);
    // The MMCM loses the lock: clk_m stops and its domain goes into reset.
    s.unlock();
    s.cycles_s(10);
    uint32_t st = s.status();
    CHECK(!(st & BIT(STATUS, BUSY)) && !(st & BIT(STATUS, DONE)) && !(st & BIT(STATUS, MMCM_LOCKED)),
          "STATUS %08x without lock", st);
    CHECK(s.rd(MEMPROBE_ID_OFFSET) == MEMPROBE_ID_VALUE, "ID without lock");
    s.wr(MEMPROBE_CTRL_OFFSET, BIT(CTRL, START));
    s.cycles_s(10);
    CHECK(s.status() & BIT(STATUS, CFG_ERR), "START without lock accepted");
    clear_status(s, BIT(STATUS, CFG_ERR));
    // The model drops the transaction that the reset ended.
    s.hp.drop_all();
    s.d->hp_rvalid = 0;
    s.d->hp_bvalid = 0;
    s.restart_clk_m();
    s.lock(1);
    CHECK(s.status() & BIT(STATUS, MMCM_LOCKED), "no MMCM_LOCKED");
    clear(s);
    Run w = random_run(s.rng, MEMPROBE_MODE_OP_WRITE);
    run(s, w);
    check_results(s);
}

int main(int argc, char** argv) {
    const char* pattern_file = nullptr;
    uint64_t seed = 1;
    for (int i = 1; i < argc; i++) {
        if (!std::strcmp(argv[i], "--pattern") && i + 1 < argc) pattern_file = argv[++i];
        else if (!std::strcmp(argv[i], "--seed") && i + 1 < argc) seed = std::strtoull(argv[++i], nullptr, 0);
        else FATAL("usage: tb_core --pattern FILE [--seed N]");
    }
    if (!pattern_file) FATAL("--pattern is necessary");

    g_step = "pattern file";
    check_pattern_file(pattern_file);

    Sim s(seed);
    std::printf("seed %llu: clk_s %llu ps, clk_m %llu ps\n", (unsigned long long)seed,
                (unsigned long long)s.per_s, (unsigned long long)s.per_m);
    g_step = "reset";
    s.reset();
    // The registers answer before the MMCM is locked.
    CHECK(s.rd(MEMPROBE_ID_OFFSET) == MEMPROBE_ID_VALUE, "ID before lock");
    CHECK(!(s.status() & BIT(STATUS, MMCM_LOCKED)), "MMCM_LOCKED before lock");
    s.lock();
    CHECK(s.status() & BIT(STATUS, MMCM_LOCKED), "no MMCM_LOCKED");

    test_reset_values(s);
    test_identity(s);
    test_registers(s);
    test_refusals(s);
    test_fill_check(s);
    test_read_write(s, false, false);
    test_read_write(s, true, false);
    test_read_write(s, false, true);
    test_concurrent(s, 12);
    test_abort_and_busy(s);
    test_irq(s);
    test_clocks(s);
    // Twice, with one more command between: the acknowledge toggle has the
    // other value at the second loss of the lock.
    test_unlock(s);
    clear(s);
    test_unlock(s);

    if (g_failures) {
        std::printf("seed %llu: %d failures\n", (unsigned long long)seed, g_failures);
        return 1;
    }
    std::printf("seed %llu: pass\n", (unsigned long long)seed);
    return 0;
}
