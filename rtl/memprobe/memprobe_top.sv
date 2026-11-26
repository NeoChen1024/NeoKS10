// Top of the memory path test design for the PYNQ-Z2 (docs/mem-path-experiment.md,
// Part B). It follows the rules of the board interface (docs/pl-interface.md of
// the board repository, tag pl-interface-v1):
//   - the PS7 cell is instantiated directly; IRQF2P[19:16] stay 0
//   - FCLK0 (100 MHz) clocks M_AXI_GP0 and the registers; the GP0 side does not
//     depend on the MMCM, so the registers answer also if the MMCM is not locked
//   - an MMCM makes clk_m (CLK_MHZ) from FCLK0 for S_AXI_HP0 to S_AXI_HP3
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
    // releases its reset on its clock. FCLKRESETN[0] also resets the MMCM, so
    // LOCKED is 0 then; clk_m is in reset while LOCKED is 0. (No gate between
    // the two signals: a gate output on an asynchronous clear can glitch.)
    logic [1:0] rst_s_sync, rst_m_sync;
    always_ff @(posedge clk_s or negedge fclk_resetn[0])
        if (!fclk_resetn[0]) rst_s_sync <= '0;
        else                 rst_s_sync <= {rst_s_sync[0], 1'b1};
    always_ff @(posedge clk_m or negedge mmcm_locked)
        if (!mmcm_locked) rst_m_sync <= '0;
        else              rst_m_sync <= {rst_m_sync[0], 1'b1};

    // M_AXI_GP0
    logic [31:0] gp_awaddr, gp_araddr, gp_wdata, gp_rdata;
    logic [11:0] gp_awid, gp_arid, gp_bid, gp_rid;
    logic [3:0]  gp_awlen, gp_arlen, gp_wstrb;
    logic [1:0]  gp_bresp, gp_rresp;
    logic        gp_awvalid, gp_awready, gp_wvalid, gp_wready, gp_wlast;
    logic        gp_bvalid, gp_bready, gp_arvalid, gp_arready, gp_rvalid, gp_rready, gp_rlast;

    // S_AXI_HP0 to S_AXI_HP3
    logic [31:0] hp_awaddr [0:3], hp_araddr [0:3];
    logic [3:0]  hp_awlen [0:3], hp_arlen [0:3];
    logic [5:0]  hp_awid [0:3], hp_arid [0:3], hp_wid [0:3], hp_rid [0:3], hp_bid [0:3];
    logic [63:0] hp_wdata [0:3], hp_rdata [0:3];
    logic [1:0]  hp_bresp [0:3], hp_rresp [0:3];
    logic        hp_awvalid [0:3], hp_awready [0:3], hp_wvalid [0:3], hp_wready [0:3], hp_wlast [0:3];
    logic        hp_bvalid [0:3], hp_bready [0:3], hp_arvalid [0:3], hp_arready [0:3];
    logic        hp_rvalid [0:3], hp_rready [0:3], hp_rlast [0:3];

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
        .SAXIHP0AWADDR(hp_awaddr[0]), .SAXIHP0AWID(hp_awid[0]), .SAXIHP0AWLEN(hp_awlen[0]),
        .SAXIHP0AWSIZE(2'b11), .SAXIHP0AWBURST(2'b01), .SAXIHP0AWCACHE(4'b0011),
        .SAXIHP0AWPROT(3'b000), .SAXIHP0AWLOCK(2'b00), .SAXIHP0AWQOS(4'd0),
        .SAXIHP0AWVALID(hp_awvalid[0]), .SAXIHP0AWREADY(hp_awready[0]),
        .SAXIHP0WDATA(hp_wdata[0]), .SAXIHP0WID(hp_wid[0]), .SAXIHP0WSTRB(8'hFF),
        .SAXIHP0WLAST(hp_wlast[0]), .SAXIHP0WVALID(hp_wvalid[0]), .SAXIHP0WREADY(hp_wready[0]),
        .SAXIHP0BRESP(hp_bresp[0]), .SAXIHP0BID(hp_bid[0]), .SAXIHP0BVALID(hp_bvalid[0]),
        .SAXIHP0BREADY(hp_bready[0]),
        .SAXIHP0ARADDR(hp_araddr[0]), .SAXIHP0ARID(hp_arid[0]), .SAXIHP0ARLEN(hp_arlen[0]),
        .SAXIHP0ARSIZE(2'b11), .SAXIHP0ARBURST(2'b01), .SAXIHP0ARCACHE(4'b0011),
        .SAXIHP0ARPROT(3'b000), .SAXIHP0ARLOCK(2'b00), .SAXIHP0ARQOS(4'd0),
        .SAXIHP0ARVALID(hp_arvalid[0]), .SAXIHP0ARREADY(hp_arready[0]),
        .SAXIHP0RDATA(hp_rdata[0]), .SAXIHP0RID(hp_rid[0]), .SAXIHP0RRESP(hp_rresp[0]),
        .SAXIHP0RLAST(hp_rlast[0]), .SAXIHP0RVALID(hp_rvalid[0]), .SAXIHP0RREADY(hp_rready[0]),
        .SAXIHP0RDISSUECAP1EN(1'b0), .SAXIHP0WRISSUECAP1EN(1'b0),

        .SAXIHP1ACLK(clk_m),
        .SAXIHP1AWADDR(hp_awaddr[1]), .SAXIHP1AWID(hp_awid[1]), .SAXIHP1AWLEN(hp_awlen[1]),
        .SAXIHP1AWSIZE(2'b11), .SAXIHP1AWBURST(2'b01), .SAXIHP1AWCACHE(4'b0011),
        .SAXIHP1AWPROT(3'b000), .SAXIHP1AWLOCK(2'b00), .SAXIHP1AWQOS(4'd0),
        .SAXIHP1AWVALID(hp_awvalid[1]), .SAXIHP1AWREADY(hp_awready[1]),
        .SAXIHP1WDATA(hp_wdata[1]), .SAXIHP1WID(hp_wid[1]), .SAXIHP1WSTRB(8'hFF),
        .SAXIHP1WLAST(hp_wlast[1]), .SAXIHP1WVALID(hp_wvalid[1]), .SAXIHP1WREADY(hp_wready[1]),
        .SAXIHP1BRESP(hp_bresp[1]), .SAXIHP1BID(hp_bid[1]), .SAXIHP1BVALID(hp_bvalid[1]),
        .SAXIHP1BREADY(hp_bready[1]),
        .SAXIHP1ARADDR(hp_araddr[1]), .SAXIHP1ARID(hp_arid[1]), .SAXIHP1ARLEN(hp_arlen[1]),
        .SAXIHP1ARSIZE(2'b11), .SAXIHP1ARBURST(2'b01), .SAXIHP1ARCACHE(4'b0011),
        .SAXIHP1ARPROT(3'b000), .SAXIHP1ARLOCK(2'b00), .SAXIHP1ARQOS(4'd0),
        .SAXIHP1ARVALID(hp_arvalid[1]), .SAXIHP1ARREADY(hp_arready[1]),
        .SAXIHP1RDATA(hp_rdata[1]), .SAXIHP1RID(hp_rid[1]), .SAXIHP1RRESP(hp_rresp[1]),
        .SAXIHP1RLAST(hp_rlast[1]), .SAXIHP1RVALID(hp_rvalid[1]), .SAXIHP1RREADY(hp_rready[1]),
        .SAXIHP1RDISSUECAP1EN(1'b0), .SAXIHP1WRISSUECAP1EN(1'b0),

        .SAXIHP2ACLK(clk_m),
        .SAXIHP2AWADDR(hp_awaddr[2]), .SAXIHP2AWID(hp_awid[2]), .SAXIHP2AWLEN(hp_awlen[2]),
        .SAXIHP2AWSIZE(2'b11), .SAXIHP2AWBURST(2'b01), .SAXIHP2AWCACHE(4'b0011),
        .SAXIHP2AWPROT(3'b000), .SAXIHP2AWLOCK(2'b00), .SAXIHP2AWQOS(4'd0),
        .SAXIHP2AWVALID(hp_awvalid[2]), .SAXIHP2AWREADY(hp_awready[2]),
        .SAXIHP2WDATA(hp_wdata[2]), .SAXIHP2WID(hp_wid[2]), .SAXIHP2WSTRB(8'hFF),
        .SAXIHP2WLAST(hp_wlast[2]), .SAXIHP2WVALID(hp_wvalid[2]), .SAXIHP2WREADY(hp_wready[2]),
        .SAXIHP2BRESP(hp_bresp[2]), .SAXIHP2BID(hp_bid[2]), .SAXIHP2BVALID(hp_bvalid[2]),
        .SAXIHP2BREADY(hp_bready[2]),
        .SAXIHP2ARADDR(hp_araddr[2]), .SAXIHP2ARID(hp_arid[2]), .SAXIHP2ARLEN(hp_arlen[2]),
        .SAXIHP2ARSIZE(2'b11), .SAXIHP2ARBURST(2'b01), .SAXIHP2ARCACHE(4'b0011),
        .SAXIHP2ARPROT(3'b000), .SAXIHP2ARLOCK(2'b00), .SAXIHP2ARQOS(4'd0),
        .SAXIHP2ARVALID(hp_arvalid[2]), .SAXIHP2ARREADY(hp_arready[2]),
        .SAXIHP2RDATA(hp_rdata[2]), .SAXIHP2RID(hp_rid[2]), .SAXIHP2RRESP(hp_rresp[2]),
        .SAXIHP2RLAST(hp_rlast[2]), .SAXIHP2RVALID(hp_rvalid[2]), .SAXIHP2RREADY(hp_rready[2]),
        .SAXIHP2RDISSUECAP1EN(1'b0), .SAXIHP2WRISSUECAP1EN(1'b0),

        .SAXIHP3ACLK(clk_m),
        .SAXIHP3AWADDR(hp_awaddr[3]), .SAXIHP3AWID(hp_awid[3]), .SAXIHP3AWLEN(hp_awlen[3]),
        .SAXIHP3AWSIZE(2'b11), .SAXIHP3AWBURST(2'b01), .SAXIHP3AWCACHE(4'b0011),
        .SAXIHP3AWPROT(3'b000), .SAXIHP3AWLOCK(2'b00), .SAXIHP3AWQOS(4'd0),
        .SAXIHP3AWVALID(hp_awvalid[3]), .SAXIHP3AWREADY(hp_awready[3]),
        .SAXIHP3WDATA(hp_wdata[3]), .SAXIHP3WID(hp_wid[3]), .SAXIHP3WSTRB(8'hFF),
        .SAXIHP3WLAST(hp_wlast[3]), .SAXIHP3WVALID(hp_wvalid[3]), .SAXIHP3WREADY(hp_wready[3]),
        .SAXIHP3BRESP(hp_bresp[3]), .SAXIHP3BID(hp_bid[3]), .SAXIHP3BVALID(hp_bvalid[3]),
        .SAXIHP3BREADY(hp_bready[3]),
        .SAXIHP3ARADDR(hp_araddr[3]), .SAXIHP3ARID(hp_arid[3]), .SAXIHP3ARLEN(hp_arlen[3]),
        .SAXIHP3ARSIZE(2'b11), .SAXIHP3ARBURST(2'b01), .SAXIHP3ARCACHE(4'b0011),
        .SAXIHP3ARPROT(3'b000), .SAXIHP3ARLOCK(2'b00), .SAXIHP3ARQOS(4'd0),
        .SAXIHP3ARVALID(hp_arvalid[3]), .SAXIHP3ARREADY(hp_arready[3]),
        .SAXIHP3RDATA(hp_rdata[3]), .SAXIHP3RID(hp_rid[3]), .SAXIHP3RRESP(hp_rresp[3]),
        .SAXIHP3RLAST(hp_rlast[3]), .SAXIHP3RVALID(hp_rvalid[3]), .SAXIHP3RREADY(hp_rready[3]),
        .SAXIHP3RDISSUECAP1EN(1'b0), .SAXIHP3WRISSUECAP1EN(1'b0)
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
        .hp_araddr(hp_araddr), .hp_arlen(hp_arlen), .hp_arid(hp_arid), .hp_arvalid(hp_arvalid),
        .hp_arready(hp_arready), .hp_rdata(hp_rdata), .hp_rresp(hp_rresp),
        .hp_rlast(hp_rlast), .hp_rid(hp_rid), .hp_rvalid(hp_rvalid), .hp_rready(hp_rready),
        .hp_awaddr(hp_awaddr), .hp_awlen(hp_awlen), .hp_awid(hp_awid), .hp_awvalid(hp_awvalid),
        .hp_awready(hp_awready), .hp_wdata(hp_wdata), .hp_wlast(hp_wlast), .hp_wid(hp_wid),
        .hp_wvalid(hp_wvalid), .hp_wready(hp_wready), .hp_bresp(hp_bresp), .hp_bid(hp_bid),
        .hp_bvalid(hp_bvalid), .hp_bready(hp_bready)
    );

    wire unused_ok = &{1'b0, fclk[3:1], fclk_resetn[3:1], gp_awaddr[31:20], gp_araddr[31:20],
                       busy};
endmodule
