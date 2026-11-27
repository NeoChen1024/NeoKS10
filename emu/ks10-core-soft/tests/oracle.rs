//! The random differential run against the reference emulator
//! (docs/emulator.md, "Differential runs").
//!
//! The reference emulators have the trace hooks of oracle/ (`make oracle`
//! builds them). KS10_ORACLE names the `pdp10-ks` program of SIMH, and
//! KS10_ORACLE_KLH10 the `kn10-ks-its` program of KLH10. The test runs
//! the cases against each one that is named. Without either, it prints
//! that it did not run, and passes.
//!
//! Each case is one instruction at 1000, then two HALTs (for a skip). The
//! ACs, the word at E and the word at E + 1 get values first, in both
//! emulators. The test compares the PC word and the 16 ACs before the
//! instruction and before the HALT, the record counts, and at the end the
//! words at E and E + 1.
//!
//! There is no paging and no interrupt. E is a direct address (no index,
//! no indirect) outside the code. The generator and `skip` decide from the
//! inputs only, before either emulator runs (docs/emulator.md,
//! "Differential runs", has the coverage).

use std::collections::BTreeMap;
use std::io::Write;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::time::{Duration, Instant};

use ks10_core_soft::flags::Flags;
use ks10_core_soft::{Machine, Stop};
use pdp10::instructions::{self as ins, Class};
use pdp10::{Half, W36};

/// The instruction.
const CODE: u32 = 0o1000;
/// Cases in one run of SIMH.
const BATCH: usize = 500;
/// The HALT: JRST 4,0.
const HALT: u64 = 0o254200_000000;
const MASK: u64 = 0o777777_777777;

/// splitmix64: a small generator, so that a seed gives the same case on
/// each host.
struct Rng(u64);

impl Rng {
    fn next(&mut self) -> u64 {
        self.0 = self.0.wrapping_add(0x9e37_79b9_7f4a_7c15);
        let mut z = self.0;
        z = (z ^ (z >> 30)).wrapping_mul(0xbf58_476d_1ce4_e5b9);
        z = (z ^ (z >> 27)).wrapping_mul(0x94d0_49bb_1331_11eb);
        z ^ (z >> 31)
    }
    fn below(&mut self, n: u64) -> u64 {
        self.next() % n
    }
    /// A word, often a boundary value.
    fn word(&mut self) -> u64 {
        const EDGES: [u64; 10] = [
            0,
            1,
            0o777777_777777,
            0o400000_000000,
            0o377777_777777,
            0o000000_777777,
            0o777777_000000,
            0o000001_000000,
            0o200000_000000,
            0o577777_777777,
        ];
        match self.below(8) {
            0 | 1 => EDGES[self.below(EDGES.len() as u64) as usize],
            2 => self.below(0o100),
            3 => MASK - self.below(0o100),
            _ => self.next() & MASK,
        }
    }
    /// A normalized single precision number with exponent `exp` (0 to
    /// 377), or 0 (PRM 2.6: bit 9 is 1 in a positive normalized number; a
    /// negative one is the two's complement of a positive one).
    fn float(&mut self, exp: u64) -> u64 {
        if self.below(16) == 0 {
            return 0;
        }
        let frac = (1 << 26) | self.below(1 << 26);
        let w = (exp << 27) | frac;
        if self.below(2) == 0 {
            w
        } else {
            w.wrapping_neg() & MASK
        }
    }
    /// A normalized double precision number: the high word as `float`,
    /// the fraction goes on in bits 1 to 35 of the low word (PRM 2.6,
    /// "Double Precision Floating Point"; bit 0 of the low word is 0).
    fn double(&mut self, exp: u64) -> (u64, u64) {
        if self.below(16) == 0 {
            return (0, 0);
        }
        let frac = (1 << 26) | self.below(1 << 26);
        let v: u128 = (u128::from((exp << 27) | frac) << 35) | u128::from(self.below(1 << 35));
        let v = if self.below(2) == 0 {
            v
        } else {
            v.wrapping_neg() & ((1 << 71) - 1)
        };
        ((v >> 35) as u64 & MASK, (v as u64) & 0o377777_777777)
    }
    /// An exponent near `exp`: the alignment of FAD and DFAD stays below
    /// 61 places (docs/discrepancies.md, "FAD with a large exponent
    /// difference").
    fn near(&mut self, exp: u64) -> u64 {
        (exp + self.below(121)).saturating_sub(60).min(0o377)
    }
}

