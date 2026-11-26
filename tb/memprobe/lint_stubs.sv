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
    output logic [63:0] SAXIHP0RDATA,
    input  logic        SAXIHP1ACLK,
    input  logic [31:0] SAXIHP1AWADDR, SAXIHP1ARADDR,
    input  logic [5:0]  SAXIHP1AWID, SAXIHP1ARID, SAXIHP1WID,
    input  logic [3:0]  SAXIHP1AWLEN, SAXIHP1ARLEN, SAXIHP1AWCACHE, SAXIHP1ARCACHE,
                        SAXIHP1AWQOS, SAXIHP1ARQOS,
    input  logic [1:0]  SAXIHP1AWSIZE, SAXIHP1ARSIZE, SAXIHP1AWBURST, SAXIHP1ARBURST,
                        SAXIHP1AWLOCK, SAXIHP1ARLOCK,
    input  logic [2:0]  SAXIHP1AWPROT, SAXIHP1ARPROT,
    input  logic        SAXIHP1AWVALID, SAXIHP1WLAST, SAXIHP1WVALID, SAXIHP1BREADY,
                        SAXIHP1ARVALID, SAXIHP1RREADY, SAXIHP1RDISSUECAP1EN,
                        SAXIHP1WRISSUECAP1EN,
    input  logic [63:0] SAXIHP1WDATA,
    input  logic [7:0]  SAXIHP1WSTRB,
    output logic        SAXIHP1AWREADY, SAXIHP1WREADY, SAXIHP1BVALID, SAXIHP1ARREADY,
                        SAXIHP1RLAST, SAXIHP1RVALID,
    output logic [1:0]  SAXIHP1BRESP, SAXIHP1RRESP,
    output logic [5:0]  SAXIHP1BID, SAXIHP1RID,
    output logic [63:0] SAXIHP1RDATA,
    input  logic        SAXIHP2ACLK,
    input  logic [31:0] SAXIHP2AWADDR, SAXIHP2ARADDR,
    input  logic [5:0]  SAXIHP2AWID, SAXIHP2ARID, SAXIHP2WID,
    input  logic [3:0]  SAXIHP2AWLEN, SAXIHP2ARLEN, SAXIHP2AWCACHE, SAXIHP2ARCACHE,
                        SAXIHP2AWQOS, SAXIHP2ARQOS,
    input  logic [1:0]  SAXIHP2AWSIZE, SAXIHP2ARSIZE, SAXIHP2AWBURST, SAXIHP2ARBURST,
                        SAXIHP2AWLOCK, SAXIHP2ARLOCK,
    input  logic [2:0]  SAXIHP2AWPROT, SAXIHP2ARPROT,
    input  logic        SAXIHP2AWVALID, SAXIHP2WLAST, SAXIHP2WVALID, SAXIHP2BREADY,
                        SAXIHP2ARVALID, SAXIHP2RREADY, SAXIHP2RDISSUECAP1EN,
                        SAXIHP2WRISSUECAP1EN,
    input  logic [63:0] SAXIHP2WDATA,
    input  logic [7:0]  SAXIHP2WSTRB,
    output logic        SAXIHP2AWREADY, SAXIHP2WREADY, SAXIHP2BVALID, SAXIHP2ARREADY,
                        SAXIHP2RLAST, SAXIHP2RVALID,
    output logic [1:0]  SAXIHP2BRESP, SAXIHP2RRESP,
    output logic [5:0]  SAXIHP2BID, SAXIHP2RID,
    output logic [63:0] SAXIHP2RDATA,
    input  logic        SAXIHP3ACLK,
    input  logic [31:0] SAXIHP3AWADDR, SAXIHP3ARADDR,
    input  logic [5:0]  SAXIHP3AWID, SAXIHP3ARID, SAXIHP3WID,
    input  logic [3:0]  SAXIHP3AWLEN, SAXIHP3ARLEN, SAXIHP3AWCACHE, SAXIHP3ARCACHE,
                        SAXIHP3AWQOS, SAXIHP3ARQOS,
    input  logic [1:0]  SAXIHP3AWSIZE, SAXIHP3ARSIZE, SAXIHP3AWBURST, SAXIHP3ARBURST,
                        SAXIHP3AWLOCK, SAXIHP3ARLOCK,
    input  logic [2:0]  SAXIHP3AWPROT, SAXIHP3ARPROT,
    input  logic        SAXIHP3AWVALID, SAXIHP3WLAST, SAXIHP3WVALID, SAXIHP3BREADY,
                        SAXIHP3ARVALID, SAXIHP3RREADY, SAXIHP3RDISSUECAP1EN,
                        SAXIHP3WRISSUECAP1EN,
    input  logic [63:0] SAXIHP3WDATA,
    input  logic [7:0]  SAXIHP3WSTRB,
    output logic        SAXIHP3AWREADY, SAXIHP3WREADY, SAXIHP3BVALID, SAXIHP3ARREADY,
                        SAXIHP3RLAST, SAXIHP3RVALID,
    output logic [1:0]  SAXIHP3BRESP, SAXIHP3RRESP,
    output logic [5:0]  SAXIHP3BID, SAXIHP3RID,
    output logic [63:0] SAXIHP3RDATA
);
endmodule
