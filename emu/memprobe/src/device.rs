//! The memory path test design (rtl/memprobe) through its register window.
//! The register map is the block memprobe of spec/regmap.toml.

use std::thread;
use std::time::{Duration, Instant};

use plio::board::{self, BoardError, RegWindow};
use plio::regmap::memprobe as mp;
use plio::{Field, Reg};

/// Error of a measurement.
#[derive(Debug)]
pub enum Error {
    Board(BoardError),
    /// The design gave an unexpected value or state.
    Device(String),
    /// The bitstream does not have a feature that the measurement needs.
    Unsupported(String),
}

impl std::fmt::Display for Error {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Error::Board(e) => write!(f, "{e}"),
            Error::Device(s) => write!(f, "design: {s}"),
            Error::Unsupported(s) => write!(f, "not in this bitstream: {s}"),
        }
    }
}

impl std::error::Error for Error {}

impl From<BoardError> for Error {
    fn from(e: BoardError) -> Self {
        Error::Board(e)
    }
}

/// The features of the bitstream (register CAPS).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Caps {
    pub clk_mhz: u32,
    pub max_outstanding: u32,
    pub hp1: bool,
    pub size32: bool,
    pub mixed: bool,
    pub order: bool,
    pub patterns: bool,
}

impl Caps {
    pub fn from_reg(v: u32) -> Self {
        Caps {
            clk_mhz: mp::caps::CLK_MHZ.get(v),
            max_outstanding: mp::caps::MAX_OUTSTANDING.get(v),
            hp1: mp::caps::HP1.get(v) != 0,
            size32: mp::caps::SIZE32.get(v) != 0,
            mixed: mp::caps::MIXED.get(v) != 0,
            order: mp::caps::ORDER.get(v) != 0,
            patterns: mp::caps::PATTERNS.get(v) != 0,
        }
    }
}

/// Operation of a run (MODE.OP).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Op {
    Read,
    Write,
    Fill,
    Check,
}

impl Op {
    fn value(self) -> u32 {
        match self {
            Op::Read => mp::mode::op::READ,
            Op::Write => mp::mode::op::WRITE,
            Op::Fill => mp::mode::op::FILL,
            Op::Check => mp::mode::op::CHECK,
        }
    }

    pub fn name(self) -> &'static str {
        match self {
            Op::Read => "read",
            Op::Write => "write",
            Op::Fill => "fill",
            Op::Check => "check",
        }
    }

    pub fn parse(s: &str) -> Option<Op> {
        [Op::Read, Op::Write, Op::Fill, Op::Check]
            .into_iter()
            .find(|o| o.name() == s)
    }
}

/// The configuration of one run. The span is 2^span_log2 bytes from base, in
/// bytes from the start of the HP memory.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct RunConfig {
    pub op: Op,
    /// Beats in each burst: 1, 2, 4, 8 or 16.
    pub beats: u32,
    pub base: u32,
    pub span_log2: u32,
    pub count: u32,
    pub gap: u32,
    pub seed: u32,
}

impl RunConfig {
    /// The value of MODE: sequential addresses, 64-bit beats, HP0.
    pub fn mode(&self) -> Result<u32, Error> {
        let len = self.beats.wrapping_sub(1);
        if !matches!(self.beats, 1 | 2 | 4 | 8 | 16) {
            return Err(Error::Device(format!(
                "{} beats: use 1, 2, 4, 8 or 16",
                self.beats
            )));
        }
        let put = |reg: u32, f: Field, v: u32| {
            f.put(reg, v)
                .ok_or_else(|| Error::Device(format!("{v} too large")))
        };
        let mut m = 0;
        m = put(m, mp::mode::OP, self.op.value())?;
        m = put(m, mp::mode::PATTERN, mp::mode::pattern::SEQ)?;
        m = put(m, mp::mode::LEN, len)?;
        m = put(m, mp::mode::SIZE, mp::mode::size::B8)?;
        m = put(m, mp::mode::RD_PORT, mp::mode::rd_port::HP0)?;
        m = put(m, mp::mode::WR_PORT, mp::mode::wr_port::HP0)?;
        Ok(m)
    }

    /// The number of transactions of the run.
    pub fn transactions(&self) -> u64 {
        match self.op {
            Op::Fill | Op::Check => (1u64 << self.span_log2) / (u64::from(self.beats) * 8),
            Op::Read | Op::Write => u64::from(self.count),
        }
    }
}

/// One latency histogram with its statistics.
#[derive(Clone, Debug, Default)]
pub struct Hist {
    pub bins: Vec<u32>,
    pub min: u32,
    pub max: u32,
    pub sum: u64,
}

