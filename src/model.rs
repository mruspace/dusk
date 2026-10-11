//! The fault model and the redundancy policies.
//!
//! A run has two halves. `History::generate` draws everything the universe
//! does to the hardware: when each processor dies, when a salvaged spare
//! becomes available, and which deaths take a neighbor with them. `simulate`
//! then plays one policy against that history a day at a time. Every policy
//! reads the same history and the same keyed upsets, so any difference between
//! them is the policy and nothing else.

use crate::rng::{keyed_unit, SplitMix64};

pub const DAYS_PER_YEAR: f64 = 365.25;

/// Salt for the self-check coverage draw, so it is independent of the upset
/// draw for the same unit and day.
const COVERAGE_SALT: u64 = 0xC0FF_EE00_DEAD_BEEF;

/// Column of each policy in `Policy::ALL`, and so in `run_all` results.
pub const TMR: usize = 0;
pub const SIMPLEX: usize = 1;
pub const SHRINK: usize = 2;
pub const UNCHECKED: usize = 3;
pub const COLD: usize = 4;

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
    /// How fast an unpowered spare ages under cold standby, relative to a
    /// powered unit: 0 never ages on the shelf, 1 ages as if it were on. It
    /// is a value to sweep, not a measured one. Only `ColdStandby` reads it.
    pub cold_aging: f64,
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
            cold_aging: 0.3,
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
    /// Each unit's own Weibull death day, before any correlated death. Cold
    /// standby reads it as the age at which the unit wears out.
    pub wear: Vec<u64>,
    /// For a unit that a correlated death took, the unit whose death took it.
    pub killed_by: Vec<Option<usize>>,
}

impl History {
    pub fn generate(cfg: &Config, seed: u64) -> Self {
        let mut rng = SplitMix64::new(seed);
        let mut units = Vec::with_capacity(cfg.nodes + cfg.spares);
        for _ in 0..cfg.nodes {
            units.push(Unit { joins: 0, dies: cfg.lifetime_days(&mut rng) });
        }
        // A spare is the processor of an instrument that failed around it. It
        // joins when the instrument dies, and it has been aging in the same
        // radiation since launch, so it can die before it is ever salvaged.
        for _ in 0..cfg.spares {
            let joins = cfg.lifetime_days(&mut rng);
            let dies = cfg.lifetime_days(&mut rng);
            units.push(Unit { joins, dies });
        }
        let wear = units.iter().map(|u| u.dies).collect();
        let killed_by = correlate(&mut units, cfg.p_corr, &mut rng);
        Self { seed, units, wear, killed_by }
    }

    /// A history with these units and no correlated deaths.
    pub fn from_units(seed: u64, units: Vec<Unit>) -> Self {
        let wear = units.iter().map(|u| u.dies).collect();
        let killed_by = vec![None; units.len()];
        Self { seed, units, wear, killed_by }
    }
}

/// Walk deaths in time order. Each one, with probability `p`, kills one other
/// processor that is in service that day. The victim's death is walked in turn,
/// so cascades happen naturally. Returns, for each victim, its killer.
fn correlate(units: &mut [Unit], p: f64, rng: &mut SplitMix64) -> Vec<Option<usize>> {
    let mut killed_by = vec![None; units.len()];
    if p <= 0.0 {
        return killed_by;
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
            let v = victims[rng.below(victims.len())];
            units[v].dies = t;
            killed_by[v] = Some(i);
        }
    }
    killed_by
}

/// The day a unit wears out under cold standby. It ages at full rate until it
/// `joins` (a salvaged spare sits powered in its instrument), at `rate` while
/// it waits unpowered, and at full rate again once it `activates`. It wears
/// out on the first day its age reaches `wear`. `None` means it is never
/// switched on, so it can only die on the shelf.
fn cold_death(wear: u64, joins: u64, activates: Option<u64>, rate: f64) -> u64 {
    if wear <= joins {
        return wear;
    }
    let left = (wear - joins) as f64;
    // A float-to-int cast saturates, so a huge shelf life becomes u64::MAX.
    let shelf = || if rate <= 0.0 { u64::MAX } else { joins.saturating_add((left / rate).ceil() as u64) };
    match activates {
        None => shelf(),
        Some(a) => {
            let aged = rate * (a - joins) as f64;
            if aged >= left {
                shelf()
            } else {
                a.saturating_add((left - aged).ceil() as u64)
            }
        }
    }
}

