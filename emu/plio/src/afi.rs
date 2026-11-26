//! The width of the HP ports: the 32-bit mode of the AFI.
//!
//! Each HP port (`S_AXI_HP0` to `S_AXI_HP3`) has an AFI in the PS. Bit 0
//! (`32BitEn`) of `AFI_RDCHAN_CTRL` and of `AFI_WRCHAN_CTRL` selects 32-bit
//! data for the read channel and for the write channel; the two channels have
//! separate bits **[manual]** (UG585 v1.7, section 5.3.6, "32-bit Interface
//! Considerations", and Appendix B.4, axi_hp). The registers are at
//! `0xF8008000 + 0x1000 * n`, offsets 0x00 and 0x14 **[manual]** (same
//! appendix). The board does not set them: both read 0 for HP0 after boot
//! **[measured]** (board repository, `docs/pl-interface.md`, section "HP
//! ports").
//!
//! The registers are in the PS: a new bitstream does not change them. A
//! design with 32-bit data sets them itself (board interface, rule 6), and
//! only while its HP ports have no transaction.

use std::io;

use crate::mmio::{self, Mapping};

/// Physical address of the AFI registers of HP0.
pub const BASE: u64 = 0xF800_8000;
/// Distance between the register blocks of two ports, in bytes.
pub const STRIDE: usize = 0x1000;
/// Number of HP ports.
pub const PORTS: usize = 4;
/// Offset of `AFI_RDCHAN_CTRL` in the block of a port.
pub const RDCHAN_CTRL: usize = 0x00;
/// Offset of `AFI_WRCHAN_CTRL` in the block of a port.
pub const WRCHAN_CTRL: usize = 0x14;
/// `32BitEn`: 1 for 32-bit data, 0 for 64-bit data.
pub const EN_32BIT: u32 = 1 << 0;

/// A channel of an HP port.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Channel {
    Read,
    Write,
}

/// Offset of the control register of `port` and `ch` from [`BASE`].
pub fn ctrl_offset(port: usize, ch: Channel) -> Option<usize> {
    if port >= PORTS {
        return None;
    }
    Some(
        port * STRIDE
            + match ch {
                Channel::Read => RDCHAN_CTRL,
                Channel::Write => WRCHAN_CTRL,
            },
    )
}

/// The register value `v` with `32BitEn` set to `on`. The other bits stay.
pub fn with_32bit(v: u32, on: bool) -> u32 {
    if on { v | EN_32BIT } else { v & !EN_32BIT }
}

fn bad_port(port: usize) -> io::Error {
    io::Error::new(
        io::ErrorKind::InvalidInput,
        format!("HP{port}: there are {PORTS} HP ports"),
    )
}

/// The AFI registers of the four HP ports, mapped through `/dev/mem`.
pub struct Afi {
    map: Mapping,
}

impl Afi {
    pub fn open() -> io::Result<Self> {
        Ok(Afi {
            map: mmio::map_phys(BASE, PORTS * STRIDE)?,
        })
    }

    /// `32BitEn` of a channel.
    pub fn is_32bit(&self, port: usize, ch: Channel) -> io::Result<bool> {
        let off = ctrl_offset(port, ch).ok_or_else(|| bad_port(port))?;
        Ok(self.map.read32(off) & EN_32BIT != 0)
    }

    /// `32BitEn` of each port: (read channel, write channel).
    pub fn state(&self) -> io::Result<[(bool, bool); PORTS]> {
        let mut s = [(false, false); PORTS];
        for (p, v) in s.iter_mut().enumerate() {
            *v = (
                self.is_32bit(p, Channel::Read)?,
                self.is_32bit(p, Channel::Write)?,
            );
        }
        Ok(s)
    }

    /// Set `32BitEn` of a channel to `on` (read, modify, write), and read it
    /// back. Returns the earlier value.
    pub fn set_32bit(&self, port: usize, ch: Channel, on: bool) -> io::Result<bool> {
        let off = ctrl_offset(port, ch).ok_or_else(|| bad_port(port))?;
        let v = self.map.read32(off);
        if (v & EN_32BIT != 0) != on {
            self.map.write32(off, with_32bit(v, on));
        }
        let now = self.map.read32(off);
        if (now & EN_32BIT != 0) != on {
            return Err(io::Error::other(format!(
                "AFI of HP{port}, {ch:?} channel: 32BitEn stays {} after a write of {}",
                now & EN_32BIT,
                u32::from(on)
            )));
        }
        Ok(v & EN_32BIT != 0)
    }

    /// Set the read channel of `rd_port` and the write channel of `wr_port`
    /// to 32-bit (`on`) or 64-bit data. The guard sets the earlier values
    /// again when it is dropped. Call it only while the design makes no
    /// transaction, and drop the guard only after the end of the run.
    pub fn set_width(
        &self,
        rd_port: usize,
        wr_port: usize,
        on: bool,
    ) -> io::Result<WidthGuard<'_>> {
        let mut g = WidthGuard {
            afi: self,
            saved: Vec::with_capacity(2),
        };
        for (port, ch) in [(rd_port, Channel::Read), (wr_port, Channel::Write)] {
            let before = self.set_32bit(port, ch, on)?;
            g.saved.push((port, ch, before));
        }
        Ok(g)
    }
}

/// The earlier values of `32BitEn`; see [`Afi::set_width`].
pub struct WidthGuard<'a> {
    afi: &'a Afi,
    saved: Vec<(usize, Channel, bool)>,
}

impl Drop for WidthGuard<'_> {
    fn drop(&mut self) {
        for &(port, ch, v) in self.saved.iter().rev() {
            if let Err(e) = self.afi.set_32bit(port, ch, v) {
                eprintln!("warning: restore of the AFI width failed: {e}");
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn offsets() {
        // The absolute addresses of UG585 v1.7, Appendix B.4: AFI_RDCHAN_CTRL
        // of axi_hp0 to axi_hp3 at 0xF8008000, 0xF8009000, 0xF800A000,
        // 0xF800B000; AFI_WRCHAN_CTRL at 0xF8008014 and so on.
        let abs = |p, ch| BASE + ctrl_offset(p, ch).unwrap() as u64;
        assert_eq!(abs(0, Channel::Read), 0xF800_8000);
        assert_eq!(abs(3, Channel::Read), 0xF800_B000);
        assert_eq!(abs(0, Channel::Write), 0xF800_8014);
        assert_eq!(abs(2, Channel::Write), 0xF800_A014);
        assert_eq!(ctrl_offset(4, Channel::Read), None);
        assert!(ctrl_offset(3, Channel::Write).unwrap() + 4 <= PORTS * STRIDE);
    }

    #[test]
    fn only_bit_0_changes() {
        assert_eq!(with_32bit(0, true), 1);
        assert_eq!(with_32bit(0xF0, true), 0xF1);
        assert_eq!(with_32bit(0xF1, false), 0xF0);
        assert_eq!(with_32bit(0xFFFF_FFFF, false), 0xFFFF_FFFE);
        assert_eq!(with_32bit(0x0000_0F01, true), 0x0000_0F01);
    }
}
