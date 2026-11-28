# CSL board: the console registers

The bits of the console registers of the M8616 (CSL) board, read from
the drawings of MP00540 (KS10 maintenance schematics, March 1981;
`third-party.lock`), PDF pages 11 to 23. Each statement is **[manual]**
MP00540 with the sheet and the element number, unless it says
"datasheet" or "derived". The model in `emu/ks10-csl/src/ks10.rs` uses
these tables; [`console-interface.md`](console-interface.md) gives the
model, and [`unknowns.md`](unknowns.md) the open items.

Read on 2026-11-28 from renders of the PDF at 220 to 600 dpi.

## Sheet revisions (title blocks)

| PDF page | Sheet | Title | Rev |
|---|---|---|---|
| 11 | M8616-0-CSL1 | CLK, KS10 ARB. | D |
| 12 | M8616-0-CSL2 | CONSOLE CONTROL | E |
| 13 | M8616-0-CSL3 | ADDRESS DECODE | D |
| 14 | M8616-0-CSL4 | CPU CONTROL | C |
| 15 | M8616-0-CSL5 | CPU CLK CONTROL | D |
| 16 | M8616-0-CSL6 | DATA PATH 1 | C |
| 17 | M8616-0-CSL7 | DATA PATH 2 | B |
| 18 | M8616-0-CSL8 | DATA PATH 3 | B |
| 19 | M8616-0-CSL9 | REM DIAG | D |
| 20 | M8616-0-CSLA | CPU CLK CONTROL (read multiplexers) | C |
| 21 | M8616-0-CSLB | MISC (connectors J1/J2, terminators) | C |
| 22 | M8616-0-CSLC | POWER AND GROUND | C |
| 23 | M8616-0-CSLD | BATTERY +5V DIPS | A |

## 0. Conventions found on the drawings

### 0.1 Data bus names, BUS vs BUF

- 8080 E24 D0..D7 -> 8228 E50 D0..D7 -> 8228 DB0..DB7 = `CSL CSL BUS 0..7 H` (CSL2).
  The PROM outputs 0M..7M and the 8251 data pins 0..7 connect to `CSL CSL BUS 0..7 H` in the same order.
  So `CSL CSL BUS k H` = 8080 data bit k, bit 0 = least significant, not inverted.
- `CSL6 CSL BUF k H` exists only for k = 0..5. It comes from the 8097 3-state buffer E130 (CSL6).
  E130 sections: BUS 4->BUF 4, BUS 1->BUF 1, BUS 2->BUF 2, BUS 5->BUF 5, BUS 0->BUF 0, BUS 3->BUF 3.
  Both enables are on GND (always on). The outputs have no bubble.
  So `BUF k` = `BUS k`, same polarity. That the 8097 is non-inverting is datasheet knowledge, not on the drawing.

### 0.2 Leading minus sign in a signal name

A name written `-X L` is the complement of `X L`. It is the same wire as `X H`.
Likewise `-X H` is the same wire as `X L`. Evidence on the drawings:

- CSL6 E156 pin 14 makes `CSL6 MAINT ENB L`; CSL1 E27 receives it as `-CSL6 MAINT ENB H`.
- CSL5 E54 makes `CSL5 DPE/M CLK ENABLE L`; CSLA E157 receives `-CSL5 DPE/M CLK ENABLE H`.
- CSL3 E143: `WRT 204 H = NOR(Y2 L, -CSL2 ADR 0 B L)` and `WRT 205 H = NOR(Y2 L, CSL6 ADR 0 L)`.
  Ports 204 and 205 differ only if `-CSL2 ADR 0 B L` carries the level of ADR 0 H.
- CSL2 8251 C/D pin 12 = `-CSL2 ADR 0 L` (= ADR 0 H). Port 200 (A0 = 0) is then data.
  CONDEF.M80 has `CTYDAT = 200`, `CTYCTL = 201`. This agrees.
- CSLA read select: `-CSL2 ADR 0 H` (= ADR 0 L). Port 301 then selects the I0 inputs.
  CONDEF.M80 bits NONXMEM = 100, ARBRESP = 20, DATACK = 1 of SMSTS = 301 match the I0 inputs (section 2.5).

### 0.3 Address signals

- `CSL2 ADR n H` = 8080 A_n (CSL2, E24).
- `CSL2 ADR 0 L`, `CSL2 ADR 1 L` from E71 (74LS04). `CSL2 ADR 0 B H` from E136 (CSL2).
- `CSL6 ADR 0 L` from E136 pin 12, input `CSL2 ADR 0 H` (CSL6).

## 1. Console register WRITE

### 1.1 Write decode (CSL3 rev D)

| Element | Inputs | Outputs |
|---|---|---|
| E169 74LS138 | A,B,C = ADR 1,2,3 H; G2A, G2B = `CSL2 CSL REG WRT L` (8228 I/O W); G1 = `CSL2 ADR 6 H` | Y0..Y7 = `CSL3 CSL REG WRT 100,102,104,106,110,112,114,116 L` |
| E108 74LS138 | A,B,C = ADR 1,2,3 H; G1 = `CSL2 ADR 7 H`; G2A (4), G2B (5) = lines from the E169 enable net (see UNCLEAR U1) | Y0 = WRT 200 L, Y1 = WRT 202 L, Y2 -> E143 (204/205), Y3 -> E71 -> WRT 206 H, Y4 -> E136 -> WRT 210 H, Y5 -> E136 -> WRT 212 H, Y6/Y7 unused |
| E143 7402 | NOR(WRT 100 L, -CSL2 ADR 0 B L) | `CSL3 CSL REG WRT 100 H` (A0 = 0 only) |
| E143 7402 | NOR(WRT 100 L, CSL6 ADR 0 L) | `CSL3 CSL REG WRT 101 H` (A0 = 1 only) |
| E143 7402 | NOR(E108 Y2, -CSL2 ADR 0 B L) | `CSL3 CSL REG WRT 204 H` (A0 = 0) |
| E143 7402 | NOR(E108 Y2, CSL6 ADR 0 L) | `CSL3 CSL REG WRT 205 H` (A0 = 1) |
| E136 74LS04 | WRT 116 L | WRT 116 H |

A0 is not decoded by E169/E108. So the 4x4 memory strobes (102..115) cover an even/odd pair.
Port 206, 210, 212, 116 also respond to the odd neighbour (207, 211, 213, 117).
A4 and A5 are not decoded.

### 1.2 Write 100 (A0 = 0) and the power-up reset

