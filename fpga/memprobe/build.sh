#!/usr/bin/env sh
# Builds the memory path test design (rtl/memprobe) with Vivado.
#
#   [VIVADO=/path/to/vivado] [CLK_MHZ=100] [STAGE=all] [OUT=dir] sh fpga/memprobe/build.sh
#
# CLK_MHZ is the clock of the HP side: 50, 100 or 150. STAGE=synth stops after
# synthesis. OUT is build/fpga/memprobe-<CLK_MHZ> by default; it gets the
# reports and memprobe.bit. The BUILD register gets the first 8 digits of the
# git commit; if the tree has changes, the design is not for measurements.
# If timing is not met, the script deletes the bitstream and exits with 3.
set -eu
here="$(cd "$(dirname "$0")" && pwd)"
root="$(cd "$here/../.." && pwd)"
renice -n 19 -p $$ >/dev/null 2>&1 || true
vivado="${VIVADO:-vivado}"
clk="${CLK_MHZ:-100}"
stage="${STAGE:-all}"
case "$clk" in 50|100|150) ;; *) echo "CLK_MHZ must be 50, 100 or 150" >&2; exit 2 ;; esac
case "$stage" in synth|all) ;; *) echo "STAGE must be synth or all" >&2; exit 2 ;; esac

build="$(git -C "$root" rev-parse --short=8 HEAD)"
if [ -n "$(git -C "$root" status --porcelain -- rtl fpga)" ]; then
    echo "warning: rtl/ or fpga/ has changes that are not committed; BUILD $build does not identify the design" >&2
fi

out="${OUT:-$root/build/fpga/memprobe-$clk}"
rm -rf "$out"
mkdir -p "$out"
cd "$out"
"$vivado" -mode batch -nojournal -log build.log -source "$here/build.tcl" \
    -tclargs "$stage" xc7z020clg400-1 "$clk" "$build" "$here/memprobe.xdc" \
    "$root/rtl/gen/regmap_pkg.sv" \
    "$root/rtl/memprobe/memprobe_sync.sv" "$root/rtl/memprobe/memprobe_pattern.sv" \
    "$root/rtl/memprobe/memprobe_gen.sv" "$root/rtl/memprobe/memprobe_hist.sv" \
    "$root/rtl/memprobe/memprobe_regs.sv" "$root/rtl/memprobe/memprobe_core.sv" \
    "$root/rtl/memprobe/memprobe_top.sv" >build.out 2>&1 </dev/null || true
if grep -q '^ERROR' build.log; then
    echo "Vivado failed; see $out/build.log" >&2
    grep -E '^ERROR' build.log | head -5 >&2
    exit 1
fi
if [ "$stage" = synth ]; then
    echo "$out/synth_utilization.rpt"
    exit 0
fi
if [ ! -f memprobe.bit ]; then
    echo "no bitstream; see $out/build.log" >&2
    exit 1
fi
awk '/WNS\(ns\) +TNS\(ns\)/ { getline; getline; printf "setup slack %s ns, hold slack %s ns, pulse width slack %s ns\n", $1, $5, $9; exit }' timing.rpt
# Vivado writes the bitstream also when timing fails: such a bitstream is not used.
if grep -q 'Timing constraints are not met' timing.rpt; then
    echo "timing is not met; see $out/timing.rpt" >&2
    rm -f memprobe.bit
    exit 3
fi
mv memprobe.bit "memprobe-$clk.bit"
echo "$out/memprobe-$clk.bit"
