//! memprobe: the PS program of the memory path test design
//! (docs/mem-path-experiment.md, Part B). It runs as root on the board, with
//! a memprobe bitstream in the PL.
//!
//!   memprobe info
//!   memprobe latency read|write [--beats N] [--width 32|64] [--count N] [--gap N] [--load-threads N]
//!   memprobe consistency [--words N] [--seed N] [--beats N] [--width 32|64]
//!   memprobe run read|write|mixed|fill|check [RUN OPTIONS]
//!   memprobe rate read|write|mixed [RUN OPTIONS] [--load-threads N]   (B4, B5)
//!   memprobe order [RUN OPTIONS] [--reverse 0|1] [--load-threads N]      (B6)
//!     RUN OPTIONS: --beats N --width 32|64 --count N --gap N --base N --span-log2 N --seed N
//!                  --outstanding N --pattern seq|random|fixed
//!                  --read-percent P | --read-share N --verify 0|1
//!                  --rd-port 0..3 --wr-port 0..3
//!   memprobe afi-clear      (32BitEn of all HP ports to 0: 64-bit data)
//!   memprobe ps-access [--threads N] [--passes N]
//!   memprobe pread [--file PATH] [--chunk N] [--bytes N] [--threads N] [--drop-caches 1]
//!
//! Numbers can be decimal or hexadecimal with 0x. The output is CSV; lines
//! that start with '#' are comments.

use std::collections::HashMap;
use std::io::{self, Write};
use std::process::ExitCode;
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use plio::board::Region;
use plio::regmap::memprobe as mp;

mod device;
mod load;
mod measure;
mod pattern;
mod psaccess;
mod stats;

use device::{Error, Memprobe, Op, Pattern, RunConfig};

const USAGE: &str = "usage:
  memprobe info
  memprobe latency read|write [--beats N] [--width 32|64] [--count N] [--gap N] [--load-threads N]
  memprobe consistency [--words N] [--seed N] [--beats N] [--width 32|64]
  memprobe run read|write|mixed|fill|check [RUN OPTIONS]
  memprobe rate read|write|mixed [RUN OPTIONS] [--load-threads N]
  memprobe order [RUN OPTIONS] [--reverse 0|1] [--load-threads N]
    RUN OPTIONS: --beats N --width 32|64 --count N --gap N --base N --span-log2 N --seed N
                 --outstanding N --pattern seq|random|fixed --read-percent P | --read-share N --verify 0|1
                 --rd-port 0..3 --wr-port 0..3
  memprobe afi-clear
  memprobe ps-access [--threads N] [--passes N]
  memprobe pread [--file PATH] [--chunk N] [--bytes N] [--threads N] [--drop-caches 1]";

fn parse_number(s: &str) -> Option<u64> {
    match s.strip_prefix("0x") {
        Some(h) => u64::from_str_radix(h, 16).ok(),
        None => s.parse().ok(),
    }
}

/// Options "--name value" with the allowed names.
fn parse_options(args: &[String], allowed: &[&str]) -> Result<HashMap<String, u64>, String> {
    let mut out = HashMap::new();
    let mut it = args.iter();
    while let Some(a) = it.next() {
        let name = a
            .strip_prefix("--")
            .ok_or_else(|| format!("unexpected argument {a:?}"))?;
        if !allowed.contains(&name) {
            return Err(format!("unknown option --{name}"));
        }
        let v = it.next().ok_or_else(|| format!("--{name} needs a value"))?;
        let n = parse_number(v).ok_or_else(|| format!("--{name}: {v:?} is not a number"))?;
        out.insert(name.to_string(), n);
    }
    Ok(out)
}

fn opt_u32(o: &HashMap<String, u64>, name: &str, default: u32) -> Result<u32, String> {
    match o.get(name) {
        None => Ok(default),
        Some(&v) => u32::try_from(v).map_err(|_| format!("--{name}: {v} is too large")),
    }
}

fn header(dev: &Memprobe, out: &mut impl Write) -> io::Result<()> {
    let kernel = std::fs::read_to_string("/proc/sys/kernel/osrelease").unwrap_or_default();
    let now = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0);
    writeln!(
        out,
        "# memprobe: design build {:08x}, clk_m {} MHz, kernel {}, time {now} s after 1970",
        dev.build,
        dev.caps.clk_mhz,
        kernel.trim()
    )
}

