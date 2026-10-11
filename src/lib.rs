//! dusk: a Monte Carlo of redundancy policies on hardware that only ever
//! decays. `model` holds the physics and the policies; this file runs them
//! many times in parallel and holds the sweep that both binaries share.

pub mod bench;
pub mod model;
pub mod rng;

use model::{simulate, simulate_traced, Config, History, Outcome, Policy, DAYS_PER_YEAR};

/// Every run's outcome under every policy, in `Policy::ALL` order. Each run's
/// seed depends only on its index, so the thread count never changes results.
pub fn run_all(cfg: &Config, runs: usize, seed: u64, threads: usize) -> Vec<[Outcome; Policy::COUNT]> {
    run_policies(cfg, runs, seed, threads, &Policy::ALL).into_iter().map(|r| std::array::from_fn(|p| r[p])).collect()
}

/// Every run's outcome under each of `policies`, in that order, on the same
/// histories `run_all` uses.
pub fn run_policies(cfg: &Config, runs: usize, seed: u64, threads: usize, policies: &[Policy]) -> Vec<Vec<Outcome>> {
    run_columns(cfg, runs, seed, threads, policies, &[])
}

/// `run_policies`, then one more column of cold standby for each spare aging
/// rate in `cold_agings`, all on the same histories. The aging rate does not
/// change a history, so the comparison stays paired.
pub fn run_columns(
    cfg: &Config,
    runs: usize,
    seed: u64,
    threads: usize,
    policies: &[Policy],
    cold_agings: &[f64],
) -> Vec<Vec<Outcome>> {
    let mut results = vec![Vec::new(); runs];
    let chunk = runs.div_ceil(threads.max(1)).max(1);
    let colds: Vec<Config> = cold_agings.iter().map(|&g| Config { cold_aging: g, ..cfg.clone() }).collect();
    let colds = &colds;
    std::thread::scope(|s| {
        for (c, slice) in results.chunks_mut(chunk).enumerate() {
            s.spawn(move || {
                for (i, slot) in slice.iter_mut().enumerate() {
                    let h = History::generate(cfg, run_seed(seed, c * chunk + i));
                    *slot = policies.iter().map(|&p| simulate(cfg, &h, p)).collect();
                    slot.extend(colds.iter().map(|c| simulate(c, &h, Policy::ColdStandby)));
                }
            });
        }
    });
    results
}

