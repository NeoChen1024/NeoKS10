//! The memory path test design (rtl/memprobe) through its register window.
//! The register map is the block memprobe of spec/regmap.toml (version 2).

use std::thread;
use std::time::{Duration, Instant};

use plio::afi::Afi;
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
    /// Bit n: a master on S_AXI_HPn.
    pub ports: u32,
    pub size32: bool,
    pub mixed: bool,
    pub order: bool,
    pub patterns: bool,
    pub verify: bool,
}

impl Caps {
    pub fn from_reg(v: u32) -> Self {
        Caps {
            clk_mhz: mp::caps::CLK_MHZ.get(v),
            max_outstanding: mp::caps::MAX_OUTSTANDING.get(v),
            ports: mp::caps::PORTS.get(v),
            size32: mp::caps::SIZE32.get(v) != 0,
            mixed: mp::caps::MIXED.get(v) != 0,
            order: mp::caps::ORDER.get(v) != 0,
            patterns: mp::caps::PATTERNS.get(v) != 0,
            verify: mp::caps::VERIFY.get(v) != 0,
        }
    }
}

/// Operation of a run (MODE.OP).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Op {
    Read,
    Write,
    Mixed,
    Order,
    Fill,
    Check,
}

impl Op {
    const ALL: [Op; 6] = [
        Op::Read,
        Op::Write,
        Op::Mixed,
        Op::Order,
        Op::Fill,
        Op::Check,
    ];

    fn value(self) -> u32 {
        match self {
            Op::Read => mp::mode::op::READ,
            Op::Write => mp::mode::op::WRITE,
            Op::Mixed => mp::mode::op::MIXED,
            Op::Order => mp::mode::op::ORDER,
            Op::Fill => mp::mode::op::FILL,
            Op::Check => mp::mode::op::CHECK,
        }
    }

    pub fn name(self) -> &'static str {
        match self {
            Op::Read => "read",
            Op::Write => "write",
            Op::Mixed => "mixed",
            Op::Order => "order",
            Op::Fill => "fill",
            Op::Check => "check",
        }
    }

    pub fn parse(s: &str) -> Option<Op> {
        Op::ALL.into_iter().find(|o| o.name() == s)
    }
}

/// Address sequence of a run (MODE.PATTERN).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Pattern {
    Seq,
    Random,
    Fixed,
}

impl Pattern {
    const ALL: [Pattern; 3] = [Pattern::Seq, Pattern::Random, Pattern::Fixed];

    fn value(self) -> u32 {
        match self {
            Pattern::Seq => mp::mode::pattern::SEQ,
            Pattern::Random => mp::mode::pattern::RANDOM,
            Pattern::Fixed => mp::mode::pattern::FIXED,
        }
    }

    pub fn name(self) -> &'static str {
        match self {
            Pattern::Seq => "seq",
            Pattern::Random => "random",
            Pattern::Fixed => "fixed",
        }
    }

    pub fn parse(s: &str) -> Option<Pattern> {
        Pattern::ALL.into_iter().find(|p| p.name() == s)
    }
}

/// The configuration of one run. The span is 2^span_log2 bytes from base, in
/// bytes from the start of the HP memory.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct RunConfig {
    pub op: Op,
    pub pattern: Pattern,
    /// Transactions in progress at most.
    pub outstanding: u32,
    /// MIXED: share of reads in units of 1/65536.
    pub read_share: u32,
    /// READ and MIXED compare the data with the pattern of `seed`.
    pub verify: bool,
    /// HP port of the reads and of the writes: 0 to 3.
    pub rd_port: u32,
    pub wr_port: u32,
    /// ORDER: the read goes before the write.
    pub order_reverse: bool,
    /// Beats in each burst: 1, 2, 4, 8 or 16.
    pub beats: u32,
    /// 32-bit beats (MODE.SIZE B4, the 32-bit mode of the AFI), else 64-bit.
    pub width32: bool,
    pub base: u32,
    pub span_log2: u32,
    pub count: u32,
    pub gap: u32,
    pub seed: u32,
}

