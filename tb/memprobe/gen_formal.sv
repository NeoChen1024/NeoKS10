// Formal properties of memprobe_gen (rtl/memprobe/memprobe_gen.sv): the AXI3
// master on the HP port, with many transactions in progress. One clock
// domain. tb/memprobe/gen.sby runs it with IDS = 4 (the design has 16), and
// the task prove_region with the parameters of the design (IDS = 16).
//
// Assumptions: a valid configuration (memprobe_regs checks it), stable from
// START to the end of the run; an AXI slave that answers only for transactions
// that it has, with RLAST on beat AxLEN of each ID.
// The properties are in groups, to keep each proof small: define PROPS_REGION
// (each address is in the HP memory), PROPS_ADDR or PROPS_ID. gen.sby has a
// task for each.
// Properties:
//   - VALID stays until READY, and the payload (address, length, ID, data)
//     does not change.
//   - Each address is in the HP memory and in the span, aligned to the burst
//     (beats of 4 or 8 bytes, MODE.SIZE),
//     and the burst does not cross 4 KiB. AxLEN is the configured length.
//   - A new ARVALID or AWVALID has an ID that no transaction in progress has;
//     at most OUTSTANDING transactions are in progress (note "Transactions in
//     progress" of spec/regmap.toml).
//   - The W beats come in the order of the AWVALID, with WID = AWID, AxLEN + 1
//     beats each, WLAST on the last beat only.
//   - No ARVALID and AWVALID together; no VALID while the generator is not busy.
module gen_formal #(
    parameter IDS = 4,
    parameter ID_W = 2,
    parameter OFF_W = 24
) (
    input logic            clk,
    input logic            rst,
    input logic            start, clear, stop,
    input logic [2:0]      cfg_op,
    input logic [1:0]      cfg_pattern,
    input logic [3:0]      cfg_len,
    input logic [1:0]      cfg_size,
    input logic [OFF_W-1:0] cfg_base,
    input logic [4:0]      cfg_span_log2,
    input logic [31:0]     cfg_count,
    input logic [15:0]     cfg_gap,
    input logic [4:0]      cfg_outstanding,
    input logic [15:0]     cfg_read_share,
    input logic [31:0]     cfg_seed,
    input logic            cfg_verify,
    input logic            cfg_order_reverse,
    input logic            arready, rlast, rvalid, awready, wready, bvalid,
    input logic [ID_W-1:0] rid, bid,
    input logic [63:0]     rdata,
    input logic [1:0]      rresp, bresp
);
    localparam [31:0] HP_BASE = memprobe_regmap_pkg::HP_BASE;

    logic init = 1'b1;
    always_ff @(posedge clk) init <= 1'b0;
    always_comb if (init) assume(rst);

    logic [31:0]     araddr, awaddr;
    logic [3:0]      arlen, awlen;
    logic [ID_W-1:0] arid, awid, wid;
    logic            arvalid, rready, awvalid, wvalid, wlast, bready, busy;
    logic [63:0]     wdata;

    memprobe_gen #(.OFF_W(OFF_W), .IDS(IDS), .ID_W(ID_W)) dut (
        .clk(clk), .rst(rst), .start(start), .clear(clear), .stop(stop), .busy(busy),
        .cfg_op(cfg_op), .cfg_pattern(cfg_pattern), .cfg_len(cfg_len), .cfg_size(cfg_size), .cfg_base(cfg_base),
        .cfg_span_log2(cfg_span_log2), .cfg_count(cfg_count), .cfg_gap(cfg_gap),
        .cfg_outstanding(cfg_outstanding), .cfg_read_share(cfg_read_share), .cfg_seed(cfg_seed),
        .cfg_verify(cfg_verify), .cfg_order_reverse(cfg_order_reverse),
        .araddr(araddr), .arlen(arlen), .arid(arid), .arvalid(arvalid), .arready(arready),
        .rdata(rdata), .rresp(rresp), .rlast(rlast), .rid(rid), .rvalid(rvalid), .rready(rready),
        .awaddr(awaddr), .awlen(awlen), .awid(awid), .awvalid(awvalid), .awready(awready),
        .wdata(wdata), .wid(wid), .wlast(wlast), .wvalid(wvalid), .wready(wready),
        .bresp(bresp), .bid(bid), .bvalid(bvalid), .bready(bready),
        .smp_rd_first(), .smp_rd_last(), .smp_r_latency(), .smp_wr_resp(), .smp_b_latency(),
        .run_cycles(), .reads(), .writes(), .stalls(), .data_errors(), .first_err_addr(),
        .first_err_data(), .resp_errors(), .order_mismatches(), .run_resp_err(), .run_data_err()
    );

    // ---- Assumptions on the configuration ------------------------------------------
    wire [2:0] len_log2 = 3'(cfg_len[0]) + 3'(cfg_len[1]) + 3'(cfg_len[2]) + 3'(cfg_len[3]);
    wire [4:0] burst_log2 = 5'(cfg_size) + 5'(len_log2);
    wire [OFF_W:0] span = (OFF_W+1)'(1) << cfg_span_log2;
    wire whole_span = cfg_op == memprobe_regmap_pkg::MODE_OP_FILL || cfg_op == memprobe_regmap_pkg::MODE_OP_CHECK;
    always_comb begin
        assume(cfg_len == 4'd0 || cfg_len == 4'd1 || cfg_len == 4'd3 || cfg_len == 4'd7
               || cfg_len == 4'd15);
        assume(cfg_op == memprobe_regmap_pkg::MODE_OP_READ || cfg_op == memprobe_regmap_pkg::MODE_OP_WRITE
               || cfg_op == memprobe_regmap_pkg::MODE_OP_MIXED || cfg_op == memprobe_regmap_pkg::MODE_OP_ORDER
               || whole_span);
        assume(cfg_pattern == memprobe_regmap_pkg::MODE_PATTERN_SEQ
               || (!whole_span && (cfg_pattern == memprobe_regmap_pkg::MODE_PATTERN_RANDOM
                                   || cfg_pattern == memprobe_regmap_pkg::MODE_PATTERN_FIXED)));
        assume(cfg_span_log2 >= burst_log2 && cfg_span_log2 <= 5'(OFF_W));
        assume(((OFF_W+1)'(cfg_base) & (span - 1'b1)) == '0);
        assume(cfg_outstanding >= 5'd1 && cfg_outstanding <= 5'(IDS));
        assume(cfg_size == memprobe_regmap_pkg::MODE_SIZE_B4 || cfg_size == memprobe_regmap_pkg::MODE_SIZE_B8);
    end
    always_ff @(posedge clk)
        if (!init && ($past(busy) || $past(start))) begin
            assume(cfg_op == $past(cfg_op) && cfg_len == $past(cfg_len) && cfg_pattern == $past(cfg_pattern));
            assume(cfg_size == $past(cfg_size));
            assume(cfg_base == $past(cfg_base) && cfg_span_log2 == $past(cfg_span_log2));
            assume(cfg_count == $past(cfg_count) && cfg_gap == $past(cfg_gap));
            assume(cfg_outstanding == $past(cfg_outstanding) && cfg_read_share == $past(cfg_read_share));
            assume(cfg_seed == $past(cfg_seed) && cfg_verify == $past(cfg_verify));
            assume(cfg_order_reverse == $past(cfg_order_reverse));
        end

    // ---- Record of the transactions ------------------------------------------------
    logic [IDS-1:0]  in_use;       // from VALID to RLAST or B
    logic [IDS-1:0]  rd_open;      // after AR handshake, until RLAST
    logic [IDS-1:0]  aw_seen, w_seen;
    logic [3:0]      rbeat [0:IDS-1];
    logic            ar_hold, aw_hold;    // VALID without READY in the last cycle
    logic [ID_W-1:0] wq [0:IDS-1];  // AW IDs in the order of their AWVALID, until WLAST
    logic [ID_W:0]   wq_n;
    logic [3:0]      wbeat;

    wire ar_new = arvalid && !ar_hold;
    wire aw_new = awvalid && !aw_hold;
    wire ar_hs = arvalid && arready, aw_hs = awvalid && awready, w_hs = wvalid && wready;
    wire r_end = rvalid && rready && rlast, b_end = bvalid && bready;

    always_ff @(posedge clk)
        if (rst) begin
            in_use <= '0; rd_open <= '0; aw_seen <= '0; w_seen <= '0;
            ar_hold <= 1'b0; aw_hold <= 1'b0; wq_n <= '0; wbeat <= '0;
            for (int i = 0; i < IDS; i++) begin rbeat[i] <= '0; wq[i] <= '0; end
        end else begin
            ar_hold <= arvalid && !arready;
            aw_hold <= awvalid && !awready;
            for (int i = 0; i < IDS; i++) begin
                if ((ar_new && arid == ID_W'(i)) || (aw_new && awid == ID_W'(i))) in_use[i] <= 1'b1;
                if ((r_end && rid == ID_W'(i)) || (b_end && bid == ID_W'(i))) in_use[i] <= 1'b0;
                if (ar_hs && arid == ID_W'(i)) begin rd_open[i] <= 1'b1; rbeat[i] <= '0; end
                else if (rvalid && rready && rid == ID_W'(i)) begin
                    rbeat[i] <= rbeat[i] + 1'b1;
                    if (rlast) rd_open[i] <= 1'b0;
                end
                if (aw_hs && awid == ID_W'(i)) aw_seen[i] <= 1'b1;
                if (w_hs && wlast && wid == ID_W'(i)) w_seen[i] <= 1'b1;
                if (b_end && bid == ID_W'(i)) begin aw_seen[i] <= 1'b0; w_seen[i] <= 1'b0; end
            end
            // The queue of W bursts: push at a new AWVALID, pop at WLAST.
            if (w_hs) wbeat <= wlast ? 4'd0 : wbeat + 1'b1;
            if (w_hs && wlast) begin
                for (int i = 0; i < IDS - 1; i++) wq[i] <= wq[i + 1];
                if (aw_new && wq_n != 0) wq[wq_n - 1'b1] <= awid;
                else if (!aw_new) wq_n <= wq_n - 1'b1;
            end else if (aw_new) begin
                wq[wq_n[ID_W-1:0]] <= awid;
                wq_n <= wq_n + 1'b1;
            end
        end

    // ---- The slave -----------------------------------------------------------------
    always_comb
        if (!rst) begin
            if (rvalid) assume(rd_open[rid] && rlast == (rbeat[rid] == cfg_len));
            if (bvalid) assume(aw_seen[bid] && w_seen[bid]);
        end

    // ---- Properties ----------------------------------------------------------------
    wire [OFF_W:0] burst_mask = ((OFF_W+1)'(1) << burst_log2) - 1'b1;
    wire [OFF_W:0] ar_off = (OFF_W+1)'(araddr[OFF_W-1:0]);
    wire [OFF_W:0] aw_off = (OFF_W+1)'(awaddr[OFF_W-1:0]);
    wire ar_ok = (araddr[31:OFF_W] == HP_BASE[31:OFF_W])
              && ar_off >= (OFF_W+1)'(cfg_base) && ar_off < (OFF_W+1)'(cfg_base) + span
              && (ar_off & burst_mask) == '0
              && 13'(araddr[11:0]) + (13'd1 << burst_log2) <= 13'h1000;
    wire aw_ok = (awaddr[31:OFF_W] == HP_BASE[31:OFF_W])
              && aw_off >= (OFF_W+1)'(cfg_base) && aw_off < (OFF_W+1)'(cfg_base) + span
              && (aw_off & burst_mask) == '0
              && 13'(awaddr[11:0]) + (13'd1 << burst_log2) <= 13'h1000;
    logic [ID_W+1:0] n_in_use;
    always_comb begin
        n_in_use = '0;
        for (int i = 0; i < IDS; i++) n_in_use = n_in_use + (ID_W+2)'(in_use[i]);
    end
    // The W burst on the bus belongs to the oldest AW without its W beats, or,
    // if there is none, to the AWVALID of this cycle (its W starts with it).
    wire w_order_ok = (wq_n != 0) ? (wid == wq[0]) : (aw_new && wid == awid);

`ifdef PROPS_REGION
    // The bus address is the base of the HP memory with an offset of OFF_W
    // bits: no access outside the reserved region.
    always_comb
        if (!init && !rst) begin
            if (arvalid) assert(araddr[31:OFF_W] == HP_BASE[31:OFF_W]);
            if (awvalid) assert(awaddr[31:OFF_W] == HP_BASE[31:OFF_W]);
        end
`endif

`ifdef PROPS_ADDR
    always_ff @(posedge clk)
        if (!init && !rst && !$past(rst)) begin
            if ($past(arvalid && !arready))
                assert(arvalid && araddr == $past(araddr) && arlen == $past(arlen) && arid == $past(arid));
            if ($past(awvalid && !awready))
                assert(awvalid && awaddr == $past(awaddr) && awlen == $past(awlen) && awid == $past(awid));
            if ($past(wvalid && !wready))
                assert(wvalid && wdata == $past(wdata) && wlast == $past(wlast) && wid == $past(wid));
        end

    always_comb
        if (!init && !rst) begin
            if (arvalid) assert(ar_ok && arlen == cfg_len);
            if (awvalid) assert(aw_ok && awlen == cfg_len);
            assert(!(arvalid && awvalid));
            if (arvalid || awvalid || wvalid) assert(busy);
        end
`endif

`ifdef PROPS_ID
    always_comb
        if (!init && !rst) begin
            if (ar_new) assert(!in_use[arid]);
            if (aw_new) assert(!in_use[awid]);
            // In progress: the recorded ones and a VALID of this cycle.
            // ORDER has its write and its read in progress: 2.
            assert(n_in_use + (ID_W+2)'(ar_new) + (ID_W+2)'(aw_new)
                   <= (cfg_op == memprobe_regmap_pkg::MODE_OP_ORDER ? (ID_W+2)'(2) : (ID_W+2)'(cfg_outstanding)));
            if (wvalid) assert(w_order_ok && wlast == (wbeat == cfg_len));
            assert(wbeat <= cfg_len);
        end
`endif

    // Reachability (gen.sby, task cover).
    always_comb
        if (!init && !rst) begin
            cover(n_in_use == (ID_W+2)'(IDS) && cfg_outstanding == 5'(IDS));
            cover(rvalid && rready && rlast && cfg_len == 4'd3 && rid == ID_W'(IDS - 1));
            cover(wvalid && wready && wlast && cfg_len == 4'd1 && cfg_size == memprobe_regmap_pkg::MODE_SIZE_B4);
            cover(bvalid && bready && cfg_len == 4'd1 && wq_n == 1);
            cover(wvalid && wready && wlast && cfg_len == 4'd0 && !awvalid);
        end
endmodule