/// The units as cold standby sees them: one powered, the rest cold, switched
/// on in index order when the active one dies. Each returned unit is in
/// service only while it is powered, so at most one is alive on any day. A
/// unit that is never switched on has an empty interval.
///
/// A correlated death keeps the victim and killer of the powered history: the
/// victim dies on the day its killer dies here, if it has joined by then and
/// has not already died. A unit that lives longer here than in the powered
/// history is never picked as a new victim, so this slightly under-correlates.
pub fn cold_timeline(cfg: &Config, h: &History) -> Vec<Unit> {
    let n = h.units.len();
    let joins: Vec<u64> = h.units.iter().map(|u| u.joins).collect();
    let mut on: Vec<Option<u64>> = vec![None; n];
    let mut dies: Vec<Option<u64>> = vec![None; n];
    if n > 0 {
        on[0] = Some(joins[0]);
    }
    // Fix one death at a time, the earliest first. A death can only make the
    // next activation earlier, never move a fixed death.
    loop {
        let candidate = |i: usize| {
            let own = cold_death(h.wear[i], joins[i], on[i], cfg.cold_aging);
            match h.killed_by[i].and_then(|k| dies[k]) {
                Some(t) if joins[i] <= t => own.min(t),
                _ => own,
            }
        };
        let Some((day, i)) = (0..n).filter(|&i| dies[i].is_none()).map(|i| (candidate(i), i)).min() else {
            break;
        };
        dies[i] = Some(day);
        if on[i].is_some() {
            let next = (0..n).filter(|&j| dies[j].is_none()).min_by_key(|&j| (day.max(joins[j]), j));
            if let Some(j) = next {
                on[j] = Some(day.max(joins[j]));
            }
        }
    }
    (0..n)
        .map(|i| {
            let d = dies[i].unwrap_or(u64::MAX);
            Unit { joins: on[i].unwrap_or(d), dies: d }
        })
        .collect()
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Policy {
    /// Triple modular redundancy with a fixed majority. Replaces a dead member
    /// from any spare, runs on as a pair once it cannot, and stops for good
    /// when it drops below two: a lone processor cannot outvote anything.
    FixedTmr,
    /// One self-checking processor at a time, swapping in the next when it
    /// dies. Lives as long as any hardware does but never votes. The spares
    /// stay powered, so they age as fast as the active one.
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
    /// Fixed TMR that falls back on its own to a lone processor that does not
    /// check itself. It runs at full rate on one processor, and every upset
    /// there is a wrong result. It isolates what self-check is worth.
    UncheckedFallback,
    /// Every unit, one powered and the rest cold, switched on in index order
    /// when the active one dies. The active one self-checks, as in standby
    /// simplex. A cold spare ages at `Config::cold_aging` of the powered rate.
    /// The nearest model here of dual-string with autonomous switchover.
    ColdStandby,
}

impl Policy {
    pub const COUNT: usize = 5;
    pub const ALL: [Policy; Policy::COUNT] = [
        Policy::FixedTmr,
        Policy::StandbySimplex,
        Policy::ShrinkingQuorum,
        Policy::UncheckedFallback,
        Policy::ColdStandby,
    ];

    pub fn name(self) -> &'static str {
        match self {
            Policy::FixedTmr => "fixed TMR",
            Policy::StandbySimplex => "standby simplex",
            Policy::ShrinkingQuorum => "shrinking quorum",
            Policy::GroundFallback { .. } => "TMR + ground fallback",
            Policy::UncheckedFallback => "TMR + unchecked simplex",
            Policy::ColdStandby => "cold standby",
        }
    }
}

