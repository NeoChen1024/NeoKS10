# Commands of AGENTS.md, section "Build and verification".
# Heavy jobs run with nice -n 19.

PYTHON    ?= python3
CARGO     ?= cargo
VERILATOR ?= verilator
NICE      ?= nice -n 19

.PHONY: gen check check-gen check-py check-rust check-rtl check-capi check-oracle cosim formal mutate synth cross references its-boot its-boot-soft its-boot-neoks10 microcode microcode-test diag diag-soft diag-uop verify verify-build verify-jobs oracle oracle-sims oracle-klh10

# The targets that need these give them to cargo themselves. Without this,
# a variable on the command line reaches every recipe, and `cargo test` of
# check-rust also runs the tests of the disk image.
unexport KS10_ITS_DISK KS10_DIAG_DIR KS10_ORACLE KS10_ORACLE_KLH10

BUILD_DIR ?= build
MEMPROBE_RTL = rtl/gen/regmap_pkg.sv rtl/memprobe/memprobe_sync.sv \
               rtl/memprobe/memprobe_pattern.sv rtl/memprobe/memprobe_gen.sv \
               rtl/memprobe/memprobe_hist_bank.sv rtl/memprobe/memprobe_hist.sv \
               rtl/memprobe/memprobe_regs.sv \
               rtl/memprobe/memprobe_core.sv
MEMPROBE_TOP = rtl/memprobe/memprobe_top.sv
COSIM_SEEDS ?= 1 2 3 4 5 6

# Write the files that come from spec/.
gen:
	$(PYTHON) spec/gen_regmap.py
	$(PYTHON) spec/gen_instructions.py
	$(PYTHON) spec/gen_devices.py
	$(PYTHON) spec/gen_microcode.py --its $(ITS_TREE)
	$(PYTHON) spec/gen_capi.py
	cd emu && $(CARGO) run -q --locked -p neoks10 -- help --markdown > ../docs/console.md

# Format, lint, unit tests, a check that the generated files are current,
# and a short random differential run against the reference emulator.
check: check-gen check-py check-rust check-rtl check-capi check-oracle

# The instruction table is also compared with the ITS sources of the
# submodule 3rdparty/its (microcode, MIDAS, KSDEFS).
ITS_TREE = 3rdparty/its
check-gen:
	$(PYTHON) spec/gen_regmap.py --check
	$(PYTHON) spec/gen_instructions.py --check --its $(ITS_TREE)
	$(PYTHON) spec/gen_devices.py --check --its $(ITS_TREE)
	$(PYTHON) spec/gen_microcode.py --check --its $(ITS_TREE)
	$(PYTHON) spec/gen_capi.py --check
	cd emu && $(CARGO) run -q --locked -p neoks10 -- help --markdown | diff -u ../docs/console.md -

check-py:
	$(PYTHON) -m unittest discover -s spec

check-rust:
	cd emu && $(CARGO) fmt --check
	cd emu && $(NICE) $(CARGO) clippy --locked --all-targets -- -D warnings
	cd emu && $(NICE) $(CARGO) test --locked

# 2000 random instructions against each reference emulator that `make
# oracle` built (emu/ks10-core-soft/tests/oracle.rs). Without one, the target says
# SKIPPED for it and does not fail.
ORACLE_CASES ?= 2000
check-oracle:
	@vars=""; \
	if [ -x $(ORACLE_SIMH) ]; then vars="$$vars KS10_ORACLE=$(ORACLE_SIMH)"; \
	else echo "check-oracle: SIMH SKIPPED, $(ORACLE_SIMH) is missing (make oracle)"; fi; \
	if [ -x $(ORACLE_KLH10) ]; then vars="$$vars KS10_ORACLE_KLH10=$(ORACLE_KLH10)"; \
	else echo "check-oracle: KLH10 SKIPPED, $(ORACLE_KLH10) is missing (make oracle)"; fi; \
	if [ -n "$$vars" ]; then \
	    cd emu && env $$vars KS10_ORACLE_CASES=$(ORACLE_CASES) \
	        $(NICE) $(CARGO) test -q --locked --release -p ks10-core-soft --test oracle -- --nocapture; \
	fi

