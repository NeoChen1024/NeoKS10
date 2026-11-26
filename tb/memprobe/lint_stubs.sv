// The ports of the Xilinx primitives that memprobe_top uses, for a lint of
// the top with Verilator only. They have no behaviour. Vivado uses its own
// primitives; synthesis finds a wrong port name.
/* verilator lint_off UNUSEDSIGNAL */
/* verilator lint_off UNDRIVEN */
/* verilator lint_off DECLFILENAME */
/* verilator lint_off MULTITOP */
/* verilator lint_off UNUSEDPARAM */
// LOCKED goes to the reset synchronizer of clk_m and, through memprobe_sync,
// to STATUS.MMCM_LOCKED.
/* verilator lint_off SYNCASYNCNET */
module BUFG (input logic I, output logic O);
    assign O = I;
endmodule

module MMCME2_BASE #(
    parameter real CLKIN1_PERIOD = 0.0,
    parameter DIVCLK_DIVIDE = 1,
    parameter real CLKFBOUT_MULT_F = 5.0,
    parameter real CLKOUT0_DIVIDE_F = 1.0
) (
    input  logic CLKIN1, CLKFBIN, RST, PWRDWN,
    output logic CLKFBOUT, CLKOUT0, LOCKED     // the outputs that memprobe_top uses
);
endmodule

module PS7 (
    output logic [3:0]  FCLKCLK, FCLKRESETN,
    input  logic [3:0]  FCLKCLKTRIGN,
    input  logic [19:0] IRQF2P,
    input  logic        MAXIGP0ACLK,
    output logic [31:0] MAXIGP0AWADDR, MAXIGP0ARADDR, MAXIGP0WDATA,
    output logic [11:0] MAXIGP0AWID, MAXIGP0ARID,
    output logic [3:0]  MAXIGP0AWLEN, MAXIGP0ARLEN, MAXIGP0WSTRB,
    output logic        MAXIGP0AWVALID, MAXIGP0WLAST, MAXIGP0WVALID, MAXIGP0BREADY,
                        MAXIGP0ARVALID, MAXIGP0RREADY,
    input  logic        MAXIGP0AWREADY, MAXIGP0WREADY, MAXIGP0BVALID, MAXIGP0ARREADY,
                        MAXIGP0RLAST, MAXIGP0RVALID,
    input  logic [11:0] MAXIGP0BID, MAXIGP0RID,
    input  logic [1:0]  MAXIGP0BRESP, MAXIGP0RRESP,
    input  logic [31:0] MAXIGP0RDATA,
    input  logic        SAXIHP0ACLK,
    input  logic [31:0] SAXIHP0AWADDR, SAXIHP0ARADDR,
    input  logic [5:0]  SAXIHP0AWID, SAXIHP0ARID, SAXIHP0WID,
    input  logic [3:0]  SAXIHP0AWLEN, SAXIHP0ARLEN, SAXIHP0AWCACHE, SAXIHP0ARCACHE,
                        SAXIHP0AWQOS, SAXIHP0ARQOS,
    input  logic [1:0]  SAXIHP0AWSIZE, SAXIHP0ARSIZE, SAXIHP0AWBURST, SAXIHP0ARBURST,
                        SAXIHP0AWLOCK, SAXIHP0ARLOCK,
    input  logic [2:0]  SAXIHP0AWPROT, SAXIHP0ARPROT,
    input  logic        SAXIHP0AWVALID, SAXIHP0WLAST, SAXIHP0WVALID, SAXIHP0BREADY,
                        SAXIHP0ARVALID, SAXIHP0RREADY, SAXIHP0RDISSUECAP1EN,
                        SAXIHP0WRISSUECAP1EN,
    input  logic [63:0] SAXIHP0WDATA,
    input  logic [7:0]  SAXIHP0WSTRB,
    output logic        SAXIHP0AWREADY, SAXIHP0WREADY, SAXIHP0BVALID, SAXIHP0ARREADY,
                        SAXIHP0RLAST, SAXIHP0RVALID,
    output logic [1:0]  SAXIHP0BRESP, SAXIHP0RRESP,
    output logic [5:0]  SAXIHP0BID, SAXIHP0RID,
    output logic [63:0] SAXIHP0RDATA
);
endmodule
