// One latency histogram of the memory path test design, with its statistics.
//
// The samples go to BANKS banks (memprobe_hist_bank) in turn. A bank needs
// four cycles for a sample; with four banks the histogram takes one sample in
// each cycle. Bin n of the histogram is the sum of bin n of all banks; the read
// port adds them, and stops at 0xFFFFFFFF.
//
// Domain of clk (clk_m): samples and statistics. Domain of rd_clk (FCLK0): the
// read port, with rd_data three cycles of rd_clk after rd_addr.
//
// Bin n counts the samples with latency n; the last bin counts the samples
// with that latency or more. The statistics (minimum, maximum, sum) use all
// samples, also the samples that a full FIFO dropped.
module memprobe_hist #(
    parameter BINS_LOG2 = 10,
    parameter BANKS_LOG2 = 2
) (
    input  logic                 clk,
    input  logic                 rst,
    input  logic                 clear,     // pulse, only while busy is 0
    input  logic                 smp_valid,
    input  logic [31:0]          smp_value,
    output logic                 busy,
    output logic                 drop,      // a sample was dropped in this cycle
    output logic [31:0]          min,
    output logic [31:0]          max,
    output logic [63:0]          sum,
    output logic [31:0]          drops,

    input  logic                 rd_clk,
    input  logic [BINS_LOG2-1:0] rd_addr,
    output logic [31:0]          rd_data
);
    localparam BINS = 1 << BINS_LOG2;
    localparam BANKS = 1 << BANKS_LOG2;
    localparam BW = (BANKS_LOG2 > 0) ? BANKS_LOG2 : 1;   // width of the bank number
    localparam TW = 33 + BANKS_LOG2;                       // width of the sum of the banks

    wire [BINS_LOG2-1:0] bin = (smp_value >= 32'(BINS - 1)) ? BINS_LOG2'(BINS - 1)
                                                            : smp_value[BINS_LOG2-1:0];
    logic [BW-1:0]         next_bank;
    logic [BANKS-1:0]      bank_busy, bank_drop;
    logic [31:0]           bank_rd [0:BANKS-1];

    genvar g;
    generate for (g = 0; g < BANKS; g++) begin : bank
        memprobe_hist_bank #(.BINS_LOG2(BINS_LOG2)) b (
            .clk(clk), .rst(rst), .clear(clear),
            .smp_valid(smp_valid && (BANKS == 1 || next_bank == BW'(g))), .smp_bin(bin),
            .busy(bank_busy[g]), .drop(bank_drop[g]),
            .rd_clk(rd_clk), .rd_addr(rd_addr), .rd_data(bank_rd[g])
        );
    end endgenerate

    assign busy = |bank_busy;
    assign drop = |bank_drop;

    always_ff @(posedge clk) begin
        if (rst || (clear && !busy)) begin
            next_bank <= '0;
            min       <= '1;
            max       <= '0;
            sum       <= '0;
            drops     <= '0;
        end else begin
            if (smp_valid) begin
                next_bank <= (next_bank == BW'(BANKS - 1)) ? '0 : next_bank + 1'b1;
                if (smp_value < min) min <= smp_value;
                if (smp_value > max) max <= smp_value;
                sum <= sum + 64'(smp_value);
            end
            if (drop && drops != '1) drops <= drops + 1'b1;
        end
    end

    // The sum of the banks, in the domain of rd_clk.
    logic [TW-1:0] total;
    always_comb begin
        total = '0;
        for (int i = 0; i < BANKS; i++) total = total + TW'(bank_rd[i]);
    end
    always_ff @(posedge rd_clk)
        rd_data <= (total[TW-1:32] != '0) ? 32'hFFFF_FFFF : total[31:0];
endmodule
