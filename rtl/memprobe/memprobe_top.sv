// Top of the memory path test design for the PYNQ-Z2 (docs/mem-path-experiment.md,
// Part B). It follows the rules of the board interface (docs/pl-interface.md of
// the board repository, tag pl-interface-v1):
//   - the PS7 cell is instantiated directly; IRQF2P[19:16] stay 0
//   - FCLK0 (100 MHz) clocks M_AXI_GP0 and the registers; the GP0 side does not
//     depend on the MMCM, so the registers answer also if the MMCM is not locked
//   - an MMCM makes clk_m (CLK_MHZ) from FCLK0 for S_AXI_HP0
//   - the interrupt is IRQF2P[0]
// The design has no I/O pin.
module memprobe_top #(
    parameter [31:0] BUILD   = 32'h0,
    parameter        CLK_MHZ = 100      // 50, 100 or 150
) ();
    // VCO = 100 MHz * 9 = 900 MHz; the divider gives 50, 100 or 150 MHz.
    localparam real CLKOUT_DIVIDE = 900.0 / CLK_MHZ;

    logic [3:0] fclk, fclk_resetn;
    logic       clk_s, clk_m, clk_m_mmcm, mmcm_fb, mmcm_locked;
    BUFG fclk0_bufg (.I(fclk[0]), .O(clk_s));

    MMCME2_BASE #(
        .CLKIN1_PERIOD(10.0),
        .DIVCLK_DIVIDE(1),
        .CLKFBOUT_MULT_F(9.0),
        .CLKOUT0_DIVIDE_F(CLKOUT_DIVIDE)
    ) mmcm (
        .CLKIN1(clk_s), .CLKFBIN(mmcm_fb), .CLKFBOUT(mmcm_fb), .CLKOUT0(clk_m_mmcm),
        .LOCKED(mmcm_locked), .RST(!fclk_resetn[0]), .PWRDWN(1'b0)
    );
    BUFG clk_m_bufg (.I(clk_m_mmcm), .O(clk_m));

    // Resets: FCLKRESETN[0] is 0 while FPGA_RST_CTRL[0] is 1. Each domain
    // releases its reset on its clock. clk_m also stays in reset while the MMCM
    // is not locked.
    logic [1:0] rst_s_sync, rst_m_sync;
    always_ff @(posedge clk_s or negedge fclk_resetn[0])
        if (!fclk_resetn[0]) rst_s_sync <= '0;
        else                 rst_s_sync <= {rst_s_sync[0], 1'b1};
    wire rst_m_n = fclk_resetn[0] && mmcm_locked;
    always_ff @(posedge clk_m or negedge rst_m_n)
        if (!rst_m_n) rst_m_sync <= '0;
        else          rst_m_sync <= {rst_m_sync[0], 1'b1};

    // M_AXI_GP0
    logic [31:0] gp_awaddr, gp_araddr, gp_wdata, gp_rdata;
    logic [11:0] gp_awid, gp_arid, gp_bid, gp_rid;
    logic [3:0]  gp_awlen, gp_arlen, gp_wstrb;
    logic [1:0]  gp_bresp, gp_rresp;
    logic        gp_awvalid, gp_awready, gp_wvalid, gp_wready, gp_wlast;
    logic        gp_bvalid, gp_bready, gp_arvalid, gp_arready, gp_rvalid, gp_rready, gp_rlast;

    // S_AXI_HP0
    logic [31:0] hp_awaddr, hp_araddr;
    logic [3:0]  hp_awlen, hp_arlen;
    logic [63:0] hp_wdata, hp_rdata;
    logic [1:0]  hp_bresp, hp_rresp;
    logic        hp_awvalid, hp_awready, hp_wvalid, hp_wready, hp_wlast;
    logic        hp_bvalid, hp_bready, hp_arvalid, hp_arready, hp_rvalid, hp_rready, hp_rlast;

    logic irq, busy;

    PS7 ps7_i (
        .FCLKCLK(fclk), .FCLKRESETN(fclk_resetn), .FCLKCLKTRIGN(4'b0),
        .IRQF2P({19'b0, irq}),

        .MAXIGP0ACLK(clk_s),
        .MAXIGP0AWADDR(gp_awaddr), .MAXIGP0AWID(gp_awid), .MAXIGP0AWLEN(gp_awlen),
        .MAXIGP0AWVALID(gp_awvalid), .MAXIGP0AWREADY(gp_awready),
        .MAXIGP0WDATA(gp_wdata), .MAXIGP0WSTRB(gp_wstrb), .MAXIGP0WLAST(gp_wlast),
        .MAXIGP0WVALID(gp_wvalid), .MAXIGP0WREADY(gp_wready),
        .MAXIGP0BID(gp_bid), .MAXIGP0BRESP(gp_bresp), .MAXIGP0BVALID(gp_bvalid),
        .MAXIGP0BREADY(gp_bready),
        .MAXIGP0ARADDR(gp_araddr), .MAXIGP0ARID(gp_arid), .MAXIGP0ARLEN(gp_arlen),
        .MAXIGP0ARVALID(gp_arvalid), .MAXIGP0ARREADY(gp_arready),
        .MAXIGP0RDATA(gp_rdata), .MAXIGP0RID(gp_rid), .MAXIGP0RRESP(gp_rresp),
        .MAXIGP0RLAST(gp_rlast), .MAXIGP0RVALID(gp_rvalid), .MAXIGP0RREADY(gp_rready),

        .SAXIHP0ACLK(clk_m),
        .SAXIHP0AWADDR(hp_awaddr), .SAXIHP0AWID(6'd0), .SAXIHP0AWLEN(hp_awlen),
        .SAXIHP0AWSIZE(2'b11), .SAXIHP0AWBURST(2'b01), .SAXIHP0AWCACHE(4'b0011),
        .SAXIHP0AWPROT(3'b000), .SAXIHP0AWLOCK(2'b00), .SAXIHP0AWQOS(4'd0),
        .SAXIHP0AWVALID(hp_awvalid), .SAXIHP0AWREADY(hp_awready),
        .SAXIHP0WDATA(hp_wdata), .SAXIHP0WID(6'd0), .SAXIHP0WSTRB(8'hFF),
        .SAXIHP0WLAST(hp_wlast), .SAXIHP0WVALID(hp_wvalid), .SAXIHP0WREADY(hp_wready),
        .SAXIHP0BRESP(hp_bresp), .SAXIHP0BVALID(hp_bvalid), .SAXIHP0BREADY(hp_bready),
        .SAXIHP0ARADDR(hp_araddr), .SAXIHP0ARID(6'd0), .SAXIHP0ARLEN(hp_arlen),
        .SAXIHP0ARSIZE(2'b11), .SAXIHP0ARBURST(2'b01), .SAXIHP0ARCACHE(4'b0011),
        .SAXIHP0ARPROT(3'b000), .SAXIHP0ARLOCK(2'b00), .SAXIHP0ARQOS(4'd0),
        .SAXIHP0ARVALID(hp_arvalid), .SAXIHP0ARREADY(hp_arready),
        .SAXIHP0RDATA(hp_rdata), .SAXIHP0RRESP(hp_rresp), .SAXIHP0RLAST(hp_rlast),
        .SAXIHP0RVALID(hp_rvalid), .SAXIHP0RREADY(hp_rready),
        .SAXIHP0RDISSUECAP1EN(1'b0), .SAXIHP0WRISSUECAP1EN(1'b0)
    );

    memprobe_core #(.BUILD(BUILD), .CLK_MHZ(CLK_MHZ)) core (
        .clk_s(clk_s), .rst_s(!rst_s_sync[1]),
        .gp_awaddr(gp_awaddr[19:0]), .gp_awid(gp_awid), .gp_awlen(gp_awlen),
        .gp_awvalid(gp_awvalid), .gp_awready(gp_awready), .gp_wdata(gp_wdata),
        .gp_wstrb(gp_wstrb), .gp_wlast(gp_wlast), .gp_wvalid(gp_wvalid),
        .gp_wready(gp_wready), .gp_bid(gp_bid), .gp_bresp(gp_bresp), .gp_bvalid(gp_bvalid),
        .gp_bready(gp_bready), .gp_araddr(gp_araddr[19:0]), .gp_arid(gp_arid),
        .gp_arlen(gp_arlen), .gp_arvalid(gp_arvalid), .gp_arready(gp_arready),
        .gp_rdata(gp_rdata), .gp_rid(gp_rid), .gp_rresp(gp_rresp), .gp_rlast(gp_rlast),
        .gp_rvalid(gp_rvalid), .gp_rready(gp_rready), .irq(irq), .busy(busy),
        .clk_m(clk_m), .rst_m(!rst_m_sync[1]), .mmcm_locked(mmcm_locked),
        .hp_araddr(hp_araddr), .hp_arlen(hp_arlen), .hp_arvalid(hp_arvalid),
        .hp_arready(hp_arready), .hp_rdata(hp_rdata), .hp_rresp(hp_rresp),
        .hp_rlast(hp_rlast), .hp_rvalid(hp_rvalid), .hp_rready(hp_rready),
        .hp_awaddr(hp_awaddr), .hp_awlen(hp_awlen), .hp_awvalid(hp_awvalid),
        .hp_awready(hp_awready), .hp_wdata(hp_wdata), .hp_wlast(hp_wlast),
        .hp_wvalid(hp_wvalid), .hp_wready(hp_wready), .hp_bresp(hp_bresp),
        .hp_bvalid(hp_bvalid), .hp_bready(hp_bready)
    );

    wire unused_ok = &{1'b0, fclk[3:1], fclk_resetn[3:1], gp_awaddr[31:20], gp_araddr[31:20],
                       busy};
endmodule
