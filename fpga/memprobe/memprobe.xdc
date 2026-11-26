## Constraints of the memory path test design (rtl/memprobe/memprobe_top.sv).
## The design has no I/O pin.

## Configuration bank voltage of the board: the lines of pynq_z2_master.xdc in
## the board repository (3rdparty/PYNQ-Z2).
set_property CFGBVS VCCO [current_design]
set_property CONFIG_VOLTAGE 3.3 [current_design]

## FCLK0: 100 MHz in the board preset (docs/pl-interface.md of the board
## repository). The PS7 cell defines no clock. The MMCM gives clk_m; Vivado
## derives that clock from this one.
create_clock -name fclk0 -period 10.000 [get_pins ps7_i/FCLKCLK[0]]

## Paths between fclk0 and clk_m (memprobe_core.sv, "Crossings"):
##   - toggles and levels into the first stage of a memprobe_sync
##   - the Gray code of CLKM_COUNT
##   - configuration and results, which are stable while the other domain
##     uses them; the toggles arrive 2 or more cycles after them
## A delay of one period of the faster clock is sufficient for all of them.
## set_clock_groups is not used: it would remove this limit.
set clk_s [get_clocks fclk0]
set clk_m [get_clocks -of_objects [get_nets clk_m]]
set cross [expr {min([get_property PERIOD $clk_s], [get_property PERIOD $clk_m])}]
set_max_delay -datapath_only -from $clk_s -to $clk_m $cross
set_max_delay -datapath_only -from $clk_m -to $clk_s $cross
