// The memory path test design without the PS7 cell (docs/mem-path-experiment.md,
// Part B). memprobe_top connects it to the PS7; the testbench connects it to
// models.
//
// Two clock domains:
//   clk_s  FCLK0, the clock of the board interface: the GP0 slave and the
//          registers (memprobe_regs)
//   clk_m  the clock of the HP port: the generator and the histograms
// Crossings:
//   - Commands: a toggle from clk_s to clk_m, and an acknowledge toggle back.
//   - Configuration: stable while busy; clk_m copies it at the command.
//   - Results: the counters of clk_m are stable while no run is in progress.
//     clk_s reads them directly. A read during a run is not guaranteed.
//   - Histograms: block RAM with one port on each clock.
//   - CLKM_COUNT: Gray code through a synchronizer.
//   - STOP and MMCM_LOCKED: levels through synchronizers.
// The constraints for these paths are in fpga/memprobe.

/* verilator lint_off IMPORTSTAR */
import memprobe_regmap_pkg::*;
/* verilator lint_on IMPORTSTAR */

module memprobe_core #(
    parameter [31:0] BUILD   = 32'h0,
    parameter        CLK_MHZ = 100
) (
    input  logic        clk_s,
    input  logic        rst_s,
    input  logic [19:0] gp_awaddr,
    input  logic [11:0] gp_awid,
    input  logic [3:0]  gp_awlen,
    input  logic        gp_awvalid,
    output logic        gp_awready,
    input  logic [31:0] gp_wdata,
    input  logic [3:0]  gp_wstrb,
    input  logic        gp_wlast,
    input  logic        gp_wvalid,
    output logic        gp_wready,
    output logic [11:0] gp_bid,
    output logic [1:0]  gp_bresp,
    output logic        gp_bvalid,
    input  logic        gp_bready,
    input  logic [19:0] gp_araddr,
    input  logic [11:0] gp_arid,
    input  logic [3:0]  gp_arlen,
    input  logic        gp_arvalid,
    output logic        gp_arready,
    output logic [31:0] gp_rdata,
    output logic [11:0] gp_rid,
    output logic [1:0]  gp_rresp,
    output logic        gp_rlast,
    output logic        gp_rvalid,
    input  logic        gp_rready,
    output logic        irq,
    output logic        busy,          // for a LED

    input  logic        clk_m,
    input  logic        rst_m,
    input  logic        mmcm_locked,   // asynchronous
    // S_AXI_HP0 to S_AXI_HP3. The reads use the port of MODE.RD_PORT, the
    // writes the port of MODE.WR_PORT; the other ports have no VALID.
    output logic [31:0] hp_araddr  [0:3],
    output logic [3:0]  hp_arlen   [0:3],
    output logic [1:0]  hp_arsize  [0:3],
    output logic [5:0]  hp_arid    [0:3],
    output logic        hp_arvalid [0:3],
    input  logic        hp_arready [0:3],
    input  logic [63:0] hp_rdata   [0:3],
    input  logic [1:0]  hp_rresp   [0:3],
    input  logic        hp_rlast   [0:3],
    input  logic [5:0]  hp_rid     [0:3],
    input  logic        hp_rvalid  [0:3],
    output logic        hp_rready  [0:3],
    output logic [31:0] hp_awaddr  [0:3],
    output logic [3:0]  hp_awlen   [0:3],
    output logic [1:0]  hp_awsize  [0:3],
    output logic [5:0]  hp_awid    [0:3],
    output logic        hp_awvalid [0:3],
    input  logic        hp_awready [0:3],
    output logic [63:0] hp_wdata   [0:3],
    output logic [7:0]  hp_wstrb   [0:3],
    output logic        hp_wlast   [0:3],
    output logic [5:0]  hp_wid     [0:3],
    output logic        hp_wvalid  [0:3],
    input  logic        hp_wready  [0:3],
    input  logic [1:0]  hp_bresp   [0:3],
    input  logic [5:0]  hp_bid     [0:3],
    input  logic        hp_bvalid  [0:3],
    output logic        hp_bready  [0:3]
);
    localparam OFF_W = HP_SIZE_LOG2;
    localparam IDS = 16;
    localparam GEN_ID_W = 4;
    localparam BINS_LOG2 = 10;

    // ---- Domain of clk_s ----------------------------------------------------------------
    logic [19:0]      ext_addr;
    logic [31:0]      ext_data;
    logic             locked_s, ack_s;
    logic             cmd_toggle, cmd_clear, stop_s;
    logic [2:0]       cfg_op;
    logic [1:0]       cfg_pattern, cfg_size, cfg_rd_port, cfg_wr_port;
    logic [3:0]       cfg_len;
    logic [OFF_W-1:0] cfg_base;
    logic [4:0]       cfg_span_log2, cfg_outstanding;
    logic [31:0]      cfg_count, cfg_seed;
    logic [15:0]      cfg_gap, cfg_read_share;
    logic             cfg_verify, cfg_order_reverse;
    logic             run_resp_err, run_data_err, run_hist_drop;
    logic             ack_toggle;

    memprobe_sync sync_locked (.clk(clk_s), .rst(rst_s), .d(mmcm_locked), .q(locked_s));
    memprobe_sync sync_ack    (.clk(clk_s), .rst(rst_s), .d(ack_toggle),  .q(ack_s));

    memprobe_regs #(
        .BUILD(BUILD), .CLK_MHZ(CLK_MHZ), .MAX_OUTSTANDING(IDS), .CAP_PORTS(4'b1111),
        .CAP_SIZE32(1), .CAP_MIXED(1), .CAP_ORDER(1), .CAP_PATTERNS(1), .CAP_VERIFY(1)
    ) regs (
        .clk(clk_s), .rst(rst_s),
        .awaddr(gp_awaddr), .awid(gp_awid), .awlen(gp_awlen), .awvalid(gp_awvalid),
        .awready(gp_awready), .wdata(gp_wdata), .wstrb(gp_wstrb), .wlast(gp_wlast),
        .wvalid(gp_wvalid), .wready(gp_wready), .bid(gp_bid), .bresp(gp_bresp),
        .bvalid(gp_bvalid), .bready(gp_bready), .araddr(gp_araddr), .arid(gp_arid),
        .arlen(gp_arlen), .arvalid(gp_arvalid), .arready(gp_arready), .rdata(gp_rdata),
        .rid(gp_rid), .rresp(gp_rresp), .rlast(gp_rlast), .rvalid(gp_rvalid),
        .rready(gp_rready),
        .ext_addr(ext_addr), .ext_data(ext_data), .irq(irq),
        .mmcm_locked(locked_s), .cmd_toggle(cmd_toggle), .cmd_clear(cmd_clear),
        .stop(stop_s), .ack_toggle(ack_s), .run_resp_err(run_resp_err),
        .run_data_err(run_data_err), .run_hist_drop(run_hist_drop), .busy(busy),
        .cfg_op(cfg_op), .cfg_pattern(cfg_pattern), .cfg_len(cfg_len), .cfg_size(cfg_size),
        .cfg_rd_port(cfg_rd_port), .cfg_wr_port(cfg_wr_port), .cfg_verify(cfg_verify),
        .cfg_order_reverse(cfg_order_reverse), .cfg_outstanding(cfg_outstanding),
        .cfg_read_share(cfg_read_share), .cfg_base(cfg_base),
        .cfg_span_log2(cfg_span_log2), .cfg_count(cfg_count), .cfg_gap(cfg_gap),
        .cfg_seed(cfg_seed)
    );

    // ---- Domain of clk_m: commands ------------------------------------------------------
    logic cmd_m, cmd_prev, stop_m;
    memprobe_sync sync_cmd  (.clk(clk_m), .rst(rst_m), .d(cmd_toggle), .q(cmd_m));
    memprobe_sync sync_stop (.clk(clk_m), .rst(rst_m), .d(stop_s),     .q(stop_m));

    logic [2:0]       m_op;
    logic [1:0]       m_pattern, m_size;
    logic [3:0]       m_len;
    logic [OFF_W-1:0] m_base;
    logic [4:0]       m_span_log2, m_outstanding;
    logic [31:0]      m_count, m_seed;
    logic [15:0]      m_gap, m_read_share;
    logic             m_verify, m_order_reverse;
    logic [1:0]       m_rd_port, m_wr_port;
    logic             gen_start, gen_clear, gen_busy;
    logic [2:0]       hist_busy;

    // M_IDLE: wait for a command. M_WAIT: one cycle for busy to become 1.
    // M_RUN: wait until the generator and the histograms have finished.
    localparam [1:0] M_IDLE = 2'd0, M_WAIT = 2'd1, M_RUN = 2'd2;
    logic [1:0] mstate;

    always_ff @(posedge clk_m) begin
        gen_start <= 1'b0;
        gen_clear <= 1'b0;
        if (rst_m) begin
            mstate      <= M_IDLE;
            cmd_prev    <= 1'b0;
            ack_toggle  <= 1'b0;
            m_op        <= '0;
            m_pattern   <= '0;
            m_outstanding <= 5'd1;
            m_read_share  <= '0;
            m_verify    <= 1'b0;
            m_order_reverse <= 1'b0;
            m_rd_port   <= '0;
            m_wr_port   <= '0;
            m_len       <= '0;
            m_size      <= MODE_SIZE_B8;
            m_base      <= '0;
            m_span_log2 <= '0;
            m_count     <= '0;
            m_seed      <= '0;
            m_gap       <= '0;
        end else case (mstate)
            M_IDLE:
                if (cmd_m != cmd_prev) begin
                    cmd_prev <= cmd_m;
                    // The configuration and cmd_clear were stable before the toggle.
                    if (cmd_clear) gen_clear <= 1'b1;
                    else begin
                        gen_start   <= 1'b1;
                        m_op        <= cfg_op;
                        m_pattern   <= cfg_pattern;
                        m_outstanding <= cfg_outstanding;
                        m_read_share  <= cfg_read_share;
                        m_verify    <= cfg_verify;
                        m_order_reverse <= cfg_order_reverse;
                        m_rd_port   <= cfg_rd_port;
                        m_wr_port   <= cfg_wr_port;
                        m_len       <= cfg_len;
                        m_size      <= cfg_size;
                        m_base      <= cfg_base;
                        m_span_log2 <= cfg_span_log2;
                        m_count     <= cfg_count;
                        m_seed      <= cfg_seed;
                        m_gap       <= cfg_gap;
                    end
                    mstate <= M_WAIT;
                end
            M_WAIT: mstate <= M_RUN;
            M_RUN:
                if (!gen_busy && hist_busy == 3'b000) begin
                    ack_toggle <= !ack_toggle;
                    mstate     <= M_IDLE;
                end
            default: mstate <= M_IDLE;
        endcase
    end

    // ---- Domain of clk_m: generator and histograms ---------------------------------------
    logic        smp_rd_first, smp_rd_last, smp_wr_resp;
    logic [31:0] smp_r_latency, smp_b_latency;
    logic [GEN_ID_W-1:0] gen_arid, gen_awid, gen_wid;
    logic [31:0] order_mismatches;

    // The channels of the generator, and the ports.
    logic [31:0] g_araddr, g_awaddr;
    logic [3:0]  g_arlen, g_awlen;
    logic [63:0] g_rdata, g_wdata;
    logic [1:0]  g_rresp, g_bresp;
    logic [5:0]  g_rid, g_bid;
    logic        g_arvalid, g_arready, g_rlast, g_rvalid, g_rready;
    logic        g_awvalid, g_awready, g_wlast, g_wvalid, g_wready, g_bvalid, g_bready;
    always_comb begin
        for (int p = 0; p < 4; p++) begin
            hp_araddr[p]  = g_araddr;
            hp_arlen[p]   = g_arlen;
            hp_arsize[p]  = m_size;
            hp_arid[p]    = 6'(gen_arid);
            hp_arvalid[p] = g_arvalid && m_rd_port == 2'(p);
            hp_rready[p]  = g_rready;
            hp_awaddr[p]  = g_awaddr;
            hp_awlen[p]   = g_awlen;
            hp_awsize[p]  = m_size;
            hp_awid[p]    = 6'(gen_awid);
            hp_awvalid[p] = g_awvalid && m_wr_port == 2'(p);
            hp_wdata[p]   = g_wdata;
            // B4: the 32-bit mode of the AFI takes the strobes of bits 31:0.
            hp_wstrb[p]   = (m_size == MODE_SIZE_B4) ? 8'h0F : 8'hFF;
            hp_wlast[p]   = g_wlast;
            hp_wid[p]     = 6'(gen_wid);
            hp_wvalid[p]  = g_wvalid && m_wr_port == 2'(p);
            hp_bready[p]  = g_bready;
        end
        g_arready = hp_arready[m_rd_port];
        g_rdata   = hp_rdata[m_rd_port];
        g_rresp   = hp_rresp[m_rd_port];
        g_rlast   = hp_rlast[m_rd_port];
        g_rid     = hp_rid[m_rd_port];
        g_rvalid  = hp_rvalid[m_rd_port];
        g_awready = hp_awready[m_wr_port];
        g_wready  = hp_wready[m_wr_port];
        g_bresp   = hp_bresp[m_wr_port];
        g_bid     = hp_bid[m_wr_port];
        g_bvalid  = hp_bvalid[m_wr_port];
    end
    logic [63:0] run_cycles, reads, writes, stalls, first_err_data;
    logic [31:0] data_errors, first_err_addr, resp_errors;

    memprobe_gen #(.OFF_W(OFF_W), .IDS(IDS), .ID_W(GEN_ID_W)) gen (
        .clk(clk_m), .rst(rst_m),
        .start(gen_start), .clear(gen_clear), .stop(stop_m), .busy(gen_busy),
        .cfg_op(m_op), .cfg_pattern(m_pattern), .cfg_len(m_len), .cfg_size(m_size), .cfg_base(m_base),
        .cfg_span_log2(m_span_log2), .cfg_count(m_count), .cfg_gap(m_gap),
        .cfg_outstanding(m_outstanding), .cfg_read_share(m_read_share), .cfg_seed(m_seed),
        .cfg_verify(m_verify), .cfg_order_reverse(m_order_reverse),
        .araddr(g_araddr), .arlen(g_arlen), .arid(gen_arid), .arvalid(g_arvalid),
        .arready(g_arready), .rdata(g_rdata), .rresp(g_rresp), .rlast(g_rlast),
        .rid(g_rid[GEN_ID_W-1:0]), .rvalid(g_rvalid), .rready(g_rready),
        .awaddr(g_awaddr), .awlen(g_awlen), .awid(gen_awid), .awvalid(g_awvalid),
        .awready(g_awready), .wdata(g_wdata), .wid(gen_wid), .wlast(g_wlast),
        .wvalid(g_wvalid), .wready(g_wready), .bresp(g_bresp), .bid(g_bid[GEN_ID_W-1:0]),
        .bvalid(g_bvalid), .bready(g_bready), .order_mismatches(order_mismatches),
        .smp_rd_first(smp_rd_first), .smp_rd_last(smp_rd_last), .smp_r_latency(smp_r_latency),
        .smp_wr_resp(smp_wr_resp), .smp_b_latency(smp_b_latency),
        .run_cycles(run_cycles), .reads(reads), .writes(writes), .stalls(stalls),
        .data_errors(data_errors), .first_err_addr(first_err_addr),
        .first_err_data(first_err_data), .resp_errors(resp_errors),
        .run_resp_err(run_resp_err), .run_data_err(run_data_err)
    );

    logic [2:0]          hist_drop;
    logic [31:0]         h_min [0:2];
    logic [31:0]         h_max [0:2];
    logic [63:0]         h_sum [0:2];
    logic [31:0]         h_drops [0:2];
    logic [31:0]         h_rd [0:2];
    wire  [2:0]          h_smp = {smp_wr_resp, smp_rd_last, smp_rd_first};
    wire  [31:0]         h_val [0:2];
    assign h_val[0] = smp_r_latency;
    assign h_val[1] = smp_r_latency;
    assign h_val[2] = smp_b_latency;
    // The design uses IDs below IDS; the upper bits of RID and BID are 0.
    wire unused_cfg = &{1'b0, g_rid[5:GEN_ID_W], g_bid[5:GEN_ID_W]};

    genvar g;
    generate for (g = 0; g < 3; g++) begin : hist
        memprobe_hist #(.BINS_LOG2(BINS_LOG2)) h (
            .clk(clk_m), .rst(rst_m), .clear(gen_clear),
            .smp_valid(h_smp[g]), .smp_value(h_val[g]),
            .busy(hist_busy[g]), .drop(hist_drop[g]),
            .min(h_min[g]), .max(h_max[g]), .sum(h_sum[g]), .drops(h_drops[g]),
            .rd_clk(clk_s), .rd_addr(ext_addr[BINS_LOG2+1:2]), .rd_data(h_rd[g])
        );
    end endgenerate

    // A drop in this run, for STATUS.HIST_DROP.
    always_ff @(posedge clk_m)
        if (rst_m || gen_start) run_hist_drop <= 1'b0;
        else if (hist_drop != 3'b000) run_hist_drop <= 1'b1;

    // CLKM_COUNT: Gray code into clk_s.
    logic [31:0] clkm_count, clkm_gray, clkm_gray_s, clkm_count_s;
    always_ff @(posedge clk_m)
        if (rst_m) begin
            clkm_count <= '0;
            clkm_gray  <= '0;
        end else begin
            clkm_count <= clkm_count + 1'b1;
            clkm_gray  <= clkm_count ^ (clkm_count >> 1);
        end
    memprobe_sync #(.W(32)) sync_clkm (.clk(clk_s), .rst(rst_s), .d(clkm_gray), .q(clkm_gray_s));
    always_comb
        for (int i = 0; i < 32; i++) clkm_count_s[i] = ^(clkm_gray_s >> i);

    // HIST_DROPS is the sum of the three counters; it stops at 0xFFFFFFFF. The
    // sum is a register of clk_m, so that only a stable value crosses.
    wire [33:0] drops_sum = 34'(h_drops[0]) + 34'(h_drops[1]) + 34'(h_drops[2]);
    logic [31:0] drops_total;
    always_ff @(posedge clk_m)
        drops_total <= (drops_sum[33:32] != 2'b00) ? 32'hFFFF_FFFF : drops_sum[31:0];

    // ---- Domain of clk_s: reads of the results -------------------------------------------

    localparam [31:0] HIST_MASK = ~(HIST_RD_FIRST_DEPTH * 4 - 1);
    wire [31:0] a = 32'(ext_addr);

    always_ff @(posedge clk_s) begin
        if ((a & HIST_MASK) == HIST_RD_FIRST_OFFSET)     ext_data <= h_rd[0];
        else if ((a & HIST_MASK) == HIST_RD_LAST_OFFSET) ext_data <= h_rd[1];
        else if ((a & HIST_MASK) == HIST_WR_RESP_OFFSET) ext_data <= h_rd[2];
        else case (a)
            RUN_CYCLES_LO_OFFSET:     ext_data <= run_cycles[31:0];
            RUN_CYCLES_HI_OFFSET:     ext_data <= run_cycles[63:32];
            READS_LO_OFFSET:          ext_data <= reads[31:0];
            READS_HI_OFFSET:          ext_data <= reads[63:32];
            WRITES_LO_OFFSET:         ext_data <= writes[31:0];
            WRITES_HI_OFFSET:         ext_data <= writes[63:32];
            STALLS_LO_OFFSET:         ext_data <= stalls[31:0];
            STALLS_HI_OFFSET:         ext_data <= stalls[63:32];
            DATA_ERRORS_OFFSET:       ext_data <= data_errors;
            FIRST_ERR_ADDR_OFFSET:    ext_data <= first_err_addr;
            FIRST_ERR_DATA_LO_OFFSET: ext_data <= first_err_data[31:0];
            FIRST_ERR_DATA_HI_OFFSET: ext_data <= first_err_data[63:32];
            ORDER_MISMATCHES_OFFSET:  ext_data <= order_mismatches;
            HIST_DROPS_OFFSET:        ext_data <= drops_total;
            RESP_ERRORS_OFFSET:       ext_data <= resp_errors;
            CLKM_COUNT_OFFSET:        ext_data <= clkm_count_s;
            RD_FIRST_MIN_OFFSET:      ext_data <= h_min[0];
            RD_FIRST_MAX_OFFSET:      ext_data <= h_max[0];
            RD_FIRST_SUM_LO_OFFSET:   ext_data <= h_sum[0][31:0];
            RD_FIRST_SUM_HI_OFFSET:   ext_data <= h_sum[0][63:32];
            RD_LAST_MIN_OFFSET:       ext_data <= h_min[1];
            RD_LAST_MAX_OFFSET:       ext_data <= h_max[1];
            RD_LAST_SUM_LO_OFFSET:    ext_data <= h_sum[1][31:0];
            RD_LAST_SUM_HI_OFFSET:    ext_data <= h_sum[1][63:32];
            WR_RESP_MIN_OFFSET:       ext_data <= h_min[2];
            WR_RESP_MAX_OFFSET:       ext_data <= h_max[2];
            WR_RESP_SUM_LO_OFFSET:    ext_data <= h_sum[2][31:0];
            WR_RESP_SUM_HI_OFFSET:    ext_data <= h_sum[2][63:32];
            default:                  ext_data <= 32'd0;
        endcase
    end
endmodule