| 8080 bit | Element (sheet) | Signal | Polarity / note |
|---|---|---|---|
| 7 | E138 74LS175 D1, out Q-bar pin 6 (CSL3) | `CSL3 RESET L` | Stored inverted: bit 7 = 1 drives RESET L low (asserted). E138 CLR = `CSL5 PANEL RESET L` |
| 6 | E138 D0, Q pin 2 (CSL3) | `CSL3 PE DETECT H` | true |
| 5 | E138 D2, Q pin 10 (CSL3) | `CSL3 CRM DETECT H` | true |
| 4 | E138 D3, Q pin 15 (CSL3) | `CSL3 DP PE DETECT H` | true |
| 3 | E151 74LS74 (pins 12/11/9) (CSL3) | `CSL3 CACHE EN H` | true; PRE/CLR on +3VB (no reset) |
| 2 | E142 74LS74 (pins 2/3/5) (CSL4) | `CSL4 1 MSEC EN H` | true; PRE/CLR on +3VB |
| 1 | none found | - | see UNCLEAR U2 |
| 0 | none found | - | see UNCLEAR U2 |

Bits 2-6 use `CSL CSL BUS k H` (not BUF). Clock of all of them: `CSL3 CSL REG WRT 100 H`.
CONDEF.M80 `DEFLTE = 174` (cache, 1 ms, all parity) matches bits 6..2.

Reset chain (CSL5):
- E145 74LS74 (pins 1-6): D = GND, CLK = E159 74LS123 Q-bar (4.6 ms one-shot on `CSL5 AC LO H`),
  CLR = `CSL5 PANEL RESET L`, PRE = `CSL3 CSL REG WRT 100 L`. Q pin 5 = `CSL5 RESET (1) L`.
- So any write to port 100 (A0 ignored here: WRT 100 L is E169 Y0) presets E145 and ends `CSL5 RESET (1) L`.
  Panel reset or AC LO asserts it.
- E131 74S00 (as OR of lows): `CSL5 RESET H` = `RESET (1) L` asserted OR `CSL3 RESET L` asserted.
  E136 -> `CSL5 RESET L`; E122 -> `CSL5 RESETB L`.
- CSL9 E65: `CSL9 USART CLR H` = `CSL5 RESET (1) L` asserted OR `CSL9 CLR USART L` (pulled up, test socket only).
  Both 8251s are held in reset until the first write to port 100.

### 1.3 Write 101 (A0 = 1), E107 74LS175 (CSL3), CLK = WRT 101 H, CLR = +3VA

| 8080 bit | Path | Signal | Polarity |
|---|---|---|---|
| 0 (BUF 0) | E107 D2 -> Q pin 10 -> E104 75452 (both inputs) | `CSL3 FAULT\C\ L` | bit = 1 -> output low (asserted) |
| 1 (BUF 1) | E107 D0 -> Q pin 2 -> E104 75452 | `CSL3 REMOTE\C\ L` | bit = 1 -> low |
| 2 (BUF 2) | E107 D1 -> Q pin 7 -> E105 75452 | `CSL3 STATE\C\ L` | bit = 1 -> low |
| 3 (BUF 3) | E107 D3 -> Q-bar pin 14 | `CSL3 DTR L` | stored inverted; bit = 1 -> DTR L low (asserted) |
| 4-7 | none | - | not stored |

How 101 differs from 100: different decode (E143 with `CSL6 ADR 0 L`), different register (E107),
it uses `CSL6 CSL BUF 0..3` instead of `CSL CSL BUS 2..7`, it has no reset, and it does not touch the E145 reset flop
(that flop is preset by `WRT 100 L`, which is active for both 100 and 101: see UNCLEAR U3).
`CSL3 DTR L` goes to CSL9 as `-CSL3 DTR H` -> E182 1488 -> `CSL9 EIA REM DIAG DIR H` and E110 7437 -> `CSL9 TTL REM DIAG DIR H` (KLINIK line).
CONDEF.M80: `LIGHTS = 101`, `DTR = 101`, `STBIT = 4` (state light = bit 2) agree.

### 1.4 Writes 102-115: the 74LS170 4x4 memories and the 8646 bus transceivers

Wiring common to every 74LS170 (CSL6, CSL7, CSL8):
- WR ADR: pin 13 (drawn "0") = `-CSL2 ADR 0 H` or `-CSL6 ADR 0 H` (both = ADR 0 L level). Pin 14 (drawn "1") = GND.
- RD ADR: pin 4 (drawn "0") = `CSL4 DATA CYCLE H`. Pin 5 (drawn "1") = GND. RD EN pin 11 = GND (always on).
- WR EN pin 12 = the decoder strobe (WRT 102/104/106/110/112/114 L), active for the even port and the odd port.
- Effect: an EVEN port writes the word that is read while `CSL4 DATA CYCLE H` = 1.
  An ODD port writes the word that is read while DATA CYCLE = 0 (the COM/ADR cycle).
  The program agrees: CONDEF.M80 `W2835 = 102` ... `W0003 = 112` (data), `A2835 = 103` ... `A0003 = 113` (address).
- Pin 13 is the WB input of a 74LS170 by its datasheet (not on the drawing). Read and write both use the B bit. So the two locations are 0 and 2 by datasheet numbering.
- Which port writes which location: the ODD port (A0 = 1, select pin 13 low) writes the location that is read when `CSL4 DATA CYCLE H` = 0 (COM/ADR cycle). The EVEN port (A0 = 0, select pin 13 high) writes the location that is read when DATA CYCLE = 1 (data cycle).
- Outputs (open collector, pulled up 1 kOhm) go to 8646 `TRN Dn`. 8646 bus pins `BUS Dn` (bubbled) = `BUS DATA nn L` on the KS10 bus. 8646 `REC Dn` = `CSLx R DATA nn H`.
- 8646: TRN EN = `CSL4 T ENB L` / `CSL4 T ENB A L`; TRN CLK = `CSLA T CLK A/B H`; REC LCH = `CSL9 R CLK A/B L` (E68 uses `-CSL9 R CLK L`).

Bit mapping (8080 bit k -> KS10 bus bit, DEC numbering):

