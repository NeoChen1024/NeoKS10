// A C++ caller of the C interface of the microcode-level model
// (spec/capi.toml, emu/ks10-capi). It links libks10_capi.a and calls
// each function once. `make check` builds and runs it.
//
// The program is that of emu/ks10-core-uop/tests/record.rs: MOVEI 1,123,
// MOVEM 1,2000, AOS 2000 and HALT at 1000, started with the console
// command ST. The expected values come from the instructions and from
// the definition of the record (docs/microcode-model.md, "The
// instruction record"): 4 records with the PC values 1000 to 1003, then
// no record in the halt loop, and 124 at 2000.

#include <cstdio>
#include <cstdlib>
#include <cstring>

#include "ks10_capi.h"

static int failures = 0;

static void check(bool ok, const char *what) {
    if (!ok) {
        std::printf("FAIL: %s\n", what);
        failures++;
    }
}

int main() {
    check(ks10_uop_master_reset(nullptr) == KS10_NULL, "NULL handle");
    ks10_uop *m = ks10_uop_new(0);
    check(m != nullptr, "ks10_uop_new");
    if (m == nullptr) {
        return 1;
    }
    check(ks10_uop_master_reset(m) == KS10_OK, "master reset");
    const uint64_t program[] = {
        0201040000123ull,  // MOVEI 1,123
        0202040002000ull,  // MOVEM 1,2000
        0350000002000ull,  // AOS 2000
        0254200001003ull,  // HALT 1003
    };
    for (uint32_t k = 0; k < 4; k++) {
        check(ks10_uop_deposit(m, 01000 + k, program[k]) == KS10_OK, "deposit");
    }
    check(ks10_uop_deposit(m, 0, 1ull << 36) == KS10_BAD_ARG, "a word of 37 bits");
    check(ks10_uop_start(m, 01000) == KS10_OK, "start");
    ks10_record r;
    std::memset(&r, 0, sizeof r);
    for (uint64_t k = 0; k < 4; k++) {
        int s = ks10_uop_next_record(m, 100000, &r);
        check(s == KS10_OK, "a record");
        check(r.count == k + 1, "the count");
        check((r.pc & 0777777) == 01000 + k, "the PC");
    }
    check(r.ac[1] == 0123, "AC 1");
    check(r.writes == 1, "one write of AOS");
    check(ks10_uop_next_record(m, 100000, &r) == KS10_LIMIT, "no record in the halt loop");
    uint64_t w = 0;
    check(ks10_uop_examine(m, 02000, &w) == KS10_OK && w == 0124, "examine 2000");
    char text[8];
    check(ks10_uop_stop_text(m, text, sizeof text) == 0, "no stop");
    ks10_uop_free(m);
    ks10_uop_free(nullptr);
    if (failures == 0) {
        std::printf("capi: OK\n");
    }
    return failures == 0 ? 0 : 1;
}
