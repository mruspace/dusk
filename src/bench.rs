//! Check the model against the bench of mruspace/flight.
//!
//! The bench runs the real `quorum` code on three processes, with replicas
//! killed at fixed ticks and upsets injected at a fixed rate. This module
//! replays the same schedules through `simulate`, one tick as one day, and
//! compares the means. It checks the policy model against the real code. It
//! does not calibrate the environment: the bench sets the upset rate and the
//! failure times itself, so it cannot measure them.
//!
//! Units: in self-check, one bench result spans two ticks, while the model
//! counts each day at half output. Correct results match as they are. A wrong
//! result in the model is a day, so it is converted to bench results by the
//! self-check throughput.

use crate::model::{simulate, Config, History, Policy, Unit, DAYS_PER_YEAR};
use crate::rng;

/// One bench run, the columns this check reads.
#[derive(Clone, Debug, PartialEq)]
pub struct Run {
    pub policy: String,
    pub upset_rate: f64,
    pub schedule: String,
    pub ticks: u64,
    pub useful: f64,
    pub wrong: f64,
    pub common_mode: f64,
    pub faults: String,
}

/// Read the bench CSV. Fails with a message on a missing column or a bad value.
pub fn parse(csv: &str) -> Result<Vec<Run>, String> {
    let mut lines = csv.lines();
    let header: Vec<&str> = lines.next().ok_or("empty file")?.split(',').collect();
    let col = |name: &str| {
        header
            .iter()
            .position(|h| *h == name)
            .ok_or(format!("no column {name}: use bench results from flight with a fault_schedule column"))
    };
    let c = [
        col("policy")?,
        col("upset_rate")?,
        col("schedule")?,
        col("ticks")?,
        col("useful")?,
        col("wrong")?,
        col("common_mode")?,
        col("fault_schedule")?,
    ];
    let mut runs = Vec::new();
    for (n, line) in lines.enumerate().filter(|(_, l)| !l.trim().is_empty()) {
        let f: Vec<&str> = line.split(',').collect();
        let get = |i: usize| f.get(c[i]).copied().ok_or(format!("line {}: too few columns", n + 2));
        let num = |i: usize| -> Result<f64, String> {
            get(i)?.parse::<f64>().map_err(|_| format!("line {}: not a number", n + 2))
        };
        runs.push(Run {
            policy: get(0)?.to_string(),
            upset_rate: num(1)?,
            schedule: get(2)?.to_string(),
            ticks: num(3)? as u64,
            useful: num(4)?,
            wrong: num(5)?,
            common_mode: num(6)?,
            faults: get(7)?.trim().to_string(),
        });
    }
    Ok(runs)
}

/// Replica death ticks from a fault schedule such as `kill:0@5000 kill:1@10000`.
/// `None` if the schedule has a fault the model does not have (stuck, hang,
/// corrupt): the model knows processors that die, not ones that lie.
pub fn deaths(faults: &str, ticks: u64) -> Option<[u64; 3]> {
    let mut dies = [u64::MAX; 3];
    for f in faults.split_whitespace() {
        let rest = f.strip_prefix("kill:")?;
        let (replica, tick) = rest.split_once('@')?;
        let r: usize = replica.parse().ok()?;
        let t: u64 = tick.parse().ok()?;
        // bench tick t is model day t - 1; a kill takes effect that tick
        *dies.get_mut(r)? = dies[r].min(t.saturating_sub(1));
    }
    Some(dies.map(|d| d.min(ticks)))
}

/// Model mean and spread for one group, over `samples` upset streams.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Prediction {
    pub samples: u64,
    pub useful: f64,
    pub useful_sd: f64,
    pub wrong: f64,
    pub wrong_sd: f64,
}

impl Prediction {
    /// How far a bench mean over `n` runs is from the model, in standard
    /// errors. The spread is the model's: if the model is right, the bench runs
    /// are draws from it. `slack` absorbs a known offset that is not noise.
    pub fn z(mean: f64, sd: f64, samples: u64, bench: f64, n: usize, slack: f64) -> f64 {
        let gap = (bench - mean).abs() - slack;
        if gap <= 0.0 {
            return 0.0;
        }
        let se = (sd * sd / n as f64 + sd * sd / samples as f64).sqrt();
        let z = if se > 0.0 { gap / se } else { f64::INFINITY };
        z.copysign(bench - mean)
    }
}

