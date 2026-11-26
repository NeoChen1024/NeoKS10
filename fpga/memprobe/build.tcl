# Synthesis, place and route of the memory path test design, without a project.
# build.sh runs it in the output directory:
#
#   vivado -mode batch -source build.tcl -tclargs <stage> <part> <clk_mhz> <build> <xdc> <source>...
#
# stage: "synth" stops after synthesis; "all" also writes memprobe.bit.
# build: 8 hexadecimal digits for the BUILD register.
set sources [lassign $argv stage part clk_mhz build xdc]

read_verilog -sv $sources
synth_design -top memprobe_top -part $part \
    -generic CLK_MHZ=$clk_mhz -generic "BUILD=32'h$build"
report_utilization -file synth_utilization.rpt
report_utilization -hierarchical -file synth_utilization_hier.rpt
if {$stage eq "synth"} {
    write_checkpoint -force synth.dcp
    exit
}

# The constraints use the clock period of clk_m.
set clk_m_period [expr {1000.0 / $clk_mhz}]
read_xdc $xdc
opt_design
place_design
phys_opt_design
route_design
report_timing_summary -max_paths 20 -file timing.rpt
report_clock_interaction -file clocks.rpt
report_cdc -details -file cdc.rpt
report_methodology -file methodology.rpt
report_utilization -file utilization.rpt
report_drc -file drc.rpt
write_bitstream -force memprobe.bit
