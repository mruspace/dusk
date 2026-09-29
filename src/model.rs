//! The fault model and the three redundancy policies.
//!
//! A run has two halves. `History::generate` draws everything the universe
//! does to the hardware: when each processor dies, when a salvaged spare
//! becomes available, and which deaths take a neighbour with them. `simulate`
//! then plays one policy against that history a day at a time. Every policy
//! reads the same history and the same keyed upsets, so any difference between
//! them is the policy and nothing else.

use crate::rng::{keyed_unit, SplitMix64};

pub const DAYS_PER_YEAR: f64 = 365.25;

/// Salt for the self-check coverage draw, so it is independent of the upset
/// draw for the same unit and day.
const COVERAGE_SALT: u64 = 0xC0FF_EE00_DEAD_BEEF;

#[derive(Clone, Debug)]
pub struct Config {
    pub horizon_years: f64,
    /// Processors in service at launch.
    pub nodes: usize,
    /// Processors salvaged later from instruments that die around them.
    pub spares: usize,
    /// Weibull shape. Above 1 means wear-out: the hazard rises with age.
    pub shape: f64,
    /// Weibull scale, the age by which 63% of processors have died.
    pub scale_years: f64,
    /// Chance that a death also kills one other processor in service that
    /// day. It can cascade.
    pub p_corr: f64,
    /// Chance, per processor per day, of a transient fault that corrupts that
    /// day's result. This is what gets past EDAC and scrubbing, not the raw
    /// bit-flip rate.
    pub p_upset: f64,
    /// Fraction of upsets a lone processor's self-check catches.
    pub coverage: f64,
    /// Useful output of a lone self-checking processor, as a fraction of a
    /// voting set's. Running everything twice and comparing costs about half.
    pub selfcheck_throughput: f64,
}

impl Default for Config {
    fn default() -> Self {
        Self {
            horizon_years: 1000.0,
            nodes: 3,
            spares: 0,
            shape: 1.5,
            scale_years: 125.0,
            p_corr: 0.1,
            p_upset: 1e-3,
            coverage: 0.99,
            selfcheck_throughput: 0.5,
        }
    }
}

impl Config {
    pub fn horizon_days(&self) -> u64 {
        (self.horizon_years * DAYS_PER_YEAR) as u64
    }

