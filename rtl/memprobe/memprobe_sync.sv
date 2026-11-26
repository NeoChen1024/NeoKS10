// Two-stage synchronizer for signals that cross into the domain of clk.
// Use it for a level, for a toggle, or for a Gray-coded counter. A bus
// through it must change one bit at a time, or be stable for more than
// two cycles of clk before the destination uses it.
//
// rst (synchronous, of the domain of clk) sets the stages to 0. A toggle
// synchronizer needs it: if the source domain resets its toggle to 0 while
// the stages keep an old 1, the destination sees a false toggle after a
// short reset.
module memprobe_sync #(
    parameter W = 1
) (
    input  logic         clk,
    input  logic         rst,
    input  logic [W-1:0] d,
    output logic [W-1:0] q
);
    (* ASYNC_REG = "TRUE" *) logic [W-1:0] s1, s2;
    always_ff @(posedge clk)
        if (rst) begin
            s1 <= '0;
            s2 <= '0;
        end else begin
            s1 <= d;
            s2 <= s1;
        end
    assign q = s2;
endmodule
