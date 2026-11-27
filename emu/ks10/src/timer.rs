//! The time base, the interval timer, the ITS quantum counter and the PC
//! sample table **[microcode]** (inout.50, "TIMER CONTROL": TICK, TOCK;
//! "WRTIME & RDTIME"; "WRINT & RDINT"; itspag.98, PF-INT and the .IF/PCST
//! part).
//!
//! The hardware has a 1 ms clock. At each tick the microcode (TOCK) adds
//! 2^12 to the time base, adds 2^12 to the quantum counter if no PI level
//! is in progress, and counts the interval down by 2^12; when it reaches 0
//! or less, the period is added back and the APR flag "interval done" is
//! set. The low 12 bits of the time base are a hardware counter that runs
//! inside the millisecond (RDTIME).
//!
//! Virtual time: one millisecond is [`INSTRUCTIONS_PER_MS`] completed
//! instructions (docs/emulator.md, "Time").

use pdp10::{Half, W36};

use crate::apr::flag;
use crate::machine::Machine;

/// **[estimate]**: 1 µs for each instruction, the default of
/// docs/emulator.md ("Time"). The KS10 instruction times are not known
/// yet (docs/unknowns.md).
pub const INSTRUCTIONS_PER_MS: u64 = 1000;

/// Units of the time base in one millisecond (TOCK: "2^12 UNITS PER MS").
const UNITS_PER_MS: u64 = 1 << 12;

const MASK36: u64 = 0o777777_777777;

#[derive(Clone, Copy, Debug, Default)]
pub struct Timer {
    /// WORK[TIME0], WORK[TIME1]: the time base without the hardware part.
    pub time0: W36,
    pub time1: W36,
    /// WORK[PERIOD] and WORK[TTG] (time to go).
    pub period: W36,
    pub ttg: W36,
    /// WORK[QUAN], the ITS quantum counter (SPM, LPMR).
    pub quan: W36,
    /// The instruction count of the last tick.
    pub last_tick: u64,
}

impl Timer {
    /// The hardware part of the time base: units since the last tick.
    fn hardware(&self, now: u64) -> u64 {
        let k = now
            .saturating_sub(self.last_tick)
            .min(INSTRUCTIONS_PER_MS - 1);
        k * UNITS_PER_MS / INSTRUCTIONS_PER_MS
    }

    /// RDTIME: the double word time base.
    pub fn read(&self, now: u64) -> (W36, W36) {
        (self.time0, W36::wrap(self.time1.get() + self.hardware(now)))
    }

    /// WRTIME: the low 12 bits belong to the hardware counter and are
    /// cleared (mask 770000 on the right half).
    pub fn write(&mut self, hi: W36, lo: W36) {
        self.time0 = hi;
        self.time1 = W36::wrap(lo.get() & !0o7777);
    }
}

/// Do the ticks that are due before the next instruction. With the wall
/// clock, the host does the ticks ([`Machine::tick`]).
pub fn run(m: &mut Machine) {
    if m.wall_clock {
        return;
    }
    while m.count >= m.timer.last_tick + INSTRUCTIONS_PER_MS {
        m.timer.last_tick += INSTRUCTIONS_PER_MS;
        tock(m);
    }
}

/// One tick. True if the interval expired at it.
pub(crate) fn tock(m: &mut Machine) -> bool {
    let t = &mut m.timer;
    let low = t.time1.get() + UNITS_PER_MS;
    if low & (1 << 35) != 0 {
        // The low word overflowed into its sign: carry into the high word.
        t.time0 = W36::wrap((t.time0.get() + 1) & MASK36);
        t.time1 = W36::ZERO;
    } else {
        t.time1 = W36::wrap(low);
    }
    if m.pi.in_progress == 0 {
        t.quan = W36::wrap((t.quan.get() + UNITS_PER_MS) & MASK36);
    }
    let ttg = t.ttg.signed() - UNITS_PER_MS as i64;
    let expired = ttg <= 0;
    if expired {
        t.ttg = W36::wrap((ttg + t.period.signed()) as u64 & MASK36);
        m.apr.flags |= flag::INTERVAL;
    } else {
        t.ttg = W36::wrap(ttg as u64);
    }
    sample(m);
    expired
}

