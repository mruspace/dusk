//! dusk: a Monte Carlo of redundancy policies on hardware that only ever
//! decays. `model` holds the physics and the policies; this file runs them
//! many times in parallel and holds the sweep that both binaries share.

pub mod model;
pub mod rng;

use model::{simulate, simulate_traced, Config, History, Outcome, Policy, DAYS_PER_YEAR};

/// Every run's outcome under every policy, in `Policy::ALL` order. Each run's
/// seed depends only on its index, so the thread count never changes results.
pub fn run_all(cfg: &Config, runs: usize, seed: u64, threads: usize) -> Vec<[Outcome; 3]> {
    run_policies(cfg, runs, seed, threads, &Policy::ALL)
        .into_iter()
        .map(|r| [r[0], r[1], r[2]])
        .collect()
}

/// Every run's outcome under each of `policies`, in that order, on the same
/// histories `run_all` uses.
pub fn run_policies(cfg: &Config, runs: usize, seed: u64, threads: usize, policies: &[Policy]) -> Vec<Vec<Outcome>> {
    let mut results = vec![Vec::new(); runs];
    let chunk = runs.div_ceil(threads.max(1)).max(1);
    std::thread::scope(|s| {
        for (c, slice) in results.chunks_mut(chunk).enumerate() {
            s.spawn(move || {
                for (i, slot) in slice.iter_mut().enumerate() {
                    let h = History::generate(cfg, run_seed(seed, c * chunk + i));
                    *slot = policies.iter().map(|&p| simulate(cfg, &h, p)).collect();
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

/// A 95% interval from the paired bootstrap: resample whole missions with
/// replacement, recompute `stat` on each resample, take the 2.5th and 97.5th
/// percentiles. Missions are resampled, not policies, so the pairing holds.
pub fn bootstrap(n: usize, seed: u64, stat: impl Fn(&[usize]) -> f64) -> (f64, f64) {
    const RESAMPLES: usize = 2000;
    let mut rng = rng::SplitMix64::new(rng::mix(seed ^ 0xB007_57A9));
    let mut idx = vec![0; n];
    let mut v: Vec<f64> = (0..RESAMPLES)
        .map(|_| {
            idx.iter_mut().for_each(|i| *i = rng.below(n));
            stat(&idx)
        })
        .filter(|x| x.is_finite())
        .collect();
    v.sort_by(|a, b| a.total_cmp(b));
    let at = |q: f64| v[((q * (v.len() - 1) as f64).round() as usize).min(v.len() - 1)];
    (at(0.025), at(0.975))
}

/// Sum of `f` over the missions picked by `idx`, for policy column `p`.
pub fn sum_at<R: AsRef<[Outcome]>>(r: &[R], idx: &[usize], p: usize, f: impl Fn(&Outcome) -> f64) -> f64 {
    idx.iter().map(|&i| f(&r[i].as_ref()[p])).sum()
}

/// Ratio of total useful work, policy `a` over policy `b`, with its 95% interval.
pub fn work_ratio<R: AsRef<[Outcome]> + Sync>(r: &[R], a: usize, b: usize, seed: u64) -> (f64, (f64, f64)) {
    let all: Vec<usize> = (0..r.len()).collect();
    let stat = |idx: &[usize]| sum_at(r, idx, a, |o| o.useful_days) / sum_at(r, idx, b, |o| o.useful_days);
    (stat(&all), bootstrap(r.len(), seed, stat))
}

/// How many years of useful work one wrong result must cost before policy `b`
/// beats policy `a`: extra work over extra wrong results. `None` when `a` makes
/// no more wrong results than `b`, so no price tips the balance.
pub fn break_even_years<R: AsRef<[Outcome]>>(r: &[R], idx: &[usize], a: usize, b: usize) -> Option<f64> {
    let extra_wrong = sum_at(r, idx, a, |o| o.wrong_days as f64) - sum_at(r, idx, b, |o| o.wrong_days as f64);
    let extra_work = sum_at(r, idx, a, |o| o.useful_days) - sum_at(r, idx, b, |o| o.useful_days);
    (extra_wrong > 0.0).then(|| extra_work / extra_wrong / DAYS_PER_YEAR)
}

/// One point on the ground curve: support lasting `ground_years`, a fallback
/// command `latency_days` after TMR loses its majority.
pub struct GroundPoint {
    pub ground_years: f64,
    pub latency_days: u64,
    /// Shrinking quorum's useful work over TMR + ground fallback's.
    pub ratio: f64,
    pub ci: (f64, f64),
}

/// The shrinking quorum against TMR with a ground team, for each support
/// lifetime and latency. `f64::INFINITY` means support never ends.
pub fn ground_curve(
    cfg: &Config,
    runs: usize,
    seed: u64,
    threads: usize,
    ground_years: &[f64],
    latencies: &[u64],
) -> Vec<GroundPoint> {
    let mut policies = vec![Policy::ShrinkingQuorum];
    for &g in ground_years {
        for &l in latencies {
            let ground_days = if g.is_finite() { (g * DAYS_PER_YEAR) as u64 } else { u64::MAX };
            policies.push(Policy::GroundFallback { latency_days: l, ground_days });
        }
    }
    let r = run_policies(cfg, runs, seed, threads, &policies);
    let mut out = Vec::new();
    let mut col = 1;
    for &g in ground_years {
        for &l in latencies {
            let (ratio, ci) = work_ratio(&r, 0, col, seed);
            out.push(GroundPoint { ground_years: g, latency_days: l, ratio, ci });
            col += 1;
        }
    }
    out
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
    fn bootstrap_brackets_the_point_estimate() {
        let r = run_all(&Config::default(), 300, 2, 4);
        let (point, (lo, hi)) = work_ratio(&r, 2, 0, 2);
        assert!(lo < point && point < hi, "{lo} {point} {hi}");
        assert!(lo >= 1.0);
    }

    #[test]
    fn ground_curve_ends_match_the_bracketing_policies() {
        let cfg = Config::default();
        let pts = ground_curve(&cfg, 200, 4, 4, &[0.0, f64::INFINITY], &[0]);
        let r = run_all(&cfg, 200, 4, 4);
        let (vs_tmr, _) = work_ratio(&r, 2, 0, 4);
        assert!((pts[0].ratio - vs_tmr).abs() < 1e-12);
        assert!((pts[1].ratio - 1.0).abs() < 1e-12);
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