check-rtl:
	$(VERILATOR) --lint-only -Wall rtl/gen/regmap_pkg.sv
	$(VERILATOR) --lint-only -Wall --top-module memprobe_top \
	    tb/memprobe/lint_stubs.sv $(MEMPROBE_RTL) $(MEMPROBE_TOP)

# A C++ caller of the C interface of the microcode-level model
# (spec/capi.toml): it links the static library of emu/ks10-capi. The
# libraries after it are those that `--print native-static-libs` names.
CAPI_LIB = emu/target/release/libks10_capi.a
CAPI_SMOKE = $(BUILD_DIR)/tb/capi/smoke
check-capi:
	cd emu && $(NICE) $(CARGO) build -q --locked --release -p ks10-capi
	mkdir -p $(BUILD_DIR)/tb/capi
	$(CXX) -std=c++17 -Wall -Wextra -Werror -Itb/gen -o $(CAPI_SMOKE) tb/capi/smoke.cpp \
	    $(CAPI_LIB) -lgcc_s -lutil -lrt -lpthread -lm -ldl
	$(CAPI_SMOKE)

# Verilator testbenches. Each seed also selects the ratio of the two clocks.
TB_CORE = $(BUILD_DIR)/tb/memprobe/tb_core

$(TB_CORE): $(MEMPROBE_RTL) tb/memprobe/tb_core.cpp tb/gen/regmap.h
	mkdir -p $(BUILD_DIR)/tb/memprobe/obj
	$(NICE) $(VERILATOR) --cc --exe --build -j 0 -Wall --top-module memprobe_core \
	    -GBUILD=32\'h5EED0001 -Mdir $(BUILD_DIR)/tb/memprobe/obj -o ../tb_core \
	    -CFLAGS -O1 -CFLAGS -std=c++17 $(MEMPROBE_RTL) tb/memprobe/tb_core.cpp

# tb_hist with the 4 banks of the design, and with 1 bank (a full FIFO).
TB_HIST4 = $(BUILD_DIR)/tb/memprobe/tb_hist4
TB_HIST1 = $(BUILD_DIR)/tb/memprobe/tb_hist1
HIST_SRC = rtl/memprobe/memprobe_hist_bank.sv rtl/memprobe/memprobe_hist.sv tb/memprobe/tb_hist.cpp

$(TB_HIST4): $(HIST_SRC)
	mkdir -p $(BUILD_DIR)/tb/memprobe/obj_hist4
	$(NICE) $(VERILATOR) --cc --exe --build -j 0 -Wall --top-module memprobe_hist -GBANKS_LOG2=2 \
	    -Mdir $(BUILD_DIR)/tb/memprobe/obj_hist4 -o ../tb_hist4 -CFLAGS -O1 -CFLAGS -std=c++17 $(HIST_SRC)

$(TB_HIST1): $(HIST_SRC)
	mkdir -p $(BUILD_DIR)/tb/memprobe/obj_hist1
	$(NICE) $(VERILATOR) --cc --exe --build -j 0 -Wall --top-module memprobe_hist -GBANKS_LOG2=0 \
	    -Mdir $(BUILD_DIR)/tb/memprobe/obj_hist1 -o ../tb_hist1 -CFLAGS -O1 -CFLAGS -std=c++17 $(HIST_SRC)

cosim: $(TB_CORE) $(TB_HIST4) $(TB_HIST1)
	for s in $(COSIM_SEEDS); do \
	    $(NICE) $(TB_HIST4) --banks 4 --seed $$s || exit 1; \
	    $(NICE) $(TB_HIST1) --banks 1 --seed $$s || exit 1; \
	    $(NICE) $(TB_CORE) --pattern tests/memprobe-pattern.txt --seed $$s || exit 1; \
	done

