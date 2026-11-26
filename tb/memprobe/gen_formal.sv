// Formal properties of memprobe_gen (rtl/memprobe/memprobe_gen.sv): the AXI3
// master on the HP port. One clock domain. tb/memprobe/gen.sby runs it.
//
// Assumptions: a valid configuration (memprobe_regs checks it), stable from
// START to the end of the run, and an AXI slave that answers only for
// transactions that it has.
// Properties:
//   - VALID stays until READY, and the payload does not change.
//   - Each address is in the HP memory and in the span, aligned to the burst,
//     and the burst does not cross 4 KiB.
//   - AxLEN is the configured length; each write has AxLEN + 1 W beats, with
//     WLAST on the last beat only.
//   - One transaction in progress at most; no W beat without its AW.
//   - No VALID while the generator is not busy.
module gen_formal (
    input logic        clk,
    input logic        rst,
    input logic        start, clear, stop,
    input logic [2:0]  cfg_op,
    input logic [3:0]  cfg_len,
    input logic [23:0] cfg_base,
    input logic [4:0]  cfg_span_log2,
    input logic [31:0] cfg_count,
    input logic [15:0] cfg_gap,
    input logic [31:0] cfg_seed,
    input logic        arready, rlast, rvalid, awready, wready, bvalid,
    input logic [63:0] rdata,
    input logic [1:0]  rresp, bresp
);
    localparam [31:0] HP_BASE = memprobe_regmap_pkg::HP_BASE;

    logic        init = 1'b1;
    always_ff @(posedge clk) init <= 1'b0;
    always_comb if (init) assume(rst);

    logic [31:0] araddr, awaddr;
    logic [3:0]  arlen, awlen;
    logic        arvalid, rready, awvalid, wvalid, wlast, bready, busy;
    logic [63:0] wdata;

    memprobe_gen #(.OFF_W(24)) dut (
        .clk(clk), .rst(rst), .start(start), .clear(clear), .stop(stop), .busy(busy),
        .cfg_op(cfg_op), .cfg_len(cfg_len), .cfg_base(cfg_base), .cfg_span_log2(cfg_span_log2),
        .cfg_count(cfg_count), .cfg_gap(cfg_gap), .cfg_seed(cfg_seed),
        .araddr(araddr), .arlen(arlen), .arvalid(arvalid), .arready(arready),
        .rdata(rdata), .rresp(rresp), .rlast(rlast), .rvalid(rvalid), .rready(rready),
        .awaddr(awaddr), .awlen(awlen), .awvalid(awvalid), .awready(awready),
        .wdata(wdata), .wlast(wlast), .wvalid(wvalid), .wready(wready),
        .bresp(bresp), .bvalid(bvalid), .bready(bready),
        .smp_rd_first(), .smp_rd_last(), .smp_wr_resp(), .smp_latency(),
        .run_cycles(), .reads(), .writes(), .stalls(), .data_errors(), .first_err_addr(),
        .first_err_data(), .resp_errors(), .run_resp_err(), .run_data_err()
    );

    // ---- Assumptions on the configuration ------------------------------------------
    wire [2:0] len_log2 = 3'(cfg_len[0]) + 3'(cfg_len[1]) + 3'(cfg_len[2]) + 3'(cfg_len[3]);
    wire [4:0] burst_log2 = 5'd3 + 5'(len_log2);
    wire [24:0] span = 25'd1 << cfg_span_log2;
    always_comb begin
        assume(cfg_len == 4'd0 || cfg_len == 4'd1 || cfg_len == 4'd3 || cfg_len == 4'd7
               || cfg_len == 4'd15);
        assume(cfg_op == memprobe_regmap_pkg::MODE_OP_READ || cfg_op == memprobe_regmap_pkg::MODE_OP_WRITE
               || cfg_op == memprobe_regmap_pkg::MODE_OP_FILL || cfg_op == memprobe_regmap_pkg::MODE_OP_CHECK);
        assume(cfg_span_log2 >= burst_log2 && cfg_span_log2 <= 5'd24);
        assume((25'(cfg_base) & (span - 1'b1)) == 25'd0);
    end
    always_ff @(posedge clk)
        if (!init && ($past(busy) || $past(start))) begin
            assume(cfg_op == $past(cfg_op) && cfg_len == $past(cfg_len));
            assume(cfg_base == $past(cfg_base) && cfg_span_log2 == $past(cfg_span_log2));
            assume(cfg_count == $past(cfg_count) && cfg_gap == $past(cfg_gap));
            assume(cfg_seed == $past(cfg_seed));
        end

    // ---- The slave: answers only for its transactions --------------------------------
    logic       rd_out, wr_out, aw_seen, w_seen;
    logic [3:0] rbeat, wbeat;
    always_ff @(posedge clk)
        if (rst) begin
            rd_out <= 1'b0; wr_out <= 1'b0; aw_seen <= 1'b0; w_seen <= 1'b0;
            rbeat <= '0; wbeat <= '0;
        end else begin
            if (arvalid && arready) begin rd_out <= 1'b1; rbeat <= '0; end
            if (rvalid && rready) begin
                rbeat <= rbeat + 1'b1;
                if (rlast) rd_out <= 1'b0;
            end
            if (awvalid && awready) begin aw_seen <= 1'b1; wr_out <= 1'b1; end
            if (wvalid && wready) begin
                wbeat <= wbeat + 1'b1;
                if (wlast) begin w_seen <= 1'b1; wbeat <= '0; end
            end
            if (bvalid && bready) begin wr_out <= 1'b0; aw_seen <= 1'b0; w_seen <= 1'b0; end
        end
    always_comb begin
        if (!rst) begin
            assume(!rvalid || rd_out);
            if (rvalid) assume(rlast == (rbeat == cfg_len));
            assume(!bvalid || (aw_seen && w_seen));
        end
    end

    // ---- Properties ------------------------------------------------------------------
    wire [24:0] burst_mask = (25'd1 << burst_log2) - 1'b1;
    wire [24:0] ar_off = 25'(araddr[23:0]);
    wire [24:0] aw_off = 25'(awaddr[23:0]);
    wire ar_ok = (araddr[31:24] == HP_BASE[31:24])
              && ar_off >= 25'(cfg_base) && ar_off < 25'(cfg_base) + span
              && (ar_off & burst_mask) == 25'd0
              && 13'(araddr[11:0]) + (13'd1 << burst_log2) <= 13'h1000;
    wire aw_ok = (awaddr[31:24] == HP_BASE[31:24])
              && aw_off >= 25'(cfg_base) && aw_off < 25'(cfg_base) + span
              && (aw_off & burst_mask) == 25'd0
              && 13'(awaddr[11:0]) + (13'd1 << burst_log2) <= 13'h1000;

    always_ff @(posedge clk)
        if (!init && !rst && !$past(rst)) begin
            if ($past(arvalid && !arready))
                assert(arvalid && araddr == $past(araddr) && arlen == $past(arlen));
            if ($past(awvalid && !awready))
                assert(awvalid && awaddr == $past(awaddr) && awlen == $past(awlen));
            if ($past(wvalid && !wready))
                assert(wvalid && wdata == $past(wdata) && wlast == $past(wlast));
        end

    always_comb
        if (!init && !rst) begin
            if (arvalid) assert(ar_ok && arlen == cfg_len);
            if (awvalid) assert(aw_ok && awlen == cfg_len);
            if (wvalid) assert(wlast == (wbeat == cfg_len));
            assert(wbeat <= cfg_len);
            if (arvalid || awvalid) assert(!rd_out && !wr_out);
            if (wvalid) assert(awvalid || (wr_out && !w_seen));
            if (arvalid || awvalid || wvalid) assert(busy);
            assert(!(rd_out && wr_out));
        end

    // Reachability: the assumptions do not exclude the transactions (gen.sby, task cover).
    always_comb
        if (!init && !rst) begin
            cover(rvalid && rready && rlast && cfg_len == 4'd3);
            cover(bvalid && bready && cfg_len == 4'd1);
            cover(wvalid && wready && wlast && cfg_len == 4'd0 && !awvalid);
        end
endmodule