/// The results registers (they add up from CLEAR).
#[derive(Clone, Debug, Default)]
pub struct Results {
    pub run_cycles: u64,
    pub reads: u64,
    pub writes: u64,
    pub stalls: u64,
    pub data_errors: u32,
    pub first_err_addr: u32,
    pub first_err_data: u64,
    pub resp_errors: u32,
    pub hist_drops: u32,
    pub rd_first: Hist,
    pub rd_last: Hist,
    pub wr_resp: Hist,
}

/// The flags of STATUS after a run.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct RunStatus {
    pub resp_err: bool,
    pub data_err: bool,
    pub hist_drop: bool,
}

/// The design in the PL.
pub struct Memprobe {
    regs: RegWindow,
    pub caps: Caps,
    pub build: u32,
}

impl Memprobe {
    /// Check the PL and FCLK0, open the register window, and check the ID and
    /// the version of the register map.
    pub fn open() -> Result<Self, Error> {
        board::check_pl_ready()?;
        let regs = RegWindow::open("pl", mp::WINDOW_BYTES)?;
        let id = regs.read(mp::ID.offset);
        if id != mp::ID_VALUE {
            return Err(Error::Device(format!(
                "ID {id:#010x}, expected {:#010x}: another design is in the PL",
                mp::ID_VALUE
            )));
        }
        let version = regs.read(mp::VERSION.offset);
        if version != mp::VERSION_VALUE {
            return Err(Error::Device(format!(
                "register map version {version}, this program has {}",
                mp::VERSION_VALUE
            )));
        }
        let caps = Caps::from_reg(regs.read(mp::CAPS.offset));
        let build = regs.read(mp::BUILD.offset);
        Ok(Memprobe { regs, caps, build })
    }

    fn rd(&self, r: Reg) -> u32 {
        self.regs.read(r.offset)
    }

    fn wr(&self, r: Reg, v: u32) {
        self.regs.write(r.offset, v)
    }

    fn rd64(&self, lo: Reg, hi: Reg) -> u64 {
        u64::from(self.rd(lo)) | (u64::from(self.rd(hi)) << 32)
    }

    pub fn status(&self) -> u32 {
        self.rd(mp::STATUS)
    }

    pub fn mmcm_locked(&self) -> bool {
        mp::status::MMCM_LOCKED.get(self.status()) != 0
    }

    /// Wait until STATUS.BUSY is 0.
    fn wait_idle(&self, timeout: Duration) -> Result<u32, Error> {
        let t0 = Instant::now();
        let mut polls = 0u32;
        loop {
            let st = self.status();
            if mp::status::BUSY.get(st) == 0 {
                return Ok(st);
            }
            if t0.elapsed() > timeout {
                self.wr(mp::CTRL, mp::ctrl::ABORT.mask());
                return Err(Error::Device(format!(
                    "no end after {timeout:?}; ABORT written"
                )));
            }
            // Few reads of the register window during a run.
            polls += 1;
            if polls > 100 {
                thread::sleep(Duration::from_millis(1));
            }
        }
    }

    fn command(&self, bit: Field) -> Result<(), Error> {
        if !self.mmcm_locked() {
            return Err(Error::Device("the MMCM is not locked".into()));
        }
        let w1c = mp::status::DONE.mask()
            | mp::status::RESP_ERR.mask()
            | mp::status::CFG_ERR.mask()
            | mp::status::HIST_DROP.mask()
            | mp::status::DATA_ERR.mask();
        self.wr(mp::STATUS, w1c);
        self.wr(mp::CTRL, bit.mask());
        if mp::status::CFG_ERR.get(self.status()) != 0 {
            return Err(Error::Device(
                "the design refused the command (CFG_ERR)".into(),
            ));
        }
        Ok(())
    }

    /// CTRL.CLEAR: all results to 0.
    pub fn clear(&self) -> Result<(), Error> {
        self.command(mp::ctrl::CLEAR)?;
        self.wait_idle(Duration::from_secs(1))?;
        Ok(())
    }

    /// One run. The caller makes a barrier after its own writes to the
    /// region (plio::mmio::barrier), before this call.
    pub fn run(&self, cfg: &RunConfig, timeout: Duration) -> Result<RunStatus, Error> {
        let mode = cfg.mode()?;
        if self.caps.max_outstanding < 1 {
            return Err(Error::Unsupported("CAPS.MAX_OUTSTANDING is 0".into()));
        }
        self.wr(mp::MODE, mode);
        self.wr(mp::BASE, cfg.base);
        self.wr(mp::SPAN_LOG2, cfg.span_log2);
        self.wr(mp::COUNT, cfg.count);
        self.wr(mp::GAP, cfg.gap);
        self.wr(mp::OUTSTANDING, 1);
        self.wr(mp::SEED, cfg.seed);
        if self.rd(mp::MODE) != mode {
            return Err(Error::Device("MODE does not keep the value".into()));
        }
        plio::mmio::barrier();
        self.command(mp::ctrl::START)?;
        let st = self.wait_idle(timeout)?;
        if mp::status::DONE.get(st) == 0 {
            return Err(Error::Device(format!(
                "run ended without DONE, STATUS {st:#010x}"
            )));
        }
        Ok(RunStatus {
            resp_err: mp::status::RESP_ERR.get(st) != 0,
            data_err: mp::status::DATA_ERR.get(st) != 0,
            hist_drop: mp::status::HIST_DROP.get(st) != 0,
        })
    }