/// The microcode entries (DRAM J, spec/instructions.toml) of the
/// instructions that are not in the cases.
fn excluded(op: &ins::Op) -> bool {
    // A jump goes outside the code.
    const JUMPS: [&str; 15] = [
        "JUMP", "JRST", "AOJ", "SOJ", "AOBJ", "JFCL", "JFFO", "JSR", "JSP", "JSA", "JRA", "PUSHJ",
        "POPJ", "XCT", "EXTEND",
    ];
    // A pointer in an AC or in memory gives an address that the case
    // does not set.
    const POINTERS: [&str; 9] = [
        "PUSH", "POP", "ADJSP", "BLT", "IBP", "ILDB", "LDB", "IDPB", "DPB",
    ];
    // Other address spaces: the previous context and the I/O space.
    const SPACES: [&str; 8] = [
        "XCTR", "XCTRI", "UMOVE", "UMOVEM", "IORDI", "IORD", "IOWR", "IOWRI",
    ];
    op.class != Class::Inst
        || op.dram.a == ins::a::IOT
        || op.dram.j == "IOT710"
        || JUMPS.contains(&op.dram.j)
        || POINTERS.contains(&op.dram.j)
        || SPACES.contains(&op.dram.j)
}

/// The floating point instructions, by microcode entry: their operands
/// are normalized numbers.
fn float_kind(op: &ins::Op) -> Option<Float> {
    Some(match op.dram.j {
        "FAD" | "FSB" | "FMP" | "FDV" => Float::Single,
        "FIX" => Float::Fix,
        "FSC" => Float::Scale,
        "DFAD" | "DFSB" | "DFMP" | "DFDV" => Float::Double,
        _ => return None,
    })
}

#[derive(Clone, Copy)]
enum Float {
    /// AC and E (or E,,0 for an immediate form).
    Single,
    /// E.
    Fix,
    /// AC; E is the scale.
    Scale,
    /// AC, AC + 1 and E, E + 1.
    Double,
}

/// The shift count of E (ks10.47, SHIFT: bits 18 and 28 to 35).
fn count(e: u32) -> i32 {
    let low = (e & 0o377) as i32;
    if e & 0o400000 != 0 { low - 256 } else { low }
}

/// Cases where a reference emulator does not do what the microcode does,
/// from the inputs. Each arm has its entry in docs/discrepancies.md.
fn skip(c: &Case, r: Ref) -> Option<&'static str> {
    match r {
        Ref::Simh => skip_simh(c),
        Ref::Klh10 => skip_klh10(c),
    }
}

/// "ASH: TRAP 1 when an early step loses significance"
/// (docs/discrepancies.md): both emulators set Trap 1 for any overflow of
/// ASH and ASHC, and the microcode sets it only for an overflow of the
/// last step. For such a case the comparison takes Trap 1 (PC word bit 10)
/// of the emulator as 0, and compares the rest.
fn microcode_trap_1(c: &Case, mut b: Run) -> Run {
    if early_overflow_only(c) {
        for r in b.records.iter_mut().skip(1) {
            r[0] &= !(1 << (35 - 10));
        }
    }
    b
}

