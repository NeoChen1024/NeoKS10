//! Run a stand-alone diagnostic: load SAV files (5-byte words, as
//! t10backup extracts them), start at an address, and report why the
//! processor stopped.
//!
//!   cargo run --example diag -- [--start OCTAL] [--limit N] FILE.SAV...

use ks10::sav::{self, Packing};
use ks10::{Machine, Stop};
use pdp10::{Half, W36};

fn main() {
    let mut args = std::env::args().skip(1);
    let mut start = None;
    let mut limit = 10_000_000u64;
    let mut files = Vec::new();
    while let Some(a) = args.next() {
        match a.as_str() {
            "--start" => start = args.next().and_then(|s| Half::parse_radix(&s, 8)),
            "--limit" => limit = args.next().and_then(|s| s.parse().ok()).unwrap_or(limit),
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
    let stop = m.run(limit);
    let pascnt = m.mem[0o30047];
    println!("stop: {stop:?} after {} instructions", m.count);
    println!(
        "PC {}  flags {:06o}  PASCNT (30047) {}",
        m.pc,
        m.flags.0,
        pascnt.halves_text()
    );
    if let Stop::Halt { pc } | Stop::Unimplemented { pc, .. } | Stop::Nxm { pc, .. } = stop {
        let a = pc.get() as usize;
        for k in a.saturating_sub(3)..(a + 3).min(m.mem.len()) {
            let w: W36 = m.mem[k];
            println!("  {k:06o}: {}", w.halves_text());
        }
    }
}
