//! Run a stand-alone diagnostic: load SAV files (5-byte words, as
//! t10backup extracts them), start at an address, and report why the
//! processor stopped.
//!
//!   cargo run --example diag -- [--start OCTAL] [--limit N] [--trace N]
//!       [--answer PROMPT=TEXT]... FILE.SAV...
//!
//! --trace N prints the PC and the instruction of the last N instructions.
//! --answer PROMPT=TEXT types TEXT on the CTY when PROMPT appears in the
//! CTY output; "\r" in TEXT is a carriage return. The answers are used in
//! order, each one `console::ANSWER_DELAY` instructions after its prompt.
//! The CTY output is printed at the end.

use ks10::sav::{self, Packing};
use ks10::{Machine, Stop};
use pdp10::{Half, W36};

fn main() {
    let mut args = std::env::args().skip(1);
    let mut start = None;
    let mut limit = 10_000_000u64;
    let mut trace = 0usize;
    let mut files = Vec::new();
    let mut script = ks10::console::Script::default();
    while let Some(a) = args.next() {
        match a.as_str() {
            "--start" => start = args.next().and_then(|s| Half::parse_radix(&s, 8)),
            "--limit" => limit = args.next().and_then(|s| s.parse().ok()).unwrap_or(limit),
            "--trace" => trace = args.next().and_then(|s| s.parse().ok()).unwrap_or(0),
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
        let s = match sav::parse(&sav::words(&bytes, Packing::Ansi5)) {
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
    if let Stop::Halt { pc, .. }
    | Stop::Unimplemented { pc, .. }
    | Stop::Nxm { pc, .. }
    | Stop::Pager { pc } = stop
    {
        let a = pc.get() as usize;
        for k in a.saturating_sub(3)..(a + 3).min(m.mem.len()) {
            let w: W36 = m.mem[k];
            println!("  {k:06o}: {}", w.halves_text());
        }
    }
}