/// The PC sample table (PF-INT, .IF/PCST): if the AOBJN pointer in PCST is
/// negative, store the PC word at its right half (physical) and add 1,,1.
fn sample(m: &mut Machine) {
    let p = m.pcst;
    if !p.bit(0) {
        return;
    }
    m.pcst = W36::from_halves(Half::wrap(p.lh().get() + 1), Half::wrap(p.rh().get() + 1));
    let w = m.flags.pc_word(m.pc);
    // A store above the memory is lost; the microcode would take a page
    // fail on the physical write, which comes with the pager.
    let _ = m.deposit(p.rh().get() as u32, w);
}

#[cfg(test)]
mod tests {
    use super::*;

    // With the wall clock, steps do no ticks; each Machine::tick is one
    // TOCK, and the idle loop skips to the limit, not to a tick.
    #[test]
    fn wall_clock_ticks() {
        let mut m = Machine::new();
        m.wall_clock = true;
        m.ac[0o17] = W36::from_halves(Half::wrap(0o344000), Half::wrap(0o17));
        m.pc = Half::wrap(0o17);
        let mut skipped = 0;
        while m.count < 50_000 {
            skipped += m.step_idle(50_000).unwrap();
        }
        // The DZ11 polls every CHAR_TIME: one step for each.
        assert_eq!(m.count, 50_000);
        assert!(skipped > 49_900, "{skipped}");
        assert_eq!(m.timer.time1.get(), 0);
        m.tick();
        m.tick();
        assert_eq!(m.timer.time1.get(), 2 * UNITS_PER_MS);
        assert_eq!(m.timer.last_tick, 50_000);
    }

    // inout.50, TOCK: 2^12 per ms; the interval sets the flag when the
    // time to go reaches 0, and the period is added back.
    #[test]
    fn ticks() {
        let mut m = Machine::new();
        m.timer.period = W36::wrap(3 * UNITS_PER_MS);
        m.timer.ttg = m.timer.period;
        m.count = 2 * INSTRUCTIONS_PER_MS;
        run(&mut m);
        assert_eq!(m.timer.time1.get(), 2 * UNITS_PER_MS);
        assert_eq!(m.apr.flags & flag::INTERVAL, 0);
        m.count = 3 * INSTRUCTIONS_PER_MS;
        run(&mut m);
        assert_ne!(m.apr.flags & flag::INTERVAL, 0);
        assert_eq!(m.timer.ttg.get(), 3 * UNITS_PER_MS);
        assert_eq!(m.timer.quan.get(), 3 * UNITS_PER_MS);
        // Half a millisecond later, the hardware part is 2^11.
        m.count += INSTRUCTIONS_PER_MS / 2;
        assert_eq!(m.timer.read(m.count).1.get(), 3 * UNITS_PER_MS + 2048);
    }

    #[test]
    fn low_word_carry_and_write() {
        let mut t = Timer::default();
        t.write(W36::wrap(5), W36::wrap(0o12345));
        assert_eq!(t.time1.get(), 0o10000);
        let mut m = Machine::new();
        m.timer.time1 = W36::wrap((1 << 35) - UNITS_PER_MS);
        m.count = INSTRUCTIONS_PER_MS;
        run(&mut m);
        assert_eq!((m.timer.time0.get(), m.timer.time1.get()), (1, 0));
    }

    // inout.50, TOCK2 (.IF/ITS): the quantum counter does not count while
    // a PI level is in progress.
    #[test]
    fn quantum_stops_in_pi() {
        let mut m = Machine::new();
        m.pi.in_progress = crate::apr::Pi::level_bit(4);
        m.count = INSTRUCTIONS_PER_MS;
        run(&mut m);
        assert_eq!(m.timer.quan.get(), 0);
        assert_eq!(m.timer.time1.get(), UNITS_PER_MS);
    }
}
