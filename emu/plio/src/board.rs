//! The board interface on Linux: the UIO device of the register window, the
//! reserved memory region, and the checks before the first register read.
//!
//! The interface is `docs/pl-interface.md` of the board repository
//! (submodule `3rdparty/PYNQ-Z2`, tag pl-interface-v1). A read of the register
//! window without a configured PL, or without FCLK0, stops the PS until a
//! power cycle. Thus [`check_pl_ready`] comes before [`RegWindow::open`].

use std::fs;
use std::io;
use std::path::{Path, PathBuf};

use crate::mmio::{self, Mapping};

/// `INT_STS` of the devcfg: bit 2 is `PCFG_DONE`, 1 when the PL is configured.
/// Source: `examples/pl-interface/pltest.c` of the board repository, which
/// makes the same check before its first register read.
const DEVCFG_INT_STS: u64 = 0xF800_700C;
const PCFG_DONE: u32 = 1 << 2;

/// `FPGA0_THR_CNT` of the SLCR: bit 0 is 1 while Linux stops FCLK0.
/// **[measured]** in the board repository, `docs/pl-interface.md`, section
/// "Why fclk-enable is necessary".
const FPGA0_THR_CNT: u64 = 0xF800_0178;

/// `FPGA_RST_CTRL` of the SLCR: bit 0 is 1 while `FCLKRESETN[0]` holds the PL
/// in reset. The registers of the design do not answer then. Source: the
/// board repository, `docs/pl-interface.md`, section "PL reset"; after a JTAG
/// load the register is 0 **[measured]** there.
const FPGA_RST_CTRL: u64 = 0xF800_0240;

/// Error of the board interface.
#[derive(Debug)]
pub enum BoardError {
    /// A system call or a file failed.
    Io(String, io::Error),
    /// The PL is not configured, FCLK0 is off, or the PL is in reset: no
    /// register read is safe.
    PlNotReady {
        int_sts: u32,
        thr_cnt: u32,
        rst_ctrl: u32,
    },
    /// No UIO device has the name.
    NoUio(String),
    /// The device tree has no reserved region with this base and size.
    NoRegion { base: u64, size: u64 },
    /// A value of the system is not the expected one.
    Unexpected(String),
}

impl std::fmt::Display for BoardError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            BoardError::Io(what, e) => write!(f, "{what}: {e}"),
            BoardError::PlNotReady {
                int_sts,
                thr_cnt,
                rst_ctrl,
            } => write!(
                f,
                "the PL is not configured, FCLK0 is off, or the PL is in reset (devcfg INT_STS \
                 {int_sts:#010x}, FPGA0_THR_CNT {thr_cnt:#010x}, FPGA_RST_CTRL {rst_ctrl:#010x}): \
                 no register read"
            ),
            BoardError::NoUio(name) => write!(f, "no UIO device with the name {name:?}"),
            BoardError::NoRegion { base, size } => {
                write!(
                    f,
                    "no reserved memory region {base:#x}, {size:#x} bytes, in the device tree"
                )
            }
            BoardError::Unexpected(s) => f.write_str(s),
        }
    }
}

impl std::error::Error for BoardError {}

fn io_err(what: impl Into<String>) -> impl FnOnce(io::Error) -> BoardError {
    let what = what.into();
    move |e| BoardError::Io(what, e)
}

/// Check that the PL is configured, that FCLK0 runs, and that the PL is not in
/// reset. Reads three PS registers through `/dev/mem`.
pub fn check_pl_ready() -> Result<(), BoardError> {
    let int_sts = mmio::read_phys32(DEVCFG_INT_STS).map_err(io_err("read of devcfg INT_STS"))?;
    let thr_cnt = mmio::read_phys32(FPGA0_THR_CNT).map_err(io_err("read of FPGA0_THR_CNT"))?;
    let rst_ctrl = mmio::read_phys32(FPGA_RST_CTRL).map_err(io_err("read of FPGA_RST_CTRL"))?;
    if !pl_ready(int_sts, thr_cnt, rst_ctrl) {
        return Err(BoardError::PlNotReady {
            int_sts,
            thr_cnt,
            rst_ctrl,
        });
    }
    Ok(())
}

