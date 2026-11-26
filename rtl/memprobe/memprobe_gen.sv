// Traffic generator of the memory path test design (docs/mem-path-experiment.md,
// Part B), on one AXI3 HP port, in the domain of clk_m.
//
// This version has one transaction in progress at a time, sequential
// addresses and 64-bit beats (CAPS of the block memprobe gives this to the PS).
// The register map is spec/regmap.toml, block memprobe.
//
// Operations (MODE.OP):
//   READ   cfg_count read bursts; the data is not checked
//   WRITE  cfg_count write bursts with the data pattern
//   FILL   write bursts with the data pattern over the span once
//   CHECK  read bursts over the span once; compare with the data pattern
//
// The bus address is the base of the HP memory with the offset in the low
// OFF_W bits. The offset register has OFF_W bits, so no address is outside
// the HP memory, whatever the configuration.
module memprobe_gen #(
    parameter OFF_W = memprobe_regmap_pkg::HP_SIZE_LOG2
) (
    input  logic             clk,
    input  logic             rst,

    // Commands. start and clear are pulses, accepted only while busy is 0.
    input  logic             start,
    input  logic             clear,
    input  logic             stop,        // level: ABORT, issue no new transaction
    output logic             busy,

    // Configuration. Stable from start to the end of the run. The PS side
    // has checked it (memprobe_regs).
    input  logic [2:0]       cfg_op,
    input  logic [3:0]       cfg_len,     // AxLEN: 0, 1, 3, 7 or 15
    input  logic [OFF_W-1:0] cfg_base,    // a multiple of the span
    input  logic [4:0]       cfg_span_log2,
    input  logic [31:0]      cfg_count,
    input  logic [15:0]      cfg_gap,
    input  logic [31:0]      cfg_seed,

    // AXI3 master, ID 0, INCR bursts of 8-byte beats.
    output logic [31:0]      araddr,
    output logic [3:0]       arlen,
    output logic             arvalid,
    input  logic             arready,
    input  logic [63:0]      rdata,
    input  logic [1:0]       rresp,
    input  logic             rlast,
    input  logic             rvalid,
    output logic             rready,
    output logic [31:0]      awaddr,
    output logic [3:0]       awlen,
    output logic             awvalid,
    input  logic             awready,
    output logic [63:0]      wdata,
    output logic             wlast,
    output logic             wvalid,
    input  logic             wready,
    input  logic [1:0]       bresp,
    input  logic             bvalid,
    output logic             bready,

    // Latency samples (spec/regmap.toml, note "Latency"), one cycle after the
    // handshake. One value for all three: with one transaction in progress,
    // they have the same start.
    output logic             smp_rd_first,
    output logic             smp_rd_last,
    output logic             smp_wr_resp,
    output logic [31:0]      smp_latency,

    // Results. clear sets them to 0; each run adds to them.
    output logic [63:0]      run_cycles,
    output logic [63:0]      reads,
    output logic [63:0]      writes,
    output logic [63:0]      stalls,
    output logic [31:0]      data_errors,
    output logic [31:0]      first_err_addr,
    output logic [63:0]      first_err_data,
    output logic [31:0]      resp_errors,
    // Flags of the last run. start sets them to 0.
    output logic             run_resp_err,
    output logic             run_data_err
);
    localparam [2:0] OP_READ  = memprobe_regmap_pkg::MODE_OP_READ;
    localparam [2:0] OP_FILL  = memprobe_regmap_pkg::MODE_OP_FILL;
    localparam [2:0] OP_CHECK = memprobe_regmap_pkg::MODE_OP_CHECK;
    localparam [31:0] HP_ADDR = memprobe_regmap_pkg::HP_BASE;

    localparam [2:0] S_IDLE  = 3'd0,
                     S_NEXT  = 3'd1,     // decide on the next transaction
                     S_AR    = 3'd2,     // ARVALID
                     S_R     = 3'd3,     // read data
                     S_AW_W  = 3'd4,     // AWVALID and the write data
                     S_B     = 3'd5;     // write response
    logic [2:0] state;

    wire is_read = (cfg_op == OP_READ) || (cfg_op == OP_CHECK);
    wire whole_span = (cfg_op == OP_FILL) || (cfg_op == OP_CHECK);

    // Bytes in a burst: 8 << log2(len + 1). For the accepted lengths, log2(len + 1)
    // is the number of 1 bits of len.
    wire [2:0] len_log2 = 3'(cfg_len[0]) + 3'(cfg_len[1]) + 3'(cfg_len[2]) + 3'(cfg_len[3]);
    wire [5:0] burst_log2 = 6'd3 + 6'(len_log2);
    // The span is at most the HP memory (memprobe_regs checks it): 2^OFF_W - 1 at most.
    wire [OFF_W:0] span_mask_w = ({{OFF_W{1'b0}}, 1'b1} << cfg_span_log2) - 1'b1;
    wire [OFF_W-1:0] span_mask = span_mask_w[OFF_W-1:0];
    wire unused_mask_bit = span_mask_w[OFF_W];
    wire [OFF_W-1:0] burst_bytes = OFF_W'(1) << burst_log2;

    logic [OFF_W-1:0] seq;        // offset of the next burst in the span
    logic [OFF_W-1:0] cur;        // offset of the burst in progress
    logic [OFF_W-1:0] beat_off;   // offset of the next R or W beat
    logic [31:0]      remaining;  // transactions to issue
    logic [15:0]      since;      // edges since the last address handshake (saturates)
    logic [31:0]      now;        // free-running time of the latency samples
    logic [31:0]      t_addr;     // time of the last address handshake
    logic [3:0]       wbeat;
    logic             aw_done, w_done, r_first, err_seen;

    wire [63:0] expect_beat;

    // The data check is one cycle after the R beat, for the timing from the
    // PS7 cell at 150 MHz.
    logic             chk_v;
    logic [63:0]      chk_data, chk_expect;
    logic [OFF_W-1:0] chk_off;
    wire        bad_lo = chk_data[31:0] != chk_expect[31:0];
    wire        bad_hi = chk_data[63:32] != chk_expect[63:32];
    wire [32:0] errors_sum = 33'(data_errors) + 33'(bad_lo) + 33'(bad_hi);
    memprobe_pattern #(.OFF_W(OFF_W)) pat (
        .seed(cfg_seed), .index(beat_off[OFF_W-1:3]), .beat(expect_beat));

    assign araddr  = HP_ADDR | 32'(cur);
    assign awaddr  = HP_ADDR | 32'(cur);
    assign arlen   = cfg_len;
    assign awlen   = cfg_len;
    assign arvalid = (state == S_AR);
    assign rready  = (state == S_R);
    assign awvalid = (state == S_AW_W) && !aw_done;
    assign wvalid  = (state == S_AW_W) && !w_done;
    assign wdata   = expect_beat;
    assign wlast   = (wbeat == cfg_len);
    assign bready  = (state == S_B);
    assign busy    = (state != S_IDLE) || chk_v || smp_rd_last || smp_wr_resp;

    wire ar_hs = arvalid && arready;
    wire r_hs  = rvalid && rready;
    wire aw_hs = awvalid && awready;
    wire w_hs  = wvalid && wready;
    wire b_hs  = bvalid && bready;

    wire gap_ok = (since >= cfg_gap);
    wire more = (remaining != 0) && !stop;

    // The samples are registers: they come one cycle after the handshake,
    // with the latency of the handshake.
    always_ff @(posedge clk) begin
        smp_rd_first <= !rst && r_hs && r_first;
        smp_rd_last  <= !rst && r_hs && rlast;
        smp_wr_resp  <= !rst && b_hs;
        smp_latency  <= now - t_addr;
    end

    // A stall: the next transaction could issue, but the bus or the limit of one
    // transaction in progress stops it.
    wire stall = (state == S_AR && !arready)
              || (state == S_AW_W && !aw_done && !awready)
              || ((state == S_R || state == S_B || (state == S_AW_W && aw_done)) && more && gap_ok);

    always_ff @(posedge clk) begin
        if (rst) begin
            state          <= S_IDLE;
            now            <= '0;
            since          <= '1;
            run_cycles     <= '0;
            reads          <= '0;
            writes         <= '0;
            stalls         <= '0;
            data_errors    <= '0;
            first_err_addr <= '0;
            first_err_data <= '0;
            resp_errors    <= '0;
            err_seen       <= 1'b0;
            run_resp_err   <= 1'b0;
            run_data_err   <= 1'b0;
            seq            <= '0;
            cur            <= '0;
            beat_off       <= '0;
            remaining      <= '0;
            t_addr         <= '0;
            wbeat          <= '0;
            aw_done        <= 1'b0;
            w_done         <= 1'b0;
            r_first        <= 1'b0;
            chk_v          <= 1'b0;
            chk_data       <= '0;
            chk_expect     <= '0;
            chk_off        <= '0;
        end else begin
            chk_v      <= r_hs && cfg_op == OP_CHECK;
            chk_data   <= rdata;
            chk_expect <= expect_beat;
            chk_off    <= beat_off;
            if (chk_v && (bad_lo || bad_hi)) begin
                // Count 32-bit words; stop at 0xFFFFFFFF.
                data_errors  <= errors_sum[32] ? 32'hFFFF_FFFF : errors_sum[31:0];
                run_data_err <= 1'b1;
                if (!err_seen) begin
                    err_seen       <= 1'b1;
                    // The first word of the beat that differs.
                    first_err_addr <= 32'(chk_off) | (bad_lo ? 32'd0 : 32'd4);
                    first_err_data <= chk_data;
                end
            end

            now <= now + 1'b1;
            if (since != '1) since <= since + 1'b1;
            if (busy) run_cycles <= run_cycles + 1'b1;
            if (stall) stalls <= stalls + 1'b1;

            if (clear && !busy) begin
                run_cycles     <= '0;
                reads          <= '0;
                writes         <= '0;
                stalls         <= '0;
                data_errors    <= '0;
                first_err_addr <= '0;
                first_err_data <= '0;
                resp_errors    <= '0;
                err_seen       <= 1'b0;
            end

            case (state)
                S_IDLE:
                    if (start) begin
                        seq          <= '0;
                        since        <= '1;
                        run_resp_err <= 1'b0;
                        run_data_err <= 1'b0;
                        remaining    <= whole_span ? 32'(1) << (cfg_span_log2 - 5'(burst_log2))
                                                   : cfg_count;
                        state        <= S_NEXT;
                    end

                S_NEXT:
                    if (!more) state <= S_IDLE;
                    else if (gap_ok) begin
                        cur      <= cfg_base | (seq & span_mask);
                        beat_off <= cfg_base | (seq & span_mask);
                        seq      <= (seq + burst_bytes) & span_mask;
                        wbeat    <= '0;
                        aw_done  <= 1'b0;
                        w_done   <= 1'b0;
                        r_first  <= 1'b1;
                        state    <= is_read ? S_AR : S_AW_W;
                    end

                S_AR:
                    if (ar_hs) begin
                        t_addr    <= now;
                        since     <= 16'd1;
                        remaining <= remaining - 1'b1;
                        state     <= S_R;
                    end

                S_R:
                    if (r_hs) begin
                        r_first  <= 1'b0;
                        beat_off <= beat_off + 8;
                        if (rresp != 2'b00) begin
                            if (resp_errors != '1) resp_errors <= resp_errors + 1'b1;
                            run_resp_err <= 1'b1;
                        end
                        if (rlast) begin
                            reads <= reads + 1'b1;
                            state <= S_NEXT;
                        end
                    end

                S_AW_W: begin
                    if (aw_hs) begin
                        aw_done   <= 1'b1;
                        t_addr    <= now;
                        since     <= 16'd1;
                        remaining <= remaining - 1'b1;
                    end
                    if (w_hs) begin
                        wbeat    <= wbeat + 1'b1;
                        beat_off <= beat_off + 8;
                        if (wlast) w_done <= 1'b1;
                    end
                    if ((aw_done || aw_hs) && (w_done || (w_hs && wlast))) state <= S_B;
                end

                S_B:
                    if (b_hs) begin
                        writes <= writes + 1'b1;
                        if (bresp != 2'b00) begin
                            if (resp_errors != '1) resp_errors <= resp_errors + 1'b1;
                            run_resp_err <= 1'b1;
                        end
                        state <= S_NEXT;
                    end

                default: state <= S_IDLE;
            endcase
        end
    end
endmodule
