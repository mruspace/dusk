//! Writes the README charts to `docs/`, one light and one dark file each, in
//! the look of mru.space: warm paper, ink greys, one slate accent, Charter for
//! text and Futura for titles. Mru's policy wears the accent; the two
//! baselines stay grey, so the chart reads as one line against context.
//!
//! Run with `cargo run --release --bin charts`. The numbers come from the same
//! seeds as `dusk` and `dusk --sweep`, so they match the README tables.

use dusk::model::Config;
use dusk::{break_even_years, bootstrap, ground_curve, run_all, sweeps, timeline, work_ratio, GroundPoint, Totals};
use std::fmt::Write as _;
use std::path::Path;

const RUNS: usize = 2000;
const SWEEP_RUNS: usize = 1000;
const SEED: u64 = 1;

struct Theme {
    name: &'static str,
    bg: &'static str,
    ink: &'static str,
    soft: &'static str,
    faint: &'static str,
    rule: &'static str,
    /// Line colour per policy, in `Policy::ALL` order.
    series: [&'static str; 3],
    wash: f64,
}

// Surfaces, inks and accent are the mru.space tokens. The simplex grey is the
// one value not on the site: it sits under 3:1 on purpose, as the quietest
// line, so it always carries a direct label.
const LIGHT: Theme = Theme {
    name: "light",
    bg: "#fafaf8",
    ink: "#16161a",
    soft: "#44444c",
    faint: "#6f6f78",
    rule: "#e3e3dd",
    series: ["#6f6f78", "#a3a3aa", "#3a4a5a"],
    wash: 0.10,
};

const DARK: Theme = Theme {
    name: "dark",
    bg: "#121214",
    ink: "#ecece8",
    soft: "#c0c0bc",
    faint: "#8f8f8a",
    rule: "#2a2a2e",
    series: ["#8f8f8a", "#5c5c60", "#9fb4cb"],
    wash: 0.14,
};

const TMR: usize = 0;
const SIMPLEX: usize = 1;
const SHRINK: usize = 2;

fn style(t: &Theme) -> String {
    format!(
        "<style>\
         text{{font-family:Charter,'Source Serif 4','Iowan Old Style','Palatino Linotype',Georgia,serif;fill:{soft}}}\
         .title{{font-family:Futura,Jost,'Century Gothic','Avenir Next',Avenir,system-ui,sans-serif;font-weight:500;font-size:19px;fill:{ink}}}\
         .sub{{font-size:13.5px}}\
         .label{{font-size:14px;fill:{ink}}}\
         .note{{font-size:12.5px;fill:{faint}}}\
         .tick{{font-family:ui-monospace,'SF Mono',SFMono-Regular,Menlo,Consolas,monospace;font-size:11px;fill:{faint};font-variant-numeric:tabular-nums}}\
         .value{{font-family:ui-monospace,'SF Mono',SFMono-Regular,Menlo,Consolas,monospace;font-size:11.5px;fill:{soft}}}\
         .grid{{stroke:{rule};stroke-width:1}}\
         </style>",
        ink = t.ink,
        soft = t.soft,
        faint = t.faint,
        rule = t.rule,
    )
}

fn open(w: f64, h: f64, t: &Theme, title: &str, desc: &str) -> String {
    format!(
        "<svg xmlns=\"http://www.w3.org/2000/svg\" viewBox=\"0 0 {w} {h}\" width=\"{w}\" height=\"{h}\" role=\"img\" aria-labelledby=\"t d\">\
         <title id=\"t\">{title}</title><desc id=\"d\">{desc}</desc>{}\
         <rect width=\"{w}\" height=\"{h}\" fill=\"{}\"/>",
        style(t),
        t.bg
    )
}

/// A dot with a ring in the surface colour, so it stays legible on a line.
fn dot(s: &mut String, x: f64, y: f64, fill: &str, bg: &str) {
    write!(s, "<circle cx=\"{x:.1}\" cy=\"{y:.1}\" r=\"4.5\" fill=\"{fill}\" stroke=\"{bg}\" stroke-width=\"2\"/>").unwrap();
}

// ---------------------------------------------------------------- timeline

fn output_chart(t: &Theme, line: &[Vec<f64>; 3], years: usize) -> String {
    let (w, h) = (760.0, 420.0);
    let (x0, x1, y0, y1) = (52.0, 736.0, 112.0, 372.0);
    let sx = |yr: f64| x0 + (x1 - x0) * yr / years as f64;
    let sy = |v: f64| y1 - (y1 - y0) * v;

    let mut s = open(
        w,
        h,
        t,
        "Useful output over the mission",
        "Mean daily useful output of three redundancy policies across 2,000 simulated missions. \
         Fixed TMR stops once fewer than two processors survive. Standby simplex runs at half rate for the whole life. \
         The shrinking quorum matches TMR while it can vote, then keeps going at half rate on one processor.",
    );

    s += "<text class=\"title\" x=\"0\" y=\"24\">Useful output over the mission</text>";
    s += "<text class=\"sub\" x=\"0\" y=\"47\">Mean over 2,000 simulated missions. 100% is a full voting set working every day.</text>";
    s += "<text class=\"sub\" x=\"0\" y=\"66\">The shaded area under the shrinking quorum is its total useful work.</text>";

    // Legend: Mru's policy first, then the baselines.
    let mut lx = 0.0;
    for (p, name) in [(SHRINK, "shrinking quorum (Mru)"), (TMR, "fixed TMR"), (SIMPLEX, "standby simplex")] {
        write!(
            s,
            "<line x1=\"{lx}\" y1=\"87\" x2=\"{}\" y2=\"87\" stroke=\"{}\" stroke-width=\"2\" stroke-linecap=\"round\"/>\
             <text class=\"note\" x=\"{}\" y=\"91\" style=\"fill:{}\">{name}</text>",
            lx + 18.0,
            t.series[p],
            lx + 25.0,
            t.soft
        )
        .unwrap();
        lx += 25.0 + name.len() as f64 * 6.6 + 26.0;
    }

    // Grid and ticks.
    for i in 0..=4 {
        let v = i as f64 / 4.0;
        let y = sy(v);
        write!(s, "<line class=\"grid\" x1=\"{x0}\" y1=\"{y}\" x2=\"{x1}\" y2=\"{y}\"/>").unwrap();
        write!(s, "<text class=\"tick\" x=\"{}\" y=\"{}\" text-anchor=\"end\">{}%</text>", x0 - 8.0, y + 4.0, i * 25).unwrap();
    }
    for yr in (0..=years).step_by(50) {
        write!(
            s,
            "<text class=\"tick\" x=\"{:.1}\" y=\"{}\" text-anchor=\"middle\">{yr}</text>",
            sx(yr as f64),
            y1 + 18.0
        )
        .unwrap();
    }
    write!(s, "<text class=\"note\" x=\"{x1}\" y=\"{}\" text-anchor=\"end\">mission year</text>", y1 + 38.0).unwrap();

    // Each year's mean sits at its midpoint; the line starts at launch.
    let pts = |v: &[f64]| -> Vec<(f64, f64)> {
        let mut p = vec![(sx(0.0), sy(v[0]))];
        p.extend(v.iter().enumerate().map(|(i, &x)| (sx(i as f64 + 0.5), sy(x))));
        p
    };
    let path = |p: &[(f64, f64)]| -> String {
        p.iter()
            .enumerate()
            .map(|(i, (x, y))| format!("{}{x:.1},{y:.1}", if i == 0 { "M" } else { "L" }))
            .collect()
    };

    let shrink = pts(&line[SHRINK]);
    let last = shrink.last().unwrap().0;
    write!(
        s,
        "<path d=\"{}L{last:.1},{y1}L{x0},{y1}Z\" fill=\"{}\" fill-opacity=\"{}\"/>",
        path(&shrink),
        t.series[SHRINK],
        t.wash
    )
    .unwrap();
    for p in [SIMPLEX, TMR, SHRINK] {
        write!(
            s,
            "<path d=\"{}\" fill=\"none\" stroke=\"{}\" stroke-width=\"2\" stroke-linejoin=\"round\" stroke-linecap=\"round\"/>",
            path(&pts(&line[p])),
            t.series[p]
        )
        .unwrap();
    }

    // Direct labels, placed where each line is on its own.
    let at = |p: usize, yr: usize| (sx(yr as f64 + 0.5), sy(line[p][yr]));
    let crossing = |p: usize, level: f64| line[p].iter().position(|&v| v < level).unwrap_or(years - 1);

    let (x, y) = at(SIMPLEX, 12);
    write!(s, "<text class=\"note\" x=\"{x:.1}\" y=\"{:.1}\">standby simplex</text>", y + 18.0).unwrap();

    let yr = crossing(TMR, 0.62);
    let (x, y) = at(TMR, yr);
    write!(s, "<text class=\"note\" x=\"{:.1}\" y=\"{:.1}\" text-anchor=\"end\">fixed TMR</text>", x - 10.0, y + 4.0).unwrap();

    let yr = crossing(SHRINK, 0.55);
    let (x, y) = at(SHRINK, yr);
    write!(s, "<text class=\"note\" x=\"{:.1}\" y=\"{:.1}\">shrinking quorum</text>", x + 10.0, y - 8.0).unwrap();

    s += "</svg>\n";
    s
}

// ---------------------------------------------------------------- sweep

struct Row {
    name: &'static str,
    values: String,
    work: (f64, f64),
    wrong: (f64, f64),
    /// Each value tried, with its work ratio and wrong-result share.
    points: Vec<(String, f64, Option<f64>)>,
}

/// Voyager 1 and 2 launched in 1977; this is how long their ground support
/// has lasted so far, the longest of any deep-space mission.
const VOYAGER_YEARS: f64 = 49.0;
const GROUND_LATENCY_DAYS: u64 = 30;

// ---------------------------------------------------------------- ground

fn ground_chart(t: &Theme, pts: &[GroundPoint]) -> String {
    let (w, h) = (760.0, 400.0);
    let (x0, x1, y0, y1) = (52.0, 736.0, 104.0, 352.0);
    let max_years = pts.last().unwrap().ground_years;
    let (lo, hi) = (1.0, 1.4);
    let sx = |g: f64| x0 + (x1 - x0) * g / max_years;
    let sy = |v: f64| y1 - (y1 - y0) * (v - lo) / (hi - lo);
    let accent = t.series[SHRINK];

    let mut s = open(
        w,
        h,
        t,
        "What autonomy adds, by how long ground support lasts",
        "The shrinking quorum's useful work relative to fixed TMR with a ground team that commands the same \
         fallback 30 days after it is needed. With no ground team the gain is about 1.33x; it falls as support \
         lasts longer and reaches 1.00x if support never ends.",
    );
    s += "<text class=\"title\" x=\"0\" y=\"24\">What autonomy adds, by how long ground support lasts</text>";
    s += "<text class=\"sub\" x=\"0\" y=\"47\">Shrinking quorum vs fixed TMR with a ground team that commands the same fallback. At 0, there is none.</text>";
    s += "<text class=\"sub\" x=\"0\" y=\"66\">Answer time barely matters: 1 day and 6 months give the same result. Band: 95% CI.</text>";

    for i in 0..=4 {
        let v = lo + (hi - lo) * i as f64 / 4.0;
        let y = sy(v);
        write!(s, "<line class=\"grid\" x1=\"{x0}\" y1=\"{y:.1}\" x2=\"{x1}\" y2=\"{y:.1}\"/>").unwrap();
        write!(s, "<text class=\"tick\" x=\"{}\" y=\"{:.1}\" text-anchor=\"end\">{v:.1}x</text>", x0 - 8.0, y + 4.0).unwrap();
    }
    for g in (0..=max_years as usize).step_by(50) {
        write!(s, "<text class=\"tick\" x=\"{:.1}\" y=\"{}\" text-anchor=\"middle\">{g}</text>", sx(g as f64), y1 + 18.0).unwrap();
    }
    write!(s, "<text class=\"note\" x=\"{x1}\" y=\"{}\" text-anchor=\"end\">years of ground support</text>", y1 + 38.0).unwrap();

    // The 95% band, then the line.
    let upper: String = pts.iter().map(|p| format!("L{:.1},{:.1}", sx(p.ground_years), sy(p.ci.1))).collect();
    let lower: String = pts.iter().rev().map(|p| format!("L{:.1},{:.1}", sx(p.ground_years), sy(p.ci.0))).collect();
    write!(s, "<path d=\"M{}Z\" fill=\"{accent}\" fill-opacity=\"{}\"/>", &(upper + &lower)[1..], t.wash + 0.06).unwrap();
    let line: String = pts
        .iter()
        .enumerate()
        .map(|(i, p)| format!("{}{:.1},{:.1}", if i == 0 { "M" } else { "L" }, sx(p.ground_years), sy(p.ratio)))
        .collect();
    write!(s, "<path d=\"{line}\" fill=\"none\" stroke=\"{accent}\" stroke-width=\"2\" stroke-linejoin=\"round\" stroke-linecap=\"round\"/>").unwrap();

    // Voyager's ground support so far, and where it leaves the curve.
    let vx = sx(VOYAGER_YEARS);
    let vy = sy(interp(pts, VOYAGER_YEARS));
    write!(s, "<line x1=\"{vx:.1}\" y1=\"{y0}\" x2=\"{vx:.1}\" y2=\"{y1}\" stroke=\"{}\" stroke-width=\"1\"/>", t.faint).unwrap();
    write!(s, "<text class=\"note\" x=\"{:.1}\" y=\"{}\">Voyager so far, {VOYAGER_YEARS} years</text>", vx + 8.0, y0 + 12.0).unwrap();
    dot(&mut s, vx, vy, accent, t.bg);
    write!(s, "<text class=\"value\" x=\"{:.1}\" y=\"{:.1}\">{:.2}x</text>", vx + 10.0, vy - 8.0, interp(pts, VOYAGER_YEARS)).unwrap();

    let first = &pts[0];
    dot(&mut s, sx(0.0), sy(first.ratio), accent, t.bg);
    write!(s, "<text class=\"value\" x=\"{:.1}\" y=\"{:.1}\">{:.2}x</text>", sx(0.0) + 10.0, sy(first.ratio) - 10.0, first.ratio).unwrap();
    write!(
        s,
        "<text class=\"note\" x=\"{x1}\" y=\"{:.1}\" text-anchor=\"end\">support that never ends: 1.00x</text>",
        sy(1.0) - 24.0
    )
    .unwrap();

    s += "</svg>\n";
    s
}

/// Linear interpolation of the ground curve at `g` years.
fn interp(pts: &[GroundPoint], g: f64) -> f64 {
    let i = pts.iter().position(|p| p.ground_years >= g).unwrap_or(pts.len() - 1).max(1);
    let (a, b) = (&pts[i - 1], &pts[i]);
    a.ratio + (b.ratio - a.ratio) * (g - a.ground_years) / (b.ground_years - a.ground_years)
}

// ---------------------------------------------------------------- data for the page

fn json_f(v: f64) -> String {
    if v.is_finite() { format!("{:.4}", v) } else { "null".into() }
}

fn json_list(v: &[f64]) -> String {
    format!("[{}]", v.iter().map(|&x| json_f(x)).collect::<Vec<_>>().join(","))
}

struct Headline {
    work_vs_tmr: (f64, (f64, f64)),
    wrong_vs_simplex: (f64, (f64, f64)),
    break_even_years: (f64, (f64, f64)),
    useful_years: [f64; 3],
    service_years: [f64; 3],
    wrong_per_mission: [f64; 3],
}

fn data_json(h: &Headline, line: &[Vec<f64>; 3], rows: &[Row], default: (f64, f64), ground: &[GroundPoint]) -> String {
    let pair = |(v, (lo, hi)): (f64, (f64, f64))| format!("{{\"value\":{},\"lo\":{},\"hi\":{}}}", json_f(v), json_f(lo), json_f(hi));
    let sweep: Vec<String> = rows
        .iter()
        .map(|r| {
            let pts: Vec<String> = r
                .points
                .iter()
                .map(|(l, w, x)| format!("{{\"value\":\"{l}\",\"work\":{},\"wrong\":{}}}", json_f(*w), x.map_or("null".into(), json_f)))
                .collect();
            format!("{{\"name\":\"{}\",\"values\":\"{}\",\"points\":[{}]}}", r.name, r.values, pts.join(","))
        })
        .collect();
    let ground: Vec<String> = ground
        .iter()
        .map(|p| format!("{{\"years\":{},\"ratio\":{},\"lo\":{},\"hi\":{}}}", json_f(p.ground_years), json_f(p.ratio), json_f(p.ci.0), json_f(p.ci.1)))
        .collect();
    let policies = ["fixed TMR", "standby simplex", "shrinking quorum"];
    let table: Vec<String> = (0..3)
        .map(|p| {
            format!(
                "{{\"policy\":\"{}\",\"useful_years\":{},\"service_years\":{},\"wrong_per_mission\":{}}}",
                policies[p],
                json_f(h.useful_years[p]),
                json_f(h.service_years[p]),
                json_f(h.wrong_per_mission[p])
            )
        })
        .collect();
    format!(
        "{{\"runs\":{RUNS},\"sweep_runs\":{SWEEP_RUNS},\"seed\":{SEED},\
         \"headline\":{{\"work_vs_tmr\":{},\"wrong_vs_simplex\":{},\"break_even_years\":{}}},\
         \"policies\":[{}],\
         \"timeline\":{{\"tmr\":{},\"simplex\":{},\"shrink\":{}}},\
         \"sweep\":{{\"default\":{{\"work\":{},\"wrong\":{}}},\"rows\":[{}]}},\
         \"ground\":{{\"latency_days\":{GROUND_LATENCY_DAYS},\"voyager_years\":{VOYAGER_YEARS},\"points\":[{}]}}}}\n",
        pair(h.work_vs_tmr),
        pair(h.wrong_vs_simplex),
        pair(h.break_even_years),
        table.join(","),
        json_list(&line[TMR]),
        json_list(&line[SIMPLEX]),
        json_list(&line[SHRINK]),
        json_f(default.0),
        json_f(default.1),
        sweep.join(","),
        ground.join(",")
    )
}

fn sensitivity_chart(t: &Theme, rows: &[Row], default: (f64, f64)) -> String {
    let row_h = 40.0;
    let top = 150.0;
    let bottom = top + row_h * (rows.len() as f64 - 1.0);
    let (w, h) = (760.0, bottom + 40.0);

    struct Panel {
        x0: f64,
        x1: f64,
        lo: f64,
        hi: f64,
        title: &'static str,
        note: &'static str,
        ticks: [f64; 5],
    }
    let panels = [
        Panel {
            x0: 250.0,
            x1: 474.0,
            lo: 1.0,
            hi: 2.0,
            title: "Useful work vs fixed TMR",
            note: "higher is better; 1.0x is no gain",
            ticks: [1.0, 1.25, 1.5, 1.75, 2.0],
        },
        Panel {
            x0: 516.0,
            x1: 740.0,
            lo: 0.0,
            hi: 1.0,
            title: "Wrong results vs standby simplex",
            note: "lower is better; 100% is no gain",
            ticks: [0.0, 0.25, 0.5, 0.75, 1.0],
        },
    ];

    let mut s = open(
        w,
        h,
        t,
        "How much the result depends on the assumptions",
        "For each uncertain parameter, varied alone with the rest at default, the range of the shrinking quorum's \
         useful work relative to fixed TMR and its wrong results relative to standby simplex.",
    );
    s += "<text class=\"title\" x=\"0\" y=\"24\">How much the result depends on the assumptions</text>";
    s += "<text class=\"sub\" x=\"0\" y=\"47\">Each parameter varied on its own, the rest at default. 1,000 missions per point.</text>";
    s += "<text class=\"sub\" x=\"0\" y=\"66\">A line spans the results across that parameter's values. The vertical rule is the default run.</text>";

    for (i, r) in rows.iter().enumerate() {
        let y = top + row_h * i as f64;
        write!(s, "<text class=\"label\" x=\"0\" y=\"{}\">{}</text>", y - 1.0, r.name).unwrap();
        write!(s, "<text class=\"tick\" x=\"0\" y=\"{}\">{}</text>", y + 14.0, r.values).unwrap();
    }

    for (k, p) in panels.iter().enumerate() {
        let sx = |v: f64| p.x0 + (p.x1 - p.x0) * (v - p.lo) / (p.hi - p.lo);
        write!(s, "<text class=\"label\" x=\"{}\" y=\"104\">{}</text>", p.x0, p.title).unwrap();
        write!(s, "<text class=\"note\" x=\"{}\" y=\"122\">{}</text>", p.x0, p.note).unwrap();

        for &v in &p.ticks {
            let x = sx(v);
            write!(s, "<line class=\"grid\" x1=\"{x:.1}\" y1=\"{}\" x2=\"{x:.1}\" y2=\"{}\"/>", top - 14.0, bottom + 12.0).unwrap();
            let label = if k == 0 { format!("{}x", if v.fract() == 0.0 { format!("{v:.1}") } else { v.to_string() }) } else { format!("{:.0}%", v * 100.0) };
            write!(s, "<text class=\"tick\" x=\"{x:.1}\" y=\"{}\" text-anchor=\"middle\">{label}</text>", bottom + 28.0).unwrap();
        }

        let d = sx(if k == 0 { default.0 } else { default.1 });
        write!(
            s,
            "<line x1=\"{d:.1}\" y1=\"{}\" x2=\"{d:.1}\" y2=\"{}\" stroke=\"{}\" stroke-width=\"1\"/>",
            top - 14.0,
            bottom + 12.0,
            t.faint
        )
        .unwrap();

        let ranges: Vec<(f64, f64)> = rows.iter().map(|r| if k == 0 { r.work } else { r.wrong }).collect();
        let lowest = ranges.iter().map(|r| r.0).fold(f64::INFINITY, f64::min);
        let highest = ranges.iter().map(|r| r.1).fold(f64::NEG_INFINITY, f64::max);
        let fmt = |v: f64| if k == 0 { format!("{v:.2}x") } else { format!("{:.0}%", v * 100.0) };

        for (i, &(lo, hi)) in ranges.iter().enumerate() {
            let y = top + row_h * i as f64 + 2.0;
            let (a, b) = (sx(lo), sx(hi));
            let accent = t.series[SHRINK];
            if b - a < 9.0 {
                // Ends closer than a dot: one pill, not two dots in a pile.
                write!(
                    s,
                    "<line x1=\"{a:.1}\" y1=\"{y}\" x2=\"{b:.1}\" y2=\"{y}\" stroke=\"{accent}\" stroke-width=\"9\" stroke-linecap=\"round\"/>"
                )
                .unwrap();
            } else {
                write!(
                    s,
                    "<line x1=\"{a:.1}\" y1=\"{y}\" x2=\"{b:.1}\" y2=\"{y}\" stroke=\"{accent}\" stroke-width=\"2\" stroke-linecap=\"round\"/>"
                )
                .unwrap();
                dot(&mut s, a, y, accent, t.bg);
                dot(&mut s, b, y, accent, t.bg);
            }
            // Only the extremes of the whole panel get a number.
            if lo == lowest {
                write!(s, "<text class=\"value\" x=\"{:.1}\" y=\"{:.1}\" text-anchor=\"end\">{}</text>", a - 9.0, y + 4.0, fmt(lo)).unwrap();
            }
            if hi == highest {
                write!(s, "<text class=\"value\" x=\"{:.1}\" y=\"{:.1}\">{}</text>", b + 9.0, y + 4.0, fmt(hi)).unwrap();
            }
        }
    }

    s += "</svg>\n";
    s
}

fn range(v: &[f64]) -> (f64, f64) {
    (v.iter().copied().fold(f64::INFINITY, f64::min), v.iter().copied().fold(f64::NEG_INFINITY, f64::max))
}

fn main() {
    let threads = std::thread::available_parallelism().map_or(4, |n| n.get());
    let base = Config::default();

    // Plot until the last policy's mean output is under half a percent, to
    // the next fifty years.
    let full = timeline(&base, RUNS, SEED, threads, base.horizon_years as usize);
    let tail = (0..full[0].len()).rev().find(|&y| full.iter().any(|l| l[y] >= 0.005)).unwrap_or(0);
    let years = (tail / 50 + 1) * 50;
    let line = full.map(|l| l[..years].to_vec());

    let d = Totals::of(&run_all(&base, SWEEP_RUNS, SEED, threads));
    let default = (d.work_vs_tmr(), d.wrong_vs_simplex().unwrap_or(0.0));

    let rows: Vec<Row> = sweeps(&base)
        .into_iter()
        .map(|sw| {
            let totals: Vec<Totals> =
                sw.configs.iter().map(|(_, c)| Totals::of(&run_all(c, SWEEP_RUNS, SEED, threads))).collect();
            let work: Vec<f64> = totals.iter().map(Totals::work_vs_tmr).collect();
            let wrong: Vec<f64> = totals.iter().filter_map(Totals::wrong_vs_simplex).collect();
            let values = sw.configs.iter().map(|(l, _)| l.rsplit(' ').next().unwrap().to_string()).collect::<Vec<_>>();
            let points = values
                .iter()
                .zip(&totals)
                .map(|(v, t)| (v.clone(), t.work_vs_tmr(), t.wrong_vs_simplex()))
                .collect();
            Row {
                name: sw.name,
                values: format!("{} to {}", values.first().unwrap(), values.last().unwrap()),
                work: range(&work),
                wrong: range(&wrong),
                points,
            }
        })
        .collect();

    // The headline numbers, with 95% intervals, from the same 2,000 missions
    // as the timeline and the `dusk` report.
    let r = run_all(&base, RUNS, SEED, threads);
    let (tmr, simplex, shrink) = (TMR, SIMPLEX, SHRINK);
    let all: Vec<usize> = (0..r.len()).collect();
    let wrong_share = |idx: &[usize]| {
        let w = |p: usize| idx.iter().map(|&i| r[i][p].wrong_days as f64).sum::<f64>();
        w(shrink) / w(simplex)
    };
    let be = |idx: &[usize]| break_even_years(&r, idx, shrink, tmr).unwrap_or(f64::NAN);
    let mean = |f: &dyn Fn(&dusk::model::Outcome) -> f64, p: usize| r.iter().map(|o| f(&o[p])).sum::<f64>() / r.len() as f64;
    let headline = Headline {
        work_vs_tmr: work_ratio(&r, shrink, tmr, SEED),
        wrong_vs_simplex: (wrong_share(&all), bootstrap(r.len(), SEED, wrong_share)),
        break_even_years: (be(&all), bootstrap(r.len(), SEED, be)),
        useful_years: [0, 1, 2].map(|p| mean(&|o| o.useful_years(), p)),
        service_years: [0, 1, 2].map(|p| mean(&|o| o.end_years(), p)),
        wrong_per_mission: [0, 1, 2].map(|p| mean(&|o| o.wrong_days as f64, p)),
    };

    // Ground support from none to 250 years, answering in 30 days.
    let span: Vec<f64> = (0..=50).map(|i| i as f64 * 5.0).collect();
    let ground = ground_curve(&base, RUNS, SEED, threads, &span, &[GROUND_LATENCY_DAYS]);

    let dir = Path::new(env!("CARGO_MANIFEST_DIR")).join("docs");
    std::fs::create_dir_all(&dir).expect("create docs/");
    std::fs::write(dir.join("data.json"), data_json(&headline, &line, &rows, default, &ground)).expect("write data.json");
    println!("wrote {}", dir.join("data.json").display());
    for t in [&LIGHT, &DARK] {
        for (stem, svg) in [
            ("output", output_chart(t, &line, years)),
            ("sensitivity", sensitivity_chart(t, &rows, default)),
            ("ground", ground_chart(t, &ground)),
        ] {
            let path = dir.join(format!("{stem}-{}.svg", t.name));
            std::fs::write(&path, svg).expect("write chart");
            println!("wrote {}", path.display());
        }
    }

    println!();
    println!(
        "headline: {:.2}x vs TMR ({:.2}-{:.2}); {:.0}% of simplex wrong ({:.0}-{:.0}); break-even {:.0} y ({:.0}-{:.0}); at Voyager {VOYAGER_YEARS} y: {:.2}x",
        headline.work_vs_tmr.0,
        headline.work_vs_tmr.1 .0,
        headline.work_vs_tmr.1 .1,
        headline.wrong_vs_simplex.0 * 100.0,
        headline.wrong_vs_simplex.1 .0 * 100.0,
        headline.wrong_vs_simplex.1 .1 * 100.0,
        headline.break_even_years.0,
        headline.break_even_years.1 .0,
        headline.break_even_years.1 .1,
        interp(&ground, VOYAGER_YEARS)
    );
    println!("plotted to year {years}; default work vs TMR {:.2}x, wrong vs simplex {:.0}%", default.0, default.1 * 100.0);
    for r in &rows {
        println!(
            "{:<22} work {:.2}x-{:.2}x   wrong {:.0}%-{:.0}%",
            r.name,
            r.work.0,
            r.work.1,
            r.wrong.0 * 100.0,
            r.wrong.1 * 100.0
        );
    }
}
