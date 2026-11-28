//! Run a stand-alone diagnostic: load SAV files (5-byte words, as
//! t10backup extracts them), start at an address, and report why the
//! processor stopped.
//!
//!   cargo run --example diag -- [--start OCTAL] [--limit N] [--trace N]
//!       [--answer PROMPT=TEXT]... FILE.SAV...
//!
//! --trace N prints the PC and the instruction of the last N instructions.
//! --dump OCTAL[,N] prints N words (default 1) from OCTAL at the end.
//! --deposit OCTAL=OCTAL writes a word into memory after the loading.
//! --answer PROMPT=TEXT types TEXT on the CTY when PROMPT appears in the
//! CTY output; "\r" in TEXT is a carriage return. The answers are used in
//! order, each one when the CTY output is quiet after its prompt
//! (`console::QUIET`).
//! The CTY output is printed at the end.

use ks10_core_soft::{Machine, Stop};
use ks10_machine::sav;
use pdp10::{Half, W36};

fn main() {
    let mut args = std::env::args().skip(1);
    let mut start = None;
    let mut limit = 10_000_000u64;
    let mut trace = 0usize;
    let mut files = Vec::new();
    let mut script = ks10_machine::console::Script::default();
    let mut dumps: Vec<(u32, u32)> = Vec::new();
    let mut deposits: Vec<(u32, W36)> = Vec::new();
    while let Some(a) = args.next() {
        match a.as_str() {
            "--start" => start = args.next().and_then(|s| Half::parse_radix(&s, 8)),
            "--limit" => limit = args.next().and_then(|s| s.parse().ok()).unwrap_or(limit),
            "--trace" => trace = args.next().and_then(|s| s.parse().ok()).unwrap_or(0),
            "--dump" => {
                let a = args.next().unwrap_or_default();
                let (x, n) = a.split_once(',').unwrap_or((&a, "1"));
                if let (Ok(x), Ok(n)) = (u32::from_str_radix(x, 8), n.parse()) {
                    dumps.push((x, n));
                }
            }
            "--deposit" => {
                let a = args.next().unwrap_or_default();
                if let Some((x, v)) = a.split_once('=')
                    && let (Ok(x), Some(v)) = (u32::from_str_radix(x, 8), W36::parse_radix(v, 8))
                {
                    deposits.push((x, v));
                }
            }
            "--answer" => {
                let a = args.next().unwrap_or_default();
                let (p, t) = a.split_once('=').unwrap_or((&a, ""));
                script.answer(p, &t.replace("\\r", "\r"));
            }
            _ => files.push(a),
        }
    }
    let mut m = Machine::new();
    let mut last_start = Half::ZERO;
    for f in &files {
        let bytes = match std::fs::read(f) {
            Ok(b) => b,
            Err(e) => {
                eprintln!("{f}: {e}");
                std::process::exit(2);
            }
        };
        let s = match sav::parse(&sav::words(&bytes)) {
            Ok(s) => s,
            Err(e) => {
                eprintln!("{f}: {e:?}");
                std::process::exit(2);
            }
        };
        for (addr, ws) in &s.blocks {
            for (k, w) in ws.iter().enumerate() {
                let a = addr.get() as u32 + k as u32;
                if m.deposit(a, *w).is_err() {
                    eprintln!("{f}: address {a:o} outside memory");
                    std::process::exit(2);
                }
            }
        }
        println!("{f}: {} blocks, start {}", s.blocks.len(), s.start);
        last_start = s.start;
    }
    m.pc = start.unwrap_or(last_start);
    for (a, w) in deposits {
        if m.deposit(a, w).is_err() {
            eprintln!("--deposit: address {a:o} outside memory");
            std::process::exit(2);
        }
    }
    let mut last = std::collections::VecDeque::new();
    let stop = loop {
        if m.count >= limit {
            break Stop::Limit;
        }
        script.tick(m.count, &mut m.console);
        if trace > 0 {
            let pc = m.pc;
            last.push_back((
                pc,
                m.mem.get(pc.get() as usize).copied().unwrap_or(W36::ZERO),
            ));
            if last.len() > trace {
                last.pop_front();
            }
        }
        if let Err(s) = m.step() {
            break s;
        }
    };
    for (pc, w) in &last {
        println!("  trace {pc}: {}", w.halves_text());
    }
    let out: String = m
        .console
        .output
        .iter()
        .map(|&b| char::from(b & 0o177))
        .collect();
    println!("--- CTY output ---\n{}\n---", out.replace('\r', ""));
    let pascnt = m.mem[0o30047];
    println!("stop: {stop:?} after {} instructions", m.count);
    println!(
        "PC {}  flags {:06o}  PASCNT (30047) {}",
        m.pc,
        m.flags.0,
        pascnt.halves_text()
    );
    println!("EBR {}  UBR {}", m.ebr.halves_text(), m.ubr.halves_text());
    if let Stop::Halt { pc, .. } | Stop::Unimplemented { pc, .. } = stop {
        let a = pc.get() as usize;
        for k in a.saturating_sub(3)..(a + 3).min(m.mem.len()) {
            let w: W36 = m.mem[k];
            println!("  {k:06o}: {}", w.halves_text());
        }
    }
    for (x, n) in dumps {
        for a in x..x.saturating_add(n) {
            if let Some(w) = m.mem.get(a as usize) {
                println!("  {a:06o}: {}", w.halves_text());
            }
        }
    }
}
