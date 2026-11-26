//! A memory load on the PS for B1 and B2 (docs/mem-path-experiment.md): each
//! thread copies between two buffers of 16 MiB, much larger than the L2
//! cache, so that the copies go to the DDR memory through the CPU cache.

use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::thread::{self, JoinHandle};
use std::time::Instant;

const BUF_BYTES: usize = 16 << 20;

/// Threads that copy memory until [`Load::stop`].
pub struct Load {
    stop: Arc<AtomicBool>,
    threads: Vec<JoinHandle<u64>>,
    start: Instant,
}

impl Load {
    /// Start `threads` threads. With 0 threads, there is no load.
    pub fn start(threads: u32) -> Load {
        Load::with_size(threads, BUF_BYTES)
    }

    fn with_size(threads: u32, bytes: usize) -> Load {
        let stop = Arc::new(AtomicBool::new(false));
        let threads = (0..threads)
            .map(|i| {
                let stop = Arc::clone(&stop);
                thread::spawn(move || {
                    let mut a = vec![i as u8 + 1; bytes];
                    let mut b = vec![0u8; bytes];
                    let mut copied = 0u64;
                    while !stop.load(Ordering::Relaxed) {
                        b.copy_from_slice(&a);
                        std::mem::swap(&mut a, &mut b);
                        copied += bytes as u64;
                    }
                    // Keep the copies: the compiler must not remove them.
                    std::hint::black_box(&a);
                    copied
                })
            })
            .collect();
        Load {
            stop,
            threads,
            start: Instant::now(),
        }
    }

    /// Stop the threads. Returns the copy rate in MiB/s (read and write
    /// together count once).
    pub fn stop(self) -> f64 {
        self.stop.store(true, Ordering::Relaxed);
        let bytes: u64 = self
            .threads
            .into_iter()
            .map(|t| t.join().unwrap_or(0))
            .sum();
        bytes as f64 / self.start.elapsed().as_secs_f64() / f64::from(1u32 << 20)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn load_copies_and_stops() {
        let l = Load::with_size(2, 1 << 16);
        thread::sleep(std::time::Duration::from_millis(20));
        assert!(l.stop() > 0.0);
        assert_eq!(Load::start(0).stop(), 0.0);
    }
}
