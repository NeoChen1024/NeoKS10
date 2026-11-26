// Register window of the memory path test design on M_AXI_GP0, in the domain
// of FCLK0 (clk). The register map is spec/regmap.toml, block memprobe.
//
// This module has the identification, control, status and configuration
// registers, and FCLK_COUNT. A read of another offset goes to the ext port:
// the module puts the offset on ext_addr, and ext_data must be valid
// FETCH - 1 cycles later. Unknown offsets must read 0.
//
// The slave answers every request: it accepts AW and AR when idle, takes W
// beats without wait, and gives one R beat for each beat of a read burst.
// Bursts increment the offset by 4 bytes for each beat.
//
// Commands to the domain of clk_m: cmd_toggle changes for each accepted
// START or CLEAR, and cmd_clear tells which. The configuration outputs do not
// change while busy is 1. The other domain changes ack_toggle when the
// command is complete (ack_toggle arrives here through a synchronizer).

// yosys reads a package import only at the level of the file.
/* verilator lint_off IMPORTSTAR */
import memprobe_regmap_pkg::*;
/* verilator lint_on IMPORTSTAR */

module memprobe_regs #(
    parameter [31:0] BUILD           = 32'h0,
    parameter        CLK_MHZ         = 100,
    parameter        MAX_OUTSTANDING = 1,
    parameter [3:0]  CAP_PORTS       = 4'b0001,
    parameter        CAP_SIZE32      = 0,
    parameter        CAP_MIXED       = 0,
    parameter        CAP_ORDER       = 0,
    parameter        CAP_PATTERNS    = 0,
    parameter        CAP_VERIFY      = 0,
    parameter        FETCH           = 5,
    parameter        OFF_W           = HP_SIZE_LOG2
) (
    input  logic        clk,
    input  logic        rst,

    // AXI3 slave (M_AXI_GP0 of the PS). Only bits 19:0 of the address are used.
    input  logic [19:0] awaddr,
    input  logic [11:0] awid,
    input  logic [3:0]  awlen,
    input  logic        awvalid,
    output logic        awready,
    input  logic [31:0] wdata,
    input  logic [3:0]  wstrb,
    input  logic        wlast,
    input  logic        wvalid,
    output logic        wready,
    output logic [11:0] bid,
    output logic [1:0]  bresp,
    output logic        bvalid,
    input  logic        bready,
    input  logic [19:0] araddr,
    input  logic [11:0] arid,
    input  logic [3:0]  arlen,
    input  logic        arvalid,
    output logic        arready,
    output logic [31:0] rdata,
    output logic [11:0] rid,
    output logic [1:0]  rresp,
    output logic        rlast,
    output logic        rvalid,
    input  logic        rready,

    output logic [19:0] ext_addr,
    input  logic [31:0] ext_data,

    output logic        irq,

    // To and from the domain of clk_m.
    input  logic        mmcm_locked,     // synchronized
    output logic        cmd_toggle,
    output logic        cmd_clear,
    output logic        stop,            // CTRL.ABORT while busy
    input  logic        ack_toggle,      // synchronized
    input  logic        run_resp_err,    // stable when ack_toggle changes
    input  logic        run_data_err,
    input  logic        run_hist_drop,
    output logic        busy,

    output logic [2:0]       cfg_op,
    output logic [1:0]       cfg_pattern,
    output logic [3:0]       cfg_len,
    output logic [1:0]       cfg_size,
    output logic [1:0]       cfg_rd_port,
    output logic [1:0]       cfg_wr_port,
    output logic             cfg_verify,
    output logic             cfg_order_reverse,
    output logic [4:0]       cfg_outstanding,
    output logic [15:0]      cfg_read_share,
    output logic [OFF_W-1:0] cfg_base,
    output logic [4:0]       cfg_span_log2,
    output logic [31:0]      cfg_count,
    output logic [15:0]      cfg_gap,
    output logic [31:0]      cfg_seed
);
    // ---- Registers -----------------------------------------------------------
    logic [31:0] mode, base, span_log2, count, gap, outstanding, read_share, seed, irq_en;
    logic        st_done, st_resp_err, st_cfg_err, st_hist_drop, st_data_err;
    logic [31:0] fclk_count;
    logic        ack_seen;

    localparam [31:0] CAPS = (32'(CLK_MHZ)         << CAPS_CLK_MHZ_LSB)
                           | (32'(MAX_OUTSTANDING) << CAPS_MAX_OUTSTANDING_LSB)
                           | (32'(CAP_PORTS)       << CAPS_PORTS_LSB)
                           | (32'(CAP_VERIFY)      << CAPS_VERIFY_LSB)
                           | (32'(CAP_SIZE32)      << CAPS_SIZE32_LSB)
                           | (32'(CAP_MIXED)       << CAPS_MIXED_LSB)
                           | (32'(CAP_ORDER)       << CAPS_ORDER_LSB)
                           | (32'(CAP_PATTERNS)    << CAPS_PATTERNS_LSB);

    wire [31:0] status = (32'(busy)         << STATUS_BUSY_LSB)
                       | (32'(st_done)      << STATUS_DONE_LSB)
                       | (32'(st_resp_err)  << STATUS_RESP_ERR_LSB)
                       | (32'(st_cfg_err)   << STATUS_CFG_ERR_LSB)
                       | (32'(st_hist_drop) << STATUS_HIST_DROP_LSB)
                       | (32'(st_data_err)  << STATUS_DATA_ERR_LSB)
                       | (32'(mmcm_locked)  << STATUS_MMCM_LOCKED_LSB);

    assign irq = st_done && irq_en[IRQ_EN_DONE_LSB];

    // ---- Configuration check -------------------------------------------------
    wire [2:0] m_op   = 3'(mode >> MODE_OP_LSB);
    wire [1:0] m_pat  = 2'(mode >> MODE_PATTERN_LSB);
    wire [3:0] m_len  = 4'(mode >> MODE_LEN_LSB);
    wire [1:0] m_size = 2'(mode >> MODE_SIZE_LSB);
    wire [1:0] m_rdp  = 2'(mode >> MODE_RD_PORT_LSB);
    wire [1:0] m_wrp  = 2'(mode >> MODE_WR_PORT_LSB);
    wire       m_ver  = mode[MODE_VERIFY_LSB];
    wire       m_rev  = mode[MODE_ORDER_REVERSE_LSB];
    wire [4:0] m_span = 5'(span_log2);

    wire op_ok = (m_op == MODE_OP_READ) || (m_op == MODE_OP_WRITE)
              || (m_op == MODE_OP_FILL) || (m_op == MODE_OP_CHECK)
              || (m_op == MODE_OP_MIXED && CAP_MIXED != 0)
              || (m_op == MODE_OP_ORDER && CAP_ORDER != 0);
    wire whole_span = (m_op == MODE_OP_FILL) || (m_op == MODE_OP_CHECK);
    wire pat_ok = (m_pat == MODE_PATTERN_SEQ)
               || ((m_pat == MODE_PATTERN_RANDOM || m_pat == MODE_PATTERN_FIXED)
                   && CAP_PATTERNS != 0 && !whole_span);
    wire len_ok = (m_len == 4'd0) || (m_len == 4'd1) || (m_len == 4'd3)
               || (m_len == 4'd7) || (m_len == 4'd15);
    wire size_ok = (m_size == MODE_SIZE_B8) || (m_size == MODE_SIZE_B4 && CAP_SIZE32 != 0);
    wire port_ok = CAP_PORTS[m_rdp] && CAP_PORTS[m_wrp];
    wire ver_ok  = !m_ver || CAP_VERIFY != 0;
    wire rev_ok  = !m_rev || m_op == MODE_OP_ORDER;
    wire [5:0] burst_log2 = 6'(m_size) + 6'(m_len[0]) + 6'(m_len[1]) + 6'(m_len[2]) + 6'(m_len[3]);
    wire span_ok = (6'(m_span) >= burst_log2) && (32'(m_span) <= HP_SIZE_LOG2);
    wire [32:0] span_bytes = 33'd1 << m_span;
    wire base_ok = ((33'(base) & (span_bytes - 1'b1)) == 33'd0)
                && (33'(base) + span_bytes <= 33'(HP_SIZE));
    wire out_ok = (outstanding >= 32'd1) && (outstanding <= 32'(MAX_OUTSTANDING));
    wire cfg_ok = op_ok && pat_ok && len_ok && size_ok && port_ok && ver_ok && rev_ok && span_ok
               && base_ok && out_ok && (seed != 32'd0);

    assign cfg_op        = m_op;
    assign cfg_pattern   = m_pat;
    assign cfg_len       = m_len;
    assign cfg_size      = m_size;
    assign cfg_rd_port   = m_rdp;
    assign cfg_wr_port   = m_wrp;
    assign cfg_verify    = m_ver;
    assign cfg_order_reverse = m_rev;
    assign cfg_outstanding = 5'(outstanding);
    assign cfg_read_share  = read_share[15:0];
    assign cfg_base      = base[OFF_W-1:0];
    assign cfg_span_log2 = m_span;
    assign cfg_count     = count;
    assign cfg_gap       = gap[15:0];
    assign cfg_seed      = seed;

    // ---- Read path -----------------------------------------------------------
    function automatic [31:0] own_read(input [19:0] a);
        case (32'(a))
            ID_OFFSET:          own_read = ID_VALUE;
            VERSION_OFFSET:     own_read = VERSION_VALUE;
            BUILD_OFFSET:       own_read = BUILD;
            CAPS_OFFSET:        own_read = CAPS;
            STATUS_OFFSET:      own_read = status;
            IRQ_EN_OFFSET:      own_read = irq_en;
            MODE_OFFSET:        own_read = mode;
            BASE_OFFSET:        own_read = base;
            SPAN_LOG2_OFFSET:   own_read = span_log2;
            COUNT_OFFSET:       own_read = count;
            GAP_OFFSET:         own_read = gap;
            OUTSTANDING_OFFSET: own_read = outstanding;
            READ_SHARE_OFFSET:  own_read = read_share;
            SEED_OFFSET:        own_read = seed;
            FCLK_COUNT_OFFSET:  own_read = fclk_count;
            default:            own_read = 32'd0;
        endcase
    endfunction

    function automatic is_own(input [19:0] a);
        case (32'(a))
            ID_OFFSET, VERSION_OFFSET, BUILD_OFFSET, CAPS_OFFSET, CTRL_OFFSET, STATUS_OFFSET,
            IRQ_EN_OFFSET, MODE_OFFSET, BASE_OFFSET, SPAN_LOG2_OFFSET, COUNT_OFFSET, GAP_OFFSET,
            OUTSTANDING_OFFSET, READ_SHARE_OFFSET, SEED_OFFSET, FCLK_COUNT_OFFSET:
                is_own = 1'b1;
            default: is_own = 1'b0;
        endcase
    endfunction

    localparam [1:0] R_ADDR = 2'd0, R_FETCH = 2'd1, R_DATA = 2'd2;
    logic [1:0]  rs;
    logic [3:0]  rleft;
    logic [2:0]  rwait;
    assign arready  = (rs == R_ADDR);
    assign rvalid   = (rs == R_DATA);
    assign rlast    = (rleft == 4'd0);
    assign rresp    = 2'b00;

    always_ff @(posedge clk) begin
        if (rst) begin
            rs       <= R_ADDR;
            ext_addr <= '0;
            rleft    <= '0;
            rwait    <= '0;
            rid      <= '0;
            rdata    <= '0;
        end else case (rs)
            R_ADDR:
                if (arvalid) begin
                    ext_addr <= {araddr[19:2], 2'b00};
                    rleft    <= arlen;
                    rid      <= arid;
                    rwait    <= 3'(FETCH - 1);
                    rs       <= R_FETCH;
                end
            R_FETCH:
                if (rwait != 0) rwait <= rwait - 1'b1;
                else begin
                    rdata <= is_own(ext_addr) ? own_read(ext_addr) : ext_data;
                    rs    <= R_DATA;
                end
            R_DATA:
                if (rready) begin
                    if (rleft == 0) rs <= R_ADDR;
                    else begin
                        rleft    <= rleft - 1'b1;
                        ext_addr <= ext_addr + 20'd4;
                        rwait    <= 3'(FETCH - 1);
                        rs       <= R_FETCH;
                    end
                end
            default: rs <= R_ADDR;
        endcase
    end

    // ---- Write path ----------------------------------------------------------
    localparam [1:0] W_ADDR = 2'd0, W_DATA = 2'd1, W_RESP = 2'd2;
    logic [1:0]  ws;
    logic [19:0] wa;
    assign awready = (ws == W_ADDR);
    assign wready  = (ws == W_DATA);
    assign bvalid  = (ws == W_RESP);
    assign bresp   = 2'b00;

    wire        we = wvalid && wready;
    wire [31:0] bmask = {{8{wstrb[3]}}, {8{wstrb[2]}}, {8{wstrb[1]}}, {8{wstrb[0]}}};
    wire [31:0] wd = wdata & bmask;

    function automatic [31:0] merge(input [31:0] old, input [31:0] d, input [31:0] m,
                                    input [31:0] fields);
        merge = ((old & ~m) | (d & m)) & fields;
    endfunction

    always_ff @(posedge clk) begin
        if (rst) begin
            ws  <= W_ADDR;
            wa  <= '0;
            bid <= '0;
        end else case (ws)
            W_ADDR: if (awvalid) begin
                        wa  <= {awaddr[19:2], 2'b00};
                        bid <= awid;
                        ws  <= W_DATA;
                    end
            W_DATA: if (wvalid) begin
                        wa <= wa + 20'd4;
                        if (wlast) ws <= W_RESP;
                    end
            W_RESP: if (bready) ws <= W_ADDR;
            default: ws <= W_ADDR;
        endcase
    end

    // The write burst ends at WLAST; the length is not needed.
    wire unused_ok = &{1'b0, awaddr[1:0], awlen, araddr[1:0]};

    // ---- Control -------------------------------------------------------------
    wire w_ctrl   = we && 32'(wa) == CTRL_OFFSET;
    wire w_status = we && 32'(wa) == STATUS_OFFSET;
    wire w_cfg    = we && !busy;
    wire [2:0] cmd = {wd[CTRL_CLEAR_LSB], wd[CTRL_ABORT_LSB], wd[CTRL_START_LSB]};
    wire cmd_one = (cmd == 3'b001) || (cmd == 3'b010) || (cmd == 3'b100);
    wire do_start = w_ctrl && cmd == 3'b001 && !busy && cfg_ok && mmcm_locked;
    wire do_clear = w_ctrl && cmd == 3'b100 && !busy && mmcm_locked;
    wire refuse   = w_ctrl && (!cmd_one && cmd != 3'b000
                               || cmd == 3'b001 && (busy || !cfg_ok || !mmcm_locked));
    wire ack      = busy && (ack_toggle != ack_seen);

    always_ff @(posedge clk) begin
        if (rst) begin
            mode         <= MODE_RESET;
            base         <= BASE_RESET;
            span_log2    <= SPAN_LOG2_RESET;
            count        <= COUNT_RESET;
            gap          <= GAP_RESET;
            outstanding  <= OUTSTANDING_RESET;
            read_share   <= READ_SHARE_RESET;
            seed         <= SEED_RESET;
            irq_en       <= IRQ_EN_RESET;
            st_done      <= 1'b0;
            st_resp_err  <= 1'b0;
            st_cfg_err   <= 1'b0;
            st_hist_drop <= 1'b0;
            st_data_err  <= 1'b0;
            busy         <= 1'b0;
            stop         <= 1'b0;
            cmd_toggle   <= 1'b0;
            cmd_clear    <= 1'b0;
            ack_seen     <= 1'b0;
            fclk_count   <= '0;
        end else begin
            fclk_count <= fclk_count + 1'b1;

            if (w_cfg) case (32'(wa))
                MODE_OFFSET:        mode        <= merge(mode,        wdata, bmask, MODE_MASK);
                BASE_OFFSET:        base        <= merge(base,        wdata, bmask, BASE_MASK);
                SPAN_LOG2_OFFSET:   span_log2   <= merge(span_log2,   wdata, bmask, SPAN_LOG2_MASK);
                COUNT_OFFSET:       count       <= merge(count,       wdata, bmask, COUNT_MASK);
                GAP_OFFSET:         gap         <= merge(gap,         wdata, bmask, GAP_MASK);
                OUTSTANDING_OFFSET: outstanding <= merge(outstanding, wdata, bmask, OUTSTANDING_MASK);
                READ_SHARE_OFFSET:  read_share  <= merge(read_share,  wdata, bmask, READ_SHARE_MASK);
                SEED_OFFSET:        seed        <= merge(seed,        wdata, bmask, SEED_MASK);
                default: ;
            endcase
            if (we && 32'(wa) == IRQ_EN_OFFSET)
                irq_en <= merge(irq_en, wdata, bmask, IRQ_EN_MASK);

            // w1c bits. A bit that is set in the same cycle stays 1.
            if (w_status) begin
                if (wd[STATUS_DONE_LSB])      st_done      <= 1'b0;
                if (wd[STATUS_RESP_ERR_LSB])  st_resp_err  <= 1'b0;
                if (wd[STATUS_CFG_ERR_LSB])   st_cfg_err   <= 1'b0;
                if (wd[STATUS_HIST_DROP_LSB]) st_hist_drop <= 1'b0;
                if (wd[STATUS_DATA_ERR_LSB])  st_data_err  <= 1'b0;
            end

            if (refuse) st_cfg_err <= 1'b1;
            if (do_start || do_clear) begin
                busy       <= 1'b1;
                cmd_toggle <= !cmd_toggle;
                cmd_clear  <= do_clear;
            end
            if (w_ctrl && cmd == 3'b010 && busy) stop <= 1'b1;

            if (!busy) ack_seen <= ack_toggle;
            if (ack) begin
                ack_seen <= ack_toggle;
                busy     <= 1'b0;
                stop     <= 1'b0;
                if (!cmd_clear) begin
                    st_done <= 1'b1;
                    if (run_resp_err)  st_resp_err  <= 1'b1;
                    if (run_data_err)  st_data_err  <= 1'b1;
                    if (run_hist_drop) st_hist_drop <= 1'b1;
                end
            end

            // If the MMCM is not locked, the domain of clk_m is in reset. A
            // command in progress ends without DONE.
            if (!mmcm_locked) begin
                busy       <= 1'b0;
                stop       <= 1'b0;
                cmd_toggle <= 1'b0;
            end
        end
    end
endmodule
