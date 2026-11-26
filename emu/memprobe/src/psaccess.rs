//! B8 (docs/mem-path-experiment.md): access of the PS to the reserved region
//! through the mapping without the CPU cache. This does not use the PL.
//!
//! - Word accesses: a loop of 32-bit or 64-bit volatile reads or writes over a
//!   part of the region, as a device model accesses single guest words.
//! - `pread`: a file into the region, as a device model reads a disk block
//!   into the guest memory (AGENTS.md: device I/O uses pread and pwrite).
//!
//! With more than one thread, each thread has its own part of the region and
//! its own mapping, and the threads start together.

use std::fs::File;
use std::io::{self, Seek, SeekFrom};
use std::sync::Barrier;
use std::thread;
use std::time::{Duration, Instant};

use plio::mmio::Mapping;

/// A kind of word access.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Access {
    Read32,
    Write32,
    Read64,
    Write64,
}

impl Access {
    pub const ALL: [Access; 4] = [
        Access::Read32,
        Access::Write32,
        Access::Read64,
        Access::Write64,
    ];

    pub fn name(self) -> &'static str {
        match self {
            Access::Read32 => "read32",
            Access::Write32 => "write32",
            Access::Read64 => "read64",
            Access::Write64 => "write64",
        }
    }

    fn bytes(self) -> usize {
        match self {
            Access::Read32 | Access::Write32 => 4,
            Access::Read64 | Access::Write64 => 8,
        }
    }
}

/// The work of one thread: bytes moved, number of operations, and the time.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct Work {
    pub bytes: u64,
    pub ops: u64,
    pub time: Duration,
}

/// `passes` loops of `access` over all of `m`.
pub fn words(m: &Mapping, access: Access, passes: u32) -> Work {
    let step = access.bytes();
    let n = m.len() / step;
    let t0 = Instant::now();
    let mut acc = 0u64;
    for p in 0..passes {
        let v = u64::from(p);
        match access {
            Access::Read32 => (0..n).for_each(|i| acc ^= u64::from(m.read32(i * 4))),
            Access::Write32 => (0..n).for_each(|i| m.write32(i * 4, (i as u32) ^ (v as u32))),
            Access::Read64 => (0..n).for_each(|i| acc ^= m.read64(i * 8)),
            Access::Write64 => (0..n).for_each(|i| m.write64(i * 8, (i as u64) ^ v)),
        }
    }
    plio::mmio::barrier();
    let time = t0.elapsed();
    std::hint::black_box(acc);
    let ops = n as u64 * u64::from(passes);
    Work {
        bytes: ops * step as u64,
        ops,
        time,
    }
}

/// The size of a file or of a block device.
pub fn file_len(f: &mut File) -> io::Result<u64> {
    let len = f.seek(SeekFrom::End(0))?;
    f.seek(SeekFrom::Start(0))?;
    Ok(len)
}

/// `pread` of `total` bytes in chunks of `chunk` bytes from `file` (of
/// `file_len` bytes, from `file_start`) into `m`. Both offsets wrap.
pub fn pread(
    m: &Mapping,
    file: &File,
    file_len: u64,
    file_start: u64,
    chunk: usize,
    total: u64,
) -> io::Result<Work> {
    if chunk == 0 || chunk > m.len() || file_len < chunk as u64 {
        return Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            "chunk size does not fit",
        ));
    }
    let mut foff = file_start - file_start % chunk as u64;
    let mut moff = 0usize;
    let mut work = Work::default();
    let t0 = Instant::now();
    while work.bytes < total {
        if foff + chunk as u64 > file_len {
            foff = 0;
        }
        if moff + chunk > m.len() {
            moff = 0;
        }
        let n = m.pread_from(file, foff, moff, chunk)?;
        if n != chunk {
            return Err(io::Error::new(
                io::ErrorKind::UnexpectedEof,
                format!("pread gave {n} of {chunk} bytes"),
            ));
        }
        foff += chunk as u64;
        moff += chunk;
        work.bytes += chunk as u64;
        work.ops += 1;
    }
    work.time = t0.elapsed();
    Ok(work)
}