pub fn predict(run: &Run, policy: Policy, dies: [u64; 3], samples: u64, seed: u64) -> Prediction {
    let cfg = Config {
        // the + 0.5 keeps the floor in horizon_days at exactly `ticks`
        horizon_years: (run.ticks as f64 + 0.5) / DAYS_PER_YEAR,
        nodes: 3,
        spares: 0,
        p_corr: 0.0,
        p_upset: run.upset_rate,
        coverage: 1.0 - run.common_mode,
        selfcheck_throughput: 0.5,
        ..Config::default()
    };
    let units = dies.iter().map(|&d| Unit { joins: 0, dies: d }).collect::<Vec<_>>();
    let (mut u, mut u2, mut w, mut w2) = (0.0, 0.0, 0.0, 0.0);
    for s in 0..samples {
        let h = History::from_units(rng::mix(seed.wrapping_add(s)), units.clone());
        let o = simulate(&cfg, &h, policy);
        let wrong = o.wrong_days as f64 * cfg.selfcheck_throughput;
        u += o.useful_days;
        u2 += o.useful_days * o.useful_days;
        w += wrong;
        w2 += wrong * wrong;
    }
    let n = samples as f64;
    let sd = |sum: f64, sq: f64| (sq / n - (sum / n).powi(2)).max(0.0).sqrt();
    Prediction { samples, useful: u / n, useful_sd: sd(u, u2), wrong: w / n, wrong_sd: sd(w, w2) }
}

/// Half a result: in self-check the model earns half a result per day, the
/// bench a whole one every other tick, so the two can differ by one half.
pub const PARITY: f64 = 0.5;

pub fn mean(v: &[f64]) -> f64 {
    v.iter().sum::<f64>() / v.len() as f64
}

#[cfg(test)]
mod tests {
    use super::*;

    const CSV: &str = "policy,seed,upset_rate,schedule,ticks,useful,wrong,common_mode,fault_schedule\n\
        tmr,1,0,staggered-kills,20000,9999,0,0.05,kill:0@5000 kill:1@10000\n\
        shrink,1,0,staggered-kills,20000,15000,0,0.05,kill:0@5000 kill:1@10000\n\
        shrink,1,0,stuck,20000,14500,0,0.05,stuck:1@4000 kill:0@9000\n";

    #[test]
    fn reads_the_bench_columns() {
        let runs = parse(CSV).unwrap();
        assert_eq!(runs.len(), 3);
        assert_eq!(runs[0].faults, "kill:0@5000 kill:1@10000");
        assert!(parse("policy,seed\ntmr,1\n").is_err());
    }

    #[test]
    fn only_kills_are_modeled() {
        assert_eq!(deaths("kill:0@5000 kill:1@10000", 20000), Some([4999, 9999, 20000]));
        assert_eq!(deaths("", 20000), Some([20000; 3]));
        assert_eq!(deaths("stuck:1@4000 kill:0@9000", 20000), None);
    }

    #[test]
    fn without_upsets_the_model_gives_the_bench_counts() {
        // fixed TMR: correct results on ticks 1 to 9,999. Shrinking quorum: the
        // same, then one result every other tick up to 20,000.
        let runs = parse(CSV).unwrap();
        let dies = deaths(&runs[0].faults, 20000).unwrap();
        let tmr = predict(&runs[0], Policy::FixedTmr, dies, 3, 1);
        let shrink = predict(&runs[1], Policy::ShrinkingQuorum, dies, 3, 1);
        assert_eq!(tmr.useful, 9999.0);
        assert_eq!(shrink.useful, 9999.0 + 0.5 * 10001.0);
        assert_eq!((tmr.wrong, shrink.wrong), (0.0, 0.0));
        // the bench's 15,000 is within the self-check parity of the model
        assert_eq!(Prediction::z(shrink.useful, shrink.useful_sd, 3, 15000.0, 1, PARITY), 0.0);
    }

    #[test]
    fn z_uses_the_model_spread() {
        // 10 runs with no wrong result, where the model expects 0.02 per run
        let z = Prediction::z(0.02, 0.02f64.sqrt(), 1000, 0.0, 10, 0.0);
        assert!(z < 0.0 && z > -1.0, "{z}");
    }
}
