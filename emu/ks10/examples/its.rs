//! Boot ITS from a disk image with the 8080 `BT` command, or from a tape
//! with `MB`, and run.
//!
//!   cargo run --release --example its -- [--limit N] [--answer P=T]...
//!       [--tape N=TAPE]... [--mt N] [DISK.dsk]
//!
//! --tape mounts a tape image on transport N (read only if the file is
//! read only). --mt N boots from transport N instead of the disk.
//!
//! The CTY output goes to stdout as it comes. --answer types TEXT after
//! PROMPT appears (`console::Script`); "\r" is a carriage return and
//! "\e" an escape.

use std::io::Write;

use ks10::console::Script;
use ks10::{Machine, Stop, boot};

/// Text with \\r, \\n, \\e and \\xHH escapes.
fn unescape(x: &str) -> String {
    let mut out = String::new();
    let mut it = x.chars().peekable();
    while let Some(c) = it.next() {
        if c != '\\' {
            out.push(c);
            continue;
        }
        match it.next() {
            Some('r') => out.push('\r'),
            Some('n') => out.push('\n'),
            Some('e') => out.push('\x1b'),
            Some('x') => {
                let h: String = it.by_ref().take(2).collect();
                if let Ok(v) = u8::from_str_radix(&h, 16) {
                    out.push(char::from(v));
                }
            }
            Some(o) => out.push(o),
            None => out.push('\\'),
        }
    }
    out
}

fn main() {
    let mut args = std::env::args().skip(1);
    let mut limit = 200_000_000u64;
    let mut script = Script::default();
    let mut disk = None;
    let mut trace = 0usize;
    let mut tapes = Vec::new();
    let mut mt = None;
    while let Some(a) = args.next() {
        match a.as_str() {
            "--limit" => limit = args.next().and_then(|s| s.parse().ok()).unwrap_or(limit),
            "--trace" => trace = args.next().and_then(|s| s.parse().ok()).unwrap_or(0),
            "--answer" => {
                let a = args.next().unwrap_or_default();
                let (p, t) = a.split_once('=').unwrap_or((&a, ""));
                let un = |x: &str| unescape(x);
                script.answer(&un(p), &un(t));
            }
            "--tape" => {
                let a = args.next().unwrap_or_default();
                let (n, f) = a.split_once('=').unwrap_or(("0", &a));
                tapes.push((n.parse::<usize>().unwrap_or(0), f.to_string()));
            }
            "--mt" => mt = args.next().and_then(|s| s.parse::<u16>().ok()),
            _ => disk = Some(a),
        }
    }
    if disk.is_none() && mt.is_none() {
        eprintln!(
            "usage: its [--limit N] [--answer P=T]... [--tape N=TAPE]... [--mt N] [DISK.dsk]"
        );
        std::process::exit(2);
    }
    let open = |path: &str| {
        let rw = std::fs::OpenOptions::new()
            .read(true)
            .write(true)
            .open(path);
        match rw {
            Ok(f) => (f, false),
            Err(_) => match std::fs::File::open(path) {
                Ok(f) => (f, true),
                Err(e) => {
                    eprintln!("{path}: {e}");
                    std::process::exit(2);
                }
            },
        }
    };
    let mut m = Machine::new();
    if let Some(disk) = &disk {
        let (f, ro) = open(disk);
        m.io.disk.attach(0, f, ro);
    }
    for (n, path) in &tapes {
        let (f, ro) = open(path);
        m.io.tape.attach(*n, f, ro);
    }
    let booted = match mt {
        Some(n) => boot::boot_tape(&mut m, n),
        None => boot::boot_disk(&mut m, 0),
    };
    if let Err(e) = booted {
        eprintln!("boot: {e:?}");
        std::process::exit(1);
    }
    let mut shown = 0;
    let out = std::io::stdout();
    let mut last = std::collections::VecDeque::new();
    let stop = loop {
        if m.count >= limit {
            break Stop::Limit;
        }
        if trace > 0 {
            let w = m.mem.get(m.pc.get() as usize).copied().unwrap_or_default();
            last.push_back((m.pc, w, m.flags.0));
            if last.len() > trace {
                last.pop_front();
            }
        }
        script.tick(m.count, &mut m.console);
        if let Err(s) = m.step() {
            break s;
        }
        if m.console.output.len() > shown {
            let mut o = out.lock();
            let b: Vec<u8> = m.console.output[shown..]
                .iter()
                .map(|c| c & 0o177)
                .collect();
            let _ = o.write_all(&b);
            let _ = o.flush();
            shown = m.console.output.len();
        }
    };
    for (pc, w, f) in &last {
        println!("  {pc} {} flags {f:06o}", w.halves_text());
    }
    if let Some(d) = &m.io.disk.drives[0] {
        println!(
            "disk 0: cmd {:o} pip {} attn {} da {:o} dc {:o} er1 {:o}; cs1 {:o} cs2 {:o} wc {:o}; events {:?}",
            d.cmd,
            d.pip,
            d.attn,
            d.da,
            d.dc,
            d.er1,
            m.io.disk.cs1,
            m.io.disk.cs2,
            m.io.disk.wc,
            m.io.events
        );
    }
    println!(
        "APR enabled {:o} flags {:o} level {}; PI {}; flags {:o}; 33 = {}; 32 = {}; paging {}",
        m.apr.enabled,
        m.apr.flags,
        m.apr.level,
        m.pi.word().halves_text(),
        m.flags.0,
        m.mem[0o33].halves_text(),
        m.mem[0o32].halves_text(),
        m.paging
    );
    let ept = m.ept_addr(0) as usize;
    println!(
        "EBR {} EPT+440..442: {} {} {}",
        m.ebr.halves_text(),
        m.mem[ept + 0o440].halves_text(),
        m.mem[ept + 0o441].halves_text(),
        m.mem[ept + 0o442].halves_text()
    );
    for a in std::env::var("ITS_DUMP")
        .unwrap_or_default()
        .split(',')
        .filter(|x| !x.is_empty())
    {
        if let Ok(a) = u64::from_str_radix(a, 8) {
            let saved = m.flags;
            m.flags = ks10::flags::Flags(0);
            let w = m.vread(pdp10::Half::wrap(a), ks10::machine::Ref::D1);
            m.flags = saved;
            println!("  exec {a:06o}: {:?}", w.map(|w| w.halves_text()));
        }
    }
    println!("\n--- stop: {stop:?} after {} steps, PC {}", m.count, m.pc);
}