fn pl_ready(int_sts: u32, thr_cnt: u32, rst_ctrl: u32) -> bool {
    int_sts & PCFG_DONE != 0 && thr_cnt & 1 == 0 && rst_ctrl & 1 == 0
}

/// Find the UIO device with `name` under `sysfs` (normally `/sys/class/uio`).
/// Returns its directory name, for example `uio0`.
pub fn find_uio(sysfs: &Path, name: &str) -> Result<String, BoardError> {
    let dir = fs::read_dir(sysfs).map_err(io_err(sysfs.display().to_string()))?;
    let mut found: Vec<String> = Vec::new();
    for entry in dir.flatten() {
        let dev = entry.file_name().to_string_lossy().into_owned();
        if !dev.starts_with("uio") {
            continue;
        }
        if let Ok(n) = fs::read_to_string(entry.path().join("name"))
            && n.trim_end() == name
        {
            found.push(dev);
        }
    }
    found.sort();
    match found.len() {
        0 => Err(BoardError::NoUio(name.to_string())),
        1 => Ok(found.remove(0)),
        _ => Err(BoardError::Unexpected(format!(
            "more than one UIO device named {name:?}: {found:?}"
        ))),
    }
}

fn read_be_cells(bytes: &[u8]) -> Vec<u32> {
    bytes
        .as_chunks::<4>()
        .0
        .iter()
        .map(|c| u32::from_be_bytes(*c))
        .collect()
}

fn cells_value(cells: &[u32]) -> u64 {
    cells
        .iter()
        .fold(0u64, |acc, &c| (acc << 32) | u64::from(c))
}

/// The regions (base, size) of the reserved-memory node of a device tree
/// directory (normally `/proc/device-tree`).
pub fn reserved_regions(devtree: &Path) -> Result<Vec<(u64, u64)>, BoardError> {
    let node = devtree.join("reserved-memory");
    let cells = |name: &str| -> Result<usize, BoardError> {
        let p = node.join(name);
        let b = fs::read(&p).map_err(io_err(p.display().to_string()))?;
        let v = read_be_cells(&b);
        match v.as_slice() {
            [n] if (1..=2).contains(n) => Ok(*n as usize),
            _ => Err(BoardError::Unexpected(format!(
                "{} is not 1 or 2",
                p.display()
            ))),
        }
    };
    let ac = cells("#address-cells")?;
    let sc = cells("#size-cells")?;
    let mut out = Vec::new();
    let dir = fs::read_dir(&node).map_err(io_err(node.display().to_string()))?;
    let mut children: Vec<PathBuf> = dir
        .flatten()
        .map(|e| e.path())
        .filter(|p| p.is_dir())
        .collect();
    children.sort();
    for child in children {
        let Ok(b) = fs::read(child.join("reg")) else {
            continue;
        };
        let v = read_be_cells(&b);
        for pair in v.chunks_exact(ac + sc) {
            out.push((cells_value(&pair[..ac]), cells_value(&pair[ac..])));
        }
    }
    Ok(out)
}

/// Check that the device tree reserves exactly (`base`, `size`).
pub fn check_region(devtree: &Path, base: u64, size: u64) -> Result<(), BoardError> {
    if reserved_regions(devtree)?.contains(&(base, size)) {
        Ok(())
    } else {
        Err(BoardError::NoRegion { base, size })
    }
}

/// The register window of the PL: map 0 of the UIO device.
#[derive(Debug)]
pub struct RegWindow {
    map: Mapping,
    /// The UIO device, for example `uio0`.
    pub device: String,
}