/// ASH or ASHC left: a step loses a bit that differs from the sign, but
/// the last step does not. The bits that the steps lose are the magnitude
/// from the top, then zeros.
fn early_overflow_only(c: &Case) -> bool {
    let op = &ins::OPS[(c.inst >> 27) as usize];
    let a = ((c.inst >> 23) & 0o17) as usize;
    let n = count((c.inst & 0o777777) as u32);
    let (hi, lo) = (u128::from(c.ac[a]), u128::from(c.ac[(a + 1) & 0o17]));
    let (mag, bits) = match op.dram.j {
        "ASH" => (hi & ((1 << 35) - 1), 35),
        "ASHC" => (((hi & ((1 << 35) - 1)) << 35) | (lo & ((1 << 35) - 1)), 70),
        _ => return false,
    };
    if n <= 0 {
        return false;
    }
    let sign = (hi >> 35) & 1;
    let lost = |step: u32| {
        if step > bits {
            0
        } else {
            (mag >> (bits - step)) & 1
        }
    };
    let k = n as u32;
    (1..k).any(|s| lost(s) != sign) && lost(k) == sign
}

fn skip_klh10(c: &Case) -> Option<&'static str> {
    let op = &ins::OPS[(c.inst >> 27) as usize];
    let e = (c.inst & 0o777777) as u32;
    let a = ((c.inst >> 23) & 0o17) as usize;
    let word = |x: u32| {
        if x < 0o20 {
            c.ac[x as usize]
        } else {
            c.mem[&x]
        }
    };
    match op.dram.j {
        // "HLLE in KLH10": a positive source with a bit of 1 to 3 set.
        // HLLEI has the source 0,,E.
        // "DIV in KLH10: a high word equal to the divisor" (the special
        // case of kn10ops.c, ddivstep).
        "DIV" => {
            let signed = |w: u64| (w as i64) << 28 >> 28;
            let lo = c.ac[(a + 1) & 0o17];
            let d = (i128::from(signed(c.ac[a])) << 35) | i128::from(lo & 0o377777_777777);
            let v = if op.code & 0o3 == 1 {
                u64::from(e)
            } else {
                word(e)
            };
            let dv = i128::from(signed(v)).abs();
            let high = d.abs() >> 35;
            (dv != 0 && high == dv).then_some("DIV high word equal to the divisor")
        }
        "HLLE" if op.code != 0o531 => {
            let src = if op.code == 0o532 { c.ac[a] } else { word(e) };
            (src >> 35 == 0 && (src >> 32) & 0o7 != 0).then_some("HLLE bits 1 to 3")
        }
        _ => None,
    }
}

fn skip_simh(c: &Case) -> Option<&'static str> {
    let op = &ins::OPS[(c.inst >> 27) as usize];
    let e = (c.inst & 0o777777) as u32;
    let a = ((c.inst >> 23) & 0o17) as usize;
    let n = count(e);
    let signed = |w: u64| (w as i64) << 28 >> 28;
    // C(E), or 0,,E for the immediate form (mode 1 of 220 to 227).
    let operand = || {
        if op.code & 0o3 == 1 {
            u64::from(e)
        } else if e < 0o20 {
            c.ac[e as usize]
        } else {
            c.mem[&e]
        }
    };
    match op.dram.j {
        // "CIRC".
        "CIRC" => Some("CIRC"),
        // "FDV: the quotient".
        "FDV" => Some("FDV"),
        // "IDIV of -2^35 by 1 or -1".
        "IDIV" => (c.ac[a] == 1 << 35 && signed(operand()).unsigned_abs() == 1)
            .then_some("IDIV -2^35 by 1 or -1"),
        // "DIV of -2^70 by 0".
        "DIV" => {
            (c.ac[a] == 1 << 35 && c.ac[(a + 1) & 0o17] & 0o377777_777777 == 0 && operand() == 0)
                .then_some("DIV of -2^70 by 0")
        }
        // "IMUL: a negative product that does not fit".
        "IMUL" => {
            let p = i128::from(signed(c.ac[a])) * i128::from(signed(operand()));
            let m = p.unsigned_abs();
            (p < 0 && m > 1 << 35 && m >> 35 == 1).then_some("IMUL negative overflow")
        }
        // "Shift count -256".
        "ASH" | "LSH" | "ASHC" | "LSHC" if n == -256 => Some("shift count -256"),
        // "ASH and ASHC: a left shift of many places".
        "ASH" if n > 28 => Some("ASH left by more than 28"),
        "ASHC" if n > 29 => Some("ASHC left by more than 29"),
        _ => None,
    }
}