/// The first 4 MiB of the reserved region in `threads` equal parts, one
/// mapping for each part.
fn region_parts(threads: u32) -> Result<Vec<plio::mmio::Mapping>, String> {
    if !(1..=4).contains(&threads) {
        return Err("--threads must be 1 to 4".into());
    }
    let span = 1usize << measure::SPAN_LOG2;
    let part = span / threads as usize;
    (0..threads as usize)
        .map(|i| {
            Region::open_part(u64::from(mp::HP_BASE), mp::HP_SIZE as usize, i * part, part)
                .map(Region::into_mapping)
                .map_err(|e| e.to_string())
        })
        .collect()
}

/// Remove "--name value" from `args`; the value is a word.
fn take_word(args: &mut Vec<String>, name: &str) -> Result<Option<String>, String> {
    let flag = format!("--{name}");
    match args.iter().position(|a| *a == flag) {
        None => Ok(None),
        Some(i) => {
            if i + 1 >= args.len() {
                return Err(format!("{flag} needs a value"));
            }
            let v = args.remove(i + 1);
            args.remove(i);
            Ok(Some(v))
        }
    }
}

/// --width: 32 for 32-bit beats, 64 (default) for 64-bit beats.
fn width32(o: &HashMap<String, u64>) -> Result<bool, String> {
    match o.get("width") {
        None | Some(64) => Ok(false),
        Some(32) => Ok(true),
        Some(w) => Err(format!("--width {w}: use 32 or 64")),
    }
}

/// The configuration of a run from the options, and the number of threads of
/// the PS memory load (--load-threads).
fn run_config(op: Op, args: &[String], beats: u32, count: u32) -> Result<(RunConfig, u32), String> {
    let mut rest = args.to_vec();
    let pattern = match take_word(&mut rest, "pattern")? {
        None => Pattern::Seq,
        Some(p) => Pattern::parse(&p)
            .ok_or_else(|| format!("--pattern {p:?}: use seq, random or fixed"))?,
    };
    let o = parse_options(
        &rest,
        &[
            "beats",
            "width",
            "count",
            "gap",
            "base",
            "span-log2",
            "seed",
            "outstanding",
            "read-share",
            "read-percent",
            "verify",
            "load-threads",
            "rd-port",
            "wr-port",
            "reverse",
        ],
    )?;
    // READ_SHARE has 16 bits: 100 % gives 65535 of 65536.
    let read_share = match o.get("read-percent") {
        Some(&p) if p <= 100 => (p * 65536 / 100).min(65535) as u32,
        Some(&p) => return Err(format!("--read-percent {p} is more than 100")),
        None => opt_u32(&o, "read-share", 0x8000)?,
    };
    let cfg = RunConfig {
        op,
        pattern,
        outstanding: opt_u32(&o, "outstanding", 1)?,
        read_share,
        verify: opt_u32(&o, "verify", 0)? != 0,
        rd_port: opt_u32(&o, "rd-port", 0)?,
        wr_port: opt_u32(&o, "wr-port", 0)?,
        order_reverse: opt_u32(&o, "reverse", 0)? != 0,
        beats: opt_u32(&o, "beats", beats)?,
        width32: width32(&o)?,
        base: opt_u32(&o, "base", 0)?,
        span_log2: opt_u32(&o, "span-log2", measure::SPAN_LOG2)?,
        count: opt_u32(&o, "count", count)?,
        gap: opt_u32(&o, "gap", 0)?,
        seed: opt_u32(&o, "seed", 1)?,
    };
    Ok((cfg, opt_u32(&o, "load-threads", 0)?))
}

fn plain_header(what: &str, out: &mut impl Write) -> io::Result<()> {
    let kernel = std::fs::read_to_string("/proc/sys/kernel/osrelease").unwrap_or_default();
    let now = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0);
    writeln!(
        out,
        "# memprobe {what}: kernel {}, time {now} s after 1970",
        kernel.trim()
    )
}

fn info(dev: &Memprobe) -> Result<(), Error> {
    let mut out = io::stdout().lock();
    let (fclk, clkm) = dev.clocks(Duration::from_secs(1));
    let w = |out: &mut io::StdoutLock<'_>, s: String| {
        writeln!(out, "{s}").map_err(|e| Error::Device(e.to_string()))
    };
    header(dev, &mut out).map_err(|e| Error::Device(e.to_string()))?;
    w(&mut out, format!("caps: {:?}", dev.caps))?;
    w(&mut out, format!("status: {:#010x}", dev.status()))?;
    w(&mut out, format!("fclk0: {:.3} MHz", fclk / 1e6))?;
    w(&mut out, format!("clk_m: {:.3} MHz", clkm / 1e6))?;
    let expect = f64::from(dev.caps.clk_mhz) * 1e6;
    if (clkm - expect).abs() > expect * 0.01 {
        return Err(Error::Device(format!(
            "clk_m is {clkm:.0} Hz, CAPS.CLK_MHZ {}",
            dev.caps.clk_mhz
        )));
    }
    let region = Region::open(u64::from(mp::HP_BASE), mp::HP_SIZE as usize)?;
    w(
        &mut out,
        format!(
            "region: {:#x}, {:#x} bytes, mapped",
            region.base,
            region.len()
        ),
    )?;
    // 1: 32-bit data. A run sets the bits that it needs and restores them.
    let afi = dev
        .afi_state()?
        .iter()
        .enumerate()
        .map(|(p, (r, w))| format!("HP{p} rd {} wr {}", u8::from(*r), u8::from(*w)))
        .collect::<Vec<_>>()
        .join(", ");
    w(&mut out, format!("afi 32BitEn: {afi}"))?;
    Ok(())
}