| Ports | 74LS170 (sheet) | 8646 | Mapping, from the traced wires |
|---|---|---|---|
| 102/103 | E158 (CSL6): BUS 0,1,2,3 | E139 | BUS0->35, BUS1->34, BUS2->33, BUS3->32 |
| 102/103 | E144 (CSL6): BUS 4,5,6,7 | E132 | BUS4->31, BUS5->30, BUS6->29, BUS7->28 |
| 104/105 | E115 (CSL6): BUS 0,1,2,3 | E111 | BUS0->27, BUS1->26, BUS2->25, BUS3->24 |
| 104/105 | E123 (CSL6): BUS 4,5,6,7 | E118 | BUS4->23, BUS5->22, BUS6->21, BUS7->20 |
| 106/107 | E116 (CSL7): BUS 0,1,2,3 | E97 | BUS0->19, BUS1->18, BUS2->17, BUS3->16 (nets `CSL7 TD16..TD19 H`) |
| 106/107 | E94 (CSL7): BUS 4,5,6,7 | E82 | BUS4->15, BUS5->14, BUS6->13, BUS7->12 |
| 110/111 | E87 (CSL7): BUS 0,1,2,3 | E83 | BUS0->11, BUS1->10, BUS2->09, BUS3->08 |
| 110/111 | E79 (CSL7): BUS 4,5,6,7 | E74 | BUS4->07, BUS5->06, BUS6->05, BUS7->04 |
| 112/113 | E69 (CSL8): BUF 0,1,2,3 | E66 | BUF0->03, BUF1->02, BUF2->01, BUF3->00 |
| 114/115 | E62 (CSL8): BUF 0,1,2,3 | E68 | see next table |

In short: port 102+2p, bit k -> KS10 bit 35-8p-k (p = 0..4). Port 112 has only bits 0-3. Bits 4-7 of 112/113 are not stored.
Bus side is asserted low (`BUS DATA nn L`); the 4x4 stores the true value (bit = 1 drives the line low).

Write 114/115, E62 74LS170 (CSL8), WR EN = `CSL3 CSL REG WRT 114 L`, -> E68 8646:

| 8080 bit | 8646 pin | KS10 bus signal | Receive net |
|---|---|---|---|
| 0 (BUF 0) | TRN D3 | `BUS DATA CYCLE L` | `CSL8 R DATA H` |
| 1 (BUF 1) | TRN D0 | `BUS I/O DATA CYCLE L` | `CSL8 R I/O DATA H` |
| 2 (BUF 2) | TRN D1 | `BUS COM/ADR CYCLE L` | `CSL8 R COM/ADR H` |
| 3 (BUF 3) | TRN D2 | `BUS BAD DATA CYCLE L` | `CSL8 R BAD DATA H` |
| 4-7 | - | not stored | - |

Port 114 (even) = word for the data cycle; port 115 (odd) = word for the COM/ADR cycle.
CONDEF.M80: `DTARB = 114`, `BUSARB = 115`.

Other effects of `CSL3 CSL REG WRT 114 L` (CSL3): it clears E145 (pin 13) -> `CSL3 NEXM H` = 0, and (through E100 74S08 with `CSL8 MEM BUSY L`; the gate label reads E100 or E108 - E108 is the 74LS138 on the same sheet, CSL4 has a 74S08 E100) clears E135 (pin 13), the NXM timer start flop.

Parity: not written by the 8080. 74S280 E133 (CSL8) makes `BUS PARITY RIGHT L` (via E146 TRN D2) from the 8646 OD-PAR outputs TPR1..TPR4 and TD18, TD19.
74S280 E67 makes `BUS PARITY LEFT L` (via E59 TRN D1) from TPL1..TPL4 and TD16, TD17.
Received parity: `CSL8 R PAR RIGHT H` (E146 REC D2), `CSL8 R PAR LEFT H` (E59 REC D1).
Other bus lines on CSL8: E167 drives `BUS AC LO L` from `CSL5 AC LO H` and `BUS RESET L` from `CSL5 RESET H` (TRN EN = GND, always driven); receives `CSL8 R AC LO H`, `CSL8 R RESET H`, `CSL8 R I/O BUSY H`. E59 receives `CSL8 R MEM BUSY H`.

### 1.5 Write 116, E171 half 1 (CSL4), CLK = `CSL3 CSL REG WRT 116 H`

| 8080 bit | Path | Signal |
|---|---|---|
| 0 (BUS 0) | E171 D pin 2 -> Q pin 5 -> E170 74S175 D3 (pin 13) -> R3 pin 15 -> E163 74S175 D (pin 5) -> Q-bar pin 6 | `CSL4 INT 10 L` (to the KS10, FT2) |

E171 CLR (pin 1) = `CSL4 INT 10 L`: the flop clears itself when INT 10 is asserted. So it is a pulse.
E170 and E163 clock on `CSL5 ENABLE (0) H`, clear on `CSL5 RESET L`. Bits 1-7 are not used.

### 1.6 Write 204 (A0 = 0), E156 74LS174 (CSL4), CLK = WRT 204 H, CLR = `CSL5 RESET L`

| 8080 bit | E156 | Signal |
|---|---|---|
| 0 (BUF 0) | D0 -> R0 pin 2 | `CSL4 CRA/M RESET H` |
| 1 (BUF 1) | D2 -> R2 pin 7 | `CSL4 STACK RESET H` |
| 2 (BUF 2) | D1 -> R1 pin 5 | `CSL4 DP RESET H` |
| 3 (BUF 3) | D5 -> R5 pin 15 | `CSL4 SS MODE H` |
| 4 (BUF 4) | D4 -> R4 pin 12 | `CSL4 CRAM ADR LOAD H` |
| 5 (BUF 5) | D3 -> R3 pin 10 | `CSL4 CRAM WRITE H` |
| 6, 7 | - | not stored (no BUF 6/7 exists) |

All true (Q outputs).

### 1.7 Write 205 (A0 = 1)

| 8080 bit | Element (sheet) | Signal | Polarity / reset |
|---|---|---|---|
| 0 (BUF 0) | E155 74LS174 D1 -> R1 pin 5 (CSL4) | `CSL4 DIAG 1 H` | true; CLR `CSL5 RESET L` |
| 1 (BUF 1) | E155 D0 -> R0 pin 2 | `CSL4 DIAG 2 H` | true |
| 2 (BUF 2) | E155 D3 -> R3 pin 10 | `CSL4 DIAG 4 H` | true |
| 3 (BUF 3) | E155 D4 -> R4 pin 12 | `CSL4 DIAG 8 H` | true |
| 4 (BUF 4) | E155 D2 -> R2 pin 7 | `CSL4 TRAP EN H` | true |
| 5 (BUF 5) | E155 D5 -> R5 pin 15 | no connection | - |
| 5 (BUS 5) | E161 74LS74 (pins 12/11/9) (CSL4) | `CSL4 10 INT H` | true; PRE (pin 10) = `DPMB CSL INTERRUPT L` (the KS10 sets it); CLR = +3VB |
| 6 (BUS 6) | E156 74LS175 D3 -> Q-bar pin 14 (CSL6) | `CSL6 MAINT ENB L` | stored inverted: bit 6 = 1 -> MAINT ENB L low; CLR `CSL5 RESET L` |
| 7 (BUS 7) | E156 74LS175 D0 -> Q pin 2 (CSL6) | `CSL6 CLK 0 H` | true |