impl RegWindow {
    /// Open the UIO device named `name` and map `len` bytes of map 0. The size
    /// of map 0 must be `len`. Call [`check_pl_ready`] first.
    pub fn open(name: &str, len: usize) -> Result<Self, BoardError> {
        let sysfs = Path::new("/sys/class/uio");
        let device = find_uio(sysfs, name)?;
        let size_path = sysfs.join(&device).join("maps/map0/size");
        let size_text =
            fs::read_to_string(&size_path).map_err(io_err(size_path.display().to_string()))?;
        let size = parse_hex(size_text.trim()).ok_or_else(|| {
            BoardError::Unexpected(format!("{}: {size_text:?}", size_path.display()))
        })?;
        if size != len as u64 {
            return Err(BoardError::Unexpected(format!(
                "map 0 of {device} has {size:#x} bytes, the register map {len:#x}"
            )));
        }
        let dev_path = format!("/dev/{device}");
        let file = fs::OpenOptions::new()
            .read(true)
            .write(true)
            .open(&dev_path)
            .map_err(io_err(dev_path.clone()))?;
        // Map 0 of a UIO device is at offset 0.
        let map = Mapping::new(&file, 0, len).map_err(io_err(format!("mmap of {dev_path}")))?;
        Ok(RegWindow { map, device })
    }

    /// Read the register at byte offset `off`.
    pub fn read(&self, off: usize) -> u32 {
        self.map.read32(off)
    }

    /// Write the register at byte offset `off`.
    pub fn write(&self, off: usize, value: u32) {
        self.map.write32(off, value)
    }
}

fn parse_hex(s: &str) -> Option<u64> {
    u64::from_str_radix(s.strip_prefix("0x").unwrap_or(s), 16).ok()
}

/// The reserved memory region, mapped without the CPU cache.
#[derive(Debug)]
pub struct Region {
    map: Mapping,
    /// Physical address of byte 0.
    pub base: u64,
}

impl Region {
    /// Check that the device tree reserves (`base`, `size`), then map it
    /// through `/dev/mem` with `O_SYNC`.
    pub fn open(base: u64, size: usize) -> Result<Self, BoardError> {
        check_region(Path::new("/proc/device-tree"), base, size as u64)?;
        let map = mmio::map_phys(base, size).map_err(io_err("mmap of /dev/mem"))?;
        Ok(Region { map, base })
    }

    /// Check that the device tree reserves (`base`, `size`), then map only
    /// `len` bytes from byte `off` of it. `off` is a multiple of the page size.
    /// `base` of the result is the physical address of byte 0 of this part.
    pub fn open_part(base: u64, size: usize, off: usize, len: usize) -> Result<Self, BoardError> {
        if off.checked_add(len).is_none_or(|end| end > size) {
            return Err(BoardError::Unexpected(format!(
                "part {off:#x}+{len:#x} outside the region"
            )));
        }
        check_region(Path::new("/proc/device-tree"), base, size as u64)?;
        let addr = base + off as u64;
        let map = mmio::map_phys(addr, len).map_err(io_err("mmap of /dev/mem"))?;
        Ok(Region { map, base: addr })
    }

    /// Size in bytes.
    pub fn len(&self) -> usize {
        self.map.len()
    }

    /// Always false.
    pub fn is_empty(&self) -> bool {
        self.map.is_empty()
    }

    /// Read the 32-bit word at byte offset `off`.
    pub fn read32(&self, off: usize) -> u32 {
        self.map.read32(off)
    }

    /// Write the 32-bit word at byte offset `off`.
    pub fn write32(&self, off: usize, value: u32) {
        self.map.write32(off, value)
    }

    /// The mapping, for [`Mapping::pread_from`].
    pub fn mapping(&self) -> &Mapping {
        &self.map
    }