fn run_command(args: &[String]) -> Result<(), String> {
    let cmd = args.first().map(String::as_str).ok_or("no command")?;
    let e = |e: Error| e.to_string();
    match cmd {
        "info" => {
            parse_options(&args[1..], &[])?;
            info(&Memprobe::open().map_err(e)?).map_err(e)
        }
        "afi-clear" => {
            parse_options(&args[1..], &[])?;
            Memprobe::open().map_err(e)?.afi_clear().map_err(e)
        }
        "latency" => {
            let op = match args.get(1).map(String::as_str) {
                Some("read") => Op::Read,
                Some("write") => Op::Write,
                _ => return Err("latency needs read or write".into()),
            };
            let o = parse_options(
                &args[2..],
                &["beats", "width", "count", "gap", "load-threads"],
            )?;
            let dev = Memprobe::open().map_err(e)?;
            let mut out = io::stdout().lock();
            header(&dev, &mut out).map_err(|e| e.to_string())?;
            writeln!(out, "{}", measure::LATENCY_HEADER).map_err(|e| e.to_string())?;
            let threads = opt_u32(&o, "load-threads", 0)?;
            let load = load::Load::start(threads);
            // Let the load reach its rate before the run.
            std::thread::sleep(Duration::from_millis(if threads > 0 { 200 } else { 0 }));
            let result = measure::latency(
                &dev,
                op,
                opt_u32(&o, "beats", 1)?,
                width32(&o)?,
                opt_u32(&o, "count", 10_000)?,
                opt_u32(&o, "gap", 0)?,
                &mut out,
            );
            let rate = load.stop();
            if threads > 0 {
                writeln!(
                    out,
                    "# PS memory load: {threads} threads, {rate:.0} MiB/s copied"
                )
                .map_err(|e| e.to_string())?;
            }
            result.map_err(e)
        }
        "consistency" => {
            let o = parse_options(&args[1..], &["words", "seed", "beats", "width"])?;
            let words = o.get("words").copied().unwrap_or(10_000_000);
            let seed = opt_u32(&o, "seed", 1)?;
            let beats = opt_u32(&o, "beats", 16)?;
            let w32 = width32(&o)?;
            let dev = Memprobe::open().map_err(e)?;
            let region = Region::open(u64::from(mp::HP_BASE), mp::HP_SIZE as usize)
                .map_err(|x| x.to_string())?;
            let mut out = io::stdout().lock();
            header(&dev, &mut out).map_err(|e| e.to_string())?;
            let c = measure::consistency(&dev, &region, words, seed, beats, w32).map_err(e)?;
            let width = if w32 { 32 } else { 64 };
            writeln!(out, "direction,beats,width,words,mismatches").map_err(|e| e.to_string())?;
            writeln!(
                out,
                "ps_to_pl,{beats},{width},{},{}",
                c.ps_to_pl_words, c.ps_to_pl_errors
            )
            .map_err(|e| e.to_string())?;
            writeln!(
                out,
                "pl_to_ps,{beats},{width},{},{}",
                c.pl_to_ps_words, c.pl_to_ps_errors
            )
            .map_err(|e| e.to_string())?;
            if c.ps_to_pl_errors + c.pl_to_ps_errors != 0 {
                return Err("mismatches found".into());
            }
            Ok(())
        }
        "run" => {
            let op = args
                .get(1)
                .and_then(|s| Op::parse(s))
                .ok_or("run needs read, write, mixed, fill or check")?;
            let (cfg, _) = run_config(op, &args[2..], 16, 1)?;
            let dev = Memprobe::open().map_err(e)?;
            dev.clear().map_err(e)?;
            let st = dev.run(&cfg, Duration::from_secs(60)).map_err(e)?;
            let r = dev.results();
            let mut out = io::stdout().lock();
            header(&dev, &mut out).map_err(|e| e.to_string())?;
            writeln!(out, "# {cfg:?}\n# {st:?}").map_err(|e| e.to_string())?;
            writeln!(
                out,
                "run_cycles,reads,writes,stalls,data_errors,first_err_addr,first_err_data,resp_errors,hist_drops"
            )
            .map_err(|e| e.to_string())?;
            writeln!(
                out,
                "{},{},{},{},{},{:#x},{:#018x},{},{}",
                r.run_cycles,
                r.reads,
                r.writes,
                r.stalls,
                r.data_errors,
                r.first_err_addr,
                r.first_err_data,
                r.resp_errors,
                r.hist_drops
            )
            .map_err(|e| e.to_string())
        }
        "order" => {
            let (cfg, threads) = run_config(Op::Order, &args[1..], 1, 10_000)?;
            let dev = Memprobe::open().map_err(e)?;
            let mut out = io::stdout().lock();
            header(&dev, &mut out).map_err(|e| e.to_string())?;
            writeln!(out, "{}", measure::ORDER_HEADER).map_err(|e| e.to_string())?;
            let load = load::Load::start(threads);
            std::thread::sleep(Duration::from_millis(if threads > 0 { 200 } else { 0 }));
            let result = measure::order(&dev, &cfg, &mut out);
            let copied = load.stop();
            if threads > 0 {
                writeln!(
                    out,
                    "# PS memory load: {threads} threads, {copied:.0} MiB/s copied"
                )
                .map_err(|e| e.to_string())?;
            }
            result.map(|_| ()).map_err(e)
        }
        "rate" => {
            let op = match args.get(1).map(String::as_str) {
                Some("read") => Op::Read,
                Some("write") => Op::Write,
                Some("mixed") => Op::Mixed,
                _ => return Err("rate needs read, write or mixed".into()),
            };
            let (cfg, threads) = run_config(op, &args[2..], 1, 100_000)?;
            let dev = Memprobe::open().map_err(e)?;
            let mut out = io::stdout().lock();
            header(&dev, &mut out).map_err(|e| e.to_string())?;
            writeln!(out, "{}", measure::RATE_HEADER).map_err(|e| e.to_string())?;
            let load = load::Load::start(threads);
            std::thread::sleep(Duration::from_millis(if threads > 0 { 200 } else { 0 }));
            let result = measure::rate(&dev, &cfg, &mut out);
            let copied = load.stop();
            if threads > 0 {
                writeln!(
                    out,
                    "# PS memory load: {threads} threads, {copied:.0} MiB/s copied"
                )
                .map_err(|e| e.to_string())?;
            }
            result.map(|_| ()).map_err(e)
        }
        "ps-access" => {
            let o = parse_options(&args[1..], &["threads", "passes"])?;
            let threads = opt_u32(&o, "threads", 1)?;
            let passes = opt_u32(&o, "passes", 8)?;
            let mut out = io::stdout().lock();
            let io_e = |e: io::Error| e.to_string();
            plain_header("ps-access (B8)", &mut out).map_err(io_e)?;
            writeln!(out, "{}", psaccess::HEADER).map_err(io_e)?;
            for access in psaccess::Access::ALL {
                let parts = region_parts(threads)?;
                let w = psaccess::parallel(parts, |_, m| Ok(psaccess::words(m, access, passes)))
                    .map_err(io_e)?;
                let unit = if matches!(access, psaccess::Access::Read32 | psaccess::Access::Write32)
                {
                    4
                } else {
                    8
                };
                writeln!(
                    out,
                    "{}",
                    psaccess::row(access.name(), threads as usize, unit, &w)
                )
                .map_err(io_e)?;
            }
            Ok(())
        }
        "pread" => {
            // --file takes a path; the other options take numbers.
            let mut rest: Vec<String> = Vec::new();
            let mut file = String::from("/dev/mmcblk0");
            let mut it = args[1..].iter();
            while let Some(a) = it.next() {
                if a == "--file" {
                    file = it.next().ok_or("--file needs a value")?.clone();
                } else {
                    rest.push(a.clone());
                }
            }
            let o = parse_options(&rest, &["chunk", "bytes", "threads", "drop-caches"])?;
            let threads = opt_u32(&o, "threads", 1)?;
            let chunk = opt_u32(&o, "chunk", 65536)? as usize;
            let bytes = o.get("bytes").copied().unwrap_or(256 << 20);
            let io_e = |e: io::Error| format!("{file}: {e}");
            let mut files = Vec::new();
            let mut len = 0;
            for _ in 0..threads {
                let mut f = std::fs::File::open(&file).map_err(io_e)?;
                len = psaccess::file_len(&mut f).map_err(io_e)?;
                files.push(f);
            }
            if o.get("drop-caches").copied().unwrap_or(0) != 0 {
                // Clean pages only; the page cache then reads the file again.
                std::fs::write("/proc/sys/vm/drop_caches", "3\n")
                    .map_err(|e| format!("drop_caches: {e}"))?;
            }
            let parts = region_parts(threads)?;
            let per_thread = bytes / u64::from(threads.max(1));
            let w = psaccess::parallel(parts, |i, m| {
                let start = len / u64::from(threads) * i as u64;
                psaccess::pread(m, &files[i], len, start, chunk, per_thread)
            })
            .map_err(io_e)?;
            let mut out = io::stdout().lock();
            plain_header(&format!("pread (B8) of {file}"), &mut out).map_err(|e| e.to_string())?;
            writeln!(out, "{}", psaccess::HEADER).map_err(|e| e.to_string())?;
            writeln!(
                out,
                "{}",
                psaccess::row("pread", threads as usize, chunk, &w)
            )
            .map_err(|e| e.to_string())
        }
        _ => Err(format!("unknown command {cmd:?}")),
    }
}