/// What one policy delivered on one mission. A day is one result. Every day
/// in service is exactly one of useful, lost, wrong or idle.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct Outcome {
    /// Sum over mission days of the throughput credit for a correct result.
    /// A vote with at most one upset among its three: 1.0. A pair with no
    /// upset: 1.0. A self-checking loner with no upset: `selfcheck_throughput`.
    /// An unchecked loner with no upset: 1.0. Lost, wrong and idle days: 0.
    pub useful_days: f64,
    /// Days whose result was caught as bad and thrown away: a vote with two
    /// or more upsets, a pair with any upset, or a self-checking loner whose
    /// check caught its upset.
    pub lost_days: u64,
    /// Days whose delivered result was corrupted by an upset nothing caught.
    /// Only a lone processor makes one: a self-checking loner whose check
    /// missed its upset, or an unchecked loner with any upset. A vote or a
    /// pair never does, by construction.
    pub wrong_days: u64,
    /// The first day the policy can never produce output again, or the
    /// horizon. Fixed TMR: the first day with fewer than two live processors.
    /// The others: the first day with none (TMR + ground fallback also ends
    /// when it loses its majority after support has ended). Lost, wrong and
    /// idle days count as service.
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
    /// A lone processor with no self-check: full rate, nothing caught.
    Unchecked,
    /// Alive but waiting for the ground: no output, no end.
    Idle,
}

pub fn simulate(cfg: &Config, h: &History, policy: Policy) -> Outcome {
    simulate_traced(cfg, h, policy, |_, _| {})
}

