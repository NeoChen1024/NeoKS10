//! The measurements of docs/mem-path-experiment.md, Part B, that the first
//! version of the design supports: B1 to B3 (latency) and B9 (consistency).
//! The output is CSV on the writer; lines that start with '#' are comments.

use std::io::Write;
use std::time::Duration;

use plio::board::Region;
use plio::regmap::memprobe as mp;

use crate::device::{Error, Hist, Memprobe, Op, Results, RunConfig};
use crate::pattern;
use crate::stats::{self, Latency};

/// The tests use the first 4 MiB of the reserved region: the size of the
/// guest memory, 512 Ki words of 8 bytes (docs/mem-path-experiment.md).
pub const SPAN_LOG2: u32 = 22;

fn io(e: std::io::Error) -> Error {
    Error::Device(format!("output: {e}"))
}

/// A time limit for a run: 1 s, plus 100 us for each transaction.
fn timeout(transactions: u64) -> Duration {
    Duration::from_secs(1)
        + Duration::from_micros(100).saturating_mul(transactions.min(u64::from(u32::MAX)) as u32)
}

/// The CSV header of [`latency`].
pub const LATENCY_HEADER: &str = "measure,op,beats,width,count,gap,clk_mhz,samples,\
min_cyc,median_cyc,p99_cyc,max_cyc,mean_cyc,min_ns,median_ns,p99_ns,max_ns,mean_ns";

fn ns(cycles: f64, clk_mhz: u32) -> f64 {
    cycles * 1000.0 / f64::from(clk_mhz)
}

fn lat_ns(l: Latency, clk_mhz: u32) -> String {
    match l {
        Latency::Exact(n) => format!("{:.1}", ns(f64::from(n), clk_mhz)),
        Latency::AtLeast(n) => format!(">={:.1}", ns(f64::from(n), clk_mhz)),
    }
}

fn latency_row(
    name: &str,
    cfg: &RunConfig,
    clk_mhz: u32,
    h: &Hist,
    samples: u64,
) -> Option<String> {
    let s = stats::summarize(&h.bins, h.min, h.max, h.sum, samples)?;
    Some(format!(
        "{name},{},{},{},{},{},{clk_mhz},{},{},{},{},{},{:.2},{:.1},{},{},{:.1},{:.1}",
        cfg.op.name(),
        cfg.beats,
        8 * cfg.beat_bytes(),
        cfg.count,
        cfg.gap,
        s.count,
        s.min,
        s.median,
        s.p99,
        s.max,
        s.mean,
        ns(f64::from(s.min), clk_mhz),
        lat_ns(s.median, clk_mhz),
        lat_ns(s.p99, clk_mhz),
        ns(f64::from(s.max), clk_mhz),
        ns(s.mean, clk_mhz),
    ))
}

/// B1, B2, B3: `count` bursts of `beats` beats, one at a time, with `gap`
/// cycles at least from one address handshake to the next issue. Writes one
/// CSV row for each latency (read: first beat and last beat; write: response),
/// and one comment with the rate. `width32`: 32-bit beats.
pub fn latency(
    dev: &Memprobe,
    op: Op,
    beats: u32,
    width32: bool,
    count: u32,
    gap: u32,
    out: &mut impl Write,
) -> Result<(), Error> {
    if !matches!(op, Op::Read | Op::Write) {
        return Err(Error::Device("latency needs read or write".into()));
    }
    let cfg = RunConfig {
        width32,
        ..RunConfig::simple(op, beats, SPAN_LOG2, count, gap, 1)
    };
    dev.clear()?;
    let st = dev.run(&cfg, timeout(cfg.transactions()))?;
    let r = dev.results();
    if st.resp_err || r.resp_errors != 0 {
        return Err(Error::Device(format!(
            "{} responses were not OKAY",
            r.resp_errors
        )));
    }
    let done = if op == Op::Read { r.reads } else { r.writes };
    if done != u64::from(count) {
        return Err(Error::Device(format!(
            "{done} transactions, expected {count}"
        )));
    }
    let clk = dev.caps.clk_mhz;
    let rows = match op {
        Op::Read => vec![
            latency_row("rd_first", &cfg, clk, &r.rd_first, r.reads),
            latency_row("rd_last", &cfg, clk, &r.rd_last, r.reads),
        ],
        _ => vec![latency_row("wr_resp", &cfg, clk, &r.wr_resp, r.writes)],
    };
    for row in rows.into_iter().flatten() {
        writeln!(out, "{row}").map_err(io)?;
    }
    let secs = r.run_cycles as f64 / (f64::from(clk) * 1e6);
    let bytes = done as f64 * f64::from(beats) * f64::from(cfg.beat_bytes());
    writeln!(
        out,
        "# {} {count} x {beats} beats of {} bits: {} cycles, {:.0} transactions/s, {:.1} MiB/s, {} stall cycles, {} samples dropped",
        op.name(),
        8 * cfg.beat_bytes(),
        r.run_cycles,
        done as f64 / secs,
        bytes / secs / (1u64 << 20) as f64,
        r.stalls,
        r.hist_drops
    )
    .map_err(io)?;
    Ok(())
}

