# Commands of AGENTS.md, section "Build and verification".
# Heavy jobs run with nice -n 19.

PYTHON    ?= python3
CARGO     ?= cargo
VERILATOR ?= verilator
NICE      ?= nice -n 19

.PHONY: gen check check-gen check-py check-rust check-rtl

# Write the files that come from spec/.
gen:
	$(PYTHON) spec/gen_regmap.py

# Format, lint, unit tests, and a check that the generated files are current.
check: check-gen check-py check-rust check-rtl

check-gen:
	$(PYTHON) spec/gen_regmap.py --check

check-py:
	$(PYTHON) -m unittest discover -s spec

check-rust:
	cd emu && $(CARGO) fmt --check
	cd emu && $(NICE) $(CARGO) clippy --locked --all-targets -- -D warnings
	cd emu && $(NICE) $(CARGO) test --locked

check-rtl:
	$(VERILATOR) --lint-only -Wall rtl/gen/regmap_pkg.sv
