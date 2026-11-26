// Formal properties of memprobe_regs (rtl/memprobe/memprobe_regs.sv): the AXI3
// slave on M_AXI_GP0 and the command handshake. One clock domain (FCLK0).
// tb/memprobe/regs.sby runs it.
//
// The board interface requires that the design answers each request in the
// register window; a request without an answer stops the PS. These properties
// show it for any address, length, ID and ready pattern of the master.
// Assumptions: the master keeps VALID and the payload until READY, and sends
// WLAST on beat AWLEN. All inputs from the other clock domain are free.
// Properties:
//   - AWREADY and ARREADY are 1 when no request of that kind is in progress.
//   - WREADY is 1 while a write burst takes data.
//   - After a read address, each R beat comes within FETCH + 1 cycles; RID is
//     the ARID; RLAST is on beat ARLEN only. R does not change before RREADY.
//   - BVALID comes 1 cycle after WLAST; BID is the AWID. B does not change
//     before BREADY.
//   - RRESP and BRESP are OKAY.
//   - The configuration outputs do not change while busy; cmd_toggle changes
//     only when a command starts, or when the MMCM is not locked.
module regs_formal (
    input logic        clk,
    input logic        rst,
    input logic [19:0] awaddr, araddr,
    input logic [11:0] awid, arid,
    input logic [3:0]  awlen, arlen, wstrb,
    input logic        awvalid, wlast, wvalid, bready, arvalid, rready,
    input logic [31:0] wdata, ext_data,
    input logic        mmcm_locked, ack_toggle, run_resp_err, run_data_err, run_hist_drop
);
    localparam FETCH = 5;

    logic init = 1'b1;
    always_ff @(posedge clk) init <= 1'b0;
    always_comb if (init) assume(rst);

    logic        awready, wready, bvalid, arready, rvalid, rlast, irq, cmd_toggle, cmd_clear;
    logic        stop, busy;
    logic [11:0] bid, rid;
    logic [1:0]  bresp, rresp;
    logic [31:0] rdata;
    logic [19:0] ext_addr;
    logic [2:0]  cfg_op;
    logic [1:0]  cfg_pattern, cfg_size, cfg_rd_port, cfg_wr_port;
    logic        cfg_verify, cfg_order_reverse;
    logic [4:0]  cfg_outstanding;
    logic [15:0] cfg_read_share;
    logic [3:0]  cfg_len;
    logic [23:0] cfg_base;
    logic [4:0]  cfg_span_log2;
    logic [31:0] cfg_count, cfg_seed;
    logic [15:0] cfg_gap;

    memprobe_regs #(.FETCH(FETCH)) dut (
        .clk(clk), .rst(rst),
        .awaddr(awaddr), .awid(awid), .awlen(awlen), .awvalid(awvalid), .awready(awready),
        .wdata(wdata), .wstrb(wstrb), .wlast(wlast), .wvalid(wvalid), .wready(wready),
        .bid(bid), .bresp(bresp), .bvalid(bvalid), .bready(bready),
        .araddr(araddr), .arid(arid), .arlen(arlen), .arvalid(arvalid), .arready(arready),
        .rdata(rdata), .rid(rid), .rresp(rresp), .rlast(rlast), .rvalid(rvalid), .rready(rready),
        .ext_addr(ext_addr), .ext_data(ext_data), .irq(irq),
        .mmcm_locked(mmcm_locked), .cmd_toggle(cmd_toggle), .cmd_clear(cmd_clear), .stop(stop),
        .ack_toggle(ack_toggle), .run_resp_err(run_resp_err), .run_data_err(run_data_err),
        .run_hist_drop(run_hist_drop), .busy(busy),
        .cfg_op(cfg_op), .cfg_pattern(cfg_pattern), .cfg_len(cfg_len), .cfg_size(cfg_size),
        .cfg_rd_port(cfg_rd_port), .cfg_wr_port(cfg_wr_port), .cfg_verify(cfg_verify),
        .cfg_order_reverse(cfg_order_reverse), .cfg_outstanding(cfg_outstanding),
        .cfg_read_share(cfg_read_share), .cfg_base(cfg_base), .cfg_span_log2(cfg_span_log2),
        .cfg_count(cfg_count), .cfg_gap(cfg_gap), .cfg_seed(cfg_seed)
    );

    // ---- The master --------------------------------------------------------------------
    always_ff @(posedge clk)
        if (!init && !$past(rst)) begin
            if ($past(arvalid && !arready))
                assume(arvalid && araddr == $past(araddr) && arid == $past(arid) && arlen == $past(arlen));
            if ($past(awvalid && !awready))
                assume(awvalid && awaddr == $past(awaddr) && awid == $past(awid) && awlen == $past(awlen));
            if ($past(wvalid && !wready))
                assume(wvalid && wdata == $past(wdata) && wstrb == $past(wstrb) && wlast == $past(wlast));
        end

    // Model of the requests in progress.
    logic        rd_act, wr_act, w_end;
    logic [3:0]  r_len, r_beat, w_len, w_beat;
    logic [11:0] r_id, w_id;
    logic [3:0]  r_wait;     // cycles without an R beat during a read (at most FETCH)
    logic [1:0]  b_wait;     // cycles without B after WLAST
    always_ff @(posedge clk)
        if (rst) begin
            rd_act <= 1'b0; wr_act <= 1'b0; w_end <= 1'b0;
            r_len <= '0; r_beat <= '0; w_len <= '0; w_beat <= '0; r_id <= '0; w_id <= '0;
            r_wait <= '0; b_wait <= '0;
        end else begin
            if (arvalid && arready) begin
                rd_act <= 1'b1; r_len <= arlen; r_beat <= '0; r_id <= arid; r_wait <= '0;
            end else if (rd_act) begin
                if (rvalid && rready) begin
                    r_beat <= r_beat + 1'b1;
                    r_wait <= '0;
                    if (rlast) rd_act <= 1'b0;
                end else if (!rvalid && r_wait != '1) r_wait <= r_wait + 1'b1;
            end
            if (awvalid && awready) begin
                wr_act <= 1'b1; w_len <= awlen; w_id <= awid; w_end <= 1'b0; b_wait <= '0;
            end
            if (wvalid && wready) begin
                w_beat <= w_beat + 1'b1;
                if (wlast) begin w_end <= 1'b1; w_beat <= '0; end
            end
            if (wr_act && w_end && !bvalid && b_wait != '1) b_wait <= b_wait + 1'b1;
            if (bvalid && bready) begin wr_act <= 1'b0; w_end <= 1'b0; end
        end
    // The master sends WLAST on beat AWLEN. AXI3 lets W come before AW; this
    // slave takes W only after AW.
    always_comb
        if (!rst && wvalid && wr_act) assume(wlast == (w_beat == w_len));

    // ---- Properties --------------------------------------------------------------------
    always_comb
        if (!init && !rst) begin
            if (!rd_act) assert(arready && !rvalid);
            if (!wr_act) assert(awready && !wready && !bvalid);
            if (wr_act && !w_end) assert(wready && !bvalid);
            if (rd_act) assert(!arready);
            if (wr_act) assert(!awready);
            assert(r_wait <= 4'(FETCH));
            assert(b_wait <= 2'd1);
            if (rvalid) assert(rid == r_id && rlast == (r_beat == r_len) && rresp == 2'b00);
            if (bvalid) assert(bid == w_id && bresp == 2'b00 && w_end);
        end

    always_ff @(posedge clk)
        if (!init && !rst && !$past(rst)) begin
            if ($past(rvalid && !rready))
                assert(rvalid && rdata == $past(rdata) && rlast == $past(rlast) && rid == $past(rid));
            if ($past(bvalid && !bready)) assert(bvalid && bid == $past(bid));
            if ($past(busy) && busy)
                assert(cfg_op == $past(cfg_op) && cfg_len == $past(cfg_len) && cfg_pattern == $past(cfg_pattern)
                       && cfg_size == $past(cfg_size) && cfg_rd_port == $past(cfg_rd_port)
                       && cfg_wr_port == $past(cfg_wr_port) && cfg_verify == $past(cfg_verify)
                       && cfg_order_reverse == $past(cfg_order_reverse)
                       && cfg_outstanding == $past(cfg_outstanding) && cfg_read_share == $past(cfg_read_share)
                       && cfg_base == $past(cfg_base) && cfg_span_log2 == $past(cfg_span_log2)
                       && cfg_count == $past(cfg_count) && cfg_gap == $past(cfg_gap)
                       && cfg_seed == $past(cfg_seed) && cmd_clear == $past(cmd_clear));
            if (cmd_toggle != $past(cmd_toggle))
                assert((!$past(busy) && busy && $past(mmcm_locked)) || !$past(mmcm_locked));
        end

    // Reachability (regs.sby, task cover).
    always_comb
        if (!init && !rst) begin
            cover(rvalid && rready && rlast && r_len == 4'd2);
            cover(bvalid && bready && w_len == 4'd1);
            cover(busy && !cmd_clear);
        end
endmodule
