// One latency histogram of the memory path test design, with its statistics.
//
// Domain of clk (clk_m): samples, the statistics and the update of the bins.
// Domain of rd_clk (FCLK0): one read port for the register window.
//
// A sample goes into a FIFO. The update of a bin takes four cycles: a read
// of the block RAM, its output register, the increment, then the write. If the FIFO is full, the sample is not put
// into the histogram, and drops counts it. The statistics (minimum, maximum,
// sum) use all samples, also the dropped samples.
// Bin n counts the samples with latency n; the last bin counts the samples
// with that latency or more. A bin stops at 0xFFFFFFFF.
module memprobe_hist #(
    parameter BINS_LOG2 = 10,
    parameter FIFO_LOG2 = 4
) (
    input  logic                 clk,
    input  logic                 rst,
    input  logic                 clear,     // pulse, only while busy is 0: all to 0, in 2^BINS_LOG2 cycles
    input  logic                 smp_valid,
    input  logic [31:0]          smp_value,
    output logic                 busy,      // clearing, or samples not yet in the bins
    output logic                 drop,      // pulse: a sample was dropped
    output logic [31:0]          min,
    output logic [31:0]          max,
    output logic [63:0]          sum,
    output logic [31:0]          drops,

    input  logic                 rd_clk,
    input  logic [BINS_LOG2-1:0] rd_addr,
    output logic [31:0]          rd_data
);
    localparam BINS = 1 << BINS_LOG2;
    localparam DEPTH = 1 << FIFO_LOG2;

    logic [31:0] mem [0:BINS-1];
    initial for (int i = 0; i < BINS; i++) mem[i] = '0;

    // Port B: the register window.
    always_ff @(posedge rd_clk) rd_data <= mem[rd_addr];

    // FIFO of bin numbers.
    logic [BINS_LOG2-1:0] fifo [0:DEPTH-1];
    logic [FIFO_LOG2:0]   wp, rp;
    wire fifo_empty = (wp == rp);
    wire fifo_full  = (wp - rp) == (FIFO_LOG2+1)'(DEPTH);

    wire [BINS_LOG2-1:0] bin = (smp_value >= 32'(BINS - 1)) ? BINS_LOG2'(BINS - 1)
                                                            : smp_value[BINS_LOG2-1:0];

    // Port A: clearing, or the update of one bin.
    localparam [2:0] U_IDLE = 3'd0, U_READ = 3'd1, U_HOLD = 3'd2, U_INC = 3'd3, U_WRITE = 3'd4,
                     U_CLEAR = 3'd5;
    logic [2:0]           ustate;
    logic [BINS_LOG2-1:0] a_addr;
    logic [31:0]          a_rdata, a_q, a_inc;
    logic                 a_we;
    logic [31:0]          a_wdata;

    // Port A does not read in a cycle with a write (NO_CHANGE). Thus port A
    // never reads and writes in the same cycle, and the rules for a READ_FIRST
    // port with an independent clock on the other port do not apply. A read on
    // port B of a bin that port A writes in the same moment can give a wrong
    // value; the register window reads the bins only when no run is in progress.
    always_ff @(posedge clk) begin
        if (a_we) mem[a_addr] <= a_wdata;
        else      a_rdata <= mem[a_addr];
        a_q <= a_rdata;     // the output register of the block RAM
        a_inc <= (a_q == '1) ? a_q : a_q + 1'b1;
    end

    assign a_we    = (ustate == U_WRITE) || (ustate == U_CLEAR);
    assign a_wdata = (ustate == U_CLEAR) ? 32'd0 : a_inc;
    assign busy = (ustate != U_IDLE) || !fifo_empty;

    always_ff @(posedge clk) begin
        drop <= 1'b0;
        if (rst) begin
            ustate <= U_IDLE;
            a_addr <= '0;
            wp     <= '0;
            rp     <= '0;
            min    <= '1;
            max    <= '0;
            sum    <= '0;
            drops  <= '0;
        end else if (clear && ustate == U_IDLE && fifo_empty) begin
            ustate <= U_CLEAR;
            a_addr <= '0;
            min    <= '1;
            max    <= '0;
            sum    <= '0;
            drops  <= '0;
        end else begin
            if (smp_valid) begin
                if (smp_value < min) min <= smp_value;
                if (smp_value > max) max <= smp_value;
                sum <= sum + 64'(smp_value);
                if (fifo_full) begin
                    drop <= 1'b1;
                    if (drops != '1) drops <= drops + 1'b1;
                end else begin
                    fifo[wp[FIFO_LOG2-1:0]] <= bin;
                    wp <= wp + 1'b1;
                end
            end
            case (ustate)
                U_IDLE:
                    if (!fifo_empty) begin
                        a_addr <= fifo[rp[FIFO_LOG2-1:0]];
                        rp     <= rp + 1'b1;
                        ustate <= U_READ;
                    end
                U_READ:  ustate <= U_HOLD;     // a_rdata is valid in U_HOLD
                U_HOLD:  ustate <= U_INC;      // a_q is valid in U_INC
                U_INC:   ustate <= U_WRITE;    // a_inc is valid in U_WRITE
                // The write uses a_addr of this cycle. The read of the next
                // bin is in the next cycle, after the write.
                U_WRITE:
                    if (!fifo_empty) begin
                        a_addr <= fifo[rp[FIFO_LOG2-1:0]];
                        rp     <= rp + 1'b1;
                        ustate <= U_READ;
                    end else ustate <= U_IDLE;
                U_CLEAR: begin
                    a_addr <= a_addr + 1'b1;
                    if (a_addr == '1) ustate <= U_IDLE;
                end
                default: ustate <= U_IDLE;
            endcase
        end
    end
endmodule