    fn hist(&self, mem: plio::Mem, min: Reg, max: Reg, sum_lo: Reg, sum_hi: Reg) -> Hist {
        let bins = (0..mem.depth)
            .map(|i| self.regs.read(mem.word_offset(i).unwrap_or(mem.offset)))
            .collect();
        Hist {
            bins,
            min: self.rd(min),
            max: self.rd(max),
            sum: self.rd64(sum_lo, sum_hi),
        }
    }

    /// All results. Read them only when no run is in progress.
    pub fn results(&self) -> Results {
        Results {
            run_cycles: self.rd64(mp::RUN_CYCLES_LO, mp::RUN_CYCLES_HI),
            reads: self.rd64(mp::READS_LO, mp::READS_HI),
            writes: self.rd64(mp::WRITES_LO, mp::WRITES_HI),
            stalls: self.rd64(mp::STALLS_LO, mp::STALLS_HI),
            data_errors: self.rd(mp::DATA_ERRORS),
            first_err_addr: self.rd(mp::FIRST_ERR_ADDR),
            first_err_data: self.rd64(mp::FIRST_ERR_DATA_LO, mp::FIRST_ERR_DATA_HI),
            resp_errors: self.rd(mp::RESP_ERRORS),
            hist_drops: self.rd(mp::HIST_DROPS),
            rd_first: self.hist(
                mp::HIST_RD_FIRST,
                mp::RD_FIRST_MIN,
                mp::RD_FIRST_MAX,
                mp::RD_FIRST_SUM_LO,
                mp::RD_FIRST_SUM_HI,
            ),
            rd_last: self.hist(
                mp::HIST_RD_LAST,
                mp::RD_LAST_MIN,
                mp::RD_LAST_MAX,
                mp::RD_LAST_SUM_LO,
                mp::RD_LAST_SUM_HI,
            ),
            wr_resp: self.hist(
                mp::HIST_WR_RESP,
                mp::WR_RESP_MIN,
                mp::WR_RESP_MAX,
                mp::WR_RESP_SUM_LO,
                mp::WR_RESP_SUM_HI,
            ),
        }
    }

    /// The frequencies of FCLK0 and of clk_m in Hz, measured with the
    /// monotonic clock of Linux over `dur`.
    pub fn clocks(&self, dur: Duration) -> (f64, f64) {
        let f0 = self.rd(mp::FCLK_COUNT);
        let m0 = self.rd(mp::CLKM_COUNT);
        let t0 = Instant::now();
        thread::sleep(dur);
        let f1 = self.rd(mp::FCLK_COUNT);
        let m1 = self.rd(mp::CLKM_COUNT);
        let t = t0.elapsed().as_secs_f64();
        (
            f64::from(f1.wrapping_sub(f0)) / t,
            f64::from(m1.wrapping_sub(m0)) / t,
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    // The expected MODE values come from the field table of docs/regmap.md,
    // computed by hand: OP in bits 2:0, LEN in 11:8, SIZE in 13:12.
    #[test]
    fn mode_values() {
        let mut c = RunConfig {
            op: Op::Read,
            beats: 16,
            base: 0,
            span_log2: 22,
            count: 1,
            gap: 0,
            seed: 1,
        };
        assert_eq!(c.mode().unwrap(), 0x0000_3F00);
        c.op = Op::Check;
        c.beats = 1;
        assert_eq!(c.mode().unwrap(), 0x0000_3005);
        c.beats = 3;
        assert!(c.mode().is_err());
        c.beats = 0;
        assert!(c.mode().is_err());
    }

    #[test]
    fn transactions() {
        let c = RunConfig {
            op: Op::Fill,
            beats: 16,
            base: 0,
            span_log2: 22,
            count: 5,
            gap: 0,
            seed: 1,
        };
        assert_eq!(c.transactions(), (4 << 20) / 128);
        let r = RunConfig { op: Op::Read, ..c };
        assert_eq!(r.transactions(), 5);
    }

    #[test]
    fn caps_fields() {
        // CLK_MHZ 7:0, MAX_OUTSTANDING 12:8, HP1 16 (docs/regmap.md).
        let c = Caps::from_reg(0x0001_0196);
        assert_eq!(c.clk_mhz, 150);
        assert_eq!(c.max_outstanding, 1);
        assert!(c.hp1 && !c.size32 && !c.mixed);
        assert_eq!(Op::parse("check"), Some(Op::Check));
        assert_eq!(Op::parse("x"), None);
    }
}