impl RunConfig {
    /// A run of `op` with sequential addresses, one transaction in progress,
    /// and no check of reads.
    pub fn simple(op: Op, beats: u32, span_log2: u32, count: u32, gap: u32, seed: u32) -> Self {
        RunConfig {
            op,
            pattern: Pattern::Seq,
            outstanding: 1,
            read_share: 0x8000,
            verify: false,
            rd_port: 0,
            wr_port: 0,
            order_reverse: false,
            beats,
            width32: false,
            base: 0,
            span_log2,
            count,
            gap,
            seed,
        }
    }

    /// Bytes in each beat: 4 or 8.
    pub fn beat_bytes(&self) -> u32 {
        if self.width32 { 4 } else { 8 }
    }

    /// The value of MODE.
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
        m = put(m, mp::mode::PATTERN, self.pattern.value())?;
        m = put(m, mp::mode::LEN, len)?;
        let size = if self.width32 {
            mp::mode::size::B4
        } else {
            mp::mode::size::B8
        };
        m = put(m, mp::mode::SIZE, size)?;
        m = put(m, mp::mode::RD_PORT, self.rd_port)?;
        m = put(m, mp::mode::WR_PORT, self.wr_port)?;
        m = put(m, mp::mode::VERIFY, u32::from(self.verify))?;
        m = put(m, mp::mode::ORDER_REVERSE, u32::from(self.order_reverse))?;
        Ok(m)
    }

    /// The number of transactions of the run.
    pub fn transactions(&self) -> u64 {
        match self.op {
            Op::Fill | Op::Check => {
                (1u64 << self.span_log2) / (u64::from(self.beats) * u64::from(self.beat_bytes()))
            }
            Op::Read | Op::Write | Op::Mixed => u64::from(self.count),
            Op::Order => 2 * u64::from(self.count),
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
    pub order_mismatches: u32,
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
    afi: Afi,
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
        let afi = Afi::open().map_err(|e| BoardError::Io("map of the AFI registers".into(), e))?;
        Ok(Memprobe {
            regs,
            afi,
            caps,
            build,
        })
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
                // Wait for the end of the transactions after ABORT, so that
                // the caller can change the AFI after the error.
                self.wr(mp::CTRL, mp::ctrl::ABORT.mask());
                let t1 = Instant::now();
                while mp::status::BUSY.get(self.status()) != 0 {
                    if t1.elapsed() > Duration::from_secs(1) {
                        return Err(Error::Device(format!(
                            "no end after {timeout:?}, and BUSY 1 s after ABORT: load the PL again"
                        )));
                    }
                    thread::sleep(Duration::from_millis(1));
                }
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

    /// Refuse a configuration that CAPS of the bitstream does not allow.
    pub fn supports(&self, cfg: &RunConfig) -> Result<(), Error> {
        check_caps(&self.caps, cfg)
    }

    /// `32BitEn` of the AFI of each HP port: (read channel, write channel).
    pub fn afi_state(&self) -> Result<[(bool, bool); plio::afi::PORTS], Error> {
        self.afi
            .state()
            .map_err(|e| Error::Board(BoardError::Io("read of the AFI".into(), e)))
    }

    /// Set the AFI of all ports to 64-bit data. Only while no run is in
    /// progress.
    pub fn afi_clear(&self) -> Result<(), Error> {
        if mp::status::BUSY.get(self.status()) != 0 {
            return Err(Error::Device("a run is in progress".into()));
        }
        for port in 0..plio::afi::PORTS {
            for ch in [plio::afi::Channel::Read, plio::afi::Channel::Write] {
                self.afi
                    .set_32bit(port, ch, false)
                    .map_err(|e| Error::Board(BoardError::Io("write of the AFI".into(), e)))?;
            }
        }
        Ok(())
    }

    /// One run. The caller makes a barrier after its own writes to the
    /// region (plio::mmio::barrier), before this call.
    ///
    /// The AFI of RD_PORT (read channel) and WR_PORT (write channel) gets the
    /// width of MODE.SIZE before START, for 32-bit and for 64-bit runs: an
    /// earlier program that stopped can have left 32BitEn set. After the run,
    /// the earlier values come back.
    pub fn run(&self, cfg: &RunConfig, timeout: Duration) -> Result<RunStatus, Error> {
        let mode = cfg.mode()?;
        self.supports(cfg)?;
        if mp::status::BUSY.get(self.status()) != 0 {
            return Err(Error::Device("a run is in progress".into()));
        }
        let _width = self
            .afi
            .set_width(cfg.rd_port as usize, cfg.wr_port as usize, cfg.width32)
            .map_err(|e| Error::Board(BoardError::Io("AFI width".into(), e)))?;
        self.wr(mp::MODE, mode);
        self.wr(mp::BASE, cfg.base);
        self.wr(mp::SPAN_LOG2, cfg.span_log2);
        self.wr(mp::COUNT, cfg.count);
        self.wr(mp::GAP, cfg.gap);
        self.wr(mp::OUTSTANDING, cfg.outstanding);
        self.wr(mp::READ_SHARE, cfg.read_share);
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
            order_mismatches: self.rd(mp::ORDER_MISMATCHES),
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

/// The check of [`Memprobe::supports`], without the hardware.
pub fn check_caps(c: &Caps, cfg: &RunConfig) -> Result<(), Error> {
    if cfg.outstanding < 1 || cfg.outstanding > c.max_outstanding {
        return Err(Error::Unsupported(format!(
            "{} transactions in progress; CAPS.MAX_OUTSTANDING is {}",
            cfg.outstanding, c.max_outstanding
        )));
    }
    if cfg.op == Op::Mixed && !c.mixed {
        return Err(Error::Unsupported("MIXED".into()));
    }
    if cfg.op == Op::Order && !c.order {
        return Err(Error::Unsupported("ORDER".into()));
    }
    if cfg.pattern != Pattern::Seq && !c.patterns {
        return Err(Error::Unsupported(format!(
            "pattern {}",
            cfg.pattern.name()
        )));
    }
    if cfg.verify && !c.verify {
        return Err(Error::Unsupported("VERIFY".into()));
    }
    if cfg.width32 && !c.size32 {
        return Err(Error::Unsupported("32-bit beats (SIZE32)".into()));
    }
    for (what, p) in [("RD_PORT", cfg.rd_port), ("WR_PORT", cfg.wr_port)] {
        if p > 3 || c.ports & (1 << p) == 0 {
            return Err(Error::Unsupported(format!(
                "{what} HP{p}; CAPS.PORTS is {:#x}",
                c.ports
            )));
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    // The expected MODE values come from the field table of docs/regmap.md,
    // computed by hand: OP in bits 2:0, PATTERN 5:4, LEN 11:8, SIZE 13:12,
    // VERIFY 20.
    #[test]
    fn mode_values() {
        let mut c = RunConfig::simple(Op::Read, 16, 22, 1, 0, 1);
        assert_eq!(c.mode().unwrap(), 0x0000_3F00);
        c.op = Op::Check;
        c.beats = 1;
        assert_eq!(c.mode().unwrap(), 0x0000_3005);
        let m = RunConfig {
            op: Op::Mixed,
            pattern: Pattern::Random,
            verify: true,
            ..c
        };
        assert_eq!(m.mode().unwrap(), 0x0010_3012);
        c.beats = 3;
        assert!(c.mode().is_err());
        c.beats = 0;
        assert!(c.mode().is_err());
    }

    #[test]
    fn transactions() {
        let c = RunConfig::simple(Op::Fill, 16, 22, 5, 0, 1);
        assert_eq!(c.transactions(), (4 << 20) / 128);
        let r = RunConfig { op: Op::Mixed, ..c };
        assert_eq!(r.transactions(), 5);
    }

    #[test]
    fn width32() {
        // SIZE 13:12: B4 = 2, B8 = 3 (docs/regmap.md).
        let c = RunConfig {
            width32: true,
            ..RunConfig::simple(Op::Read, 16, 22, 1, 0, 1)
        };
        assert_eq!(c.mode().unwrap(), 0x0000_2F00);
        assert_eq!(c.beat_bytes(), 4);
        let f = RunConfig { op: Op::Fill, ..c };
        assert_eq!(f.transactions(), (4 << 20) / 64);
        let v2b = Caps::from_reg(0x0F3C_1064); // M2b: no SIZE32
        let v2c = Caps::from_reg(0x0F3E_1064); // M2c: SIZE32 (bit 17)
        assert!(!v2b.size32 && v2c.size32);
        assert!(check_caps(&v2b, &c).is_err());
        assert!(check_caps(&v2c, &c).is_ok());
        assert!(
            check_caps(
                &v2b,
                &RunConfig {
                    width32: false,
                    ..c
                }
            )
            .is_ok()
        );
    }

    #[test]
    fn caps_fields() {
        // CLK_MHZ 7:0, MAX_OUTSTANDING 12:8, MIXED 18, PATTERNS 20, VERIFY 21,
        // PORTS 27:24 (docs/regmap.md).
        let c = Caps::from_reg(0x0134_1096);
        assert_eq!(c.clk_mhz, 150);
        assert_eq!(c.max_outstanding, 16);
        assert_eq!(c.ports, 1);
        assert!(c.mixed && c.patterns && c.verify && !c.size32 && !c.order);
        assert_eq!(Op::parse("mixed"), Some(Op::Mixed));
        assert_eq!(Op::parse("x"), None);
        assert_eq!(Pattern::parse("random"), Some(Pattern::Random));
    }

    #[test]
    fn caps_refuse() {
        let m1 = Caps::from_reg(0x0100_0164); // the M1 bitstream: 100 MHz, 1 in progress, HP0
        let v2 = Caps::from_reg(0x0134_1064);
        let simple = RunConfig::simple(Op::Read, 1, 22, 10, 0, 1);
        assert!(check_caps(&m1, &simple).is_ok());
        let many = RunConfig {
            outstanding: 8,
            ..simple
        };
        assert!(check_caps(&m1, &many).is_err());
        assert!(check_caps(&v2, &many).is_ok());
        assert!(
            check_caps(
                &v2,
                &RunConfig {
                    outstanding: 17,
                    ..simple
                }
            )
            .is_err()
        );
        assert!(
            check_caps(
                &m1,
                &RunConfig {
                    op: Op::Mixed,
                    ..simple
                }
            )
            .is_err()
        );
        assert!(
            check_caps(
                &m1,
                &RunConfig {
                    pattern: Pattern::Random,
                    ..simple
                }
            )
            .is_err()
        );
        assert!(
            check_caps(
                &m1,
                &RunConfig {
                    verify: true,
                    ..simple
                }
            )
            .is_err()
        );
    }

    #[test]
    fn ports_and_order() {
        // ORDER 3 in bits 2:0, RD_PORT 17:16, WR_PORT 19:18, ORDER_REVERSE 21.
        let base = RunConfig::simple(Op::Order, 1, 22, 1, 0, 1);
        let o = RunConfig {
            rd_port: 2,
            wr_port: 1,
            order_reverse: true,
            ..base
        };
        assert_eq!(o.mode().unwrap(), 0x0026_3003);
        assert_eq!(o.transactions(), 2);
        assert!(RunConfig { rd_port: 4, ..base }.mode().is_err());
        let v2a = Caps::from_reg(0x0134_1064); // M2a: HP0 only, no ORDER
        let v2b = Caps::from_reg(0x0F3C_1064); // M2b: HP0 to HP3, ORDER
        assert!(check_caps(&v2a, &o).is_err());
        assert!(check_caps(&v2b, &o).is_ok());
        let read = RunConfig::simple(Op::Read, 1, 22, 1, 0, 1);
        assert!(check_caps(&v2a, &RunConfig { rd_port: 2, ..read }).is_err());
        assert!(
            check_caps(
                &v2b,
                &RunConfig {
                    rd_port: 3,
                    wr_port: 2,
                    ..read
                }
            )
            .is_ok()
        );
    }
}