Bit 5 has two flops: E155 R5 (unused output) and E161. Only E161 has an output.
`CSL4 10 INT H` is read back on port 301 bit 7 (section 2.5). It is not wired to the 8080 INT pin.
CONDEF.M80 `TRPDEF = 20` (bit 4) agrees.
MAINT ENB and CLK 0 select the master clock source on CSL1 (section 4).

### 1.8 Write 206 (CSL5), CLK = `CSL3 CSL REG WRT 206 H`

| 8080 bit | Path | Signal |
|---|---|---|
| 0 (BUF 0) | "E48" or "E40" 74LS74 (pins 2/3/5, glyph unclear, U8, CLR `CSL5 RESETB L`) -> E47 74S74 D (CLK `CSL9 R CLK H`, CLR `CSL5 RESETB L`) Q pin 9 | `CSL5 CLK RUN H` (level) |
| 1 (BUF 1) | E55 74LS74 (pins 2/3/5, CLR `CSL5 ENABLE+RESET L`) -> E48 74S175 (CLK `CSL1 T CLK H`) -> E49 74S175 -> Q pin 7 | `CSL5 SINGLE CLK (1) H` |
| 2-7 | - | not stored |

E55 is cleared by `CSL5 ENABLE+RESET L`, so bit 1 requests one clock enable. (Interpretation of the pulse length: see UNCLEAR U6.)

### 1.9 Write 210 (CSL4 and CSL3), CLK = `CSL3 CSL REG WRT 210 H`

| 8080 bit | Element (sheet) | Signal | Note |
|---|---|---|---|
| 0 (BUF 0) | E121 74LS174 D4 -> R4 pin 12 (CSL4) | `CSL4 CLOSE LATCHS H` | CLR `CSL5 RESETB L` |
| 1 (BUF 1) | E121 D3 -> R3 pin 10 | `CSL4 LATCH DATA (1) H` | |
| 2 (BUF 2) | E121 D2 -> R2 pin 7 | `CSL4 CRA R CLK ENB (1) H` | |
| 3 (BUF 3) | E121 D5 -> R5 pin 15 | `CSL4 CRA T CLK ENB (1) H` | |
| 4 (BUF 4) | E121 D0 -> R0 pin 2 | `CSL4 XMIT DATA (1) H` | |
| 5 (BUF 5) | E121 D1 -> R1 pin 5 | `CSL4 XMIT ADR (1) H` | |
| 6 (BUS 6) | E129 74LS74 (pins 2/3/5) (CSL4) | `CSL4 BUS REQ H` | PRE +3VB; CLR = E114 (`CSL5 RESETB L` or `CSL4 GRANT (1) L`) |
| 7 (BUS 7) | E151 74LS74 (pins 2/3/5) (CSL3) | `CSL3 MEM H` | CLR `CSL5 RESET L` |

Also clocked by WRT 210 H: E92 74S74 (pins 2/3/5/6, CSL4), D = +3VA, PRE from E98 NAND(`GRANT (1) H`, `LATCH DATA (1) H`),
outputs `CSL4 LATCH DATA ENB (1) H/L`.
`CSL3 MEM H` feeds E135 (CSL3) and E86 (CSL4, RETURN DATA) - it marks a memory (not I/O) cycle for the NXM timer.

### 1.10 Write 212 (CSL4), CLK = `CSL3 CSL REG WRT 212 H`

| 8080 bit | Path | Signal | Cleared by |
|---|---|---|---|
| 0 (BUS 0) | E171 half 2 (pins 12/11/9) -> E163 74S175 D pin 4 -> Q pin 2 | `CSL4 CONTINUE H` (E125 -> `CSL4 CONTINUE L`) | E171 CLR pin 13 = E168 NAND(`CSL4 CPM CMD LD H`, `CRM2 # 12B H`) |
| 1 (BUF 1) | E149 74LS74 (pins 12/11/9) -> E170 74S175 D1 -> R1 pins 7/6 | `CSL4 EXECUTE B H/L`; E168 -> `CSL4 EXECUTE H`; E125 -> `CSL4 EXECUTE L` | E149 CLR pin 13 = E168 NAND(CPM CMD LD, `CRM2 # 13B H`) |
| 2 (BUF 2) | E135 74LS74 (pins 2/3/5) -> E170 D2 -> R2 Q pin 10 = `CSL4 RUN (1) H`; Q-bar pin 11 -> E125 -> `CSL4 RUN H` | RUN | E135 CLR pin 1 = E98 NAND(CPM CMD LD, `CRM2 # 14B H`) |

E170 and E163: CLK `CSL5 ENABLE (0) H`, CLR `CSL5 RESET L`. `CSL4 CPM CMD LD H` = E114 AND(`CSL5 CRAM CLK H`, `CSL4 SPEC H`); `CSL4 SPEC H` = E125 inverse of `CRA2 SPEC/CONSOLE L`.
Also: E161 half 1 `CSL4 HALT LOOP (1) H` (D = GND, CLK `CSL5 RESET H`, set by `CRM2 # 15B` & CPM CMD LD, cleared by `CRM2 # 16B` & CPM CMD LD). It is read on port 300 bit 3.

### 1.11 Write 200-203: the USARTs (CSL3, CSL2, CSL9)

- E70 74S11 = AND(WRT 200 L, WRT 202 L, RD 200 L): low if any of them.
- E63: `CSL3 USART 1 EN L` = OR(E70, `-CSL2 ADR 1 L`) -> selected for A1 = 0 (200/201): CTY 8251 E42 (CSL2).
- E63: `CSL3 USART 2 EN L` = OR(E70, `CSL2 ADR 1 L`) -> A1 = 1 (202/203): KLINIK 8251 E6 (CSL9).
- Both 8251s: C/D pin 12 = `-CSL2 ADR 0 L` (= ADR 0 H). Even port = data, odd port = control/status.

## 2. Console register READ

### 2.1 Read decode (CSL3)

E93 74LS139, A = ADR 6 H, B = ADR 7 H, G = `CSL2 CSL REG RD L`:

| Output | Ports | Signal |
|---|---|---|
| Y0 | 000-077 | `CSL3 CSL REG RD 0 L` |
| Y1 | 100-177 | `CSL3 CSL REG RD 100 L` |
| Y2 | 200-277 | `CSL3 CSL REG RD 200 L` (to the USART enables) |
| Y3 + E101 OR with `-CSL2 ADR 1 L` (= A1 H) | 3x0, 3x1 (A1 = 0) | `CSL3 CSL REG RD 300 L` |
| Y3 + E101 OR with `CSL2 ADR 1 L` | 3x2, 3x3 (A1 = 1) | `CSL3 CSL REG RD 302 L` |