#[derive(Clone)]
struct Case {
    seed: u64,
    inst: u64,
    ac: [u64; 16],
    /// E and E + 1, if they are not ACs.
    mem: BTreeMap<u32, u64>,
}

impl Case {
    fn new(seed: u64) -> Case {
        let ops: Vec<&ins::Op> = ins::OPS.iter().filter(|o| !excluded(o)).collect();
        let mut r = Rng(seed);
        let op = ops[r.below(ops.len() as u64) as usize];
        let a = r.below(16) as usize;
        let mut ac = [0; 16];
        for w in &mut ac {
            *w = r.word();
        }
        let y = loop {
            let y = match r.below(10) {
                0..=4 => 0o2000 + r.below(0o100) as u32,
                5 => r.below(0o20) as u32,
                6 => r.below(0o100) as u32,
                7 => 0o777777 - r.below(0o100) as u32,
                _ => r.below(0o1000000) as u32,
            };
            // For a floating point instruction, E and E + 1 are not the
            // ACs of the operand: else an operand is not normalized.
            let acs = [a as u32, ((a + 1) & 0o17) as u32];
            let overlap = float_kind(op).is_some()
                && (acs.contains(&y) || acs.contains(&((y + 1) & 0o777777)));
            // The HALT writes the halt status block at 500 to 520
            // (`Machine::halt`; inout.50, WRTHSB). KLH10 does not.
            let hsb = |x: u32| (0o500..=0o520).contains(&x);
            if !(CODE - 1..CODE + 3).contains(&y) && !overlap && !hsb(y) && !hsb(y + 1) {
                break y;
            }
        };
        let mut mem = BTreeMap::new();
        for e in [y, (y + 1) & 0o777777] {
            if e >= 0o20 {
                mem.insert(e, r.word());
            }
        }
        let mut y = y;
        // Normalized operands for the floating point instructions. A word
        // at E that is an AC is the AC.
        let put = |mem: &mut BTreeMap<u32, u64>, ac: &mut [u64; 16], a: u32, w: u64| {
            if a < 0o20 {
                ac[a as usize] = w;
            } else {
                mem.insert(a, w);
            }
        };
        let next = |a: u32| (a + 1) & 0o777777;
        match float_kind(op) {
            Some(Float::Single) => {
                let x = r.below(0o400);
                ac[a] = r.float(x);
                let n = r.near(x);
                let f = r.float(n);
                if op.code & 0o7 == 0o5 {
                    // The immediate form: the operand is E,,0 (flt.5,
                    // FL-I). Then E is not an address.
                    y = (f >> 18) as u32;
                    mem.clear();
                } else {
                    put(&mut mem, &mut ac, y, f);
                }
            }
            Some(Float::Fix) => {
                let x = r.below(0o400);
                let f = r.float(x);
                put(&mut mem, &mut ac, y, f);
            }
            Some(Float::Scale) => {
                let x = r.below(0o400);
                ac[a] = r.float(x);
            }
            Some(Float::Double) => {
                let x = r.below(0o400);
                let (h, l) = r.double(x);
                ac[a] = h;
                ac[(a + 1) & 0o17] = l;
                let n = r.near(x);
                let (h, l) = r.double(n);
                put(&mut mem, &mut ac, y, h);
                put(&mut mem, &mut ac, next(y), l);
            }
            None => {}
        }
        let inst = (u64::from(op.code) << 27) | ((a as u64) << 23) | u64::from(y);
        Case {
            seed,
            inst,
            ac,
            mem,
        }
    }

    /// The memory of the case: the instruction, the HALTs, E and E + 1.
    fn memory(&self) -> BTreeMap<u32, u64> {
        let mut m = self.mem.clone();
        m.insert(CODE, self.inst);
        m.insert(CODE + 1, HALT);
        m.insert(CODE + 2, HALT);
        m
    }
}