/// The CSV header of [`rate`].
pub const RATE_HEADER: &str = "op,pattern,beats,width,count,outstanding,read_share,verify,gap,clk_mhz,\
reads,writes,cycles,transactions_per_s,mib_per_s,stall_share,\
rd_last_median_cyc,rd_last_p99_cyc,rd_last_max_cyc,wr_resp_median_cyc,wr_resp_p99_cyc,wr_resp_max_cyc,\
data_errors,samples_dropped";

fn quantiles(h: &Hist, samples: u64) -> (String, String, String) {
    match stats::summarize(&h.bins, h.min, h.max, h.sum, samples) {
        Some(s) => (s.median.to_string(), s.p99.to_string(), s.max.to_string()),
        None => ("".into(), "".into(), "".into()),
    }
}

/// One CSV row of [`rate`].
fn rate_row(cfg: &RunConfig, clk_mhz: u32, r: &Results) -> String {
    let secs = r.run_cycles as f64 / (f64::from(clk_mhz) * 1e6);
    let done = r.reads + r.writes;
    let bytes = done as f64 * f64::from(cfg.beats) * f64::from(cfg.beat_bytes());
    let (rm, rp, rx) = quantiles(&r.rd_last, r.reads);
    let (wm, wp, wx) = quantiles(&r.wr_resp, r.writes);
    format!(
        "{},{},{},{},{},{},{},{},{},{clk_mhz},{},{},{},{:.0},{:.1},{:.3},{rm},{rp},{rx},{wm},{wp},{wx},{},{}",
        cfg.op.name(),
        cfg.pattern.name(),
        cfg.beats,
        8 * cfg.beat_bytes(),
        cfg.count,
        cfg.outstanding,
        cfg.read_share,
        u8::from(cfg.verify),
        cfg.gap,
        r.reads,
        r.writes,
        r.run_cycles,
        done as f64 / secs,
        bytes / secs / f64::from(1u32 << 20),
        r.stalls as f64 / r.run_cycles.max(1) as f64,
        r.data_errors,
        r.hist_drops,
    )
}

/// B4, B5: a run of many transactions in progress in the first 4 MiB. With
/// `verify`, the PL first fills the span with the pattern of the seed, and the
/// reads compare their data with it (the writes write the same pattern).
/// Writes one CSV row.
pub fn rate(dev: &Memprobe, cfg: &RunConfig, out: &mut impl Write) -> Result<Results, Error> {
    dev.supports(cfg)?;
    if cfg.verify {
        let fill = RunConfig {
            base: cfg.base,
            ..RunConfig::simple(Op::Fill, 16, cfg.span_log2, 0, 0, cfg.seed)
        };
        dev.clear()?;
        dev.run(&fill, timeout(fill.transactions()))?;
    }
    dev.clear()?;
    let st = dev.run(cfg, timeout(cfg.transactions()))?;
    let r = dev.results();
    if st.resp_err || r.resp_errors != 0 {
        return Err(Error::Device(format!(
            "{} responses were not OKAY",
            r.resp_errors
        )));
    }
    if r.reads + r.writes != cfg.transactions() {
        return Err(Error::Device(format!(
            "{} transactions, expected {}",
            r.reads + r.writes,
            cfg.transactions()
        )));
    }
    writeln!(out, "{}", rate_row(cfg, dev.caps.clk_mhz, &r)).map_err(io)?;
    Ok(r)
}