A2..A5 are not decoded for reads.

### 2.2 Read multiplexers (CSLA rev C): polarity and select

- 0-3 and 100-103 use 74LS353 (dual 4-to-1, outputs drawn with bubbles); 300-303 use 74LS258 (quad 2-to-1, outputs drawn with bubbles).
- Output nets are named `-CSL CSL BUS k L` (= `CSL CSL BUS k H`).
- So the 8080 reads the COMPLEMENT of the electrical level of the named input.
  "bit = NOT(input)" in the tables below. (Inverting outputs of LS353/LS258 are datasheet knowledge; the drawing shows the bubbles.)
- 74LS353 select: pin 2 (drawn "2") = `-CSL2 ADR 1 H` (= A1 L), pin 14 (drawn "1") = `-CSL2 ADR 0 H` or `-CSL6 ADR 0 H` (= A0 L).
  The selected input number is 3 - (port & 3): port x0 -> input 3, x1 -> input 2, x2 -> input 1, x3 -> input 0.
- 74LS258 select pin 1 = `-CSL2 ADR 0 H` / `-CSL6 ADR 0 H` (= A0 L): even port -> inputs "1-Dx", odd port -> inputs "0-Dx".
  The pin numbers on the drawing match the datasheet (I0a = 2, I1a = 3, ... S = 1, OE = 15).

### 2.3 Reads 0-3: KS10 data from the 8646 receive latches (E119, E112, E148, E113, E127, E120, E126, all EN = RD 0 L)

| Port | b7 | b6 | b5 | b4 | b3 | b2 | b1 | b0 |
|---|---|---|---|---|---|---|---|---|
| 0 | 28 | 29 | 30 | 31 | 32 | 33 | 34 | 35 |
| 1 | 20 | 21 | 22 | 23 | 24 | 25 | 26 | 27 |
| 2 | 12 | 13 | 14 | 15 | 16 | 17 | 18 | 19 |
| 3 | 04 | 05 | 06 | 07 | 08 | 09 | 10 | 11 |

Each entry is `NOT CSLx R DATA nn H` (the 8080 gets the complement).
Elements: b7 E119 YB, b6 E148 YB, b5 E112 YB, b4 E119 YA, b3 E113 YB, b2 E126 YA, b1 E127 YB, b0 E120 YB.
`R DATA nn H` is true data (KS10 bus `BUS DATA nn L` low -> R DATA H high). CONDEF.M80 `D2835 = 0`, `D2027 = 1`, `D1219 = 2`, `D0411 = 3` agree.

### 2.4 Reads 100-103 (EN = RD 100 L). Each bit is NOT(the named signal level).

| Bit | Element | 100 (input 3) | 101 (input 2) | 102 (input 1) | 103 (input 0) |
|---|---|---|---|---|---|
| 7 | E154 YB | `CSL3 RECIEVE PE H` | `CSLA PI REQ 1 H` | `CSL8 R AC LO H` | `-UBA5 ADPT1 PARITY ERR H` |
| 6 | E154 YA | `-UBA5 ADPT3 PARITY ERR H` | `CSLA PI REQ 2 H` | `CSL8 R RESET H` | `-UBA5 ADPT4 PARITY ERR H` |
| 5 | E112 YA | `-(NC) CSLA ED1\#400\ H` (no connection) | `CSLA PI REQ 3 H` | `CSL8 R MEM BUSY H` | `CSL8 R PAR RIGHT H` |
| 4 | E148 YA | `CSL3 CRAM PE H` | `CSLA PI REQ 4 H` | `CSL8 R I/O BUSY H` | `CSL8 R PAR LEFT H` |
| 3 | E113 YA | `-MMC7 MEM PARITY ERR H` | `CSLA PI REQ 5 H` | `CSL8 R BAD DATA H` | `CSL8 R DATA 00 H` |
| 2 | E120 YA | `CSL3 DP PE H` | `CSLA PI REQ 6 H` | `CSL8 R COM/ADR H` | `CSL8 R DATA 01 H` |
| 1 | E127 YA | `CSL3 CRA PE H` | `CSLA PI REQ 7 H` | `CSL8 R I/O DATA H` | `CSL8 R DATA 02 H` |
| 0 | E126 YB | `-UBA5 ADPT2 PARITY ERR H` | `MMC9 REF ERR B H` | `CSL8 R DATA H` | `CSL8 R DATA 03 H` |

Notes:
- 103 bits 0-3 = `NOT R DATA 03..00` (KS10 bits 0-3, bit k -> KS10 bit 3-k). CONDEF.M80 `D0003 = 103`.
- `CSLA PI REQ n H` = E160/E125 74LS04 inverse of `BUS PI REQ n L`.
- `CSL3 xxx PE H` are the 25LS08 parity-error registers E141 (CRA PE, CRAM PE, DP PE, RECIEVE PE) on CSL3, clocked by `CSLA T CLK D H`, enabled by `CSL3 PE (0) L`.
- CONDEF.M80: `REFERR = 101`, `R.BUSY = 102`, `R.RESET = 102` agree.

### 2.5 Reads 300-303 (74LS258). Each bit is NOT(the named signal level).

| Bit | Element | 300 (I1, EN RD 300 L) | 301 (I0, EN RD 300 L) |
|---|---|---|---|
| 7 | E166 YC | `CSLA STOP # H` = switch E173 #3 (CTY stop bits) | `CSL4 10 INT H` |
| 6 | E166 YA | `CSLA CHAR LENGTH 1 H` = switch E173 #2 (CTY) | `CSL3 NEXM H` |
| 5 | E166 YB | `CSLA REM DIAG STOP BIT # H` = switch E173 #7 (KLINIK) | `(DL1) SPARE \#400\ H` (pulled up by E165) |
| 4 | E166 YD | `CSLA REM DIAG CHAR LENTH H` = switch E173 #4 (KLINIK) | `CSL4 BUS REQ H` |
| 3 | E164 YB | `CSL4 HALT LOOP (1) H` | `CSL3 PE (1) H` |
| 2 | E164 YC | `CSL4 RUN (1) H` | `PANEL CONSOLE ENABLE \#400\ H` (DE1, pulled up by E165) |
| 1 | E164 YA | `CSL4 EXECUTE B H` | E117 7414 inverse of `PANEL BOOT \#400\ L` (DM1, pulled up by E137, 10 nF) |
| 0 | E164 YD | `CSL4 CONTINUE H` | `CSL4 DATA ACK H` |