    fn lifetime_days(&self, rng: &mut SplitMix64) -> u64 {
        // Inverse CDF. 1 - u is in (0, 1], so the log is finite.
        let u = rng.next_f64();
        let years = self.scale_years * (-(1.0 - u).ln()).powf(1.0 / self.shape);
        (years * DAYS_PER_YEAR) as u64
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Unit {
    /// First day in service.
    pub joins: u64,
    /// First day dead.
    pub dies: u64,
}

impl Unit {
    pub fn alive_on(&self, day: u64) -> bool {
        self.joins <= day && day < self.dies
    }
}

#[derive(Clone, Debug)]
pub struct History {
    pub seed: u64,
    pub units: Vec<Unit>,
}

impl History {
    pub fn generate(cfg: &Config, seed: u64) -> Self {
        let mut rng = SplitMix64::new(seed);
        let mut units = Vec::with_capacity(cfg.nodes + cfg.spares);
        for _ in 0..cfg.nodes {
            units.push(Unit { joins: 0, dies: cfg.lifetime_days(&mut rng) });
        }
        // A spare is the processor of an instrument that failed around it. It
        // joins when the instrument dies, and it has been ageing in the same
        // radiation since launch, so it can die before it is ever salvaged.
        for _ in 0..cfg.spares {
            let joins = cfg.lifetime_days(&mut rng);
            let dies = cfg.lifetime_days(&mut rng);
            units.push(Unit { joins, dies });
        }
        correlate(&mut units, cfg.p_corr, &mut rng);
        Self { seed, units }
    }
}

/// Walk deaths in time order. Each one, with probability `p`, kills one other
/// processor that is in service that day. The victim's death is walked in turn,
/// so cascades happen naturally.
fn correlate(units: &mut [Unit], p: f64, rng: &mut SplitMix64) {
    if p <= 0.0 {
        return;
    }
    let mut walked = vec![false; units.len()];
    while let Some(i) = (0..units.len()).filter(|&i| !walked[i]).min_by_key(|&i| units[i].dies) {
        walked[i] = true;
        let t = units[i].dies;
        if rng.next_f64() >= p {
            continue;
        }
        let victims: Vec<usize> = (0..units.len()).filter(|&j| !walked[j] && units[j].alive_on(t)).collect();
        if !victims.is_empty() {
            units[victims[rng.below(victims.len())]].dies = t;
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Policy {
    /// Triple modular redundancy with a fixed majority. Replaces a dead member
    /// from any spare, runs on as a pair once it cannot, and stops for good
    /// when it drops below two: a lone processor cannot outvote anything.
    FixedTmr,
    /// One self-checking processor at a time, swapping in the next when it
    /// dies. Lives as long as any hardware does but never votes.
    StandbySimplex,
    /// Mru's policy. Votes while three are alive, compares while two are, and
    /// self-checks on one. The quorum shrinks with the hardware instead of
    /// failing at a threshold.
    ShrinkingQuorum,
    /// Fixed TMR with a ground team behind it. When TMR drops to one
    /// processor, the ground notices and commands it into self-checking simplex
    /// `latency_days` later, provided the ground is still there: support ends
    /// on day `ground_days`, and after that TMR stops as it would alone. This
    /// is what real missions do, and the baseline autonomy must beat.
    GroundFallback { latency_days: u64, ground_days: u64 },
}

impl Policy {
    pub const ALL: [Policy; 3] = [Policy::FixedTmr, Policy::StandbySimplex, Policy::ShrinkingQuorum];

    pub fn name(self) -> &'static str {
        match self {
            Policy::FixedTmr => "fixed TMR",
            Policy::StandbySimplex => "standby simplex",
            Policy::ShrinkingQuorum => "shrinking quorum",
            Policy::GroundFallback { .. } => "TMR + ground fallback",
        }
    }
}

#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct Outcome {
    /// Days of correct output, weighted by throughput.
    pub useful_days: f64,
    /// Days whose result was caught as bad and thrown away.
    pub lost_days: u64,
    /// Days that produced a wrong result nobody caught.
    pub wrong_days: u64,
    /// First day with no output possible, or the horizon.
    pub end_day: u64,
}

impl Outcome {
    pub fn useful_years(&self) -> f64 {
        self.useful_days / DAYS_PER_YEAR
    }

    pub fn end_years(&self) -> f64 {
        self.end_day as f64 / DAYS_PER_YEAR
    }
}

enum Mode {
    Vote,
    Pair,
    Single,
    /// Alive but waiting for the ground: no output, no end.
    Idle,
}

pub fn simulate(cfg: &Config, h: &History, policy: Policy) -> Outcome {
    simulate_traced(cfg, h, policy, |_, _| {})
}

/// `simulate`, also reporting each day's useful output to `credit` as it is
/// earned. The charts use this to plot output against mission time.
pub fn simulate_traced(cfg: &Config, h: &History, policy: Policy, mut credit: impl FnMut(u64, f64)) -> Outcome {
    let mut out = Outcome::default();
    let mut live = Vec::with_capacity(h.units.len());
    let upset = |unit: usize, day: u64| keyed_unit(h.seed, unit as u64, day) < cfg.p_upset;
    let mut fallback_at: Option<u64> = None;

    for day in 0..cfg.horizon_days() {
        live.clear();
        live.extend((0..h.units.len()).filter(|&i| h.units[i].alive_on(day)));
        let k = live.len().min(3);

        let voting = |k: usize| if k == 3 { Mode::Vote } else { Mode::Pair };
        let mode = match (policy, k) {
            (_, 0) | (Policy::FixedTmr, 1) => None,
            (Policy::StandbySimplex, _) | (Policy::ShrinkingQuorum, 1) => Some(Mode::Single),
            (Policy::GroundFallback { latency_days, ground_days }, 1) => {
                // The command is sent once, when TMR first loses its majority.
                let at = *fallback_at.get_or_insert(if day < ground_days {
                    day.saturating_add(latency_days)
                } else {
                    u64::MAX
                });
                match at {
                    u64::MAX => None,
                    at if day < at => Some(Mode::Idle),
                    _ => Some(Mode::Single),
                }
            }
            (_, k) => Some(voting(k)),
        };
        let Some(mode) = mode else {
            out.end_day = day;
            return out;
        };

        // The lowest-numbered live processors do the work; the rest stand by.
        match mode {
            Mode::Vote => {
                // One bad vote is outvoted. Two or more is treated as a split
                // the voter catches, never as a wrong majority.
                if live[..3].iter().filter(|&&u| upset(u, day)).count() <= 1 {
                    out.useful_days += 1.0;
                    credit(day, 1.0);
                } else {
                    out.lost_days += 1;
                }
            }
            Mode::Pair => {
                // A pair detects any disagreement but cannot say who is right.
                if live[..2].iter().any(|&u| upset(u, day)) {
                    out.lost_days += 1;
                } else {
                    out.useful_days += 1.0;
                    credit(day, 1.0);
                }
            }
            Mode::Single => {
                let u = live[0];
                if !upset(u, day) {
                    out.useful_days += cfg.selfcheck_throughput;
                    credit(day, cfg.selfcheck_throughput);
                } else if keyed_unit(h.seed ^ COVERAGE_SALT, u as u64, day) < cfg.coverage {
                    out.lost_days += 1;
                } else {
                    out.wrong_days += 1;
                }
            }
            Mode::Idle => {}
        }
    }
    out.end_day = cfg.horizon_days();
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn quiet() -> Config {
        Config { p_corr: 0.0, p_upset: 0.0, ..Config::default() }
    }

    fn fixed(deaths_years: &[u64]) -> History {
        let d = DAYS_PER_YEAR as u64;
        History { seed: 1, units: deaths_years.iter().map(|&y| Unit { joins: 0, dies: y * d }).collect() }
    }

    #[test]
    fn fixed_tmr_stops_at_the_second_death() {
        let h = fixed(&[10, 40, 90]);
        let o = simulate(&quiet(), &h, Policy::FixedTmr);
        assert_eq!(o.end_day, 40 * DAYS_PER_YEAR as u64);
        assert_eq!(o.wrong_days, 0);
    }

    #[test]
    fn shrinking_quorum_runs_to_the_last_death_at_reduced_rate() {
        let h = fixed(&[10, 40, 90]);
        let o = simulate(&quiet(), &h, Policy::ShrinkingQuorum);
        let d = DAYS_PER_YEAR as u64;
        assert_eq!(o.end_day, 90 * d);
        // Full rate for 40 years, then half rate for 50.
        assert_eq!(o.useful_days, (40 * d) as f64 + 0.5 * (50 * d) as f64);
    }

    #[test]
    fn shrinking_quorum_never_does_less_than_fixed_tmr() {
        let cfg = Config::default();
        for s in 0..200 {
            let h = History::generate(&cfg, s);
            let a = simulate(&cfg, &h, Policy::FixedTmr);
            let b = simulate(&cfg, &h, Policy::ShrinkingQuorum);
            assert!(b.useful_days >= a.useful_days, "seed {s}");
            assert!(b.end_day >= a.end_day, "seed {s}");
        }
    }

    #[test]
    fn a_vote_masks_single_upsets_and_a_loner_does_not() {
        let cfg = Config { p_corr: 0.0, p_upset: 0.05, coverage: 0.0, ..Config::default() };
        let h = fixed(&[500, 500, 500]);
        let vote = simulate(&cfg, &h, Policy::ShrinkingQuorum);
        let alone = simulate(&cfg, &h, Policy::StandbySimplex);
        assert_eq!(vote.wrong_days, 0);
        assert!(alone.wrong_days > 0);
    }

    #[test]
    fn ground_fallback_spans_fixed_tmr_to_shrinking_quorum() {
        let cfg = Config::default();
        let never = Policy::GroundFallback { latency_days: 30, ground_days: 0 };
        let instant = Policy::GroundFallback { latency_days: 0, ground_days: u64::MAX };
        for s in 0..100 {
            let h = History::generate(&cfg, s);
            assert_eq!(simulate(&cfg, &h, never), simulate(&cfg, &h, Policy::FixedTmr), "seed {s}");
            assert_eq!(simulate(&cfg, &h, instant), simulate(&cfg, &h, Policy::ShrinkingQuorum), "seed {s}");
        }
    }

    #[test]
    fn ground_fallback_waits_out_the_latency() {
        let d = DAYS_PER_YEAR as u64;
        let h = fixed(&[10, 40, 90]);
        let p = Policy::GroundFallback { latency_days: 100, ground_days: u64::MAX };
        let o = simulate(&quiet(), &h, p);
        assert_eq!(o.end_day, 90 * d);
        assert_eq!(o.useful_days, (40 * d) as f64 + 0.5 * (50 * d - 100) as f64);
        // Support that ended before TMR lost its majority cannot help.
        let gone = Policy::GroundFallback { latency_days: 1, ground_days: 39 * d };
        assert_eq!(simulate(&quiet(), &h, gone).end_day, 40 * d);
    }

    #[test]
    fn weibull_median_matches_the_formula() {
        let cfg = Config::default();
        let mut rng = SplitMix64::new(3);
        let mut v: Vec<u64> = (0..50_000).map(|_| cfg.lifetime_days(&mut rng)).collect();
        v.sort_unstable();
        let median_years = v[v.len() / 2] as f64 / DAYS_PER_YEAR;
        let expect = cfg.scale_years * std::f64::consts::LN_2.powf(1.0 / cfg.shape);
        assert!((median_years - expect).abs() / expect < 0.02, "{median_years} vs {expect}");
    }

    #[test]
    fn correlation_only_pulls_deaths_earlier() {
        let base = History::generate(&Config { p_corr: 0.0, ..Config::default() }, 9);
        let corr = History::generate(&Config { p_corr: 1.0, ..Config::default() }, 9);
        for (a, b) in base.units.iter().zip(&corr.units) {
            assert!(b.dies <= a.dies);
        }
    }

    #[test]
    fn histories_are_reproducible() {
        let cfg = Config { spares: 2, ..Config::default() };
        assert_eq!(History::generate(&cfg, 42).units, History::generate(&cfg, 42).units);
    }
}