/// One record: the PC word and the 16 ACs before an instruction.
type Record = [u64; 17];

/// The result of one emulator: the records, then E and E + 1.
#[derive(Debug, PartialEq)]
struct Run {
    records: Vec<Record>,
    mem: BTreeMap<u32, u64>,
}

/// Run a case in ks10, up to the HALT.
fn ours(c: &Case) -> Result<Run, String> {
    let mut m = Machine::new();
    for (k, &a) in c.ac.iter().enumerate() {
        m.ac[k] = W36::wrap(a);
    }
    for (&a, &w) in &c.memory() {
        m.deposit(a, W36::wrap(w))
            .map_err(|a| format!("deposit {a:o}"))?;
    }
    m.flags = Flags(0);
    m.pc = Half::wrap(u64::from(CODE));
    let mut records = Vec::new();
    loop {
        let mut r = [0; 17];
        r[0] = m.flags.pc_word(m.pc).get();
        for k in 0..16 {
            r[k + 1] = m.ac[k].get();
        }
        records.push(r);
        if records.len() > 3 {
            return Err("no HALT".into());
        }
        match m.step() {
            Ok(()) => {}
            Err(Stop::Halt { .. }) => break,
            Err(s) => return Err(format!("{s:?}")),
        }
    }
    let mem = c
        .mem
        .keys()
        .map(|&a| (a, m.mem[a as usize].get()))
        .collect();
    Ok(Run { records, mem })
}

/// A reference emulator.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Ref {
    Simh,
    Klh10,
}

impl Ref {
    const ALL: [Ref; 2] = [Ref::Simh, Ref::Klh10];

    fn name(self) -> &'static str {
        match self {
            Ref::Simh => "SIMH",
            Ref::Klh10 => "KLH10",
        }
    }

    /// The variable that names the program.
    fn var(self) -> &'static str {
        match self {
            Ref::Simh => "KS10_ORACLE",
            Ref::Klh10 => "KS10_ORACLE_KLH10",
        }
    }

    /// The commands for a batch. After each case, the words at E and
    /// E + 1 are examined.
    fn script(self, batch: &[Case]) -> String {
        let mut s = String::new();
        match self {
            Ref::Simh => {
                s += "set cpu its\n";
                for c in batch {
                    for (k, a) in c.ac.iter().enumerate() {
                        s += &format!("dep {k:o} {a:o}\n");
                    }
                    for (a, w) in &c.memory() {
                        s += &format!("dep {a:o} {w:o}\n");
                    }
                    s += &format!("dep flags 0\ndep pc {CODE:o}\ngo\necho =end\n");
                    for a in c.mem.keys() {
                        s += &format!("ex {a:o}\n");
                    }
                }
                s += "quit\n";
            }
            Ref::Klh10 => {
                // `reset` clears the flags (kn10cpu.c, apr_init).
                for c in batch {
                    s += "reset\n";
                    for (k, a) in c.ac.iter().enumerate() {
                        s += &format!("deposit {k:o} {a:o}\n");
                    }
                    for (a, w) in &c.memory() {
                        s += &format!("deposit {a:o} {w:o}\n");
                    }
                    s += &format!("go {CODE:o}\n");
                    for a in c.mem.keys() {
                        s += &format!("examine {a:o}\n");
                    }
                }
                s += "really-quit\n";
            }
        }
        s
    }

    /// The examined words of each case, from the output.
    fn mems(self, out: &str) -> Vec<BTreeMap<u32, u64>> {
        let mut mems = Vec::new();
        match self {
            Ref::Simh => {
                // "ADDR:\tWORD" after each "=end".
                for part in out.split("=end").skip(1) {
                    let mut mem = BTreeMap::new();
                    for line in part.lines() {
                        if let Some((a, w)) = line.split_once(":\t")
                            && let (Ok(a), Ok(w)) = (
                                u32::from_str_radix(a.trim(), 8),
                                u64::from_str_radix(w.trim(), 8),
                            )
                        {
                            mem.insert(a, w);
                        }
                    }
                    mems.push(mem);
                }
            }
            Ref::Klh10 => {
                // " ADDR/ WORD" (WORD is N or LH,,RH, then a tab and the
                // instruction) after each "Starting KN10".
                for part in out.split("Starting KN10").skip(1) {
                    let mut mem = BTreeMap::new();
                    for line in part.lines() {
                        let Some((a, w)) = line.split_once("/ ") else {
                            continue;
                        };
                        let w = w.split('\t').next().unwrap_or("").trim();
                        let w = match w.split_once(",,") {
                            Some((l, r)) => u64::from_str_radix(l, 8)
                                .ok()
                                .zip(u64::from_str_radix(r, 8).ok())
                                .map(|(l, r)| (l << 18) | r),
                            None => u64::from_str_radix(w, 8).ok(),
                        };
                        if let (Ok(a), Some(w)) = (u32::from_str_radix(a.trim(), 8), w) {
                            mem.insert(a, w);
                        }
                    }
                    mems.push(mem);
                }
            }
        }
        mems
    }
}

