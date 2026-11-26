// One latency histogram of the memory path test design, with its statistics.
//
// Domain of clk (clk_m): samples, the statistics and the update of the bins.
// Domain of rd_clk (FCLK0): one read port for the register window.
//
// A sample goes into a FIFO. The update of a bin takes two cycles: a read
// of the block RAM, then a write. If the FIFO is full, the sample is not put
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
    localparam [1:0] U_IDLE = 2'd0, U_READ = 2'd1, U_WRITE = 2'd2, U_CLEAR = 2'd3;
    logic [1:0]           ustate;
    logic [BINS_LOG2-1:0] a_addr;
    logic [31:0]          a_rdata;
    logic                 a_we;
    logic [31:0]          a_wdata;

    always_ff @(posedge clk) begin
        if (a_we) mem[a_addr] <= a_wdata;
        a_rdata <= mem[a_addr];
    end

    assign a_we    = (ustate == U_WRITE) || (ustate == U_CLEAR);
    assign a_wdata = (ustate == U_CLEAR) ? 32'd0
                   : (a_rdata == '1)     ? a_rdata
                   :                       a_rdata + 1'b1;
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
                U_READ:  ustate <= U_WRITE;    // a_rdata is valid in U_WRITE
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
            endcase
        end
    end
endmodule
