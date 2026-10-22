use crate::profile::probes::{StageStats, NUM_STAGES, STAGE_NAMES};

#[derive(Clone, Copy)]
pub struct StageBudget {
    pub stage: usize,
    pub max_avg_cycles: u64,
    pub max_p99_cycles: u64,
}

pub struct LatencyBudget {
    pub stages: Vec<StageBudget>,
}

impl LatencyBudget {
    pub fn hft_default() -> Self {
        Self {
            stages: vec![
                StageBudget { stage: 16, max_avg_cycles: 200, max_p99_cycles: 500 },
                StageBudget { stage: 17, max_avg_cycles: 300, max_p99_cycles: 800 },
                StageBudget { stage: 18, max_avg_cycles: 500, max_p99_cycles: 1500 },
                StageBudget { stage: 14, max_avg_cycles: 100, max_p99_cycles: 300 },
                StageBudget { stage: 15, max_avg_cycles: 100, max_p99_cycles: 300 },
                StageBudget { stage: 26, max_avg_cycles: 5000, max_p99_cycles: 15000 },
                StageBudget { stage: 21, max_avg_cycles: 1000, max_p99_cycles: 3000 },
                StageBudget { stage: 22, max_avg_cycles: 1000, max_p99_cycles: 3000 },
                StageBudget { stage: 23, max_avg_cycles: 2000, max_p99_cycles: 5000 },
                StageBudget { stage: 24, max_avg_cycles: 500, max_p99_cycles: 1500 },
                StageBudget { stage: 25, max_avg_cycles: 1000, max_p99_cycles: 3000 },
            ],
        }
    }

    pub fn check(&self, stats: &[StageStats; NUM_STAGES], ghz: f64) -> BudgetReport {
        let mut results = Vec::with_capacity(self.stages.len());
        let mut all_pass = true;

        for budget in &self.stages {
            let s = &stats[budget.stage];
            if s.count == 0 {
                results.push(BudgetResult {
                    stage_name: STAGE_NAMES[budget.stage],
                    count: 0,
                    avg_cycles: 0,
                    max_cycles: 0,
                    avg_ns: 0.0,
                    max_ns: 0.0,
                    budget_avg: budget.max_avg_cycles,
                    budget_max: budget.max_p99_cycles,
                    pass: true,
                });
                continue;
            }

            let avg = s.total_cycles / s.count;
            let max = s.max_cycles;
            let avg_ok = avg <= budget.max_avg_cycles;
            let max_ok = max <= budget.max_p99_cycles;
            let pass = avg_ok && max_ok;
            if !pass {
                all_pass = false;
            }

            results.push(BudgetResult {
                stage_name: STAGE_NAMES[budget.stage],
                count: s.count,
                avg_cycles: avg,
                max_cycles: max,
                avg_ns: avg as f64 / ghz,
                max_ns: max as f64 / ghz,
                budget_avg: budget.max_avg_cycles,
                budget_max: budget.max_p99_cycles,
                pass,
            });
        }

        BudgetReport {
            results,
            all_pass,
        }
    }
}

pub struct BudgetResult {
    pub stage_name: &'static str,
    pub count: u64,
    pub avg_cycles: u64,
    pub max_cycles: u64,
    pub avg_ns: f64,
    pub max_ns: f64,
    pub budget_avg: u64,
    pub budget_max: u64,
    pub pass: bool,
}

pub struct BudgetReport {
    pub results: Vec<BudgetResult>,
    pub all_pass: bool,
}

impl BudgetReport {
    pub fn print(&self) {
        println!("{:<24} {:>10} {:>10} {:>10} {:>8} {:>8} {:>6}",
            "Stage", "Avg(cyc)", "Max(cyc)", "Count", "Avg(ns)", "Max(ns)", "Pass");
        println!("{}", "-".repeat(82));
        for r in &self.results {
            if r.count == 0 {
                println!("{:<24} {:>10} {:>10} {:>10} {:>8} {:>8} {:>6}",
                    r.stage_name, "-", "-", "0", "-", "-", "N/A");
            } else {
                println!("{:<24} {:>10} {:>10} {:>10} {:>8.1} {:>8.1} {:>6}",
                    r.stage_name,
                    r.avg_cycles,
                    r.max_cycles,
                    r.count,
                    r.avg_ns,
                    r.max_ns,
                    if r.pass { "OK" } else { "FAIL" },
                );
            }
        }
        println!("{}", "-".repeat(82));
        println!("Overall: {}", if self.all_pass { "PASS" } else { "FAIL" });
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::profile::probes::StageStats;

    #[test]
    fn budget_check_pass() {
        let budget = LatencyBudget::hft_default();
        let mut stats = [StageStats::default(); NUM_STAGES];
        stats[16] = StageStats {
            count: 1000,
            total_cycles: 100_000,
            min_cycles: 50,
            max_cycles: 200,
        };
        let report = budget.check(&stats, 3.2);
        assert!(report.all_pass);
    }

    #[test]
    fn budget_check_fail() {
        let budget = LatencyBudget::hft_default();
        let mut stats = [StageStats::default(); NUM_STAGES];
        stats[16] = StageStats {
            count: 1000,
            total_cycles: 500_000,
            min_cycles: 100,
            max_cycles: 2000,
        };
        let report = budget.check(&stats, 3.2);
        assert!(!report.all_pass);
    }
}