/// Run a batch of cases in one run of a reference emulator. The trace
/// starts at the first fetch from 1000; a case starts at each record with
/// PC 1000.
fn reference(r: Ref, prog: &Path, dir: &Path, batch: &[Case]) -> Result<Vec<Run>, String> {
    let ini = dir.join(format!("{}.ini", r.name()));
    let trace = dir.join(format!("{}.trace", r.name()));
    let _ = std::fs::remove_file(&trace);
    std::fs::write(&ini, r.script(batch)).map_err(|e| e.to_string())?;
    let mut child = Command::new(prog)
        .arg(&ini)
        .current_dir(dir)
        .env("KS10_TRACE", &trace)
        .env("KS10_TRACE_FROM", format!("{CODE:o}"))
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .spawn()
        .map_err(|e| format!("{}: {e}", prog.display()))?;
    let mut stdout = child.stdout.take().expect("piped");
    let reader = std::thread::spawn(move || {
        let mut out = Vec::new();
        let _ = std::io::Read::read_to_end(&mut stdout, &mut out);
        String::from_utf8_lossy(&out).into_owned()
    });
    let start = Instant::now();
    loop {
        if child.try_wait().map_err(|e| e.to_string())?.is_some() {
            break;
        }
        if start.elapsed() > Duration::from_secs(60) {
            let _ = child.kill();
            let _ = child.wait();
            return Err(format!("{} did not end in 60 s", r.name()));
        }
        std::thread::sleep(Duration::from_millis(10));
    }
    let out = reader.join().map_err(|_| "reader")?;
    let mems = r.mems(&out);
    let text = std::fs::read_to_string(&trace).map_err(|e| format!("trace: {e}"))?;
    let mut runs: Vec<Vec<Record>> = Vec::new();
    for line in text.lines() {
        let f: Vec<&str> = line.split_whitespace().collect();
        if f.len() != 17 {
            return Err(format!("trace line {line:?}"));
        }
        let mut rec = [0; 17];
        for (k, f) in f.iter().enumerate() {
            rec[k] = u64::from_str_radix(f, 8).map_err(|_| format!("trace line {line:?}"))?;
        }
        if rec[0] & 0o777777 == u64::from(CODE) || runs.is_empty() {
            runs.push(Vec::new());
        }
        runs.last_mut().expect("a run").push(rec);
    }
    if runs.len() != batch.len() || mems.len() != batch.len() {
        return Err(format!(
            "{} ran {} cases and printed {} memory dumps, of {}",
            r.name(),
            runs.len(),
            mems.len(),
            batch.len()
        ));
    }
    Ok(runs
        .into_iter()
        .zip(mems)
        .map(|(records, mem)| Run { records, mem })
        .collect())
}

fn octal(w: u64) -> String {
    W36::wrap(w).halves_text()
}