| Bit | Element | 302 (I1, EN RD 302 L) | 303 (I0, EN RD 302 L) |
|---|---|---|---|
| 3 | E157 YB | `REMOTE DIAG PROT\#400\ H` (FR1, pulled up by E165) | `CSL4 R CLK ENB (0) H` |
| 2 | E157 YD | `REMOTE DIAG ENB\#400\ H` (FS1, pulled up by E148) | `CSLA CRAM CLK ENB H` (E71 inverse of `CSL5 CRA/M CLK ENABLE L`) |
| 1 | E157 YC | `CSL9 TERMINAL CARRIER H` (CTY) | `-CSL5 DPE/M CLK ENABLE H` |
| 0 | E157 YA | `CSL9 CARRIER H` (KLINIK) | `-CSL3 RAM ERROR H` |
| 4-7 | - | not driven by any element on CSL1-CSLD | see UNCLEAR U4 |

Switch polarity: each E173 switch connects its net to GND when ON; the net has a 1 kOhm pull-up (E165).
ON -> net low -> 8080 bit = 1 before the program's CMA. The CMA after IN 300/301 turns this into "ON = 0".
Switch roles from the CSL2/CSL9 tables: E173 #2/#3 are the CTY rows of the baud table, #4/#7 the KLINIK rows.
At 110 baud all four are OFF; at the other rates #3 (CTY) and #7 (KLINIK) are ON, #2 and #4 are OFF.
CONDEF.M80: `RUNFP = 300`, `TTYSW = 300`, `SMSTS = 301`, `BOOTSW = 301`, `CARRIER = 302`, `NONXMEM = 100` (bit 6 of 301), `ARBRESP = 20` (bit 4), `DATACK = 1` (bit 0) agree with this table.
`CSL4 DATA ACK H` (E129 half 2, CSL4) is cleared by `CSL3 CSL REG RD 0 L` (pin 13), so a read of port 0..77 clears DATA ACK.

### 2.6 Read 200-203

8251 data/status (see 1.11). No other source drives the bus for these ports.

## 3. 8080 interrupt, INTA, 8251 wiring, baud rate

### 3.1 INT and related 8080 pins (CSL2 E24)

| 8080 pin | Net | Source |
|---|---|---|
| INT (14) | E23 74LS32 pin 3 | OR(`CSL2 R RDY H`, E23 pin 11) |
| | E23 pin 11 | OR(`CSL9 TEST INT H`, `CSL9 R RDY H`) |
| | `CSL2 R RDY H` | CTY 8251 E42 RxRDY (pin 14) |
| | `CSL9 R RDY H` | KLINIK 8251 E6 RxRDY (pin 14) |
| | `CSL9 TEST INT H` | E28 74S04 inverse of `CSL9 TEST INT L`, which has a 1 kOhm pull-up (E5) and goes only to test socket XE17 pin 4. Normally low |
| INTE (16) out | `CSL2 INT EN H` | goes only to XE17 pin 1 |
| HOLD (13) | `CSL9 8 HOLD H` | E71 inverse of `CSL9 8 HOLD L` (pull-up, XE17 pin 2). Normally low |
| WAIT (24) out | `CSL2 8 WAIT H` | XE17 pin 9 only |
| READY (23) | 8224 READY | 8224 RDYIN = `CSL2 RDY IN H` (pull-up, XE17 pin 3) |
| RESET (12) | 8224 RESET | 8224 RESIN = `CSL5 PANEL RESET L` |

Nothing else drives INT: no TxRDY, no `CSL4 10 INT H`, no timer.
XE17 note on CSL9: "XE17 IS NOT USED, SOCKET IS FOR GR TEST".

### 3.2 Interrupt acknowledge

8228 E50 (CSL2): INTA pin 23 goes to +12.0 V through R32 1 kOhm. BUSEN pin 22 goes to a short stub (U5).
By the Intel 8228 data sheet (not on the drawing), INTA tied to +12 V through 1 kOhm makes the 8228 put RST 7 (377 octal) on the data bus during interrupt acknowledge.
No other driver on the drawings answers INTA. So the instruction is RST 7 [manual: 8228 datasheet behaviour; wiring from MP00540 CSL2].

### 3.3 8251 wiring

| Pin | CTY E42 (CSL2) | KLINIK E6 (CSL9) |
|---|---|---|
| D0-D7 | `CSL CSL BUS 0-7 H` | same |
| C/D (12) | `-CSL2 ADR 0 L` (= A0 H) | same |
| CS (11) | `CSL3 USART 1 EN L` | `CSL3 USART 2 EN L` |
| RD (13) / WR (10) | `CSL2 CSL REG RD L` / `CSL2 CSL REG WRT L` | same |
| RESET (21) | `CSL9 USART CLR H` | same |
| CLK (20) | 34702 E4 S2 (pin 1) | 34702 E11 S2 (pin 1) |
| TxC (9), RxC (25) | both to 34702 E4 Y (pin 10; the CSL2 print looks like "18", but the part has 16 pins and CSL9 reads 10) | both to 34702 E11 Y (pin 10) |
| CTS (17) | GND | GND |
| DSR (22) | no connection | no connection |
| DTR (24), RTS (23) | no connection | no connection |
| TxRDY (15), TxEMPTY (18), SYNDET (16) | no connection | no connection |
| RxRDY (14) | `CSL2 R RDY H` -> INT | `CSL9 R RDY H` -> INT |
| RxD (3) | `CSL9 TERMINAL SI H` (E124 1489L from `CSL8 TERMINAL REC DATA L`) | `CSL9 REM DIAG SI H` (E117 7414 from `CSL8 REM DIAG DATA B L`) |
| TxD (19) | `CSL2 SO H` -> E182 1488 -> `CSL9 TERMINAL XMIT DATA L` | `CSL9 SO H` -> E22 -> E182 1488 (EIA) and E110 7437 (TTL) REM DIAG XMT DATA |

Line outputs on CSL9: `CSL9 TERMINAL RTS H` and `CSL9 TERMINAL DTR H` are 1488 gates with inputs on GND (always asserted).
`CSL9 REM DIAG RTS H` same. KLINIK DTR comes from port 101 bit 3 (1.3).
Carrier inputs: `CSL8 TERMINAL CARRIER L` -> E124 -> `CSL9 TERMINAL CARRIER H`;
`CSL8 EIA REM DIAG CARRIER L` (E124) OR `CSL8 TTL REM DIAG CARRIER L` (E131) -> `CSL9 CARRIER H`. Both read on port 302.

### 3.4 Clocks of the 8080 and the 8251s