/// `simulate`, also reporting each day's useful output to `credit` as it is
/// earned. The charts use this to plot output against mission time.
pub fn simulate_traced(cfg: &Config, h: &History, policy: Policy, mut credit: impl FnMut(u64, f64)) -> Outcome {
    let cold;
    let units: &[Unit] = if policy == Policy::ColdStandby {
        cold = cold_timeline(cfg, h);
        &cold
    } else {
        &h.units
    };
    let mut out = Outcome::default();
    let mut live = Vec::with_capacity(units.len());
    let upset = |unit: usize, day: u64| keyed_unit(h.seed, unit as u64, day) < cfg.p_upset;
    let mut fallback_at: Option<u64> = None;

    for day in 0..cfg.horizon_days() {
        live.clear();
        live.extend((0..units.len()).filter(|&i| units[i].alive_on(day)));
        let k = live.len().min(3);

        let voting = |k: usize| if k == 3 { Mode::Vote } else { Mode::Pair };
        let mode = match (policy, k) {
            (_, 0) | (Policy::FixedTmr, 1) => None,
            (Policy::StandbySimplex | Policy::ColdStandby, _) | (Policy::ShrinkingQuorum, 1) => Some(Mode::Single),
            (Policy::UncheckedFallback, 1) => Some(Mode::Unchecked),
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
            Mode::Unchecked => {
                if upset(live[0], day) {
                    out.wrong_days += 1;
                } else {
                    out.useful_days += 1.0;
                    credit(day, 1.0);
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
        History::from_units(1, deaths_years.iter().map(|&y| Unit { joins: 0, dies: y * d }).collect())
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
    fn unchecked_fallback_trades_wrong_results_for_rate() {
        let cfg = Config::default();
        for s in 0..200 {
            let h = History::generate(&cfg, s);
            let u = simulate(&cfg, &h, Policy::UncheckedFallback);
            let q = simulate(&cfg, &h, Policy::ShrinkingQuorum);
            assert!(u.useful_days >= q.useful_days, "seed {s}");
            assert_eq!(u.end_day, q.end_day, "seed {s}");
            assert!(u.wrong_days >= q.wrong_days, "seed {s}");
        }
    }

    #[test]
    fn unchecked_fallback_runs_the_tail_at_full_rate() {
        let d = DAYS_PER_YEAR as u64;
        let h = fixed(&[10, 40, 90]);
        let o = simulate(&quiet(), &h, Policy::UncheckedFallback);
        assert_eq!(o.end_day, 90 * d);
        // Full rate for 40 years as TMR, then full rate for 50 on one.
        assert_eq!(o.useful_days, (40 * d) as f64 + (50 * d) as f64);
        assert_eq!(o.wrong_days, 0);
    }

    #[test]
    fn cold_standby_at_full_aging_is_standby_simplex() {
        let cfg = Config { p_corr: 0.0, cold_aging: 1.0, ..Config::default() };
        for s in 0..200 {
            let h = History::generate(&cfg, s);
            assert_eq!(simulate(&cfg, &h, Policy::ColdStandby), simulate(&cfg, &h, Policy::StandbySimplex), "seed {s}");
        }
    }

    #[test]
    fn cold_spares_age_on_the_shelf_clock() {
        // fixed() counts 365 days a year. Rate 0.5, deaths at 10, 40, 90 y.
        // Unit 0 runs from day 0 and dies on day 3650.
        // Unit 1 has aged 0.5 * 3650 = 1825 days when it is switched on, so
        // it dies on 3650 + (14600 - 1825) = 16425.
        // Unit 2 has aged 0.5 * 16425 = 8212.5 days by then, so it dies on
        // 16425 + ceil(32850 - 8212.5) = 16425 + 24638 = 41063.
        let cfg = Config { cold_aging: 0.5, ..quiet() };
        let h = fixed(&[10, 40, 90]);
        let units = cold_timeline(&cfg, &h);
        assert_eq!(
            units,
            vec![Unit { joins: 0, dies: 3650 }, Unit { joins: 3650, dies: 16425 }, Unit { joins: 16425, dies: 41063 }]
        );
        let o = simulate(&cfg, &h, Policy::ColdStandby);
        assert_eq!(o.end_day, 41063);
        assert_eq!(o.useful_days, 0.5 * 41063.0);
    }

    #[test]
    fn a_cold_spare_can_die_on_the_shelf() {
        // Unit 0 outlives both spares, which wear out unpowered at full rate.
        let cfg = Config { cold_aging: 1.0, ..quiet() };
        let h = fixed(&[90, 40, 10]);
        assert_eq!(simulate(&cfg, &h, Policy::ColdStandby).end_day, 90 * DAYS_PER_YEAR as u64);
    }

    #[test]
    fn cold_standby_never_ends_sooner_than_standby_simplex() {
        for rate in [0.0, 0.1, 0.5] {
            let cfg = Config { p_corr: 0.0, cold_aging: rate, ..Config::default() };
            for s in 0..200 {
                let h = History::generate(&cfg, s);
                let cold = simulate(&cfg, &h, Policy::ColdStandby);
                let hot = simulate(&cfg, &h, Policy::StandbySimplex);
                assert!(cold.end_day >= hot.end_day, "rate {rate} seed {s}");
            }
        }
    }

    #[test]
    fn correlated_deaths_shorten_cold_standby() {
        let (calm, corr) = (Config { p_corr: 0.0, ..Config::default() }, Config { p_corr: 1.0, ..Config::default() });
        let mut shorter = 0;
        for s in 0..200 {
            let a = simulate(&calm, &History::generate(&calm, s), Policy::ColdStandby);
            let b = simulate(&corr, &History::generate(&corr, s), Policy::ColdStandby);
            assert!(b.end_day <= a.end_day, "seed {s}");
            shorter += (b.end_day < a.end_day) as usize;
        }
        assert!(shorter > 0);
    }

    #[test]
    fn a_huge_shelf_life_does_not_overflow() {
        assert_eq!(cold_death(100, 0, None, 0.0), u64::MAX);
        assert_eq!(cold_death(u64::MAX - 1, 10, None, 1e-300), u64::MAX);
        assert_eq!(cold_death(100, 0, Some(40), 0.0), 140);
        assert_eq!(cold_death(100, 120, Some(130), 0.5), 100);
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
        assert_eq!(base.wear, corr.wear);
    }

    #[test]
    fn histories_are_reproducible() {
        let cfg = Config { spares: 2, ..Config::default() };
        assert_eq!(History::generate(&cfg, 42).units, History::generate(&cfg, 42).units);
    }
}