/// The first difference of two runs, as text. The first line names the
/// instruction in parentheses.
fn difference(c: &Case, a: &Run, b: &Run, r: Ref) -> Option<String> {
    let them = r.name();
    let op = &ins::OPS[(c.inst >> 27) as usize];
    let mut t = format!("seed {}: {} ({})\n", c.seed, octal(c.inst), op.name);
    let names = [
        "PC", "0", "1", "2", "3", "4", "5", "6", "7", "10", "11", "12", "13", "14", "15", "16",
        "17",
    ];
    let mut differ = a.records.len() != b.records.len();
    if differ {
        t += &format!(
            "  records: ks10 {}, {them} {}\n",
            a.records.len(),
            b.records.len()
        );
    }
    let before = &a.records[0];
    if let (Some(x), Some(y)) = (a.records.get(1), b.records.get(1)) {
        for (j, n) in names.iter().enumerate() {
            let mark = if x[j] != y[j] { "*" } else { " " };
            differ |= x[j] != y[j];
            t += &format!(
                "{mark} {n:>3} before {:>14} ks10 {:>14} {them} {:>14}\n",
                octal(before[j]),
                octal(x[j]),
                octal(y[j])
            );
        }
    }
    differ |= a.records.first() != b.records.first();
    for (addr, w0) in &c.mem {
        let x = a.mem.get(addr);
        let y = b.mem.get(addr);
        let mark = if x != y { "*" } else { " " };
        differ |= x != y;
        t += &format!(
            "{mark} {addr:06o} before {:>14} ks10 {:>14} {them} {:>14}\n",
            octal(*w0),
            x.map(|w| octal(*w)).unwrap_or_default(),
            y.map(|w| octal(*w)).unwrap_or_default()
        );
    }
    differ.then_some(t)
}

/// The cases of tests/oracle/cases: differences that were found, and
/// directed cases, one line each: the instruction, the 16 ACs, then ADDR:WORD for E and E + 1,
/// in octal. They do not depend on the generator.
fn saved_cases() -> Vec<Case> {
    let p = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../tests/oracle/cases");
    let Ok(text) = std::fs::read_to_string(p) else {
        return Vec::new();
    };
    let mut cases = Vec::new();
    for line in text.lines() {
        let line = line.split('#').next().unwrap_or("").trim();
        if line.is_empty() {
            continue;
        }
        let f: Vec<&str> = line.split_whitespace().collect();
        let word =
            |s: &str| u64::from_str_radix(s, 8).expect("an octal word in tests/oracle/cases");
        assert!(f.len() >= 17, "tests/oracle/cases: {line}");
        let mut ac = [0; 16];
        for k in 0..16 {
            ac[k] = word(f[k + 1]);
        }
        let mut mem = BTreeMap::new();
        for m in &f[17..] {
            let (a, w) = m.split_once(':').expect("ADDR:WORD in tests/oracle/cases");
            mem.insert(word(a) as u32, word(w));
        }
        cases.push(Case {
            seed: 0,
            inst: word(f[0]),
            ac,
            mem,
        });
    }
    cases
}

/// A case as a line of tests/oracle/cases.
fn case_line(c: &Case) -> String {
    let mut t = format!("{:012o}", c.inst);
    for a in &c.ac {
        t += &format!(" {a:012o}");
    }
    for (a, w) in &c.mem {
        t += &format!(" {a:o}:{w:012o}");
    }
    t
}

