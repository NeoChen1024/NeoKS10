//! The measurements of docs/mem-path-experiment.md, Part B, that the first
//! version of the design supports: B1 to B3 (latency) and B9 (consistency).
//! The output is CSV on the writer; lines that start with '#' are comments.

use std::io::Write;
use std::time::Duration;

use plio::board::Region;
use plio::regmap::memprobe as mp;

use crate::device::{Error, Hist, Memprobe, Op, RunConfig};
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
pub const LATENCY_HEADER: &str = "measure,op,beats,count,gap,clk_mhz,samples,\
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
        "{name},{},{},{},{},{clk_mhz},{},{},{},{},{},{:.2},{:.1},{},{},{:.1},{:.1}",
        cfg.op.name(),
        cfg.beats,
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
/// and one comment with the rate.
pub fn latency(
    dev: &Memprobe,
    op: Op,
    beats: u32,
    count: u32,
    gap: u32,
    out: &mut impl Write,
) -> Result<(), Error> {
    if !matches!(op, Op::Read | Op::Write) {
        return Err(Error::Device("latency needs read or write".into()));
    }
    let cfg = RunConfig {
        op,
        beats,
        base: 0,
        span_log2: SPAN_LOG2,
        count,
        gap,
        seed: 1,
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
    let bytes = done as f64 * f64::from(beats) * 8.0;
    writeln!(
        out,
        "# {} {count} x {beats} beats: {} cycles, {:.0} transactions/s, {:.1} MiB/s, {} stall cycles, {} samples dropped",
        op.name(),
        r.run_cycles,
        done as f64 / secs,
        bytes / secs / (1u64 << 20) as f64,
        r.stalls,
        r.hist_drops
    )
    .map_err(io)?;
    Ok(())
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
/// `words` 32-bit words went in each direction.
pub fn consistency(
    dev: &Memprobe,
    region: &Region,
    words: u64,
    seed: u32,
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
            op: Op::Check,
            beats: 16,
            base: 0,
            span_log2: SPAN_LOG2,
            count: 0,
            gap: 0,
            seed: sa,
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
        let cfg = RunConfig {
            op: Op::Read,
            beats: 1,
            base: 0,
            span_log2: 22,
            count: 4,
            gap: 0,
            seed: 1,
        };
        let h = Hist {
            bins: vec![0, 2, 1, 1],
            min: 1,
            max: 1200,
            sum: 1204,
        };
        let row = latency_row("rd_first", &cfg, 100, &h, 4).unwrap();
        assert_eq!(
            row,
            "rd_first,read,1,4,0,100,4,1,1,>=3,1200,301.00,10.0,10.0,>=30.0,12000.0,3010.0"
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
    fn time_limit() {
        assert_eq!(timeout(0), Duration::from_secs(1));
        assert_eq!(timeout(10_000), Duration::from_secs(2));
    }
}