# SymbiYosys tasks: proof and reachability, each with a limit of 600 s
# (the timeout option of the .sby file). A timeout is not a pass. All modules
# run; the exit status is 1 if a task of any module did not pass.
# MODULE selects one of: gen regs. Without MODULE, all run.
FORMAL_MODULES = gen regs
formal:
	@mkdir -p $(CURDIR)/$(BUILD_DIR)/formal
	@fail=0; for m in $(or $(MODULE),$(FORMAL_MODULES)); do \
	    log=$(CURDIR)/$(BUILD_DIR)/formal/$$m.log; \
	    (cd tb/memprobe && $(NICE) sby -f --prefix $(CURDIR)/$(BUILD_DIR)/formal/$$m $$m.sby) \
	        > $$log 2>&1; rc=$$?; \
	    grep -E "DONE \(" $$log | sed "s/^/$$m: /"; \
	    if [ $$rc -ne 0 ]; then echo "$$m: sby exit status $$rc, see $$log"; fail=1; fi; \
	done; exit $$fail

# Mutation test of one module of rtl/memprobe: make mutate MODULE=memprobe_gen
MUTANTS ?= 20
mutate:
	$(PYTHON) tb/mutate.py --module $(MODULE) --count $(MUTANTS)

# Vivado builds of rtl/memprobe. The design is checked at 100 MHz; the other
# clocks of the HP side (SYNTH_CLOCKS="50 150") are optional: if one does not
# meet timing, record it, do not change the design for it. Set VIVADO if
# vivado is not in PATH. Read build/fpga/memprobe-<MHz>/timing.rpt after it.
SYNTH_CLOCKS ?= 100
synth:
	@for c in $(SYNTH_CLOCKS); do \
	    echo "memprobe $$c MHz:"; \
	    CLK_MHZ=$$c OUT=$(CURDIR)/$(BUILD_DIR)/fpga/memprobe-$$c sh fpga/memprobe/build.sh || exit 1; \
	done

# The programs of emu/ for the board: a static armv7 musl binary in
# emu/target/armv7-unknown-linux-musleabihf/release/. The linker is in
# emu/.cargo/config.toml. Also the lint and the unit tests for that target,
# the tests under qemu-arm (32-bit ABI, code only for ARM).
CROSS_TARGET = armv7-unknown-linux-musleabihf
QEMU_ARM ?= qemu-arm
cross:
	cd emu && $(NICE) $(CARGO) clippy --locked --all-targets --target $(CROSS_TARGET) -- -D warnings
	cd emu && CARGO_TARGET_ARMV7_UNKNOWN_LINUX_MUSLEABIHF_RUNNER=$(QEMU_ARM) \
	    $(NICE) $(CARGO) test --locked --target $(CROSS_TARGET)
	cd emu && $(NICE) $(CARGO) build --locked --release --target $(CROSS_TARGET)

# Boot ITS from the disk image of the ITS build (pdp10-ks): KS10_ITS_DISK
# names its rp0.dsk; dskdmp.tape and minsys.tape are in the same
# directory. The image is a third-party file, so the target uses copies
# in the build output.
# 1. The tests of emu/ks10-core-soft that need the image (boot, idle loop, tapes).
# 2. neoks10 runs tests/its-boot/boot.script with a record; a replay of
#    the record on a fresh copy must give the same CTY output.
ITS_BOOT_DIR = $(abspath $(BUILD_DIR))/its-boot
NEOKS10 = $(CURDIR)/emu/target/release/neoks10
its-boot: its-boot-soft its-boot-neoks10

its-boot-soft:
	@test -n "$(KS10_ITS_DISK)" || { echo "its-boot: KS10_ITS_DISK must name rp0.dsk of the ITS build" >&2; exit 2; }
	cd emu && KS10_ITS_DISK="$(abspath $(KS10_ITS_DISK))" $(NICE) $(CARGO) test --locked --release -p ks10-core-soft \
	    --test idle --test tape

