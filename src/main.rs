mod model;
mod rng;

use model::{simulate, Config, History, Outcome, Policy};
use std::process::exit;

const USAGE: &str = "\
Monte Carlo of redundancy policies on hardware that only ever decays.

usage: dusk [options] [--sweep]

  --runs N                 missions per policy          (default 2000)
  --seed N                 base seed                    (default 1)
  --threads N              worker threads               (default: all cores)
  --horizon-years X        mission horizon              (default 1000)
  --nodes N                processors at launch         (default 3)
  --spares N               salvaged processors          (default 0)
  --shape X                Weibull shape                (default 1.5)
  --scale-years X          Weibull scale                (default 125)
  --p-corr X               chance a death kills another (default 0.1)
  --p-upset X              bad-result upsets / node-day (default 0.001)
  --coverage X             lone self-check coverage     (default 0.99)
  --selfcheck-throughput X lone output vs a vote        (default 0.5)
  --sweep                  vary the uncertain parameters instead
";

struct Args {
    cfg: Config,
    runs: usize,
    seed: u64,
    threads: usize,
    sweep: bool,
}

fn parse() -> Args {
    let mut a = Args {
        cfg: Config::default(),
        runs: 2000,
        seed: 1,
        threads: std::thread::available_parallelism().map_or(4, |n| n.get()),
        sweep: false,
    };
    let mut it = std::env::args().skip(1);
    while let Some(flag) = it.next() {
        if flag == "--sweep" {
            a.sweep = true;
            continue;
        }
        if flag == "-h" || flag == "--help" {
            print!("{USAGE}");
            exit(0);
        }
        if !USAGE.contains(&format!("  {flag} ")) {
            fail(&format!("unknown option {flag}"));
        }
        let Some(v) = it.next() else { fail(&format!("{flag} needs a value")) };
        let f = || v.parse::<f64>().unwrap_or_else(|_| fail(&format!("{flag}: not a number: {v}")));
        let n = || v.parse::<usize>().unwrap_or_else(|_| fail(&format!("{flag}: not a count: {v}")));
        match flag.as_str() {
            "--runs" => a.runs = n(),
            "--seed" => a.seed = n() as u64,
            "--threads" => a.threads = n().max(1),
            "--horizon-years" => a.cfg.horizon_years = f(),
            "--nodes" => a.cfg.nodes = n(),
            "--spares" => a.cfg.spares = n(),
            "--shape" => a.cfg.shape = f(),
            "--scale-years" => a.cfg.scale_years = f(),
            "--p-corr" => a.cfg.p_corr = f(),
            "--p-upset" => a.cfg.p_upset = f(),
            "--coverage" => a.cfg.coverage = f(),
            "--selfcheck-throughput" => a.cfg.selfcheck_throughput = f(),
            _ => fail(&format!("unknown option {flag}")),
        }
    }
    a
}

fn fail(msg: &str) -> ! {
    eprint!("dusk: {msg}\n\n{USAGE}");
    exit(2)
}

/// Every run's outcome under every policy, in `Policy::ALL` order. Each run's
/// seed depends only on its index, so the thread count never changes results.
fn run_all(cfg: &Config, runs: usize, seed: u64, threads: usize) -> Vec<[Outcome; 3]> {
    let mut results = vec![[Outcome::default(); 3]; runs];
    let chunk = runs.div_ceil(threads).max(1);
    std::thread::scope(|s| {
        for (c, slice) in results.chunks_mut(chunk).enumerate() {
            s.spawn(move || {
                for (i, slot) in slice.iter_mut().enumerate() {
                    let run = (c * chunk + i) as u64;
                    let h = History::generate(cfg, rng::mix(seed.wrapping_add(run)));
                    *slot = Policy::ALL.map(|p| simulate(cfg, &h, p));
                }
            });
        }
    });
    results
}

fn mean(v: &[f64]) -> f64 {
    v.iter().sum::<f64>() / v.len() as f64
}

/// Nearest-rank percentile of a sorted slice.
fn pct(sorted: &[f64], p: f64) -> f64 {
    sorted[((p / 100.0) * (sorted.len() - 1) as f64).round() as usize]
}

fn column(results: &[[Outcome; 3]], p: usize, f: impl Fn(&Outcome) -> f64) -> Vec<f64> {
    let mut v: Vec<f64> = results.iter().map(|r| f(&r[p])).collect();
    v.sort_by(|a, b| a.total_cmp(b));
    v
}

