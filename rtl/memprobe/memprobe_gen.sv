// Traffic generator of the memory path test design (docs/mem-path-experiment.md,
// Part B), on one AXI3 master, in the domain of clk_m. The register map is
// spec/regmap.toml, block memprobe (version 2); its notes "Latency",
// "Transactions in progress" and "Data pattern" define the results.
//
// Operations (MODE.OP): READ, WRITE, MIXED (COUNT bursts), FILL and CHECK (the
// span once, in sequence), ORDER (COUNT iterations of a write and a read of
// the same address, one iteration at a time). Up to IDS transactions in
// progress, each with its own AXI ID. memprobe_core connects the read channels
// and the write channels to the HP ports of MODE.RD_PORT and MODE.WR_PORT.
//
// Structure:
//   - Sequencer: prepares the next burst (operation, address) and sets its
//     ARVALID or AWVALID when GAP is over and the OUTSTANDING limit allows. In
//     the cycle of an address handshake it can set the next burst at once, so
//     that one address can go in each cycle.
//   - ORDER: a small state machine uses the same VALID registers instead of
//     the sequencer.
//   - W engine: sends the W beats in the order of the AW. A write starts its
//     W beats in the cycle of its AWVALID if no earlier write has W beats left.
//   - Responses: RREADY and BREADY are 1. The R and B signals go into a
//     register first (stage 1); stage 2 looks up the ID table, makes the
//     latency samples, compares ORDER reads, and frees the ID; the data check
//     of the pattern is in stages 3 and 4.
//
// The bus address is the base of the HP memory with the offset in the low
// OFF_W bits. The offsets have OFF_W bits, so no address is outside the HP
// memory, whatever the configuration.
module memprobe_gen #(
    parameter OFF_W = memprobe_regmap_pkg::HP_SIZE_LOG2,
    parameter IDS   = 16,
    parameter ID_W  = $clog2(IDS)
) (
    input  logic             clk,
    input  logic             rst,

    // Commands. start and clear are pulses, accepted only while busy is 0.
    input  logic             start,
    input  logic             clear,
    input  logic             stop,        // level: ABORT, start no new burst
    output logic             busy,

    // Configuration. Stable from start to the end of the run. memprobe_regs
    // has checked it.
    input  logic [2:0]       cfg_op,
    input  logic [1:0]       cfg_pattern,
    input  logic [3:0]       cfg_len,     // AxLEN: 0, 1, 3, 7 or 15
    input  logic [1:0]       cfg_size,    // AxSIZE: B4 or B8
    input  logic [OFF_W-1:0] cfg_base,    // a multiple of the span
    input  logic [4:0]       cfg_span_log2,
    input  logic [31:0]      cfg_count,
    input  logic [15:0]      cfg_gap,
    input  logic [4:0]       cfg_outstanding,   // 1 to IDS
    input  logic [15:0]      cfg_read_share,
    input  logic [31:0]      cfg_seed,
    input  logic             cfg_verify,
    input  logic             cfg_order_reverse,

    // AXI3 master: INCR bursts of 8-byte beats, or of 4-byte beats in bits
    // 31:0 (MODE.SIZE B4, the 32-bit mode of the AFI). memprobe_core drives
    // AxSIZE and WSTRB.
    output logic [31:0]      araddr,
    output logic [3:0]       arlen,
    output logic [ID_W-1:0]  arid,
    output logic             arvalid,
    input  logic             arready,
    input  logic [63:0]      rdata,
    input  logic [1:0]       rresp,
    input  logic             rlast,
    input  logic [ID_W-1:0]  rid,
    input  logic             rvalid,
    output logic             rready,
    output logic [31:0]      awaddr,
    output logic [3:0]       awlen,
    output logic [ID_W-1:0]  awid,
    output logic             awvalid,
    input  logic             awready,
    output logic [63:0]      wdata,
    output logic [ID_W-1:0]  wid,
    output logic             wlast,
    output logic             wvalid,
    input  logic             wready,
    input  logic [1:0]       bresp,
    input  logic [ID_W-1:0]  bid,
    input  logic             bvalid,
    output logic             bready,

    // Latency samples (note "Latency"), two cycles after the handshake.
    output logic             smp_rd_first,
    output logic             smp_rd_last,
    output logic [31:0]      smp_r_latency,
    output logic             smp_wr_resp,
    output logic [31:0]      smp_b_latency,

    // Results. clear sets them to 0; each run adds to them.
    output logic [63:0]      run_cycles,
    output logic [63:0]      reads,
    output logic [63:0]      writes,
    output logic [63:0]      stalls,
    output logic [31:0]      data_errors,
    output logic [31:0]      first_err_addr,
    output logic [63:0]      first_err_data,
    output logic [31:0]      resp_errors,
    output logic [31:0]      order_mismatches,
    // Flags of the last run. start sets them to 0.
    output logic             run_resp_err,
    output logic             run_data_err
);
    localparam [2:0] OP_READ  = memprobe_regmap_pkg::MODE_OP_READ;
    localparam [2:0] OP_MIXED = memprobe_regmap_pkg::MODE_OP_MIXED;
    localparam [2:0] OP_ORDER = memprobe_regmap_pkg::MODE_OP_ORDER;
    localparam [2:0] OP_FILL  = memprobe_regmap_pkg::MODE_OP_FILL;
    localparam [2:0] OP_CHECK = memprobe_regmap_pkg::MODE_OP_CHECK;
    localparam [1:0] PAT_RANDOM = memprobe_regmap_pkg::MODE_PATTERN_RANDOM;
    localparam [1:0] PAT_FIXED  = memprobe_regmap_pkg::MODE_PATTERN_FIXED;
    localparam [1:0] SIZE_B4    = memprobe_regmap_pkg::MODE_SIZE_B4;
    localparam [31:0] HP_ADDR = memprobe_regmap_pkg::HP_BASE;
    // Galois LFSR, x^32 + x^22 + x^2 + x + 1 (maximum length).
    localparam [31:0] LFSR_TAPS = 32'h8020_0003;

    function automatic [31:0] lfsr_next(input [31:0] l);
        lfsr_next = {1'b0, l[31:1]} ^ (l[0] ? LFSR_TAPS : 32'd0);
    endfunction

    // ---- Configuration values ------------------------------------------------------
    wire order      = (cfg_op == OP_ORDER);
    wire whole_span = (cfg_op == OP_FILL) || (cfg_op == OP_CHECK);
    wire check_en   = (cfg_op == OP_CHECK) || (cfg_verify && (cfg_op == OP_READ || cfg_op == OP_MIXED));
    // For the accepted lengths, log2(len + 1) is the number of 1 bits of len.
    wire [2:0] len_log2 = 3'(cfg_len[0]) + 3'(cfg_len[1]) + 3'(cfg_len[2]) + 3'(cfg_len[3]);
    wire b4 = (cfg_size == SIZE_B4);
    wire [OFF_W-1:0] beat_bytes = b4 ? OFF_W'(4) : OFF_W'(8);
    wire [5:0] burst_log2 = (b4 ? 6'd2 : 6'd3) + 6'(len_log2);
    // The span is at most the HP memory (memprobe_regs checks it).
    wire [OFF_W:0] span_mask_w = ({{OFF_W{1'b0}}, 1'b1} << cfg_span_log2) - 1'b1;
    wire [OFF_W-1:0] span_mask = span_mask_w[OFF_W-1:0];
    wire unused_mask_bit = span_mask_w[OFF_W];
    wire [OFF_W-1:0] burst_bytes = OFF_W'(1) << burst_log2;
    wire [31:0] seed_op = {cfg_seed[15:0], cfg_seed[31:16]} ^ 32'h0000_0001;
    wire [31:0] run_bursts = whole_span ? 32'(1) << (cfg_span_log2 - 5'(burst_log2)) : cfg_count;

    // ---- State ---------------------------------------------------------------------
    logic             running;        // a run: from start until all is done
    logic [31:0]      now;            // free-running time of the latency samples
    logic [15:0]      since;          // edges since the last address handshake (saturates)
    logic [31:0]      to_prep;        // bursts (ORDER: iterations) not yet prepared
    logic [32:0]      to_issue;       // bursts without address handshake
    logic [OFF_W-1:0] seq;
    logic [31:0]      lfsr_a, lfsr_op;

    logic             nx_v, nx_read;  // the prepared next burst
    logic [OFF_W-1:0] nx_off;
    logic [OFF_W-1:0] pres_off;       // the burst whose VALID is set

    logic [IDS-1:0]   id_busy;
    logic [31:0]      id_start [0:IDS-1];
    logic [OFF_W-1:0] id_off   [0:IDS-1];    // offset of the next R beat
    logic [IDS-1:0]   id_first;
    logic [5:0]       inflight;       // after the address handshake, not complete
    logic             first_seen;     // a data error since CLEAR

    wire pres_v = arvalid || awvalid;
    wire ar_hs  = arvalid && arready;
    wire aw_hs  = awvalid && awready;
    wire w_hs   = wvalid && wready;
    wire addr_hs = ar_hs || aw_hs;

    // ---- Stage 1: R and B into registers -----------------------------------------
    logic             r1_v, r1_last;
    logic [ID_W-1:0]  r1_id;
    logic [63:0]      r1_data;
    logic [1:0]       r1_resp;
    logic [31:0]      r1_time;
    logic             b1_v;
    logic [ID_W-1:0]  b1_id;
    logic [1:0]       b1_resp;
    logic [31:0]      b1_time;
    assign rready = 1'b1;
    assign bready = 1'b1;

    // ---- Stage 2 values ------------------------------------------------------------
    wire              r_done = r1_v && r1_last;
    wire              b_done = b1_v;
    wire [1:0]        dec = 2'(r_done) + 2'(b_done);
    wire [IDS-1:0]    freeing = (r_done ? (IDS'(1) << r1_id) : '0) | (b_done ? (IDS'(1) << b1_id) : '0);
    // The lowest free ID comes from the busy register only (timing at 150 MHz):
    // an ID that stage 2 frees is free in the next cycle.
    logic [ID_W-1:0]  free_id;
    wire              any_free = id_busy != '1;
    always_comb begin
        free_id = '0;
        for (int i = IDS - 1; i >= 0; i--) if (!id_busy[i]) free_id = ID_W'(i);
    end
    wire              r_err = r1_v && r1_resp != 2'b00;
    wire              b_err = b1_v && b1_resp != 2'b00;
    wire [32:0]       resp_sum = 33'(resp_errors) + 33'(r_err) + 33'(b_err);

    // ---- Sequencer: when to set the next VALID ---------------------------------------
    // The burst with VALID now counts as in progress after its handshake.
    wire [5:0] busy_after = inflight - 6'(dec) + 6'(pres_v);
    wire room   = busy_after < 6'(cfg_outstanding);
    wire gap_ok = (since >= cfg_gap);
    // A new VALID: no VALID now and GAP over, or a handshake now and GAP <= 1
    // (the next VALID then comes one edge after the handshake).
    wire slot = pres_v ? (addr_hs && cfg_gap <= 16'd1) : gap_ok;
    wire present_seq = running && !order && nx_v && !stop && room && slot && any_free;
    wire stall = running && !order && !stop && gap_ok && to_issue != 0
              && ((arvalid && !arready) || (awvalid && !awready) || (!pres_v && !room));

    // ---- ORDER ---------------------------------------------------------------------
    localparam [1:0] O_START = 2'd0, O_FIRST = 2'd1, O_GAP = 2'd2, O_WAIT = 2'd3;
    logic [1:0]       ost;
    logic [15:0]      o_cnt;
    logic [31:0]      o_n;            // number of the iteration
    logic [OFF_W-1:0] o_off;
    logic             o_second_sent, o_rd_done, o_wr_done, o_bad;
    wire  [63:0]      o_value = {cfg_seed ^ ~o_n, cfg_seed ^ o_n};
    // The value and the comparison of ORDER; B4 uses bits 31:0 only.
    wire  [63:0]      o_wvalue = b4 ? {32'd0, o_value[31:0]} : o_value;
    wire  o_differs = b4 ? (r1_data[31:0] != o_value[31:0]) : (r1_data != o_value);
    wire  o_idle  = !pres_v && inflight == 0 && !w_active && wq_empty && !r1_v && !b1_v;
    // The first transaction: the write, or the read with ORDER_REVERSE.
    wire  o_go    = running && order && ost == O_START && nx_v && !stop && o_idle;
    wire  o_mark  = cfg_order_reverse ? ar_hs : (w_hs && wlast);
    wire  o_now   = (ost == O_FIRST && o_mark && cfg_gap == 16'd0) || (ost == O_GAP && o_cnt >= cfg_gap);
    wire  o_second = running && order && o_now && !pres_v && !o_second_sent;
    wire  present_ord = o_go || o_second;
    // Read or write for the VALID of this cycle.
    wire  present_read = o_go ? cfg_order_reverse : o_second ? !cfg_order_reverse : nx_read;

    wire present = present_seq || present_ord;
    wire [OFF_W-1:0] present_off = order ? (o_go ? nx_off : o_off) : nx_off;

    // ---- W engine ------------------------------------------------------------------
    localparam WQ = IDS;
    logic [ID_W-1:0]  wq_id  [0:WQ-1];
    logic [OFF_W-1:0] wq_off [0:WQ-1];
    logic [ID_W:0]    wq_wp, wq_rp;
    wire              wq_empty = (wq_wp == wq_rp);
    logic             w_active;
    logic [3:0]       w_beat;
    logic [OFF_W-1:0] w_off;
    wire              w_free_next = !w_active || (w_hs && wlast);
    wire              present_w = present && !present_read;
    wire              w_bypass = w_free_next && wq_empty && present_w;

    wire [63:0] w_pattern;
    memprobe_pattern #(.OFF_W(OFF_W)) wpat (.seed(cfg_seed), .index(w_off[OFF_W-1:3]), .beat(w_pattern));
    assign wvalid = w_active;
    // B4: the 32-bit word at w_off, in bits 31:0.
    wire [63:0] w_word = {32'd0, w_off[2] ? w_pattern[63:32] : w_pattern[31:0]};
    assign wdata  = order ? o_wvalue : b4 ? w_word : w_pattern;
    assign wlast  = (w_beat == cfg_len);

    // ---- Checker stages 3 and 4 ----------------------------------------------------
    logic             c_v;
    logic [63:0]      c_data;
    logic [OFF_W-1:0] c_off;
    wire  [63:0]      c_expect;
    wire  [63:0]      c_pat;
    memprobe_pattern #(.OFF_W(OFF_W)) rpat (.seed(cfg_seed), .index(c_off[OFF_W-1:3]), .beat(c_pat));
    // B4: stage 2 sets bits 63:32 of c_data to 0; the expected word is in bits 31:0.
    assign c_expect = b4 ? {32'd0, c_off[2] ? c_pat[63:32] : c_pat[31:0]} : c_pat;
    logic             d_v, d_lo, d_hi;
    logic [63:0]      d_data;
    logic [OFF_W-1:0] d_off;
    wire  [32:0]      errors_sum = 33'(data_errors) + 33'(d_lo) + 33'(d_hi);

    wire pipe_busy = r1_v || b1_v || c_v || d_v || smp_rd_last || smp_wr_resp;
    assign busy = running || pipe_busy;

    assign araddr = HP_ADDR | 32'(pres_off);
    assign awaddr = HP_ADDR | 32'(pres_off);
    assign arlen  = cfg_len;
    assign awlen  = cfg_len;

    // The offset of a new burst in the span.
    wire [OFF_W-1:0] lfsr_off = OFF_W'(lfsr_a << burst_log2) & span_mask;
    wire [OFF_W-1:0] new_off  = cfg_base | (cfg_pattern == PAT_RANDOM ? lfsr_off
                                          : cfg_pattern == PAT_FIXED  ? OFF_W'(0)
                                          :                             (seq & span_mask));
    wire new_read = (cfg_op == OP_READ) || (cfg_op == OP_CHECK)
                 || (cfg_op == OP_MIXED && lfsr_op[15:0] < cfg_read_share);
    // nx is used in this cycle: the sequencer sets it, or an ORDER iteration starts.
    wire nx_used = present_seq || o_go;
    wire prepare = running && to_prep != 0 && !stop && (!nx_v || nx_used);

    // The ID table: one write port for the address handshake, one for stage 2.
    // The two never use the same ID in one cycle: stage 2 sees only IDs after
    // their address handshake.
    always_ff @(posedge clk) begin
        for (int i = 0; i < IDS; i++) begin
            if (ar_hs && arid == ID_W'(i)) begin
                id_start[i] <= now;
                id_off[i]   <= pres_off;
                id_first[i] <= 1'b1;
            end else if (aw_hs && awid == ID_W'(i)) begin
                id_start[i] <= now;
            end else if (r1_v && r1_id == ID_W'(i)) begin
                id_first[i] <= 1'b0;
                id_off[i]   <= id_off[i] + beat_bytes;
            end
        end
    end

    always_ff @(posedge clk) begin
        if (rst) begin
            running          <= 1'b0;
            now              <= '0;
            since            <= '1;
            to_prep          <= '0;
            to_issue         <= '0;
            seq              <= '0;
            lfsr_a           <= 32'd1;
            lfsr_op          <= 32'd1;
            nx_v             <= 1'b0;
            nx_read          <= 1'b0;
            nx_off           <= '0;
            pres_off         <= '0;
            arvalid          <= 1'b0;
            awvalid          <= 1'b0;
            arid             <= '0;
            awid             <= '0;
            id_busy          <= '0;
            inflight         <= '0;
            wq_wp            <= '0;
            wq_rp            <= '0;
            w_active         <= 1'b0;
            w_beat           <= '0;
            w_off            <= '0;
            wid              <= '0;
            ost              <= O_START;
            o_cnt            <= '0;
            o_n              <= '0;
            o_off            <= '0;
            o_second_sent    <= 1'b0;
            o_rd_done        <= 1'b0;
            o_wr_done        <= 1'b0;
            o_bad            <= 1'b0;
            r1_v             <= 1'b0;
            r1_last          <= 1'b0;
            r1_id            <= '0;
            r1_data          <= '0;
            r1_resp          <= '0;
            r1_time          <= '0;
            b1_v             <= 1'b0;
            b1_id            <= '0;
            b1_resp          <= '0;
            b1_time          <= '0;
            smp_rd_first     <= 1'b0;
            smp_rd_last      <= 1'b0;
            smp_wr_resp      <= 1'b0;
            smp_r_latency    <= '0;
            smp_b_latency    <= '0;
            c_v              <= 1'b0;
            c_data           <= '0;
            c_off            <= '0;
            d_v              <= 1'b0;
            d_lo             <= 1'b0;
            d_hi             <= 1'b0;
            d_data           <= '0;
            d_off            <= '0;
            first_seen       <= 1'b0;
            run_cycles       <= '0;
            reads            <= '0;
            writes           <= '0;
            stalls           <= '0;
            data_errors      <= '0;
            first_err_addr   <= '0;
            first_err_data   <= '0;
            resp_errors      <= '0;
            order_mismatches <= '0;
            run_resp_err     <= 1'b0;
            run_data_err     <= 1'b0;
        end else begin
            now <= now + 1'b1;
            if (since != '1) since <= since + 1'b1;
            if (busy) run_cycles <= run_cycles + 1'b1;
            if (stall) stalls <= stalls + 1'b1;

            // ---- Start and end of a run ----
            if (start && !busy) begin
                running      <= 1'b1;
                since        <= '1;
                seq          <= '0;
                lfsr_a       <= cfg_seed;
                lfsr_op      <= (seed_op == 32'd0) ? 32'd1 : seed_op;
                to_prep      <= run_bursts;
                to_issue     <= order ? {run_bursts, 1'b0} : 33'(run_bursts);
                ost          <= O_START;
                o_n          <= '0;
                run_resp_err <= 1'b0;
                run_data_err <= 1'b0;
            end else if (running && !pres_v && inflight == 0 && !w_active && wq_empty
                         && (to_issue == 0 || stop) && (!order || ost == O_START)) begin
                running <= 1'b0;
                nx_v    <= 1'b0;
            end

            // ---- Prepare the next burst ----
            if (prepare) begin
                nx_v    <= 1'b1;
                nx_read <= new_read;
                nx_off  <= new_off;
                to_prep <= to_prep - 1'b1;
                seq     <= (seq + burst_bytes) & span_mask;
                lfsr_a  <= lfsr_next(lfsr_a);
                lfsr_op <= lfsr_next(lfsr_op);
            end else if (nx_used) begin
                nx_v <= 1'b0;
            end

            // ---- VALID: end at the handshake, start at a new burst ----
            arvalid <= (arvalid && !ar_hs) || (present && present_read);
            awvalid <= (awvalid && !aw_hs) || (present && !present_read);
            if (present) begin
                pres_off <= present_off;
                if (present_read) arid <= free_id;
                else              awid <= free_id;
            end
            if (addr_hs) begin
                since    <= 16'd1;
                to_issue <= to_issue - 1'b1;
            end
            inflight <= inflight + 6'(ar_hs) + 6'(aw_hs) - 6'(dec);
            id_busy  <= (id_busy & ~freeing) | (present ? (IDS'(1) << free_id) : '0);

            // ---- ORDER iterations ----
            case (ost)
                O_START:
                    if (o_go) begin
                        o_off         <= nx_off;
                        o_second_sent <= 1'b0;
                        o_rd_done     <= 1'b0;
                        o_wr_done     <= 1'b0;
                        ost           <= O_FIRST;
                    end
                O_FIRST:
                    if (o_mark) begin
                        o_cnt <= 16'd1;
                        // With GAP 0 the second goes now, if no VALID waits.
                        if (o_second) begin
                            o_second_sent <= 1'b1;
                            ost <= O_WAIT;
                        end else ost <= O_GAP;
                    end
                O_GAP: begin
                    if (o_cnt != '1) o_cnt <= o_cnt + 1'b1;
                    if (o_second) begin
                        o_second_sent <= 1'b1;
                        ost <= O_WAIT;
                    end
                end
                O_WAIT: ;
                default: ost <= O_START;
            endcase
            // The iteration ends when its read and its write are complete.
            if (order && running && (ost == O_FIRST || ost == O_GAP || ost == O_WAIT)) begin
                if (r_done) o_rd_done <= 1'b1;
                if (b_done) o_wr_done <= 1'b1;
                if ((o_rd_done || r_done) && (o_wr_done || b_done) && ost == O_WAIT) begin
                    o_n <= o_n + 1'b1;
                    ost <= O_START;
                end
            end

            // ---- W engine ----
            if (present_w && !w_bypass) begin
                wq_id[wq_wp[ID_W-1:0]]  <= free_id;
                wq_off[wq_wp[ID_W-1:0]] <= present_off;
                wq_wp <= wq_wp + 1'b1;
            end
            if (w_free_next) begin
                if (!wq_empty) begin
                    w_active <= 1'b1;
                    wid      <= wq_id[wq_rp[ID_W-1:0]];
                    w_off    <= wq_off[wq_rp[ID_W-1:0]];
                    w_beat   <= '0;
                    wq_rp    <= wq_rp + 1'b1;
                end else if (w_bypass) begin
                    w_active <= 1'b1;
                    wid      <= free_id;
                    w_off    <= present_off;
                    w_beat   <= '0;
                end else begin
                    w_active <= 1'b0;
                end
            end else if (w_hs) begin
                w_beat <= w_beat + 1'b1;
                w_off  <= w_off + beat_bytes;
            end

            // ---- Stage 1 ----
            r1_v    <= rvalid;
            r1_last <= rlast;
            r1_id   <= rid;
            r1_data <= rdata;
            r1_resp <= rresp;
            r1_time <= now;
            b1_v    <= bvalid;
            b1_id   <= bid;
            b1_resp <= bresp;
            b1_time <= now;

            // ---- Stage 2 ----
            smp_rd_first  <= r1_v && id_first[r1_id];
            smp_rd_last   <= r_done;
            smp_r_latency <= r1_time - id_start[r1_id];
            smp_wr_resp   <= b_done;
            smp_b_latency <= b1_time - id_start[b1_id];
            c_v           <= r1_v && check_en;
            c_data        <= b4 ? {32'd0, r1_data[31:0]} : r1_data;
            c_off         <= id_off[r1_id];
            if (r_done) reads <= reads + 1'b1;
            if (b_done) writes <= writes + 1'b1;
            if (r_err || b_err) begin
                resp_errors  <= resp_sum[32] ? 32'hFFFF_FFFF : resp_sum[31:0];
                run_resp_err <= 1'b1;
            end
            // ORDER: a read with a beat that is not the value of the iteration.
            if (order && r1_v) begin
                if (r_done) begin
                    o_bad <= 1'b0;
                    if ((o_bad || o_differs) && order_mismatches != '1)
                        order_mismatches <= order_mismatches + 1'b1;
                end else if (o_differs) begin
                    o_bad <= 1'b1;
                end
            end

            // ---- Stages 3 and 4: data check ----
            d_v    <= c_v;
            d_lo   <= c_v && (c_data[31:0] != c_expect[31:0]);
            d_hi   <= c_v && (c_data[63:32] != c_expect[63:32]);
            d_data <= c_data;
            d_off  <= c_off;
            if (d_v && (d_lo || d_hi)) begin
                data_errors  <= errors_sum[32] ? 32'hFFFF_FFFF : errors_sum[31:0];
                run_data_err <= 1'b1;
                if (!first_seen) begin
                    first_seen     <= 1'b1;
                    first_err_addr <= 32'(d_off) | (d_lo ? 32'd0 : 32'd4);
                    first_err_data <= d_data;
                end
            end

            if (clear && !busy) begin
                run_cycles       <= '0;
                reads            <= '0;
                writes           <= '0;
                stalls           <= '0;
                data_errors      <= '0;
                first_err_addr   <= '0;
                first_err_data   <= '0;
                resp_errors      <= '0;
                order_mismatches <= '0;
                first_seen       <= 1'b0;
            end
        end
    end
endmodule