#[test]
fn random_instructions() {
    let refs: Vec<(Ref, PathBuf)> = Ref::ALL
        .iter()
        .filter_map(|&r| std::env::var_os(r.var()).map(|p| (r, PathBuf::from(p))))
        .collect();
    if refs.is_empty() {
        eprintln!(
            "KS10_ORACLE and KS10_ORACLE_KLH10 are not set: the differential run did not run"
        );
        return;
    }
    // KS10_ORACLE_CASES cases from the seed KS10_ORACLE_SEED (decimal).
    let count: u64 = std::env::var("KS10_ORACLE_CASES")
        .ok()
        .and_then(|s| s.parse().ok())
        .unwrap_or(2000);
    let first: u64 = std::env::var("KS10_ORACLE_SEED")
        .ok()
        .and_then(|s| s.parse().ok())
        .unwrap_or(1);
    let all: Vec<Case> = saved_cases()
        .into_iter()
        .chain((first..first + count).map(Case::new))
        .collect();
    // KS10_ORACLE_NOSKIP: run the cases of `skip` too (to count them).
    let noskip = std::env::var_os("KS10_ORACLE_NOSKIP").is_some();
    // KS10_ORACLE_SHOW: how many to print; KS10_ORACLE_ONLY: only these
    // instructions (names with commas).
    let shown: usize = std::env::var("KS10_ORACLE_SHOW")
        .ok()
        .and_then(|s| s.parse().ok())
        .unwrap_or(5);
    let only = std::env::var("KS10_ORACLE_ONLY").unwrap_or_default();
    let name = |c: &Case| ins::OPS[(c.inst >> 27) as usize].name;
    let dir = PathBuf::from(env!("CARGO_TARGET_TMPDIR")).join("oracle");
    std::fs::create_dir_all(&dir).expect("temporary directory");
    let mut err = std::io::stderr();
    // The cases that no named emulator compares. With all emulators,
    // there must be none: each skip of one emulator is checked by another.
    let uncompared = all
        .iter()
        .filter(|c| !noskip && refs.iter().all(|(r, _)| skip(c, *r).is_some()))
        .count();
    let _ = writeln!(err, "cases that no emulator compares: {uncompared}");
    let early = all.iter().filter(|c| early_overflow_only(c)).count();
    let _ = writeln!(err, "cases with Trap 1 of the microcode: {early}");
    if refs.len() == Ref::ALL.len() {
        assert_eq!(uncompared, 0, "cases that no emulator compares");
    }
    let mut total = 0;
    for (r, prog) in &refs {
        let r = *r;
        let mut skipped: BTreeMap<&str, usize> = BTreeMap::new();
        let mut cases = Vec::new();
        for c in &all {
            match skip(c, r).filter(|_| !noskip) {
                Some(why) => *skipped.entry(why).or_default() += 1,
                None => cases.push(c),
            }
        }
        let mut failures: Vec<(&Case, String)> = Vec::new();
        let mut records = 0;
        for chunk in cases.chunks(BATCH) {
            let batch: Vec<Case> = chunk.iter().map(|c| (*c).clone()).collect();
            let theirs =
                reference(r, prog, &dir, &batch).unwrap_or_else(|e| panic!("{}: {e}", r.name()));
            for (c, b) in chunk.iter().zip(theirs) {
                let b = &microcode_trap_1(c, b);
                match ours(c) {
                    Err(e) => failures.push((c, format!("seed {}: ks10: {e}\n", c.seed))),
                    Ok(a) => {
                        records += a.records.len();
                        if let Some(d) = difference(c, &a, b, r) {
                            failures.push((c, d));
                        }
                    }
                }
            }
        }
        let _ = writeln!(
            err,
            "{}: {} cases, {records} records, {} differences; not run: {skipped:?}",
            r.name(),
            cases.len(),
            failures.len()
        );
        assert!(records >= 2 * cases.len(), "{}: too few records", r.name());
        let mut by: BTreeMap<&str, usize> = BTreeMap::new();
        for (c, _) in &failures {
            *by.entry(name(c)).or_default() += 1;
        }
        let _ = writeln!(err, "{}: differences by instruction: {by:?}", r.name());
        let pick =
            |(c, _): &&(&Case, String)| only.is_empty() || only.split(',').any(|n| name(c) == n);
        for (c, f) in failures.iter().filter(pick).take(shown) {
            let _ = writeln!(err, "{f}  case: {}", case_line(c));
        }
        total += failures.len();
    }
    assert!(total == 0, "{total} cases differ");
}