its-boot-neoks10:
	@test -n "$(KS10_ITS_DISK)" || { echo "its-boot: KS10_ITS_DISK must name rp0.dsk of the ITS build" >&2; exit 2; }
	cd emu && $(NICE) $(CARGO) build --locked --release -p neoks10
	rm -rf $(ITS_BOOT_DIR) && mkdir -p $(ITS_BOOT_DIR)
	cp tests/its-boot/its.toml tests/its-boot/boot.script $(ITS_BOOT_DIR)/
	cp "$(KS10_ITS_DISK)" $(ITS_BOOT_DIR)/rp0.start
	cp $(ITS_BOOT_DIR)/rp0.start $(ITS_BOOT_DIR)/rp0.dsk
	cd $(ITS_BOOT_DIR) && $(NICE) $(NEOKS10) run its.toml --no-control --script boot.script \
	    --record boot.rec < /dev/null > boot.out
	grep -q "FREE BLOCKS" $(ITS_BOOT_DIR)/boot.out
	cp $(ITS_BOOT_DIR)/rp0.start $(ITS_BOOT_DIR)/rp0.dsk
	cd $(ITS_BOOT_DIR) && $(NICE) $(NEOKS10) run --no-control --replay boot.rec < /dev/null > replay.out
	cmp $(ITS_BOOT_DIR)/boot.out $(ITS_BOOT_DIR)/replay.out
	rm -f $(ITS_BOOT_DIR)/rp0.start $(ITS_BOOT_DIR)/rp0.dsk
	@echo "its-boot: ITS booted, listed SYS;, and the replay gave the same output"

# The microcode of the ITS build (emu/ks10-core-uop). KS10_ITS_DISK names rp0.dsk
# of the ITS build; output.tape, the dump tape of the build, is in the same
# directory. itstar (the submodule tools/itstar of 3rdparty/its, pinned
# there) extracts the listing KSHACK; MCR 262 into the build output.
# spec/micro.py compares its assembly with the listing, line by line. The
# test emu/ks10-core-uop/tests/ram_file.rs reads the RAM file from the disk as
# the 8080 does and compares it with the assembly (ucode.rs), and
# its_boot.rs boots ITS from the disk on the model. The listing is
# extracted again only if output.tape is newer.
MICROCODE_DIR = $(abspath $(BUILD_DIR))/microcode
MICROCODE_LISTING = $(MICROCODE_DIR)/mcr.262
ITSTAR = $(ITS_TREE)/tools/itstar
ITS_TAPE = $(abspath $(dir $(KS10_ITS_DISK)))/output.tape
microcode: microcode-test

$(MICROCODE_LISTING): $(wildcard $(ITS_TAPE))
	@test -n "$(KS10_ITS_DISK)" || { echo "microcode: KS10_ITS_DISK must name rp0.dsk of the ITS build" >&2; exit 2; }
	@test -f "$(ITS_TAPE)" || { echo "microcode: output.tape is not next to $(KS10_ITS_DISK)" >&2; exit 2; }
	@test -f $(ITSTAR)/itstar.c || { echo "microcode: run git -C $(ITS_TREE) submodule update --init tools/itstar" >&2; exit 2; }
	rm -rf $(MICROCODE_DIR)/tape && mkdir -p $(MICROCODE_DIR)/tape
	$(CC) -O2 -o $(MICROCODE_DIR)/itstar $(addprefix $(ITSTAR)/,itstar.c dirlst.c pack.c tapeio.c tm03.c unpack.c zopen.c)
	cd $(MICROCODE_DIR)/tape && $(NICE) ../itstar -xf "$(ITS_TAPE)" > ../itstar.log
	cp $(MICROCODE_DIR)/tape/kshack/mcr.262 $@.new && mv $@.new $@
	rm -rf $(MICROCODE_DIR)/tape

