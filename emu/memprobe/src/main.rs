//! memprobe: the PS program of the memory path test design
//! (docs/mem-path-experiment.md, Part B). It runs as root on the board, with
//! a memprobe bitstream in the PL.
//!
//!   memprobe info
//!   memprobe latency read|write [--beats N] [--count N] [--gap N] [--load-threads N]
//!   memprobe consistency [--words N] [--seed N]
//!   memprobe run read|write|fill|check [--beats N] [--count N] [--gap N]
//!                [--base N] [--span-log2 N] [--seed N]
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
mod stats;

use device::{Error, Memprobe, Op, RunConfig};

const USAGE: &str = "usage:
  memprobe info
  memprobe latency read|write [--beats N] [--count N] [--gap N] [--load-threads N]
  memprobe consistency [--words N] [--seed N]
  memprobe run read|write|fill|check [--beats N] [--count N] [--gap N] [--base N] [--span-log2 N] [--seed N]";

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
        "latency" => {
            let op = match args.get(1).map(String::as_str) {
                Some("read") => Op::Read,
                Some("write") => Op::Write,
                _ => return Err("latency needs read or write".into()),
            };
            let o = parse_options(&args[2..], &["beats", "count", "gap", "load-threads"])?;
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
            let o = parse_options(&args[1..], &["words", "seed"])?;
            let words = o.get("words").copied().unwrap_or(10_000_000);
            let seed = opt_u32(&o, "seed", 1)?;
            let dev = Memprobe::open().map_err(e)?;
            let region = Region::open(u64::from(mp::HP_BASE), mp::HP_SIZE as usize)
                .map_err(|x| x.to_string())?;
            let mut out = io::stdout().lock();
            header(&dev, &mut out).map_err(|e| e.to_string())?;
            let c = measure::consistency(&dev, &region, words, seed).map_err(e)?;
            writeln!(out, "direction,words,mismatches").map_err(|e| e.to_string())?;
            writeln!(out, "ps_to_pl,{},{}", c.ps_to_pl_words, c.ps_to_pl_errors)
                .map_err(|e| e.to_string())?;
            writeln!(out, "pl_to_ps,{},{}", c.pl_to_ps_words, c.pl_to_ps_errors)
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
                .ok_or("run needs read, write, fill or check")?;
            let o = parse_options(
                &args[2..],
                &["beats", "count", "gap", "base", "span-log2", "seed"],
            )?;
            let cfg = RunConfig {
                op,
                beats: opt_u32(&o, "beats", 16)?,
                base: opt_u32(&o, "base", 0)?,
                span_log2: opt_u32(&o, "span-log2", measure::SPAN_LOG2)?,
                count: opt_u32(&o, "count", 1)?,
                gap: opt_u32(&o, "gap", 0)?,
                seed: opt_u32(&o, "seed", 1)?,
            };
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
    fn commands_without_hardware() {
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
            run_command(&s(&["run", "mixed"]))
                .unwrap_err()
                .contains("run needs")
        );
    }
}
