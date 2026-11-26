//! Memory mappings of the PL registers and of physical memory.
//!
//! This is the only module of the workspace with `unsafe` code (AGENTS.md).
//! All functions outside it are safe: a [`Mapping`] checks each offset
//! against its length and alignment.
//!
//! Two kinds of mapping are used:
//! - The register window of a UIO device (`/dev/uioN`, map 0).
//! - Physical memory through `/dev/mem` opened with `O_SYNC`. On the board,
//!   Linux then maps it without the CPU cache. The reserved region of the
//!   board interface is mapped this way (see `docs/pl-interface.md` of the
//!   board repository, and README.md: the PS maps the guest memory without
//!   the CPU cache).

use std::fs::{File, OpenOptions};
use std::io;
use std::os::fd::AsRawFd;
use std::os::unix::fs::OpenOptionsExt;
use std::ptr::NonNull;

/// A shared memory mapping of a file, with volatile accesses.
#[derive(Debug)]
pub struct Mapping {
    ptr: NonNull<u8>,
    len: usize,
}

// SAFETY: the mapping is plain memory or device registers; it has no thread
// affinity. The accesses are volatile and do not create references.
unsafe impl Send for Mapping {}

impl Mapping {
    /// Map `len` bytes of `file` from `offset`, for reading and writing,
    /// shared with other processes and with the device.
    pub fn new(file: &File, offset: u64, len: usize) -> io::Result<Self> {
        if len == 0 {
            return Err(io::Error::new(
                io::ErrorKind::InvalidInput,
                "mapping of length 0",
            ));
        }
        let off = libc::off_t::try_from(offset)
            .map_err(|_| io::Error::new(io::ErrorKind::InvalidInput, "offset too large"))?;
        // SAFETY: a new mapping at an address that the kernel selects; it does
        // not change memory that Rust owns. The file descriptor is valid for
        // the call.
        let p = unsafe {
            libc::mmap(
                std::ptr::null_mut(),
                len,
                libc::PROT_READ | libc::PROT_WRITE,
                libc::MAP_SHARED,
                file.as_raw_fd(),
                off,
            )
        };
        if p == libc::MAP_FAILED {
            return Err(io::Error::last_os_error());
        }
        let ptr = NonNull::new(p.cast::<u8>())
            .ok_or_else(|| io::Error::other("mmap gave a null pointer"))?;
        Ok(Mapping { ptr, len })
    }

    /// Length in bytes.
    pub fn len(&self) -> usize {
        self.len
    }

    /// Always false: a mapping has a length of 1 byte or more.
    pub fn is_empty(&self) -> bool {
        self.len == 0
    }

    fn check(&self, off: usize, size: usize) {
        assert!(
            off.is_multiple_of(size) && off.checked_add(size).is_some_and(|end| end <= self.len),
            "access of {size} bytes at offset {off:#x} outside a mapping of {:#x} bytes, or not aligned",
            self.len
        );
    }

    /// Read the 32-bit word at byte offset `off`. Panics if `off` is outside
    /// the mapping or not a multiple of 4.
    pub fn read32(&self, off: usize) -> u32 {
        self.check(off, 4);
        // SAFETY: check() keeps the access inside the mapping and aligned.
        unsafe { self.ptr.as_ptr().add(off).cast::<u32>().read_volatile() }
    }

    /// Write the 32-bit word at byte offset `off`. Panics as [`Self::read32`].
    pub fn write32(&self, off: usize, value: u32) {
        self.check(off, 4);
        // SAFETY: check() keeps the access inside the mapping and aligned.
        unsafe {
            self.ptr
                .as_ptr()
                .add(off)
                .cast::<u32>()
                .write_volatile(value)
        }
    }

    /// Read `len` bytes of `file` from `file_offset` into the mapping at
    /// `off`, with one `pread` call. Returns the number of bytes read.
    pub fn pread_from(
        &self,
        file: &File,
        file_offset: u64,
        off: usize,
        len: usize,
    ) -> io::Result<usize> {
        assert!(
            off.checked_add(len).is_some_and(|end| end <= self.len),
            "pread of {len} bytes at offset {off:#x} outside a mapping of {:#x} bytes",
            self.len
        );
        let foff = libc::off_t::try_from(file_offset)
            .map_err(|_| io::Error::new(io::ErrorKind::InvalidInput, "file offset too large"))?;
        // SAFETY: the destination range is inside the mapping (assert above).
        // The kernel writes it; no Rust reference to that memory exists.
        let n = unsafe {
            libc::pread(
                file.as_raw_fd(),
                self.ptr.as_ptr().add(off).cast(),
                len,
                foff,
            )
        };
        usize::try_from(n).map_err(|_| io::Error::last_os_error())
    }
}