microcode-test: $(if $(KS10_ITS_DISK),$(MICROCODE_LISTING))
	@test -n "$(KS10_ITS_DISK)" || { echo "microcode: KS10_ITS_DISK must name rp0.dsk of the ITS build" >&2; exit 2; }
	$(PYTHON) spec/micro.py --its $(ITS_TREE) --compare $(MICROCODE_LISTING)
	cd emu && KS10_ITS_DISK="$(abspath $(KS10_ITS_DISK))" \
	    $(NICE) $(CARGO) test --locked -p ks10-core-uop --test ram_file --test its_boot -- --nocapture

# The DEC KS10 processor diagnostics (docs/emulator.md, "Diagnostics").
# The tape comes from `make references`. t10backup of tapeutils (the
# submodule tools/tapeutils of 3rdparty/its, pinned there) extracts the
# files into the build output. The test emu/ks10-core-soft/tests/diag.rs runs the
# programs of the run file SMCPU.CMD. emu/ks10-core-uop/tests/diag.rs runs
# them on the microcode-level model.
# The files are extracted again only if the tape is newer than the stamp.
DIAG_DIR = $(abspath $(BUILD_DIR))/diag
DIAG_TAPE = references/trailing-edge/tapes/ks_diag_gs.tap.bz2
DIAG_STAMP = $(DIAG_DIR)/extracted
TAPEUTILS = $(ITS_TREE)/tools/tapeutils
diag: diag-soft diag-uop

$(DIAG_STAMP): $(wildcard $(DIAG_TAPE))
	@test -f $(DIAG_TAPE) || { echo "diag: $(DIAG_TAPE) is missing; run make references" >&2; exit 2; }
	@test -f $(TAPEUTILS)/t10backup.c || { echo "diag: run git -C $(ITS_TREE) submodule update --init tools/tapeutils" >&2; exit 2; }
	rm -rf $(DIAG_DIR) && mkdir -p $(DIAG_DIR)/files
	$(CC) -O2 -o $(DIAG_DIR)/t10backup $(TAPEUTILS)/t10backup.c $(TAPEUTILS)/tapeio.c
	bunzip2 -c $(DIAG_TAPE) > $(DIAG_DIR)/ks_diag_gs.tap
	cd $(DIAG_DIR)/files && ../t10backup -x -f ../ks_diag_gs.tap '*' > ../t10backup.log
	touch $@

diag-soft: $(DIAG_STAMP)
	cd emu && KS10_DIAG_DIR=$(DIAG_DIR)/files $(NICE) $(CARGO) test --locked --release -p ks10-core-soft \
	    --test diag -- --nocapture

diag-uop: $(DIAG_STAMP)
	cd emu && KS10_DIAG_DIR=$(DIAG_DIR)/files $(NICE) $(CARGO) test --locked --release -p ks10-core-uop \
	    --test diag -- --nocapture

# The rows of AGENTS.md for a change of the software processor or a
# device, and before a merge: check, microcode, diag and its-boot. Run it
# as `make -j verify KS10_ITS_DISK=<rp0.dsk>`. verify-build compiles the
# test programs first, so that the parallel jobs do not wait for the
# build lock of cargo. -k runs the other jobs after a failure; -Otarget
# keeps the output of each job together.
verify: verify-build
	+$(MAKE) -k -Otarget verify-jobs

verify-jobs: check microcode-test diag its-boot

verify-build:
	@test -n "$(KS10_ITS_DISK)" || { echo "verify: KS10_ITS_DISK must name rp0.dsk of the ITS build" >&2; exit 2; }
	cd emu && $(NICE) $(CARGO) test --locked --no-run
	cd emu && $(NICE) $(CARGO) test --locked --release --no-run -p ks10-core-soft -p ks10-core-uop
	cd emu && $(NICE) $(CARGO) build --locked --release -p neoks10