/// Mean useful output per day, averaged over all runs, for each mission year
/// up to `years`. 1.0 is a full voting set working every day. Uses the same
/// seeds as `run_all`, so it describes the same missions.
pub fn timeline(cfg: &Config, runs: usize, seed: u64, threads: usize, years: usize) -> [Vec<f64>; Policy::COUNT] {
    let chunk = runs.div_ceil(threads.max(1)).max(1);
    let parts: Vec<[Vec<f64>; Policy::COUNT]> = std::thread::scope(|s| {
        let handles: Vec<_> = (0..runs)
            .step_by(chunk)
            .map(|start| {
                s.spawn(move || {
                    let mut acc: [Vec<f64>; Policy::COUNT] = std::array::from_fn(|_| vec![0.0; years]);
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
    let mut out: [Vec<f64>; Policy::COUNT] = std::array::from_fn(|_| vec![0.0; years]);
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

/// Summed totals across runs, one entry per result column. For `run_all`
/// results that is `Policy::ALL` order.
pub struct Totals {
    pub useful_days: Vec<f64>,
    pub wrong_days: Vec<f64>,
}

impl Totals {
    pub fn of<R: AsRef<[Outcome]>>(results: &[R]) -> Self {
        let cols = results.first().map_or(0, |r| r.as_ref().len());
        let mut t = Totals { useful_days: vec![0.0; cols], wrong_days: vec![0.0; cols] };
        for r in results {
            for (p, o) in r.as_ref().iter().enumerate() {
                t.useful_days[p] += o.useful_days;
                t.wrong_days[p] += o.wrong_days as f64;
            }
        }
        t
    }

    /// Column `a`'s useful work as a multiple of column `b`'s.
    pub fn work_vs(&self, a: usize, b: usize) -> f64 {
        self.useful_days[a] / self.useful_days[b]
    }

    /// Shrinking quorum's useful work as a multiple of fixed TMR's.
    pub fn work_vs_tmr(&self) -> f64 {
        self.work_vs(model::SHRINK, model::TMR)
    }

    /// Shrinking quorum's wrong results as a fraction of standby simplex's,
    /// or `None` when simplex produced none to compare against.
    pub fn wrong_vs_simplex(&self) -> Option<f64> {
        (self.wrong_days[model::SIMPLEX] > 0.0)
            .then(|| self.wrong_days[model::SHRINK] / self.wrong_days[model::SIMPLEX])
    }
}

/// The sweep's numbers for one configuration: fixed TMR, standby simplex and
/// the shrinking quorum in their usual columns, then cold standby at each of
/// `cold_agings`.
pub fn sweep_totals(cfg: &Config, runs: usize, seed: u64, threads: usize, cold_agings: &[f64]) -> Totals {
    let first = [Policy::FixedTmr, Policy::StandbySimplex, Policy::ShrinkingQuorum];
    Totals::of(&run_columns(cfg, runs, seed, threads, &first, cold_agings))
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

/// Ratio of total wrong results, policy `a` over policy `b`, with its 95%
/// interval.
pub fn wrong_ratio<R: AsRef<[Outcome]>>(r: &[R], a: usize, b: usize, seed: u64) -> (f64, (f64, f64)) {
    let all: Vec<usize> = (0..r.len()).collect();
    let w = |idx: &[usize], p: usize| sum_at(r, idx, p, |o| o.wrong_days as f64);
    let stat = |idx: &[usize]| w(idx, a) / w(idx, b);
    (stat(&all), bootstrap(r.len(), seed, stat))
}

/// Risk: the share of the missions picked by `idx` on which policy column `p`
/// delivered at least one wrong result.
pub fn risk<R: AsRef<[Outcome]>>(r: &[R], idx: &[usize], p: usize) -> f64 {
    idx.iter().filter(|&&i| r[i].as_ref()[p].wrong_days >= 1).count() as f64 / idx.len() as f64
}

/// Risk over all missions, with its 95% interval. A policy that never makes
/// a wrong result gets 0 with the interval (0, 0).
pub fn risk_ci<R: AsRef<[Outcome]>>(r: &[R], p: usize, seed: u64) -> (f64, (f64, f64)) {
    let all: Vec<usize> = (0..r.len()).collect();
    (risk(r, &all, p), bootstrap(r.len(), seed, |idx| risk(r, idx, p)))
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

/// The spare aging rates the cold standby table tries. 1.0 ages as fast as a
/// powered unit, which makes cold standby standby simplex.
pub const COLD_AGINGS: [f64; 10] = [0.1, 0.2, 0.3, 0.4, 0.5, 0.6, 0.7, 0.8, 0.9, 1.0];

/// Cold standby at one spare aging rate. A value with its 95% interval is
/// `(value, (lo, hi))`.
pub struct ColdPoint {
    pub aging: f64,
    pub useful_years: f64,
    pub service_years: f64,
    pub wrong_per_mission: f64,
    pub risk: (f64, (f64, f64)),
    /// Cold standby's useful work as a multiple of fixed TMR's.
    pub vs_tmr: (f64, (f64, f64)),
    /// Cold standby's useful work as a multiple of the shrinking quorum's.
    pub vs_shrink: (f64, (f64, f64)),
}

/// Cold standby at each of `agings`, against fixed TMR and the shrinking
/// quorum on the same missions.
pub fn cold_curve(cfg: &Config, runs: usize, seed: u64, threads: usize, agings: &[f64]) -> Vec<ColdPoint> {
    let r = run_columns(cfg, runs, seed, threads, &[Policy::FixedTmr, Policy::ShrinkingQuorum], agings);
    let mean = |c: usize, f: fn(&Outcome) -> f64| r.iter().map(|o| f(&o[c])).sum::<f64>() / r.len() as f64;
    agings
        .iter()
        .enumerate()
        .map(|(k, &aging)| {
            let c = 2 + k;
            ColdPoint {
                aging,
                useful_years: mean(c, Outcome::useful_years),
                service_years: mean(c, Outcome::end_years),
                wrong_per_mission: mean(c, |o| o.wrong_days as f64),
                risk: risk_ci(&r, c, seed),
                vs_tmr: work_ratio(&r, c, 0, seed),
                vs_shrink: work_ratio(&r, c, 1, seed),
            }
        })
        .collect()
}

/// Where cold standby stops outworking the shrinking quorum as spares age
/// faster.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum ColdCrossing {
    /// Cold standby does more useful work below about this aging rate,
    /// read off the curve by straight-line interpolation.
    Below(f64),
    /// Cold standby does more useful work at every rate tried.
    Always,
    /// Cold standby never does more useful work at any rate tried.
    Never,
}

pub fn cold_crossing(points: &[ColdPoint]) -> ColdCrossing {
    let over = |p: &ColdPoint| p.vs_shrink.0 > 1.0;
    if points.iter().all(over) {
        return ColdCrossing::Always;
    }
    match points.windows(2).find(|w| over(&w[0]) && !over(&w[1])) {
        None => ColdCrossing::Never,
        Some(w) => {
            let (a, b) = (&w[0], &w[1]);
            let f = (a.vs_shrink.0 - 1.0) / (a.vs_shrink.0 - b.vs_shrink.0);
            ColdCrossing::Below(a.aging + f * (b.aging - a.aging))
        }
    }
}

/// The sentence the sweep closes with, and the page quotes.
pub fn cold_crossing_line(points: &[ColdPoint]) -> String {
    match cold_crossing(points) {
        ColdCrossing::Below(g) => {
            format!("cold standby outworks the shrinking quorum below a spare aging rate of about {g:.2}")
        }
        ColdCrossing::Always => "cold standby outworks the shrinking quorum at every spare aging rate tried".into(),
        ColdCrossing::Never => "cold standby never outworks the shrinking quorum in this range".into(),
    }
}

/// The number formats the report prints. The charts use the same ones for
/// the strings the README quotes, so the two cannot drift apart.
pub mod fmt {
    /// A ratio: 1.34x.
    pub fn x(v: f64) -> String {
        format!("{v:.2}x")
    }

    /// A share, whole percent: 41%.
    pub fn pct(v: f64) -> String {
        format!("{:.0}%", 100.0 * v)
    }

    /// A share to one decimal: 21.8%.
    pub fn pct1(v: f64) -> String {
        format!("{:.1}%", 100.0 * v)
    }

    /// A small share to two decimals: 0.99%.
    pub fn pct2(v: f64) -> String {
        format!("{:.2}%", 100.0 * v)
    }

    /// Whole years: 136.
    pub fn years(v: f64) -> String {
        format!("{v:.0}")
    }

    /// An interval: "1.33x to 1.35x".
    pub fn range((lo, hi): (f64, f64), f: impl Fn(f64) -> String) -> String {
        format!("{} to {}", f(lo), f(hi))
    }

    /// An aging rate as a label: 0.1, 0.25, 1.0.
    pub fn aging(g: f64) -> String {
        let s = format!("{g}");
        if s.contains('.') {
            s
        } else {
            format!("{g:.1}")
        }
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
    use model::{COLD, SHRINK, SIMPLEX, TMR};

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
        let (point, (lo, hi)) = work_ratio(&r, SHRINK, TMR, 2);
        assert!(lo < point && point < hi, "{lo} {point} {hi}");
        assert!(lo >= 1.0);
    }

    #[test]
    fn ground_curve_ends_match_the_bracketing_policies() {
        let cfg = Config::default();
        let pts = ground_curve(&cfg, 200, 4, 4, &[0.0, f64::INFINITY], &[0]);
        let r = run_all(&cfg, 200, 4, 4);
        let (vs_tmr, _) = work_ratio(&r, SHRINK, TMR, 4);
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

    #[test]
    fn extra_cold_columns_match_run_all() {
        let cfg = Config::default();
        let a = run_all(&cfg, 60, 7, 3);
        let b = run_columns(&cfg, 60, 7, 2, &[Policy::FixedTmr], &[cfg.cold_aging, 1.0]);
        let c = run_all(&Config { cold_aging: 1.0, ..cfg.clone() }, 60, 7, 3);
        for i in 0..60 {
            assert_eq!(b[i], vec![a[i][TMR], a[i][COLD], c[i][COLD]], "run {i}");
        }
    }

    #[test]
    fn risk_is_zero_for_fixed_tmr_and_bounded() {
        let r = run_all(&Config::default(), 300, 2, 4);
        assert_eq!(risk_ci(&r, TMR, 2), (0.0, (0.0, 0.0)));
        let (v, (lo, hi)) = risk_ci(&r, SIMPLEX, 2);
        assert!(0.0 < lo && lo <= v && v <= hi && hi < 1.0, "{lo} {v} {hi}");
    }

    #[test]
    fn crossing_is_read_off_the_curve() {
        let p = |aging: f64, v: f64| ColdPoint {
            aging,
            useful_years: 0.0,
            service_years: 0.0,
            wrong_per_mission: 0.0,
            risk: (0.0, (0.0, 0.0)),
            vs_tmr: (0.0, (0.0, 0.0)),
            vs_shrink: (v, (v, v)),
        };
        let ColdCrossing::Below(g) = cold_crossing(&[p(0.1, 1.2), p(0.2, 0.8)]) else { panic!("no crossing") };
        assert!((g - 0.15).abs() < 1e-12, "{g}");
        assert_eq!(cold_crossing(&[p(0.1, 1.2), p(0.2, 1.1)]), ColdCrossing::Always);
        assert_eq!(cold_crossing(&[p(0.1, 0.9), p(0.2, 0.8)]), ColdCrossing::Never);
    }
}
