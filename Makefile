# Commands of AGENTS.md, section "Build and verification".
# Heavy jobs run with nice -n 19.

PYTHON    ?= python3
CARGO     ?= cargo
VERILATOR ?= verilator
NICE      ?= nice -n 19

.PHONY: gen check check-gen check-py check-rust check-rtl cosim formal mutate synth cross references

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

# Format, lint, unit tests, and a check that the generated files are current.
check: check-gen check-py check-rust check-rtl

# The instruction table is also compared with the ITS sources of the
# submodule 3rdparty/its (microcode, MIDAS, KSDEFS).
ITS_TREE = 3rdparty/its
check-gen:
	$(PYTHON) spec/gen_regmap.py --check
	$(PYTHON) spec/gen_instructions.py --check --its $(ITS_TREE)

check-py:
	$(PYTHON) -m unittest discover -s spec

check-rust:
	cd emu && $(CARGO) fmt --check
	cd emu && $(NICE) $(CARGO) clippy --locked --all-targets -- -D warnings
	cd emu && $(NICE) $(CARGO) test --locked

check-rtl:
	$(VERILATOR) --lint-only -Wall rtl/gen/regmap_pkg.sv
	$(VERILATOR) --lint-only -Wall --top-module memprobe_top \
	    tb/memprobe/lint_stubs.sv $(MEMPROBE_RTL) $(MEMPROBE_TOP)

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