/// The CSV header of [`order`].
pub const ORDER_HEADER: &str = "pattern,beats,width,count,gap,rd_port,wr_port,reverse,clk_mhz,\
mismatches,mismatch_share,rd_last_median_cyc,rd_last_p99_cyc,wr_resp_median_cyc,wr_resp_p99_cyc";

/// One CSV row of [`order`].
fn order_row(cfg: &RunConfig, clk_mhz: u32, r: &Results) -> String {
    let (rm, rp, _) = quantiles(&r.rd_last, r.reads);
    let (wm, wp, _) = quantiles(&r.wr_resp, r.writes);
    format!(
        "{},{},{},{},{},{},{},{},{clk_mhz},{},{:.4},{rm},{rp},{wm},{wp}",
        cfg.pattern.name(),
        cfg.beats,
        8 * cfg.beat_bytes(),
        cfg.count,
        cfg.gap,
        cfg.rd_port,
        cfg.wr_port,
        u8::from(cfg.order_reverse),
        r.order_mismatches,
        f64::from(r.order_mismatches) / f64::from(cfg.count.max(1)),
    )
}

/// B6: COUNT iterations of a write and a read of the same address; GAP
/// cycles from the WLAST handshake to the ARVALID (ORDER_REVERSE: the read
/// first). Writes one CSV row.
pub fn order(dev: &Memprobe, cfg: &RunConfig, out: &mut impl Write) -> Result<Results, Error> {
    if cfg.op != Op::Order {
        return Err(Error::Device("order needs the operation ORDER".into()));
    }
    dev.supports(cfg)?;
    dev.clear()?;
    let st = dev.run(cfg, timeout(cfg.transactions()))?;
    let r = dev.results();
    if st.resp_err || r.resp_errors != 0 {
        return Err(Error::Device(format!(
            "{} responses were not OKAY",
            r.resp_errors
        )));
    }
    if r.reads != u64::from(cfg.count) || r.writes != u64::from(cfg.count) {
        return Err(Error::Device(format!(
            "{} reads and {} writes, expected {} of each",
            r.reads, r.writes, cfg.count
        )));
    }
    writeln!(out, "{}", order_row(cfg, dev.caps.clk_mhz, &r)).map_err(io)?;
    Ok(r)
}

/// The result of [`consistency`].
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Consistency {
    pub ps_to_pl_words: u64,
    pub ps_to_pl_errors: u64,
    pub pl_to_ps_words: u64,
    pub pl_to_ps_errors: u64,
}