    /// The mapping, for a thread that owns it.
    pub fn into_mapping(self) -> Mapping {
        self.map
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    struct TempDir(PathBuf);
    impl TempDir {
        fn new(tag: &str) -> Self {
            let p = std::env::temp_dir().join(format!("plio-board-{tag}-{}", std::process::id()));
            let _ = fs::remove_dir_all(&p);
            fs::create_dir_all(&p).unwrap();
            TempDir(p)
        }
    }
    impl Drop for TempDir {
        fn drop(&mut self) {
            let _ = fs::remove_dir_all(&self.0);
        }
    }

    fn be(v: &[u32]) -> Vec<u8> {
        v.iter().flat_map(|x| x.to_be_bytes()).collect()
    }

    // The layout of the device tree of the board (docs/pl-interface.md of the
    // board repository): one cell for address and size.
    #[test]
    fn reserved_region_one_cell() {
        let t = TempDir::new("dt1");
        let rm = t.0.join("reserved-memory");
        fs::create_dir_all(rm.join("pl-buffer@1f000000")).unwrap();
        fs::create_dir_all(rm.join("other@10000000")).unwrap();
        fs::write(rm.join("#address-cells"), be(&[1])).unwrap();
        fs::write(rm.join("#size-cells"), be(&[1])).unwrap();
        fs::write(rm.join("ranges"), b"").unwrap();
        fs::write(
            rm.join("pl-buffer@1f000000/reg"),
            be(&[0x1f00_0000, 0x0100_0000]),
        )
        .unwrap();
        fs::write(rm.join("other@10000000/reg"), be(&[0x1000_0000, 0x1000])).unwrap();
        let r = reserved_regions(&t.0).unwrap();
        assert_eq!(r, vec![(0x1000_0000, 0x1000), (0x1f00_0000, 0x0100_0000)]);
        assert!(check_region(&t.0, 0x1f00_0000, 0x0100_0000).is_ok());
        assert!(matches!(
            check_region(&t.0, 0x1f00_0000, 0x0080_0000),
            Err(BoardError::NoRegion { .. })
        ));
    }

    #[test]
    fn reserved_region_two_cells() {
        let t = TempDir::new("dt2");
        let rm = t.0.join("reserved-memory");
        fs::create_dir_all(rm.join("buf")).unwrap();
        fs::write(rm.join("#address-cells"), be(&[2])).unwrap();
        fs::write(rm.join("#size-cells"), be(&[2])).unwrap();
        fs::write(rm.join("buf/reg"), be(&[0x1, 0x2000_0000, 0, 0x10_0000])).unwrap();
        assert_eq!(
            reserved_regions(&t.0).unwrap(),
            vec![(0x1_2000_0000, 0x10_0000)]
        );
    }

    #[test]
    fn uio_by_name() {
        let t = TempDir::new("uio");
        for (dev, name) in [("uio0", "other\n"), ("uio1", "pl\n"), ("misc", "pl\n")] {
            fs::create_dir_all(t.0.join(dev)).unwrap();
            fs::write(t.0.join(dev).join("name"), name).unwrap();
        }
        assert_eq!(find_uio(&t.0, "pl").unwrap(), "uio1");
        assert!(matches!(find_uio(&t.0, "none"), Err(BoardError::NoUio(_))));
        fs::create_dir_all(t.0.join("uio2")).unwrap();
        fs::write(t.0.join("uio2/name"), "pl\n").unwrap();
        assert!(matches!(
            find_uio(&t.0, "pl"),
            Err(BoardError::Unexpected(_))
        ));
    }

    #[test]
    fn readiness() {
        assert!(pl_ready(PCFG_DONE, 0, 0));
        assert!(pl_ready(0xFFFF_FFFF, 0xFFFF_FFFE, 0xE));
        assert!(!pl_ready(0, 0, 0)); // not configured
        assert!(!pl_ready(PCFG_DONE, 1, 0)); // FCLK0 stopped
        assert!(!pl_ready(PCFG_DONE, 0, 1)); // PL in reset
    }

    #[test]
    fn hex_sizes() {
        assert_eq!(parse_hex("0x00100000"), Some(0x10_0000));
        assert_eq!(parse_hex("1000"), Some(0x1000));
        assert_eq!(parse_hex("zz"), None);
    }
}
