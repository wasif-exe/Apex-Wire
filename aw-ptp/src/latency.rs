#[derive(Debug, Clone)]
pub struct LatencyHistogram {
    samples: Vec<u64>,
}
impl LatencyHistogram {
    pub fn new(capacity: usize) -> Self {
        Self {
            samples: Vec::with_capacity(capacity),
        }
    }
    #[inline(always)]
    pub fn record(&mut self, nanos: u64) {
        self.samples.push(nanos);
    }
    pub fn compute_percentiles(&mut self) -> Option<LatencySummary> {
        if self.samples.is_empty() {
            return None;
        }
        self.samples.sort_unstable();
        let len = self.samples.len();
        let p50_idx = (len * 50) / 100;
        let p90_idx = (len * 90) / 100;
        let p99_idx = (len * 99) / 100;
        let p99_9_idx = (len * 999) / 1000;
        let p99_99_idx = (len * 9999) / 10000;
        let sum: u64 = self.samples.iter().sum();
        let mean = sum as f64 / len as f64;
        Some(LatencySummary {
            count: len as u64,
            min: self.samples[0],
            p50: self.samples[p50_idx.min(len - 1)],
            p90: self.samples[p90_idx.min(len - 1)],
            p99: self.samples[p99_idx.min(len - 1)],
            p99_9: self.samples[p99_9_idx.min(len - 1)],
            p99_99: self.samples[p99_99_idx.min(len - 1)],
            max: self.samples[len - 1],
            mean,
        })
    }
}
#[derive(Debug, Clone, Copy, Default)]
pub struct LatencySummary {
    pub count: u64,
    pub min: u64,
    pub p50: u64,
    pub p90: u64,
    pub p99: u64,
    pub p99_9: u64,
    pub p99_99: u64,
    pub max: u64,
    pub mean: f64,
}
