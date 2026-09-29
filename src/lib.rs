//! dusk: a Monte Carlo of redundancy policies on hardware that only ever
//! decays. `model` holds the physics and the policies; this file runs them
//! many times in parallel and holds the sweep that both binaries share.

pub mod model;
pub mod rng;

use model::{simulate, simulate_traced, Config, History, Outcome, Policy, DAYS_PER_YEAR};

/// Every run's outcome under every policy, in `Policy::ALL` order. Each run's
/// seed depends only on its index, so the thread count never changes results.
pub fn run_all(cfg: &Config, runs: usize, seed: u64, threads: usize) -> Vec<[Outcome; 3]> {
    let mut results = vec![[Outcome::default(); 3]; runs];
    let chunk = runs.div_ceil(threads.max(1)).max(1);
    std::thread::scope(|s| {
        for (c, slice) in results.chunks_mut(chunk).enumerate() {
            s.spawn(move || {
                for (i, slot) in slice.iter_mut().enumerate() {
                    let h = History::generate(cfg, run_seed(seed, c * chunk + i));
                    *slot = Policy::ALL.map(|p| simulate(cfg, &h, p));
                }
            });
        }
    });
    results
}

/// Mean useful output per day, averaged over all runs, for each mission year
/// up to `years`. 1.0 is a full voting set working every day. Uses the same
/// seeds as `run_all`, so it describes the same missions.
pub fn timeline(cfg: &Config, runs: usize, seed: u64, threads: usize, years: usize) -> [Vec<f64>; 3] {
    let chunk = runs.div_ceil(threads.max(1)).max(1);
    let parts: Vec<[Vec<f64>; 3]> = std::thread::scope(|s| {
        let handles: Vec<_> = (0..runs)
            .step_by(chunk)
            .map(|start| {
                s.spawn(move || {
                    let mut acc = [vec![0.0; years], vec![0.0; years], vec![0.0; years]];
                    for run in start..(start + chunk).min(runs) {
                        let h = History::generate(cfg, run_seed(seed, run));
                        for (p, policy) in Policy::ALL.into_iter().enumerate() {
                            simulate_traced(cfg, &h, policy, |day, credit| {
                                let y = (day as f64 / DAYS_PER_YEAR) as usize;
                                if y < years {
                                    acc[p][y] += credit;
                                }
                            });
                        }
                    }
                    acc
                })
            })
            .collect();
        handles.into_iter().map(|h| h.join().expect("worker panicked")).collect()
    });
    let mut out = [vec![0.0; years], vec![0.0; years], vec![0.0; years]];
    for part in parts {
        for (total, series) in out.iter_mut().zip(part) {
            total.iter_mut().zip(series).for_each(|(t, v)| *t += v);
        }
    }
    let per_run_year = runs as f64 * DAYS_PER_YEAR;
    out.iter_mut().flatten().for_each(|v| *v /= per_run_year);
    out
}

fn run_seed(seed: u64, run: usize) -> u64 {
    rng::mix(seed.wrapping_add(run as u64))
}

/// Summed totals across runs, in `Policy::ALL` order.
pub struct Totals {
    pub useful_days: [f64; 3],
    pub wrong_days: [f64; 3],
}

impl Totals {
    pub fn of(results: &[[Outcome; 3]]) -> Self {
        let mut t = Totals { useful_days: [0.0; 3], wrong_days: [0.0; 3] };
        for r in results {
            for (p, o) in r.iter().enumerate() {
                t.useful_days[p] += o.useful_days;
                t.wrong_days[p] += o.wrong_days as f64;
            }
        }
        t
    }

    /// Shrinking quorum's useful work as a multiple of fixed TMR's.
    pub fn work_vs_tmr(&self) -> f64 {
        self.useful_days[2] / self.useful_days[0]
    }

    /// Shrinking quorum's wrong results as a fraction of standby simplex's,
    /// or `None` when simplex produced none to compare against.
    pub fn wrong_vs_simplex(&self) -> Option<f64> {
        (self.wrong_days[1] > 0.0).then(|| self.wrong_days[2] / self.wrong_days[1])
    }
}

/// One parameter the model cannot pin down, and the values the sweep tries.
pub struct Sweep {
    pub name: &'static str,
    pub configs: Vec<(String, Config)>,
}

/// Each uncertain parameter varied on its own, the rest at `base`.
pub fn sweeps(base: &Config) -> Vec<Sweep> {
    fn vary<T: Copy + std::fmt::Display>(
        name: &'static str,
        values: &[T],
        label: impl Fn(T) -> String,
        set: impl Fn(&mut Config, T),
        base: &Config,
    ) -> Sweep {
        let configs = values
            .iter()
            .map(|&v| {
                let mut c = base.clone();
                set(&mut c, v);
                (label(v), c)
            })
            .collect();
        Sweep { name, configs }
    }
    vec![
        vary("Weibull shape", &[1.0, 1.5, 2.5, 4.0], |x| format!("shape {x}"), |c, x| c.shape = x, base),
        vary("Correlated deaths", &[0.0, 0.1, 0.3, 0.6], |x| format!("p_corr {x}"), |c, x| c.p_corr = x, base),
        vary("Salvaged spares", &[0usize, 1, 2, 4], |x| format!("spares {x}"), |c, x| c.spares = x, base),
        vary("Upset rate", &[1e-5, 1e-4, 1e-3, 1e-2], |x| format!("p_upset {x:e}"), |c, x| c.p_upset = x, base),
        vary("Self-check coverage", &[0.9, 0.99, 0.999], |x| format!("coverage {x}"), |c, x| c.coverage = x, base),
        vary(
            "Self-check throughput",
            &[0.25, 0.5, 1.0],
            |x| format!("self-check throughput {x}"),
            |c, x| c.selfcheck_throughput = x,
            base,
        ),
    ]
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn timeline_integrates_to_the_same_useful_work() {
        let cfg = Config { horizon_years: 600.0, ..Config::default() };
        let (runs, seed) = (64, 5);
        let t = Totals::of(&run_all(&cfg, runs, seed, 3));
        let line = timeline(&cfg, runs, seed, 2, 601);
        for (p, series) in line.iter().enumerate() {
            let area_years: f64 = series.iter().sum();
            let mean_years = t.useful_days[p] / DAYS_PER_YEAR / runs as f64;
            assert!((area_years - mean_years).abs() < 1e-6, "policy {p}: {area_years} vs {mean_years}");
        }
    }

    #[test]
    fn thread_count_does_not_change_results() {
        let cfg = Config::default();
        let a = Totals::of(&run_all(&cfg, 50, 3, 1));
        let b = Totals::of(&run_all(&cfg, 50, 3, 6));
        assert_eq!(a.useful_days, b.useful_days);
        assert_eq!(a.wrong_days, b.wrong_days);
    }
}