/// Run `job` in one thread for each mapping, all started together. Each
/// thread owns its mapping. Returns the work of all threads together; the
/// time is that of the slowest thread.
pub fn parallel<F>(maps: Vec<Mapping>, job: F) -> io::Result<Work>
where
    F: Fn(usize, &Mapping) -> io::Result<Work> + Sync,
{
    let barrier = Barrier::new(maps.len());
    let results: Vec<io::Result<Work>> = thread::scope(|s| {
        let handles: Vec<_> = maps
            .into_iter()
            .enumerate()
            .map(|(i, m)| {
                let (barrier, job) = (&barrier, &job);
                s.spawn(move || {
                    barrier.wait();
                    job(i, &m)
                })
            })
            .collect();
        handles
            .into_iter()
            .map(|h| {
                h.join()
                    .unwrap_or_else(|_| Err(io::Error::other("thread failed")))
            })
            .collect()
    });
    let mut total = Work::default();
    for r in results {
        let w = r?;
        total.bytes += w.bytes;
        total.ops += w.ops;
        total.time = total.time.max(w.time);
    }
    Ok(total)
}

/// The CSV header of [`row`].
pub const HEADER: &str = "test,threads,unit_bytes,bytes,seconds,mib_per_s,ns_per_op";

/// One CSV row: rate of all threads, and time of one operation of one thread.
pub fn row(test: &str, threads: usize, unit: usize, w: &Work) -> String {
    let s = w.time.as_secs_f64();
    let per_thread_ops = w.ops as f64 / threads.max(1) as f64;
    format!(
        "{test},{threads},{unit},{},{s:.6},{:.1},{:.1}",
        w.bytes,
        w.bytes as f64 / s / f64::from(1u32 << 20),
        s * 1e9 / per_thread_ops
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs::OpenOptions;

    fn temp(tag: &str, len: u64) -> (File, std::path::PathBuf) {
        let path = std::env::temp_dir().join(format!("memprobe-ps-{tag}-{}", std::process::id()));
        let f = OpenOptions::new()
            .read(true)
            .write(true)
            .create(true)
            .truncate(true)
            .open(&path)
            .unwrap();
        f.set_len(len).unwrap();
        (f, path)
    }

    #[test]
    fn word_loops_count_and_write() {
        let (f, path) = temp("words", 8192);
        let m = Mapping::new(&f, 0, 8192).unwrap();
        let w = words(&m, Access::Write64, 3);
        assert_eq!((w.ops, w.bytes), (3 * 1024, 3 * 8192));
        // The last pass (2) wrote i ^ 2.
        assert_eq!(m.read64(8 * 5), 5 ^ 2);
        let r = words(&m, Access::Read32, 2);
        assert_eq!((r.ops, r.bytes), (2 * 2048, 2 * 8192));
        std::fs::remove_file(path).unwrap();
    }

    #[test]
    fn pread_wraps_and_copies() {
        let (src, sp) = temp("src", 3 * 1024);
        std::fs::write(
            &sp,
            (0..3 * 1024)
                .map(|i| (i / 1024) as u8 + 1)
                .collect::<Vec<u8>>(),
        )
        .unwrap();
        let (dst, dp) = temp("dst", 2048);
        let m = Mapping::new(&dst, 0, 2048).unwrap();
        // 5 chunks of 1 KiB from file offset 1 KiB: file chunks 1, 2, 0, 1, 2;
        // mapping chunks 0, 1, 0, 1, 0.
        let w = pread(&m, &src, 3 * 1024, 1024, 1024, 5 * 1024).unwrap();
        assert_eq!((w.ops, w.bytes), (5, 5 * 1024));
        assert_eq!(m.read32(0), u32::from_ne_bytes([3; 4]));
        assert_eq!(m.read32(1024), u32::from_ne_bytes([2; 4]));
        assert!(pread(&m, &src, 3 * 1024, 0, 4096, 1).is_err());
        std::fs::remove_file(sp).unwrap();
        std::fs::remove_file(dp).unwrap();
    }

    #[test]
    fn parallel_sums_work() {
        let (f, path) = temp("par", 8192);
        let maps = vec![
            Mapping::new(&f, 0, 4096).unwrap(),
            Mapping::new(&f, 4096, 4096).unwrap(),
        ];
        let w = parallel(maps, |_, m| Ok(words(m, Access::Read64, 1))).unwrap();
        assert_eq!((w.ops, w.bytes), (1024, 8192));
        std::fs::remove_file(path).unwrap();
    }

    #[test]
    fn csv_row() {
        let w = Work {
            bytes: 1 << 20,
            ops: 1000,
            time: Duration::from_millis(500),
        };
        assert_eq!(
            row("read32", 2, 4, &w),
            "read32,2,4,1048576,0.500000,2.0,1000000.0"
        );
        assert_eq!(
            HEADER.split(',').count(),
            row("x", 1, 4, &w).split(',').count()
        );
    }
}
