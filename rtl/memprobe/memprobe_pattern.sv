// The data pattern of spec/regmap.toml, note "Data pattern".
// For the 32-bit word at byte offset a of the HP memory: w = a >> 2, and
// the value is seed ^ w ^ (w << 11) ^ (w << 22), modulo 2^32.
// beat is the 64-bit beat at byte offset 8 * index: the word at that offset
// in bits 31:0 and the next word in bits 63:32.
module memprobe_pattern #(
    parameter OFF_W = 24
) (
    input  logic [31:0]      seed,
    input  logic [OFF_W-4:0] index,
    output logic [63:0]      beat
);
    logic [31:0] w0, w1;
    assign w0 = 32'({index, 1'b0});
    assign w1 = 32'({index, 1'b1});
    assign beat = {seed ^ w1 ^ (w1 << 11) ^ (w1 << 22),
                   seed ^ w0 ^ (w0 << 11) ^ (w0 << 22)};
endmodule