fn report(a: &Args) {
    let c = &a.cfg;
    println!("{} missions, seed {}, horizon {} years", a.runs, a.seed, c.horizon_years);
    println!(
        "{} processors + {} spares; Weibull shape {}, scale {} y; p_corr {}; \
         p_upset {:e}/node-day; coverage {}; self-check throughput {}",
        c.nodes, c.spares, c.shape, c.scale_years, c.p_corr, c.p_upset, c.coverage, c.selfcheck_throughput
    );
    println!();

    let r = run_all(c, a.runs, a.seed, a.threads);

    println!(
        "{:<17} {:>8} {:>8} {:>8} {:>8}   {:>10} {:>12}",
        "policy", "useful", "p10", "p50", "p90", "service", "wrong days"
    );
    println!("{:<17} {:>8} {:>8} {:>8} {:>8}   {:>10} {:>12}", "", "mean y", "", "", "", "mean y", "mean / run");
    for (i, p) in Policy::ALL.iter().enumerate() {
        let useful = column(&r, i, Outcome::useful_years);
        let end = column(&r, i, Outcome::end_years);
        let wrong = column(&r, i, |o| o.wrong_days as f64);
        println!(
            "{:<17} {:>8.1} {:>8.1} {:>8.1} {:>8.1}   {:>10.1} {:>12.3}",
            p.name(),
            mean(&useful),
            pct(&useful, 10.0),
            pct(&useful, 50.0),
            pct(&useful, 90.0),
            mean(&end),
            mean(&wrong)
        );
    }

    let (tmr, simplex, shrink) = (0, 1, 2);
    let ratio: Vec<f64> = {
        let mut v: Vec<f64> = r
            .iter()
            .filter(|o| o[tmr].useful_days > 0.0)
            .map(|o| o[shrink].useful_days / o[tmr].useful_days)
            .collect();
        v.sort_by(|a, b| a.total_cmp(b));
        v
    };
    let sum = |p: usize, f: fn(&Outcome) -> f64| r.iter().map(|o| f(&o[p])).sum::<f64>();
    let wrong = |o: &Outcome| o.wrong_days as f64;

    println!();
    println!(
        "shrinking quorum vs fixed TMR: {:.2}x the useful work in total; per mission median {:.2}x (p10 {:.2}x, p90 {:.2}x)",
        sum(shrink, |o| o.useful_days) / sum(tmr, |o| o.useful_days),
        pct(&ratio, 50.0),
        pct(&ratio, 10.0),
        pct(&ratio, 90.0)
    );
    let ws = sum(simplex, wrong);
    if ws > 0.0 {
        println!(
            "shrinking quorum vs standby simplex: {:.0}% of the useful work, {:.0}% of the wrong results",
            100.0 * sum(shrink, |o| o.useful_days) / sum(simplex, |o| o.useful_days),
            100.0 * sum(shrink, wrong) / ws
        );
    }
}

/// Vary each parameter the model cannot pin down and report the two numbers
/// that matter: extra work over fixed TMR, and wrong results against simplex.
fn sweep(a: &Args) {
    let runs = a.runs.min(1000);
    println!("{runs} missions per row, seed {}; unlisted parameters at their defaults", a.seed);
    println!();
    println!(
        "{:<28} {:>12} {:>14} {:>14}",
        "varied", "work vs TMR", "wrong/mission", "wrong vs simplex"
    );

    let mut rows: Vec<(String, Config)> = Vec::new();
    let base = a.cfg.clone();
    for &x in &[1.0, 1.5, 2.5, 4.0] {
        rows.push((format!("shape {x}"), Config { shape: x, ..base.clone() }));
    }
    for &x in &[0.0, 0.1, 0.3, 0.6] {
        rows.push((format!("p_corr {x}"), Config { p_corr: x, ..base.clone() }));
    }
    for &x in &[0usize, 1, 2, 4] {
        rows.push((format!("spares {x}"), Config { spares: x, ..base.clone() }));
    }
    for &x in &[1e-5, 1e-4, 1e-3, 1e-2] {
        rows.push((format!("p_upset {x:e}"), Config { p_upset: x, ..base.clone() }));
    }
    for &x in &[0.9, 0.99, 0.999] {
        rows.push((format!("coverage {x}"), Config { coverage: x, ..base.clone() }));
    }
    for &x in &[0.25, 0.5, 1.0] {
        rows.push((format!("self-check throughput {x}"), Config { selfcheck_throughput: x, ..base.clone() }));
    }

    for (label, cfg) in rows {
        let r = run_all(&cfg, runs, a.seed, a.threads);
        let sum = |p: usize, f: &dyn Fn(&Outcome) -> f64| r.iter().map(|o| f(&o[p])).sum::<f64>();
        let work = |o: &Outcome| o.useful_days;
        let wrong = |o: &Outcome| o.wrong_days as f64;
        let ws = sum(1, &wrong);
        let vs_simplex = if ws > 0.0 { format!("{:.0}%", 100.0 * sum(2, &wrong) / ws) } else { "-".into() };
        println!(
            "{:<28} {:>11.2}x {:>14.3} {:>14}",
            label,
            sum(2, &work) / sum(0, &work),
            sum(2, &wrong) / runs as f64,
            vs_simplex
        );
    }
}

fn main() {
    let a = parse();
    if a.runs == 0 || a.cfg.nodes == 0 {
        fail("--runs and --nodes must be at least 1");
    }
    if a.sweep {
        sweep(&a);
    } else {
        report(&a);
    }
}