fn main() -> ExitCode {
    let args: Vec<String> = std::env::args().skip(1).collect();
    match run_command(&args) {
        Ok(()) => ExitCode::SUCCESS,
        Err(msg) => {
            eprintln!("memprobe: {msg}");
            if msg.starts_with("no command") || msg.starts_with("unknown") || msg.contains("needs")
            {
                eprintln!("{USAGE}");
            }
            ExitCode::FAILURE
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn s(v: &[&str]) -> Vec<String> {
        v.iter().map(|x| x.to_string()).collect()
    }

    #[test]
    fn options() {
        let o = parse_options(&s(&["--count", "0x10", "--gap", "7"]), &["count", "gap"]).unwrap();
        assert_eq!(o["count"], 16);
        assert_eq!(o["gap"], 7);
        assert!(parse_options(&s(&["--count"]), &["count"]).is_err());
        assert!(parse_options(&s(&["--size", "1"]), &["count"]).is_err());
        assert!(parse_options(&s(&["count", "1"]), &["count"]).is_err());
        assert!(parse_options(&s(&["--count", "x"]), &["count"]).is_err());
        assert!(opt_u32(&o, "count", 0).is_ok());
        let big = parse_options(&s(&["--count", "0x100000000"]), &["count"]).unwrap();
        assert!(opt_u32(&big, "count", 0).is_err());
    }

    #[test]
    fn run_options() {
        let (c, t) = run_config(
            Op::Mixed,
            &s(&[
                "--pattern",
                "random",
                "--outstanding",
                "8",
                "--read-percent",
                "80",
                "--verify",
                "1",
            ]),
            1,
            100,
        )
        .unwrap();
        assert_eq!(
            (c.pattern, c.outstanding, c.read_share, c.verify, c.count, t),
            (Pattern::Random, 8, 52428, true, 100, 0)
        );
        let (c, _) = run_config(Op::Read, &s(&["--read-percent", "100"]), 1, 1).unwrap();
        assert_eq!(c.read_share, 65535);
        assert!(run_config(Op::Read, &s(&["--pattern", "zigzag"]), 1, 1).is_err());
        assert!(run_config(Op::Read, &s(&["--pattern"]), 1, 1).is_err());
        assert!(run_config(Op::Read, &s(&["--read-percent", "101"]), 1, 1).is_err());
        assert!(!run_config(Op::Read, &[], 1, 1).unwrap().0.width32);
        assert!(
            run_config(Op::Read, &s(&["--width", "32"]), 1, 1)
                .unwrap()
                .0
                .width32
        );
        assert!(
            !run_config(Op::Read, &s(&["--width", "64"]), 1, 1)
                .unwrap()
                .0
                .width32
        );
        assert!(run_config(Op::Read, &s(&["--width", "16"]), 1, 1).is_err());
    }

    #[test]
    fn commands_without_hardware() {
        assert!(
            run_command(&s(&["rate", "fill"]))
                .unwrap_err()
                .contains("rate needs")
        );
        assert!(run_command(&[]).is_err());
        assert!(
            run_command(&s(&["nothing"]))
                .unwrap_err()
                .starts_with("unknown command")
        );
        assert!(
            run_command(&s(&["latency", "fill"]))
                .unwrap_err()
                .contains("read or write")
        );
        assert!(
            run_command(&s(&["run", "zigzag"]))
                .unwrap_err()
                .contains("run needs")
        );
    }
}