/// B9: the PS writes the pattern into the first 4 MiB and the PL checks it;
/// then the PL fills it and the PS checks it. Repeats with new seeds until
/// `words` 32-bit words went in each direction. The PL uses bursts of
/// `beats` beats of 32 bits (`width32`) or 64 bits.
pub fn consistency(
    dev: &Memprobe,
    region: &Region,
    words: u64,
    seed: u32,
    beats: u32,
    width32: bool,
) -> Result<Consistency, Error> {
    if region.base != u64::from(mp::HP_BASE) {
        return Err(Error::Device(
            "the region is not the HP memory of the design".into(),
        ));
    }
    let span = 1usize << SPAN_LOG2;
    let per_pass = (span / 4) as u64;
    let passes = words.div_ceil(per_pass).max(1);
    let mut c = Consistency::default();
    let mut s = seed;
    let mut next_seed = || {
        s = s.wrapping_add(1);
        if s == 0 {
            s = 1;
        }
        s
    };
    for _ in 0..passes {
        // PS to PL.
        let sa = next_seed();
        for off in (0..span).step_by(4) {
            region.write32(off, pattern::word(sa, off as u32));
        }
        plio::mmio::barrier();
        dev.clear()?;
        let cfg = RunConfig {
            width32,
            ..RunConfig::simple(Op::Check, beats, SPAN_LOG2, 0, 0, sa)
        };
        dev.run(&cfg, timeout(cfg.transactions()))?;
        let r = dev.results();
        if r.resp_errors != 0 {
            return Err(Error::Device(format!(
                "{} responses were not OKAY",
                r.resp_errors
            )));
        }
        c.ps_to_pl_words += per_pass;
        c.ps_to_pl_errors += u64::from(r.data_errors);

        // PL to PS.
        let sb = next_seed();
        let cfg = RunConfig {
            op: Op::Fill,
            seed: sb,
            ..cfg
        };
        dev.run(&cfg, timeout(cfg.transactions()))?;
        if dev.results().resp_errors != 0 {
            return Err(Error::Device("a response was not OKAY".into()));
        }
        plio::mmio::barrier();
        for off in (0..span).step_by(4) {
            if region.read32(off) != pattern::word(sb, off as u32) {
                c.pl_to_ps_errors += 1;
            }
        }
        c.pl_to_ps_words += per_pass;
    }
    Ok(c)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rows() {
        let cfg = RunConfig::simple(Op::Read, 1, 22, 4, 0, 1);
        let h = Hist {
            bins: vec![0, 2, 1, 1],
            min: 1,
            max: 1200,
            sum: 1204,
        };
        let row = latency_row("rd_first", &cfg, 100, &h, 4).unwrap();
        assert_eq!(
            row,
            "rd_first,read,1,64,4,0,100,4,1,1,>=3,1200,301.00,10.0,10.0,>=30.0,12000.0,3010.0"
        );
        assert_eq!(LATENCY_HEADER.split(',').count(), row.split(',').count());
        assert!(
            latency_row(
                "x",
                &cfg,
                100,
                &Hist {
                    bins: vec![0; 4],
                    ..h
                },
                0
            )
            .is_none()
        );
    }

    #[test]
    fn rate_rows() {
        let cfg = RunConfig {
            outstanding: 8,
            ..RunConfig::simple(Op::Mixed, 2, 22, 100, 0, 1)
        };
        let mut r = Results {
            run_cycles: 1000,
            reads: 60,
            writes: 40,
            stalls: 250,
            ..Results::default()
        };
        r.rd_last = Hist {
            bins: vec![0, 0, 60, 0],
            min: 2,
            max: 2,
            sum: 120,
        };
        r.wr_resp = Hist {
            bins: vec![0, 40, 0, 0],
            min: 1,
            max: 1,
            sum: 40,
        };
        // 100 transactions in 10 us at 100 MHz: 10 M/s; 100 * 16 bytes in 10 us.
        let row = rate_row(&cfg, 100, &r);
        assert_eq!(
            row,
            "mixed,seq,2,64,100,8,32768,0,0,100,60,40,1000,10000000,152.6,0.250,2,2,2,1,1,1,0,0"
        );
        assert_eq!(RATE_HEADER.split(',').count(), row.split(',').count());
        // 32-bit beats: 100 * 8 bytes in 10 us.
        let row = rate_row(
            &RunConfig {
                width32: true,
                ..cfg
            },
            100,
            &r,
        );
        assert_eq!(
            row,
            "mixed,seq,2,32,100,8,32768,0,0,100,60,40,1000,10000000,76.3,0.250,2,2,2,1,1,1,0,0"
        );
    }

    #[test]
    fn order_rows() {
        let cfg = RunConfig {
            rd_port: 2,
            gap: 3,
            order_reverse: true,
            ..RunConfig::simple(Op::Order, 1, 22, 200, 3, 1)
        };
        let mut r = Results {
            reads: 200,
            writes: 200,
            order_mismatches: 50,
            ..Results::default()
        };
        r.rd_last = Hist {
            bins: vec![0, 200, 0],
            min: 1,
            max: 1,
            sum: 200,
        };
        r.wr_resp = Hist {
            bins: vec![0, 0, 200, 0],
            min: 2,
            max: 2,
            sum: 400,
        };
        let row = order_row(&cfg, 100, &r);
        assert_eq!(row, "seq,1,64,200,3,2,0,1,100,50,0.2500,1,1,2,2");
        assert_eq!(ORDER_HEADER.split(',').count(), row.split(',').count());
    }

    #[test]
    fn time_limit() {
        assert_eq!(timeout(0), Duration::from_secs(1));
        assert_eq!(timeout(10_000), Duration::from_secs(2));
    }
}
