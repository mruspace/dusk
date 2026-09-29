use dusk::model::{Config, Outcome, Policy};
use dusk::{break_even_years, bootstrap, ground_curve, run_all, sweeps, work_ratio, Totals};
use std::process::exit;

const USAGE: &str = "\
Monte Carlo of redundancy policies on hardware that only ever decays.

usage: dusk [options] [--sweep | --ground]

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
  --ground                 compare against TMR with a ground team instead
";

struct Args {
    cfg: Config,
    runs: usize,
    seed: u64,
    threads: usize,
    sweep: bool,
    ground: bool,
}

fn parse() -> Args {
    let mut a = Args {
        cfg: Config::default(),
        runs: 2000,
        seed: 1,
        threads: std::thread::available_parallelism().map_or(4, |n| n.get()),
        sweep: false,
        ground: false,
    };
    let mut it = std::env::args().skip(1);
    while let Some(flag) = it.next() {
        if flag == "--sweep" {
            a.sweep = true;
            continue;
        }
        if flag == "--ground" {
            a.ground = true;
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
    let ci = |(lo, hi): (f64, f64), f: &dyn Fn(f64) -> String| format!("95% CI {} to {}", f(lo), f(hi));
    let x = |v: f64| format!("{v:.2}x");
    let pc = |v: f64| format!("{:.0}%", 100.0 * v);
    let yr = |v: f64| format!("{v:.0}");

    println!();
    let (vs_tmr, vs_tmr_ci) = work_ratio(&r, shrink, tmr, a.seed);
    println!(
        "shrinking quorum vs fixed TMR: {} the useful work in total ({}); per mission median {:.2}x (p10 {:.2}x, p90 {:.2}x)",
        x(vs_tmr),
        ci(vs_tmr_ci, &x),
        pct(&ratio, 50.0),
        pct(&ratio, 10.0),
        pct(&ratio, 90.0)
    );
    let ws = sum(simplex, wrong);
    if ws > 0.0 {
        let share = |idx: &[usize]| {
            let w = |p: usize| idx.iter().map(|&i| r[i][p].wrong_days as f64).sum::<f64>();
            w(shrink) / w(simplex)
        };
        println!(
            "shrinking quorum vs standby simplex: {:.0}% of the useful work, {:.0}% of the wrong results ({})",
            100.0 * sum(shrink, |o| o.useful_days) / sum(simplex, |o| o.useful_days),
            100.0 * sum(shrink, wrong) / ws,
            ci(bootstrap(r.len(), a.seed, share), &pc)
        );
    }
    let all: Vec<usize> = (0..r.len()).collect();
    if let Some(be) = break_even_years(&r, &all, shrink, tmr) {
        let be_ci = bootstrap(r.len(), a.seed, |idx| break_even_years(&r, idx, shrink, tmr).unwrap_or(f64::NAN));
        println!(
            "break-even: fixed TMR comes out ahead only if one wrong result costs more than {be:.0} years of useful work ({})",
            ci(be_ci, &yr)
        );
    }
}

/// The shrinking quorum against TMR with a ground team that can command the
/// same fallback by hand, as support lasts longer and answers slower.
fn ground(a: &Args) {
    const GROUND: [f64; 7] = [0.0, 10.0, 25.0, 50.0, 100.0, 200.0, f64::INFINITY];
    const LATENCY: [u64; 3] = [1, 30, 180];
    println!("{} missions, seed {}; shrinking quorum's useful work vs TMR + ground fallback", a.runs, a.seed);
    println!();
    let pts = ground_curve(&a.cfg, a.runs, a.seed, a.threads, &GROUND, &LATENCY);
    println!(
        "{:<22} {:>9} {:>9} {:>9}   95% CI at 30 days",
        "ground support ends", "1 day", "30 days", "180 days"
    );
    for (row, g) in pts.chunks(LATENCY.len()).zip(GROUND) {
        let label = match g {
            0.0 => "at launch".to_string(),
            g if g.is_infinite() => "never".to_string(),
            g => format!("after {g} years"),
        };
        println!(
            "{:<22} {:>8.2}x {:>8.2}x {:>8.2}x   {:.2}x to {:.2}x",
            label, row[0].ratio, row[1].ratio, row[2].ratio, row[1].ci.0, row[1].ci.1
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

    for sweep in sweeps(&a.cfg) {
        for (label, cfg) in sweep.configs {
            let t = Totals::of(&run_all(&cfg, runs, a.seed, a.threads));
            let vs_simplex = t.wrong_vs_simplex().map_or("-".into(), |x| format!("{:.0}%", 100.0 * x));
            println!(
                "{:<28} {:>11.2}x {:>14.3} {:>14}",
                label,
                t.work_vs_tmr(),
                t.wrong_days[2] / runs as f64,
                vs_simplex
            );
        }
    }
}

fn main() {
    let a = parse();
    if a.runs == 0 || a.cfg.nodes == 0 {
        fail("--runs and --nodes must be at least 1");
    }
    if a.sweep && a.ground {
        fail("pick one of --sweep and --ground");
    }
    if a.ground {
        ground(&a);
    } else if a.sweep {
        sweep(&a);
    } else {
        report(&a);
    }
}