# The reference emulators with their trace hooks (oracle/). Each source is
# a submodule of 3rdparty/its, which must be at the commit of its pin
# file. The build works on a copy in the build output; the submodules stay
# clean.
ORACLE_DIR = $(abspath $(BUILD_DIR))/oracle
SIMS = $(ITS_TREE)/tools/sims
SIMS_COMMIT = $(shell sed -n 's/^commit //p' oracle/sims.pin)
ORACLE_SIMH = $(ORACLE_DIR)/sims/BIN/pdp10-ks
KLH10_SRC = $(ITS_TREE)/tools/klh10
KLH10_COMMIT = $(shell sed -n 's/^commit //p' oracle/klh10.pin)
ORACLE_KLH10 = $(ORACLE_DIR)/klh10/tmp/bld-ks-its/kn10-ks-its
oracle: oracle-sims oracle-klh10

oracle-sims:
	@test -f $(SIMS)/makefile || { echo "oracle: run git -C $(ITS_TREE) submodule update --init tools/sims" >&2; exit 2; }
	@test "$$(git -C $(SIMS) rev-parse HEAD)" = "$(SIMS_COMMIT)" || { echo "oracle: tools/sims is not at the commit of oracle/sims.pin" >&2; exit 2; }
	rm -rf $(ORACLE_DIR)/sims && mkdir -p $(ORACLE_DIR)/sims
	git -C $(SIMS) archive HEAD | tar -x -C $(ORACLE_DIR)/sims
	cd $(ORACLE_DIR)/sims && patch -p1 < $(CURDIR)/oracle/sims-trace.patch
	$(NICE) $(MAKE) -C $(ORACLE_DIR)/sims pdp10-ks

# The configuration of the ITS tree (its Makefile, target $(KLH10)).
oracle-klh10:
	@test -f $(KLH10_SRC)/configure.ac || { echo "oracle: run git -C $(ITS_TREE) submodule update --init tools/klh10" >&2; exit 2; }
	@test "$$(git -C $(KLH10_SRC) rev-parse HEAD)" = "$(KLH10_COMMIT)" || { echo "oracle: tools/klh10 is not at the commit of oracle/klh10.pin" >&2; exit 2; }
	rm -rf $(ORACLE_DIR)/klh10 && mkdir -p $(ORACLE_DIR)/klh10/tmp
	git -C $(KLH10_SRC) archive HEAD | tar -x -C $(ORACLE_DIR)/klh10
	cd $(ORACLE_DIR)/klh10 && patch -p1 < $(CURDIR)/oracle/klh10-trace.patch
	cd $(ORACLE_DIR)/klh10 && ./autogen.sh
	cd $(ORACLE_DIR)/klh10/tmp && CONFFLAGS_USR="-DKLH10_DEV_DPTM03=0" ../configure
	$(NICE) $(MAKE) -C $(ORACLE_DIR)/klh10/tmp/bld-ks-its

# Third-party files (third-party.lock) into references/, which git ignores.
# bitsavers.org has an rsync server; pdp-10.trailing-edge.com is read with
# curl. The files keep their path of the site. The check fails if a file is
# not the one of the lock file.
TRAILING_EDGE = https://pdp-10.trailing-edge.com/
references:
	@mkdir -p references
	@grep -v '^#' third-party.lock | sed -n 's|^[0-9a-f]*  https://bitsavers.org/||p' \
	    > references/.bitsavers-files
	rsync -a --files-from=references/.bitsavers-files rsync://bitsavers.org/bitsavers/ \
	    references/bitsavers/
	@for p in $$(grep -v '^#' third-party.lock | sed -n 's|^[0-9a-f]*  $(TRAILING_EDGE)||p'); do \
	    f=references/trailing-edge/$$p; \
	    test -f $$f && continue; \
	    mkdir -p $$(dirname $$f) && echo "curl $$p" \
	        && curl -s -f -A "Mozilla/5.0" -o $$f $(TRAILING_EDGE)$$p || exit 1; \
	done
	@grep -v '^#' third-party.lock \
	    | sed -e 's|https://bitsavers.org/|references/bitsavers/|' \
	          -e 's|$(TRAILING_EDGE)|references/trailing-edge/|' \
	    | sha256sum --quiet -c -
	@echo "references: all files match third-party.lock"