- Y2 14.746 MHz crystal on 8224 E35 XTAL1/XTAL2 (C38 10 pF). 8224 PH1/PH2 -> 8080. 8224 PH2 TTL (pin 6) = `CSL2 PH2 H`.
- 8224 OSC (pin 12) -> E72 74LS161 CLK.
  E72: D8 = GND, D2 = GND, D4 and D1 = `CSL2 2.45 MHZ H` (its own QD).
  LOAD = E65 NAND(QD pin 11, QB pin 13). CLR, ENP, ENT on +3VA.
  This counts 5,6,7,8,9,10 and reloads 5: divide by 6. QD = `CSL2 2.45 MHZ H` = 14.7456 MHz / 6 = 2.4576 MHz [derived from the drawing].
- 8080 clock: by the 8224 data sheet, PH1/PH2 = crystal / 9 = 1.6384 MHz (610 ns state) [datasheet, not on the drawing].
- 34702 E4 (CTY) and E11 (KLINIK): CLK IN (5) = `CSL2 2.45 MHZ H`, CLK EN (4) = GND, XTAL pins open.

### 3.5 Baud rate

The rate goes straight from DIP switches to the 34702 rate inputs. The 8080 cannot read or set it.

- CTY E4 (CSL2): R0 (11) = E10 #6, R1 (12) = E10 #5, R2 (13) = E10 #4, R3 (14) = E10 #3, each with a 1 kOhm pull-up (E5); switch ON = GND.
- KLINIK E11 (CSL9): R0..R3 from E10 #8, #7, #2, #1 (individual wire order not traced: U7).

CTY table (CSL2, "CONSOLE TERMINAL"):

| Baud | E10 #3 | #4 | #5 | #6 | E173 #2 | E173 #3 |
|---|---|---|---|---|---|---|
| 110 | OFF | OFF | OFF | OFF | OFF | OFF |
| 150 | ON | OFF | OFF | OFF | OFF | ON |
| 300 (* LA36) | OFF | ON | OFF | OFF | OFF | ON |
| 600 | OFF | OFF | OFF | ON | OFF | ON |
| 1200 (*** LA120) | OFF | OFF | ON | OFF | OFF | ON |
| 1800 | ON | OFF | ON | OFF | OFF | ON |
| 2400 | ON | ON | OFF | OFF | OFF | ON |
| 4800 | OFF | ON | ON | OFF | OFF | ON |
| 9600 | ON | ON | ON | OFF | OFF | ON |

KLINIK table (CSL9, "REMOTE DIAGNOSIS"; * = normal factory setting at 300):

| Baud | E10 #1 | #2 | #7 | #8 | E173 #4 | E173 #7 |
|---|---|---|---|---|---|---|
| 110 | OFF | OFF | OFF | OFF | OFF | OFF |
| 150 | ON | OFF | OFF | OFF | OFF | ON |
| 300 * | OFF | ON | OFF | OFF | OFF | ON |
| 600 | OFF | OFF | OFF | ON | OFF | ON |
| 1200 | OFF | OFF | ON | OFF | OFF | ON |
| 1800 | ON | OFF | ON | OFF | OFF | ON |
| 2400 | ON | ON | OFF | OFF | OFF | ON |
| 4800 | OFF | ON | ON | OFF | OFF | ON |
| 9600 | ON | ON | ON | OFF | OFF | ON |

E173 #2/#3 (CTY) and #4/#7 (KLINIK) are the switches read on port 300 bits 6/7 and 4/5 (2.5). The 8080 uses them for the 8251 mode (character length, stop bits).

## 4. KS10 clock (CSL1, CSL5, CSLA)

- CSL1 T1: "26.666 MHz CRYSTAL OSC". Its output feeds E27 74LS153 input 3-DA.
- E27 (one section), SEL pin 2 = `-CSL6 MAINT ENB H` (pull-up R19), SEL pin 14 = `-CSL1 MARGIN SEL\#400\ H` (pull-up E89):
  - 0-DA and 1-DA (pins 10, 11 tied) = `CSL6 CLK 0 H` (port 205 bit 7).
  - 2-DA = `CSL1 MARGIN CLK\#400\ H` (backplane margin clock).
  - 3-DA = the 26.666 MHz oscillator.
  - Normal: both selects high -> oscillator. Port 205 bit 6 = 1 -> MAINT ENB L low -> select 0/1 -> the 8080 clocks the KS10 by writing 205 bit 7.
- E27 YA -> CLK of both halves of E14 74S74. Half 2 (pins 12/11/9/8): D = `CSL1 MASTER R CLK (0) H` (half 1 pin 5), Q pin 9 = `CSL1 MASTER T CLK (1) H`.
  Half 1 (pins 2/3/5): D = half 2 Q-bar (pin 8), pin 5 = `CSL1 MASTER R CLK (1) L`.
  This is a 2-stage Johnson ring: divide by 4. MASTER T and MASTER R clocks have a period of 4 x 37.5 ns = 150 ns (6.67 MHz) and are 37.5 ns apart [derived from the drawing].
- T and R clocks to the boards pass through PDLT delay lines E38, E31, E9, E26, E2 (drawn "15-30 NS", factory-set taps, "NOTE 1: FACTORY ADJUSTMENT ONLY!") and 74S37 drivers (`CSL1 T CLK CRA/CRM/DPE/DPM/...`, `CSL1 R CLK ...`).
- CSL5: the microcycle is gated, not fixed. E29 74S74 (CLK `CSL1 R CLK H`) makes `CSL5 ENABLE (1) H/L` from E34 74S64 (AND-OR-INVERT) with terms: `SINGLE CLK (1)`; `FS (1)`; `PAGE FAIL START (1)`; and AND of E70(`CLK RUN H`, `ENABLE (0) H`, `CSL3 PE (0) H`), `-CSL5 WRT DLY H`, `-CSL5 READ DLY H`, `CSL5 T COUNT DONE (1) H`.
  `T COUNT DONE` comes from E15 74LS153 selected by the CRAM T field (`-CRA6 T00 L`, `-CRA6 T01 L` via E28/E44) and the E16 74LS74 "+2" counter clocked by `CSL1 R CLK H`.
  So a microinstruction lasts a number of 150 ns master periods set by its T field. (The exact count per T value: U6.)
- The 8080 crystal (14.746 MHz, CSL2) and the KS10 oscillator (26.666 MHz, CSL1) are separate. No drawing shows a phase or frequency relation between them.

## 5. 8080 memory decode (CSL2 rev E)

