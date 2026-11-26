//! Statistics of a latency histogram of the block memprobe. Bin n counts the
//! samples with latency n; the last bin counts the samples with that latency
//! or more (spec/regmap.toml).

/// A latency from the histogram.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Latency {
    /// The latency is this number of cycles.
    Exact(u32),
    /// The latency is in the last bin: this number of cycles or more.
    AtLeast(u32),
}

impl std::fmt::Display for Latency {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Latency::Exact(n) => write!(f, "{n}"),
            Latency::AtLeast(n) => write!(f, ">={n}"),
        }
    }
}

/// Summary of one histogram, in cycles.
#[derive(Clone, Debug, PartialEq)]
pub struct Summary {
    pub count: u64,
    pub min: u32,
    pub median: Latency,
    pub p99: Latency,
    pub max: u32,
    pub mean: f64,
}

/// The latency of the sample with the rank ceil(q * count), from 1.
pub fn quantile(bins: &[u32], q: f64) -> Option<Latency> {
    let count: u64 = bins.iter().map(|&b| u64::from(b)).sum();
    if count == 0 || bins.is_empty() {
        return None;
    }
    let rank = ((q * count as f64).ceil() as u64).clamp(1, count);
    let mut seen = 0u64;
    for (i, &b) in bins.iter().enumerate() {
        seen += u64::from(b);
        if seen >= rank {
            let i = i as u32;
            return Some(if i as usize == bins.len() - 1 {
                Latency::AtLeast(i)
            } else {
                Latency::Exact(i)
            });
        }
    }
    None
}

/// Summary from the bins and from MIN, MAX and SUM of the design. `samples`
/// is the number of samples (READS or WRITES); it can be more than the sum of
/// the bins if samples were dropped. None if there is no sample.
pub fn summarize(bins: &[u32], min: u32, max: u32, sum: u64, samples: u64) -> Option<Summary> {
    if samples == 0 {
        return None;
    }
    Some(Summary {
        count: samples,
        min,
        median: quantile(bins, 0.5)?,
        p99: quantile(bins, 0.99)?,
        max,
        mean: sum as f64 / samples as f64,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    // Expected values computed by hand.
    #[test]
    fn quantiles_of_small_histograms() {
        // latencies 1, 1, 2, 3+
        let bins = [0, 2, 1, 1];
        assert_eq!(quantile(&bins, 0.5), Some(Latency::Exact(1)));
        assert_eq!(quantile(&bins, 0.75), Some(Latency::Exact(2)));
        assert_eq!(quantile(&bins, 0.99), Some(Latency::AtLeast(3)));
        assert_eq!(quantile(&bins, 0.0), Some(Latency::Exact(1)));
        // 100 samples: 98 at 10, 1 at 20, 1 at 30. Rank 99 is 20.
        let mut b = vec![0u32; 64];
        b[10] = 98;
        b[20] = 1;
        b[30] = 1;
        assert_eq!(quantile(&b, 0.99), Some(Latency::Exact(20)));
        assert_eq!(quantile(&b, 0.5), Some(Latency::Exact(10)));
        assert_eq!(quantile(&[0, 0], 0.5), None);
    }

    #[test]
    fn summary() {
        let s = summarize(&[0, 2, 1, 1], 1, 1200, 1204, 4).unwrap();
        assert_eq!(s.count, 4);
        assert_eq!(s.median, Latency::Exact(1));
        assert_eq!(s.p99, Latency::AtLeast(3));
        assert_eq!(s.max, 1200);
        assert!((s.mean - 301.0).abs() < 1e-9);
        assert_eq!(s.p99.to_string(), ">=3");
        assert!(summarize(&[0, 0], 0, 0, 0, 0).is_none());
    }
}