impl Drop for Mapping {
    fn drop(&mut self) {
        // SAFETY: ptr and len are the values of the successful mmap; the
        // mapping is not used after this.
        unsafe {
            libc::munmap(self.ptr.as_ptr().cast(), self.len);
        }
    }
}

/// Complete all earlier memory accesses before the next one. Use it after the
/// CPU writes to a non-cached mapping and before a register write that makes
/// the PL read that memory: the writes can wait in a write buffer.
pub fn barrier() {
    #[cfg(target_arch = "arm")]
    // SAFETY: DSB has no operands and changes no register.
    unsafe {
        std::arch::asm!("dsb sy", options(nostack, preserves_flags));
    }
    #[cfg(not(target_arch = "arm"))]
    std::sync::atomic::fence(std::sync::atomic::Ordering::SeqCst);
}

/// Open `/dev/mem` with `O_SYNC`.
pub fn open_dev_mem() -> io::Result<File> {
    OpenOptions::new()
        .read(true)
        .write(true)
        .custom_flags(libc::O_SYNC)
        .open("/dev/mem")
}

/// Map `len` bytes of physical memory from `addr` (a multiple of the page size).
pub fn map_phys(addr: u64, len: usize) -> io::Result<Mapping> {
    Mapping::new(&open_dev_mem()?, addr, len)
}

/// Read one 32-bit register of the PS at physical address `addr`.
pub fn read_phys32(addr: u64) -> io::Result<u32> {
    let page = addr & !0xFFF;
    let m = map_phys(page, 0x1000)?;
    Ok(m.read32(usize::try_from(addr - page).unwrap_or(0)))
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Write;

    fn temp_file(tag: &str, len: usize) -> (File, std::path::PathBuf) {
        let path = std::env::temp_dir().join(format!("plio-mmio-{tag}-{}", std::process::id()));
        let mut f = OpenOptions::new()
            .read(true)
            .write(true)
            .create(true)
            .truncate(true)
            .open(&path)
            .unwrap();
        f.write_all(&vec![0u8; len]).unwrap();
        (f, path)
    }

    #[test]
    fn read_write_through_a_file_mapping() {
        let (f, path) = temp_file("rw", 8192);
        let m = Mapping::new(&f, 4096, 4096).unwrap();
        m.write32(0, 0x1234_5678);
        m.write32(4092, 0xDEAD_BEEF);
        assert_eq!(m.read32(0), 0x1234_5678);
        assert_eq!(m.read32(4092), 0xDEAD_BEEF);
        drop(m);
        let data = std::fs::read(&path).unwrap();
        assert_eq!(&data[4096..4100], &0x1234_5678u32.to_ne_bytes());
        std::fs::remove_file(path).unwrap();
    }

    // read_phys32 maps PS registers at offsets above 2 GiB of /dev/mem; the
    // offset must survive the 32-bit ABI of the board.
    #[test]
    fn mapping_above_2_gib() {
        let path = std::env::temp_dir().join(format!("plio-mmio-sparse-{}", std::process::id()));
        let f = OpenOptions::new()
            .read(true)
            .write(true)
            .create(true)
            .truncate(true)
            .open(&path)
            .unwrap();
        f.set_len(0xF800_8000).unwrap();
        let m = Mapping::new(&f, 0xF800_7000, 0x1000).unwrap();
        m.write32(0xC, 0x0000_0004);
        drop(m);
        let m = Mapping::new(&f, 0xF800_7000, 0x1000).unwrap();
        assert_eq!(m.read32(0xC), 4);
        drop(m);
        drop(f);
        std::fs::remove_file(path).unwrap();
    }

    #[test]
    #[should_panic(expected = "outside a mapping")]
    fn access_outside_panics() {
        let (f, path) = temp_file("outside", 4096);
        let m = Mapping::new(&f, 0, 4096).unwrap();
        std::fs::remove_file(path).unwrap();
        m.read32(4096);
    }

    #[test]
    #[should_panic(expected = "not aligned")]
    fn unaligned_access_panics() {
        let (f, path) = temp_file("unaligned", 4096);
        let m = Mapping::new(&f, 0, 4096).unwrap();
        std::fs::remove_file(path).unwrap();
        m.write32(2, 0);
    }

    #[test]
    fn pread_into_mapping() {
        let (f, path) = temp_file("pread", 4096);
        let m = Mapping::new(&f, 0, 4096).unwrap();
        let src_path = std::env::temp_dir().join(format!("plio-src-{}", std::process::id()));
        std::fs::write(&src_path, [1u8, 2, 3, 4, 5, 6, 7, 8]).unwrap();
        let src = File::open(&src_path).unwrap();
        assert_eq!(m.pread_from(&src, 4, 16, 4).unwrap(), 4);
        assert_eq!(m.read32(16), u32::from_ne_bytes([5, 6, 7, 8]));
        std::fs::remove_file(path).unwrap();
        std::fs::remove_file(src_path).unwrap();
    }
}