| Item | Wiring |
|---|---|
| PROM select | E36 74LS138: A/B/C = A11/A12/A13; G1 (6) = `CSL2 8 ENB H`; G2A (4) = `CSL2 PROM RD L` (8228 MEMR, pin 24); G2B (5) = GND |
| PROMs (2716) | Y0 `CSL2 2K L` -> E81 OE (pin 20) = 000000-003777; Y1 `4K L` -> E73 = 004000-007777; Y2 `6K L` -> E58 = 010000-013777; Y3 `8K L` -> E18 = 014000-017777 (octal). CE (pin 18) = GND. A0-A10 to all four. Y4-Y7 (A13 = 1) unused |
| RAM select | E65 74LS00 NAND(`CSL2 8 ENB H` (pull-up E5), `CSL2 ADR 13 H`) -> CHIP EN (pin 8) of both 2114 (E88: data bits 0-3, E80: bits 4-7). Drawn speed "450ns" |
| RAM address | A0-A9 only (2114 address pins in a permuted order, which does not matter) |
| RAM write | WR EN (pin 10) = `CSL2 RAM WRT L` (8228 MEMW, pin 26) |
| A14, A15 | 8080 pins 39 and 36: no wire drawn |
| `CSL2 8 ENB H` | 1 kOhm pull-up, otherwise only test socket XE17 pin 5. Normally high |

Consequences [derived from the drawing]:
- ROM: 8 KiB at 000000-017777. It repeats wherever A13 = 0, because A14/A15 are not decoded: 040000-057777, 100000-117777, 140000-157777.
- RAM: 1 KiB at 020000-021777. It repeats on every address with A13 = 1: 8 copies in 020000-037777, and again in 060000-077777, 120000-137777, 160000-177777.
- The RAM chip enable is not qualified by MEMR or by a memory cycle. It is on whenever A13 = 1 and the 2114 drives the bus when its WE is high.
  During IN/OUT the port number is on A8-A15 and A0-A7. Ports with bit 5 set (040-077, 140-177, 240-277, 340-377) would therefore also enable the RAM. The program does not use such ports.
- CONDEF.M80: `RAMST = 8192.` (020000 octal) agrees.

## 6. Check of the read polarity against the 8080 program

Source: `3rdparty/its/src/kshack/*.m80` (rank 1 [its]). Each line: port, what the program does, and the drawing result.

| Port (symbol) | Program (file:line) | Drawing (section 2) | Agrees |
|---|---|---|---|
| 0-3, 103 (D2835..D0003) | `IN`, then `CMA` (subrtn.m80:894-912; cmds.m80:735-741); 103 then `ANI 17` | NOT R DATA; 103 bits 4-7 are not data | yes |
| 102 (R.BUSY) | `CMA`, `ANI 60` = I/O BUSY, MEM BUSY (subrtn.m80:35) | 102 bit 4 R I/O BUSY, bit 5 R MEM BUSY, inverted | yes |
| 101 (REFERR) | `CMA`, `ANI 1` (subrtn.m80:39); hcore.m80:497 "TRUE .LO." | 101 bit 0 NOT MMC9 REF ERR B | yes |
| 100 (SMPAR) | `CMA` "FIX THE HARDWARE INVERSION" (subrtn.m80:53) | 100 inverted | yes |
| 300 (TTYSW/RUNFP) | `CMA` (hcore.m80:131); bit 3 HALT LOOP "TRUE .LO." (hcore.m80:478); bit 2 RUN "TRUE LO" (cmds.m80:624); bit 0 CONTINUE after `CMA` (cmds.m80:1148) | bits 3/2/0 = NOT HALT LOOP/RUN/CONTINUE | yes |
| 301 (SMSTS/BOOTSW) | `CMA` (subrtn.m80:719); bit 1 BOOT "TRUE .LO." (hcore.m80:367); bit 3 parity error "TRUE .LO." (hcore.m80:464); bit 2 console enable "LO if enabled" (hcore.m80:1310); bit 7 minus = 10 interrupt pending (hcore.m80:578) | bit 1 NOT(inverted BOOT L), bit 3 NOT PE(1), bit 2 NOT CONSOLE ENABLE, bit 7 NOT 10 INT | yes |
| 302 (CARRIER) | `ANI 1`; high = carrier false -> HANGUP (hcore.m80:449) | bit 0 NOT CSL9 CARRIER H | yes |

No read in this list contradicts the drawing. The CONDEF.M80 port symbols (section 0.2, 2.3-2.5) also agree.

## Items that the drawings do not settle

- U1 (mostly resolved at 600 dpi): E108 (CSL3) pins 4 and 5 (G2A, G2B) are two lines that continue the E169 pin 4/5 net, which is `CSL2 CSL REG WRT L`. No line from `CSL2 ADR 6 H` reaches E108. E169 has no A7 term and E108 has no A6 term. So a write to 3x0-3x7 strobes both a 1xx register and a 2xx register. The program does not write 3xx.
- U2: port 100 bits 0 and 1: no element on CSL1-CSLD uses them in the trace of the drawings (E138, E151, E142, E145 only).
- U3: `CSL3 CSL REG WRT 100 L` (E169 Y0) also responds to port 101. The E145 PRE on CSL5 uses this unqualified signal. So a write to 101 likely also ends the power-up reset. This follows from the wiring; it is not stated on the drawing.
- U4: port 302/303 bits 4-7: no driver found. Their value depends on the 8228/8080 input with a floating bus.
- U5: 8228 E50 BUS EN pin 22: drawn to a short line with a tick mark; its source is not clear (probably GND).
- U6: the exact pulse width of `SINGLE CLK` (206 bit 1) and the T-field-to-count mapping of E15/E16 on CSL5. The drawing gives the parts, not the timing.
- U7: KLINIK E11 R0..R3 to E10 #8/#7/#2/#1: the individual crossing wires near the pull-ups are not clear. The CTY E4 mapping (#6->R0, #5->R1, #4->R2, #3->R3) is clear. E4/E11 pin 15 (R EXT) wiring is not clear.
- U8: duplicate element number: CSL4 labels the port-204 register "6 D FF 74LS174 E156"; CSL6 labels the port-205 bits 6/7 register "4 D FF 74LS175 E156". Read at 600 dpi; both labels say E156. The 206 bit 0 flop on CSL5 (74LS74, pins 1-6) reads "E48" or "E40": the last glyph is not clear at 600 dpi. CSL5 also has a 74S175 E48 (SINGLE CLK, T COUNT DONE). CSL1 has a 74LS74 E40 that uses only pins 8-13, so "E40" fits better.
- U9: the 74LS170 location numbers (0 vs 2) and the inverting outputs of 8097 (non-inverting), 74LS353, 74LS258 come from data sheets, not from the drawing. The drawing shows bubbles only on the LS353/LS258 outputs and on the 8646 bus pins.
