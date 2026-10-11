//! Writes the README charts to `docs/`, one light and one dark file each, in
//! the look of mru.space: warm paper, ink greys, one slate accent, Charter for
//! text and Futura for titles. Mru's policy wears the accent; the baselines
//! stay grey, the newer two dashed, so the chart reads as one line against
//! context.
//!
//! Run with `cargo run --release --bin charts`. The numbers come from the same
//! seeds as `dusk` and `dusk --sweep`, so they match the README tables.

use dusk::model::{Config, Outcome, Policy, COLD, SHRINK, SIMPLEX, TMR, UNCHECKED};
use dusk::{
    bootstrap, break_even_years, cold_crossing, cold_crossing_line, cold_curve, fmt, ground_curve, risk_ci,
    run_columns, sweep_totals, sweeps, timeline, work_ratio, wrong_ratio, ColdCrossing, ColdPoint, GroundPoint, Totals,
    COLD_AGINGS,
};
use std::fmt::Write as _;
use std::path::Path;
use std::process::Command;

const RUNS: usize = 10_000;
const SWEEP_RUNS: usize = RUNS;
const SEED: u64 = 1;

struct Theme {
    name: &'static str,
    bg: &'static str,
    ink: &'static str,
    soft: &'static str,
    faint: &'static str,
    rule: &'static str,
    /// Line color per policy, in `Policy::ALL` order.
    series: [&'static str; Policy::COUNT],
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
    series: ["#6f6f78", "#a3a3aa", "#3a4a5a", "#6f6f78", "#a3a3aa"],
    wash: 0.10,
};

const DARK: Theme = Theme {
    name: "dark",
    bg: "#121214",
    ink: "#ecece8",
    soft: "#c0c0bc",
    faint: "#8f8f8a",
    rule: "#2a2a2e",
    series: ["#8f8f8a", "#5c5c60", "#9fb4cb", "#8f8f8a", "#5c5c60"],
    wash: 0.14,
};

/// Dash pattern per policy, in `Policy::ALL` order. The two newer baselines
/// reuse the grey of the policy they extend, dashed, so the accent stays the
/// only solid color that is not grey.
const DASH: [Option<&str>; Policy::COUNT] = [None, None, None, Some("6 4"), Some("2 4")];

/// The two ends of the spare aging range, where the report shows cold standby.
const AGING_ENDS: [f64; 2] = [COLD_AGINGS[0], COLD_AGINGS[COLD_AGINGS.len() - 1]];

fn dash(p: usize) -> String {
    DASH[p].map_or(String::new(), |d| format!(" stroke-dasharray=\"{d}\""))
}

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
         .halo{{paint-order:stroke;stroke:{bg};stroke-width:4px;stroke-linejoin:round}}\
         </style>",
        ink = t.ink,
        soft = t.soft,
        faint = t.faint,
        rule = t.rule,
        bg = t.bg,
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

/// The curve segments ("C..." commands, no leading move) of a monotone cubic
/// through `p` (Fritsch-Carlson). It never overshoots: between two points it
/// stays between their values, so smoothing cannot invent a bump the data
/// does not have. Works in either x direction.
fn smooth_segments(p: &[(f64, f64)]) -> String {
    let n = p.len();
    if n < 2 {
        return String::new();
    }
    let d: Vec<f64> = p.windows(2).map(|w| (w[1].1 - w[0].1) / (w[1].0 - w[0].0)).collect();
    let mut m = vec![0.0; n];
    m[0] = d[0];
    m[n - 1] = d[n - 2];
    for i in 1..n - 1 {
        m[i] = if d[i - 1] * d[i] <= 0.0 { 0.0 } else { (d[i - 1] + d[i]) / 2.0 };
    }
    for i in 0..n - 1 {
        if d[i] == 0.0 {
            m[i] = 0.0;
            m[i + 1] = 0.0;
            continue;
        }
        let (a, b) = (m[i] / d[i], m[i + 1] / d[i]);
        let h = a * a + b * b;
        if h > 9.0 {
            let t = 3.0 / h.sqrt();
            m[i] = t * a * d[i];
            m[i + 1] = t * b * d[i];
        }
    }
    let mut out = String::new();
    for i in 0..n - 1 {
        let ((x0, y0), (x1, y1)) = (p[i], p[i + 1]);
        let h = (x1 - x0) / 3.0;
        write!(out, "C{:.1},{:.1} {:.1},{:.1} {x1:.1},{y1:.1}", x0 + h, y0 + m[i] * h, x1 - h, y1 - m[i + 1] * h)
            .unwrap();
    }
    out
}

/// 10000 as "10,000".
fn thousands(n: usize) -> String {
    let s = n.to_string();
    let mut out = String::new();
    for (i, c) in s.chars().enumerate() {
        if i > 0 && (s.len() - i) % 3 == 0 {
            out.push(',');
        }
        out.push(c);
    }
    out
}

/// A whole monotone-cubic path through `p`.
fn smooth(p: &[(f64, f64)]) -> String {
    format!("M{:.1},{:.1}{}", p[0].0, p[0].1, smooth_segments(p))
}

/// A dot with a ring in the surface color, so it stays legible on a line.
fn dot(s: &mut String, x: f64, y: f64, fill: &str, bg: &str) {
    write!(s, "<circle cx=\"{x:.1}\" cy=\"{y:.1}\" r=\"4.5\" fill=\"{fill}\" stroke=\"{bg}\" stroke-width=\"2\"/>")
        .unwrap();
}

// ---------------------------------------------------------------- timeline

fn output_chart(t: &Theme, line: &[Vec<f64>; Policy::COUNT], years: usize, cold_aging: f64) -> String {
    let (w, h) = (760.0, 438.0);
    let (x0, x1, y0, y1) = (52.0, 736.0, 130.0, 390.0);
    let sx = |yr: f64| x0 + (x1 - x0) * yr / years as f64;
    let sy = |v: f64| y1 - (y1 - y0) * v;
    let cold_name = format!("cold standby (spare aging {})", fmt::aging(cold_aging));

    let mut s = open(
        w,
        h,
        t,
        "Useful output over the mission",
        &format!(
            "Mean daily useful output of five redundancy policies across {} simulated missions. \
         Fixed TMR stops once fewer than two processors survive. Standby simplex runs at half rate for the whole life. \
         The shrinking quorum matches TMR while it can vote, then keeps going at half rate on one processor. \
         TMR + unchecked simplex matches TMR, then runs at full rate on one processor with no self-check. \
         Cold standby, with spares that age at {} of the powered rate, runs at half rate for longer than standby simplex.",
            thousands(RUNS),
            fmt::aging(cold_aging)
        ),
    );

    s += "<text class=\"title\" x=\"0\" y=\"24\">Useful output over the mission</text>";
    write!(
        s,
        "<text class=\"sub\" x=\"0\" y=\"47\">Mean over {} simulated missions. 100% is a full voting set working every day.</text>",
        thousands(RUNS)
    )
    .unwrap();
    s += "<text class=\"sub\" x=\"0\" y=\"66\">The shaded area under the shrinking quorum is its total useful work.</text>";

    // Legend: Mru's policy first, then the baselines, the newer two below.
    let legend = [
        vec![
            (SHRINK, "shrinking quorum (Mru)".to_string()),
            (TMR, "fixed TMR".into()),
            (SIMPLEX, "standby simplex".into()),
        ],
        vec![(UNCHECKED, "TMR + unchecked simplex".into()), (COLD, cold_name.clone())],
    ];
    for (row, entries) in legend.iter().enumerate() {
        let y = 87.0 + 19.0 * row as f64;
        let mut lx = 0.0;
        for (p, name) in entries {
            write!(
                s,
                "<line x1=\"{lx}\" y1=\"{y}\" x2=\"{}\" y2=\"{y}\" stroke=\"{}\" stroke-width=\"2\" stroke-linecap=\"round\"{}/>\
                 <text class=\"note\" x=\"{}\" y=\"{}\" style=\"fill:{}\">{name}</text>",
                lx + 18.0,
                t.series[*p],
                dash(*p),
                lx + 25.0,
                y + 4.0,
                t.soft
            )
            .unwrap();
            lx += 25.0 + name.len() as f64 * 6.6 + 26.0;
        }
    }

    // Grid and ticks.
    for i in 0..=4 {
        let v = i as f64 / 4.0;
        let y = sy(v);
        write!(s, "<line class=\"grid\" x1=\"{x0}\" y1=\"{y}\" x2=\"{x1}\" y2=\"{y}\"/>").unwrap();
        write!(s, "<text class=\"tick\" x=\"{}\" y=\"{}\" text-anchor=\"end\">{}%</text>", x0 - 8.0, y + 4.0, i * 25)
            .unwrap();
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

    let shrink = pts(&line[SHRINK]);
    let last = shrink.last().unwrap().0;
    write!(
        s,
        "<path d=\"{}L{last:.1},{y1}L{x0},{y1}Z\" fill=\"{}\" fill-opacity=\"{}\"/>",
        smooth(&shrink),
        t.series[SHRINK],
        t.wash
    )
    .unwrap();
    for p in [COLD, UNCHECKED, SIMPLEX, TMR, SHRINK] {
        write!(
            s,
            "<path d=\"{}\" fill=\"none\" stroke=\"{}\" stroke-width=\"2\" stroke-linejoin=\"round\" stroke-linecap=\"round\"{}/>",
            smooth(&pts(&line[p])),
            t.series[p],
            dash(p)
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
    write!(s, "<text class=\"note\" x=\"{:.1}\" y=\"{:.1}\" text-anchor=\"end\">fixed TMR</text>", x - 10.0, y + 4.0)
        .unwrap();

    let yr = crossing(SHRINK, 0.55);
    let (x, y) = at(SHRINK, yr);
    write!(s, "<text class=\"note halo\" x=\"{:.1}\" y=\"{:.1}\">shrinking quorum</text>", x + 10.0, y - 8.0).unwrap();

    s += "</svg>\n";
    s
}

// ---------------------------------------------------------------- sweep

struct Row {
    name: &'static str,
    values: String,
    work: (f64, f64),
    wrong: (f64, f64),
    /// Each value tried, with its work ratio, wrong-result share, and work
    /// against cold standby at each end of the aging range.
    points: Vec<(String, f64, Option<f64>, [f64; 2])>,
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
        write!(s, "<text class=\"tick\" x=\"{}\" y=\"{:.1}\" text-anchor=\"end\">{v:.1}x</text>", x0 - 8.0, y + 4.0)
            .unwrap();
    }
    for g in (0..=max_years as usize).step_by(50) {
        write!(
            s,
            "<text class=\"tick\" x=\"{:.1}\" y=\"{}\" text-anchor=\"middle\">{g}</text>",
            sx(g as f64),
            y1 + 18.0
        )
        .unwrap();
    }
    write!(s, "<text class=\"note\" x=\"{x1}\" y=\"{}\" text-anchor=\"end\">years of ground support</text>", y1 + 38.0)
        .unwrap();

    // The 95% band, then the line.
    let upper: Vec<(f64, f64)> = pts.iter().map(|p| (sx(p.ground_years), sy(p.ci.1))).collect();
    let lower: Vec<(f64, f64)> = pts.iter().rev().map(|p| (sx(p.ground_years), sy(p.ci.0))).collect();
    write!(
        s,
        "<path d=\"{}L{:.1},{:.1}{}Z\" fill=\"{accent}\" fill-opacity=\"{}\"/>",
        smooth(&upper),
        lower[0].0,
        lower[0].1,
        smooth_segments(&lower),
        t.wash + 0.06
    )
    .unwrap();
    let line = smooth(&pts.iter().map(|p| (sx(p.ground_years), sy(p.ratio))).collect::<Vec<_>>());
    write!(s, "<path d=\"{line}\" fill=\"none\" stroke=\"{accent}\" stroke-width=\"2\" stroke-linejoin=\"round\" stroke-linecap=\"round\"/>").unwrap();

    // Voyager's ground support so far, and where it leaves the curve.
    let vx = sx(VOYAGER_YEARS);
    let vy = sy(interp(pts, VOYAGER_YEARS));
    write!(
        s,
        "<line x1=\"{vx:.1}\" y1=\"{y0}\" x2=\"{vx:.1}\" y2=\"{y1}\" stroke=\"{}\" stroke-width=\"1\"/>",
        t.faint
    )
    .unwrap();
    write!(
        s,
        "<text class=\"note\" x=\"{:.1}\" y=\"{}\">Voyager so far, {VOYAGER_YEARS} years</text>",
        vx + 8.0,
        y0 + 12.0
    )
    .unwrap();
    dot(&mut s, vx, vy, accent, t.bg);
    write!(
        s,
        "<text class=\"value\" x=\"{:.1}\" y=\"{:.1}\">{:.2}x</text>",
        vx + 10.0,
        vy - 8.0,
        interp(pts, VOYAGER_YEARS)
    )
    .unwrap();

    let first = &pts[0];
    dot(&mut s, sx(0.0), sy(first.ratio), accent, t.bg);
    write!(
        s,
        "<text class=\"value\" x=\"{:.1}\" y=\"{:.1}\">{:.2}x</text>",
        sx(0.0) + 10.0,
        sy(first.ratio) - 10.0,
        first.ratio
    )
    .unwrap();
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
    if v.is_finite() {
        format!("{:.4}", v)
    } else {
        "null".into()
    }
}

fn json_list(v: &[f64]) -> String {
    format!("[{}]", v.iter().map(|&x| json_f(x)).collect::<Vec<_>>().join(","))
}

/// Everything the page and the README quote from the default run. Values
/// with an interval are `(value, (lo, hi))`.
struct Headline {
    work_vs_tmr: (f64, (f64, f64)),
    wrong_vs_simplex: (f64, (f64, f64)),
    break_even_years: (f64, (f64, f64)),
    work_vs_simplex: f64,
    simplex_vs_tmr: f64,
    work_vs_unchecked: f64,
    wrong_vs_unchecked: (f64, (f64, f64)),
    /// Shrinking quorum over cold standby at each end of the aging range.
    work_vs_cold: [(f64, (f64, f64)); 2],
    /// One per report row: the policies, then cold standby at both ends.
    names: Vec<String>,
    useful_years: Vec<f64>,
    service_years: Vec<f64>,
    wrong_per_mission: Vec<f64>,
    risk: Vec<(f64, (f64, f64))>,
}

/// Where and how this data was made.
struct Generated {
    date: String,
    commit: String,
    host: String,
    rustc: String,
}

impl Generated {
    fn now() -> Self {
        let dir = env!("CARGO_MANIFEST_DIR");
        let run = |cmd: &str, args: &[&str]| -> Option<String> {
            let out = Command::new(cmd).args(args).current_dir(dir).output().ok()?;
            out.status.success().then(|| String::from_utf8_lossy(&out.stdout).trim().to_string())
        };
        let unknown = || "unknown".to_string();
        let commit = run("git", &["rev-parse", "--short", "HEAD"]).filter(|c| !c.is_empty()).map(|c| {
            let dirty = run("git", &["status", "--porcelain"]).is_some_and(|s| !s.is_empty());
            if dirty {
                format!("{c}-dirty")
            } else {
                c
            }
        });
        let host = match (run("uname", &["-s"]), run("uname", &["-m"])) {
            (Some(s), Some(m)) => Some(format!("{s}-{m}").to_lowercase()),
            _ => None,
        };
        Generated {
            date: run("date", &["-u", "+%Y-%m-%d"]).unwrap_or_else(unknown),
            commit: commit.unwrap_or_else(unknown),
            host: host.unwrap_or_else(unknown),
            rustc: run("rustc", &["-V"]).unwrap_or_else(unknown),
        }
    }
}

/// A JSON string with nothing to escape. Every string here is ours, so a
/// quote or backslash would be a bug.
fn json_s(v: &str) -> String {
    assert!(!v.contains(['"', '\\']), "unexpected character in {v}");
    format!("\"{v}\"")
}

fn pair((v, (lo, hi)): (f64, (f64, f64))) -> String {
    format!("{{\"value\":{},\"lo\":{},\"hi\":{}}}", json_f(v), json_f(lo), json_f(hi))
}

struct Page<'a> {
    generated: &'a Generated,
    headline: &'a Headline,
    line: &'a [Vec<f64>; Policy::COUNT],
    cold_aging: f64,
    rows: &'a [Row],
    default: (f64, f64, [f64; 2]),
    ground: &'a [GroundPoint],
    cold: &'a [ColdPoint],
    display: &'a [(&'static str, String)],
}

fn data_json(p: &Page) -> String {
    let h = p.headline;
    let g = p.generated;
    let sweep: Vec<String> = p
        .rows
        .iter()
        .map(|r| {
            let pts: Vec<String> = r
                .points
                .iter()
                .map(|(l, w, x, c)| {
                    format!(
                        "{{\"value\":\"{l}\",\"work\":{},\"wrong\":{},\"cold\":{}}}",
                        json_f(*w),
                        x.map_or("null".into(), json_f),
                        json_list(c)
                    )
                })
                .collect();
            format!("{{\"name\":\"{}\",\"values\":\"{}\",\"points\":[{}]}}", r.name, r.values, pts.join(","))
        })
        .collect();
    let ground: Vec<String> = p
        .ground
        .iter()
        .map(|q| {
            format!(
                "{{\"years\":{},\"ratio\":{},\"lo\":{},\"hi\":{}}}",
                json_f(q.ground_years),
                json_f(q.ratio),
                json_f(q.ci.0),
                json_f(q.ci.1)
            )
        })
        .collect();
    let table: Vec<String> = (0..h.names.len())
        .map(|i| {
            format!(
                "{{\"policy\":{},\"useful_years\":{},\"service_years\":{},\"wrong_per_mission\":{},\"risk\":{}}}",
                json_s(&h.names[i]),
                json_f(h.useful_years[i]),
                json_f(h.service_years[i]),
                json_f(h.wrong_per_mission[i]),
                pair(h.risk[i])
            )
        })
        .collect();
    let cold: Vec<String> = p
        .cold
        .iter()
        .map(|c| {
            format!(
                "{{\"aging\":{},\"useful_years\":{},\"service_years\":{},\"wrong_per_mission\":{},\"risk\":{},\"vs_tmr\":{},\"vs_shrink\":{}}}",
                json_f(c.aging),
                json_f(c.useful_years),
                json_f(c.service_years),
                json_f(c.wrong_per_mission),
                pair(c.risk),
                pair(c.vs_tmr),
                pair(c.vs_shrink)
            )
        })
        .collect();
    let work_vs_cold: Vec<String> = AGING_ENDS
        .iter()
        .zip(&h.work_vs_cold)
        .map(|(a, &(v, (lo, hi)))| {
            format!("{{\"aging\":{},\"value\":{},\"lo\":{},\"hi\":{}}}", json_f(*a), json_f(v), json_f(lo), json_f(hi))
        })
        .collect();
    let display: Vec<String> = p.display.iter().map(|(k, v)| format!("\"{k}\":{}", json_s(v))).collect();
    format!(
        "{{\"generated\":{{\"date\":{},\"commit\":{},\"host\":{},\"rustc\":{},\"seed\":{SEED},\"runs\":{RUNS}}},\
         \"runs\":{RUNS},\"sweep_runs\":{SWEEP_RUNS},\"seed\":{SEED},\
         \"headline\":{{\"work_vs_tmr\":{},\"wrong_vs_simplex\":{},\"break_even_years\":{},\"risk_shrink\":{},\
         \"work_vs_unchecked\":{},\"wrong_vs_unchecked\":{},\"work_vs_cold\":[{}]}},\
         \"policies\":[{}],\
         \"timeline\":{{\"tmr\":{},\"simplex\":{},\"shrink\":{},\"unchecked\":{},\"cold\":{},\"cold_aging\":{}}},\
         \"sweep\":{{\"default\":{{\"work\":{},\"wrong\":{},\"cold\":{}}},\"cold_agings\":{},\"rows\":[{}]}},\
         \"ground\":{{\"latency_days\":{GROUND_LATENCY_DAYS},\"voyager_years\":{VOYAGER_YEARS},\"points\":[{}]}},\
         \"cold\":{{\"aging_default\":{},\"shrink_vs_tmr\":{},\"simplex_vs_tmr\":{},\"crossing\":{},\"points\":[{}]}},\
         \"display\":{{{}}}}}\n",
        json_s(&g.date),
        json_s(&g.commit),
        json_s(&g.host),
        json_s(&g.rustc),
        pair(h.work_vs_tmr),
        pair(h.wrong_vs_simplex),
        pair(h.break_even_years),
        pair(h.risk[SHRINK]),
        json_f(h.work_vs_unchecked),
        pair(h.wrong_vs_unchecked),
        work_vs_cold.join(","),
        table.join(","),
        json_list(&p.line[TMR]),
        json_list(&p.line[SIMPLEX]),
        json_list(&p.line[SHRINK]),
        json_list(&p.line[UNCHECKED]),
        json_list(&p.line[COLD]),
        json_f(p.cold_aging),
        json_f(p.default.0),
        json_f(p.default.1),
        json_list(&p.default.2),
        json_list(&AGING_ENDS),
        sweep.join(","),
        ground.join(","),
        json_f(p.cold_aging),
        json_f(h.work_vs_tmr.0),
        json_f(h.simplex_vs_tmr),
        match cold_crossing(p.cold) {
            ColdCrossing::Below(a) => json_f(a),
            _ => "null".into(),
        },
        cold.join(","),
        display.join(",")
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
    write!(
        s,
        "<text class=\"sub\" x=\"0\" y=\"47\">Each parameter varied on its own, the rest at default. {} missions per point.</text>",
        thousands(SWEEP_RUNS)
    )
    .unwrap();
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
            write!(
                s,
                "<line class=\"grid\" x1=\"{x:.1}\" y1=\"{}\" x2=\"{x:.1}\" y2=\"{}\"/>",
                top - 14.0,
                bottom + 12.0
            )
            .unwrap();
            let label = if k == 0 {
                format!("{}x", if v.fract() == 0.0 { format!("{v:.1}") } else { v.to_string() })
            } else {
                format!("{:.0}%", v * 100.0)
            };
            write!(
                s,
                "<text class=\"tick\" x=\"{x:.1}\" y=\"{}\" text-anchor=\"middle\">{label}</text>",
                bottom + 28.0
            )
            .unwrap();
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
                write!(
                    s,
                    "<text class=\"value\" x=\"{:.1}\" y=\"{:.1}\" text-anchor=\"end\">{}</text>",
                    a - 9.0,
                    y + 4.0,
                    fmt(lo)
                )
                .unwrap();
            }
            if hi == highest {
                write!(s, "<text class=\"value\" x=\"{:.1}\" y=\"{:.1}\">{}</text>", b + 9.0, y + 4.0, fmt(hi))
                    .unwrap();
            }
        }
    }

    s += "</svg>\n";
    s
}

fn range(v: &[f64]) -> (f64, f64) {
    (v.iter().copied().fold(f64::INFINITY, f64::min), v.iter().copied().fold(f64::NEG_INFINITY, f64::max))
}

// ---------------------------------------------------------------- cold standby

fn cold_chart(t: &Theme, pts: &[ColdPoint], shrink_ref: f64, simplex_ref: f64) -> String {
    let (w, h) = (760.0, 400.0);
    let (x0, x1, y0, y1) = (52.0, 736.0, 104.0, 352.0);
    let (a0, a1) = (pts[0].aging, pts[pts.len() - 1].aging);
    let lowest = pts.iter().map(|p| p.vs_tmr.1 .0).fold(simplex_ref, f64::min);
    let highest = pts.iter().map(|p| p.vs_tmr.1 .1).fold(shrink_ref, f64::max);
    let (lo, hi) = ((lowest * 5.0).floor() / 5.0, (highest * 5.0).ceil() / 5.0);
    let sx = |a: f64| x0 + (x1 - x0) * (a - a0) / (a1 - a0);
    let sy = |v: f64| y1 - (y1 - y0) * (v - lo) / (hi - lo);
    let accent = t.series[SHRINK];
    let grey = t.series[COLD];
    let (first, last) = (&pts[0], &pts[pts.len() - 1]);

    let crossing = match cold_crossing(pts) {
        ColdCrossing::Below(a) => {
            format!(" It does more useful work than the shrinking quorum below a spare aging rate of about {a:.2}.")
        }
        ColdCrossing::Always => " It does more useful work than the shrinking quorum at every rate shown.".into(),
        ColdCrossing::Never => " It never does more useful work than the shrinking quorum in this range.".into(),
    };
    let mut s = open(
        w,
        h,
        t,
        "Cold standby, by how fast an unpowered spare ages",
        &format!(
            "Useful work of cold standby relative to fixed TMR, for spares that age from {} to {} of the powered rate. \
             It falls from {} to {}.{crossing} Reference lines: the shrinking quorum at {} and standby simplex at {}.",
            fmt::aging(a0),
            fmt::aging(a1),
            fmt::x(first.vs_tmr.0),
            fmt::x(last.vs_tmr.0),
            fmt::x(shrink_ref),
            fmt::x(simplex_ref)
        ),
    );
    s += "<text class=\"title\" x=\"0\" y=\"24\">Cold standby, by how fast an unpowered spare ages</text>";
    s += "<text class=\"sub\" x=\"0\" y=\"47\">Useful work vs fixed TMR, same hardware. At 1.0 a cold spare ages as fast as a powered one.</text>";
    s += "<text class=\"sub\" x=\"0\" y=\"66\">The aging rate is swept, not measured. Band: 95% CI.</text>";

    let steps = ((hi - lo) / 0.2).round() as usize;
    for i in 0..=steps {
        let v = lo + 0.2 * i as f64;
        let y = sy(v);
        write!(s, "<line class=\"grid\" x1=\"{x0}\" y1=\"{y:.1}\" x2=\"{x1}\" y2=\"{y:.1}\"/>").unwrap();
        write!(s, "<text class=\"tick\" x=\"{}\" y=\"{:.1}\" text-anchor=\"end\">{v:.1}x</text>", x0 - 8.0, y + 4.0)
            .unwrap();
    }
    for p in pts {
        write!(
            s,
            "<text class=\"tick\" x=\"{:.1}\" y=\"{}\" text-anchor=\"middle\">{}</text>",
            sx(p.aging),
            y1 + 18.0,
            fmt::aging(p.aging)
        )
        .unwrap();
    }
    write!(s, "<text class=\"note\" x=\"{x1}\" y=\"{}\" text-anchor=\"end\">spare aging rate</text>", y1 + 38.0)
        .unwrap();

    // The two references: Mru's policy, and the same policy with hot spares.
    let (ys, yp) = (sy(shrink_ref), sy(simplex_ref));
    write!(s, "<line x1=\"{x0}\" y1=\"{ys:.1}\" x2=\"{x1}\" y2=\"{ys:.1}\" stroke=\"{accent}\" stroke-width=\"2\"/>")
        .unwrap();
    write!(
        s,
        "<text class=\"note halo\" x=\"{x1}\" y=\"{:.1}\" text-anchor=\"end\">shrinking quorum, {}</text>",
        ys - 8.0,
        fmt::x(shrink_ref)
    )
    .unwrap();
    write!(s, "<line x1=\"{x0}\" y1=\"{yp:.1}\" x2=\"{x1}\" y2=\"{yp:.1}\" stroke=\"{grey}\" stroke-width=\"1\"/>")
        .unwrap();
    write!(
        s,
        "<text class=\"note halo\" x=\"{}\" y=\"{:.1}\">standby simplex, {}</text>",
        x0 + 4.0,
        yp - 8.0,
        fmt::x(simplex_ref)
    )
    .unwrap();

    // The 95% band, then the line.
    let upper: Vec<(f64, f64)> = pts.iter().map(|p| (sx(p.aging), sy(p.vs_tmr.1 .1))).collect();
    let lower: Vec<(f64, f64)> = pts.iter().rev().map(|p| (sx(p.aging), sy(p.vs_tmr.1 .0))).collect();
    write!(
        s,
        "<path d=\"{}L{:.1},{:.1}{}Z\" fill=\"{grey}\" fill-opacity=\"{}\"/>",
        smooth(&upper),
        lower[0].0,
        lower[0].1,
        smooth_segments(&lower),
        t.wash + 0.06
    )
    .unwrap();
    let line = smooth(&pts.iter().map(|p| (sx(p.aging), sy(p.vs_tmr.0))).collect::<Vec<_>>());
    write!(
        s,
        "<path d=\"{line}\" fill=\"none\" stroke=\"{grey}\" stroke-width=\"2\" stroke-linejoin=\"round\" stroke-linecap=\"round\"{}/>",
        dash(COLD)
    )
    .unwrap();

    for (p, anchor, dx) in [(first, "start", 10.0), (last, "end", -10.0)] {
        let (x, y) = (sx(p.aging), sy(p.vs_tmr.0));
        dot(&mut s, x, y, grey, t.bg);
        write!(
            s,
            "<text class=\"value halo\" x=\"{:.1}\" y=\"{:.1}\" text-anchor=\"{anchor}\">{}</text>",
            x + dx,
            y - 10.0,
            fmt::x(p.vs_tmr.0)
        )
        .unwrap();
    }
    // The name sits above the curve near its middle, where nothing else is.
    let mid = &pts[pts.len() / 2 - 1];
    write!(
        s,
        "<text class=\"note halo\" x=\"{:.1}\" y=\"{:.1}\">cold standby</text>",
        sx(mid.aging) + 6.0,
        sy(mid.vs_tmr.0) - 14.0
    )
    .unwrap();

    if let ColdCrossing::Below(a) = cold_crossing(pts) {
        let x = sx(a);
        write!(
            s,
            "<line x1=\"{x:.1}\" y1=\"{y0}\" x2=\"{x:.1}\" y2=\"{y1}\" stroke=\"{}\" stroke-width=\"1\"/>",
            t.faint
        )
        .unwrap();
        write!(
            s,
            "<text class=\"note halo\" x=\"{:.1}\" y=\"{}\">equal work at about {a:.2}</text>",
            x + 8.0,
            y0 + 12.0
        )
        .unwrap();
        dot(&mut s, x, ys, accent, t.bg);
    }

    s += "</svg>\n";
    s
}

fn main() {
    let threads = std::thread::available_parallelism().map_or(4, |n| n.get());
    let base = Config::default();
    // Before anything is written, so "-dirty" means the code, not the output.
    let generated = Generated::now();

    // Plot until the last of the first three policies' mean output is under
    // half a percent, to the next fifty years. The two newer baselines do
    // not set the range, so the original curves keep their scale.
    let full = timeline(&base, RUNS, SEED, threads, base.horizon_years as usize);
    let tail = (0..full[0].len()).rev().find(|&y| full[..3].iter().any(|l| l[y] >= 0.005)).unwrap_or(0);
    let years = (tail / 50 + 1) * 50;
    let line = full.map(|l| l[..years].to_vec());

    let d = sweep_totals(&base, SWEEP_RUNS, SEED, threads, &AGING_ENDS);
    let default = (d.work_vs_tmr(), d.wrong_vs_simplex().unwrap_or(0.0), [d.work_vs(SHRINK, 3), d.work_vs(SHRINK, 4)]);

    let rows: Vec<Row> = sweeps(&base)
        .into_iter()
        .map(|sw| {
            let totals: Vec<Totals> =
                sw.configs.iter().map(|(_, c)| sweep_totals(c, SWEEP_RUNS, SEED, threads, &AGING_ENDS)).collect();
            let work: Vec<f64> = totals.iter().map(Totals::work_vs_tmr).collect();
            let wrong: Vec<f64> = totals.iter().filter_map(Totals::wrong_vs_simplex).collect();
            let values = sw.configs.iter().map(|(l, _)| l.rsplit(' ').next().unwrap().to_string()).collect::<Vec<_>>();
            let points = values
                .iter()
                .zip(&totals)
                .map(|(v, t)| {
                    (v.clone(), t.work_vs_tmr(), t.wrong_vs_simplex(), [t.work_vs(SHRINK, 3), t.work_vs(SHRINK, 4)])
                })
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

    // The headline numbers, with 95% intervals, from the same missions as
    // the timeline and the `dusk` report, in the report's columns.
    let r = run_columns(&base, RUNS, SEED, threads, &Policy::ALL[..COLD], &AGING_ENDS);
    let mut names: Vec<String> = Policy::ALL[..COLD].iter().map(|p| p.name().to_string()).collect();
    names.extend(AGING_ENDS.iter().map(|&g| format!("cold standby, aging {}", fmt::aging(g))));
    let all: Vec<usize> = (0..r.len()).collect();
    let be = |idx: &[usize]| break_even_years(&r, idx, SHRINK, TMR).unwrap_or(f64::NAN);
    let mean = |f: fn(&Outcome) -> f64, p: usize| r.iter().map(|o| f(&o[p])).sum::<f64>() / r.len() as f64;
    let cols = 0..names.len();
    let headline = Headline {
        work_vs_tmr: work_ratio(&r, SHRINK, TMR, SEED),
        wrong_vs_simplex: wrong_ratio(&r, SHRINK, SIMPLEX, SEED),
        break_even_years: (be(&all), bootstrap(r.len(), SEED, be)),
        work_vs_simplex: work_ratio(&r, SHRINK, SIMPLEX, SEED).0,
        simplex_vs_tmr: work_ratio(&r, SIMPLEX, TMR, SEED).0,
        work_vs_unchecked: work_ratio(&r, SHRINK, UNCHECKED, SEED).0,
        wrong_vs_unchecked: wrong_ratio(&r, SHRINK, UNCHECKED, SEED),
        work_vs_cold: [work_ratio(&r, SHRINK, COLD, SEED), work_ratio(&r, SHRINK, COLD + 1, SEED)],
        useful_years: cols.clone().map(|p| mean(Outcome::useful_years, p)).collect(),
        service_years: cols.clone().map(|p| mean(Outcome::end_years, p)).collect(),
        wrong_per_mission: cols.clone().map(|p| mean(|o| o.wrong_days as f64, p)).collect(),
        risk: cols.map(|p| risk_ci(&r, p, SEED)).collect(),
        names,
    };

    // Ground support from none to 250 years, answering in 30 days.
    let span: Vec<f64> = (0..=50).map(|i| i as f64 * 5.0).collect();
    let ground = ground_curve(&base, RUNS, SEED, threads, &span, &[GROUND_LATENCY_DAYS]);

    // Cold standby by spare aging rate, and the true two-string case.
    let cold = cold_curve(&base, RUNS, SEED, threads, &COLD_AGINGS);
    let two =
        run_columns(&Config { nodes: 2, ..base.clone() }, RUNS, SEED, threads, &[Policy::ShrinkingQuorum], &AGING_ENDS);
    let two_vs_cold = [work_ratio(&two, 0, 1, SEED), work_ratio(&two, 0, 2, SEED)];

    // The strings the README quotes, in the report's own formats.
    let h = &headline;
    let x_ci = |v: (f64, f64)| fmt::range(v, fmt::x);
    let pct1_ci = |v: (f64, f64)| fmt::range(v, fmt::pct1);
    let all_work: Vec<f64> = rows.iter().flat_map(|r| [r.work.0, r.work.1]).collect();
    let all_wrong: Vec<f64> = rows.iter().flat_map(|r| [r.wrong.0, r.wrong.1]).collect();
    let (wrong_lo, wrong_hi) = range(&all_wrong);
    let mut display: Vec<(&'static str, String)> = vec![
        ("work_vs_tmr", fmt::x(h.work_vs_tmr.0)),
        ("work_vs_tmr_ci", x_ci(h.work_vs_tmr.1)),
        ("break_even", fmt::years(h.break_even_years.0)),
        ("break_even_ci", fmt::range(h.break_even_years.1, fmt::years)),
        ("wrong_vs_simplex", fmt::pct(h.wrong_vs_simplex.0)),
        ("wrong_vs_simplex_ci", fmt::range(h.wrong_vs_simplex.1, fmt::pct)),
        ("fewer_vs_simplex", fmt::pct(1.0 - h.wrong_vs_simplex.0)),
        ("fewer_vs_simplex_ci", fmt::range((1.0 - h.wrong_vs_simplex.1 .1, 1.0 - h.wrong_vs_simplex.1 .0), fmt::pct)),
        ("work_vs_simplex", fmt::pct(h.work_vs_simplex)),
    ];
    let risk_keys = ["tmr", "simplex", "shrink", "unchecked", "cold_01", "cold_10"];
    let risk_ci_keys = ["tmr_ci", "simplex_ci", "shrink_ci", "unchecked_ci", "cold_01_ci", "cold_10_ci"];
    for (i, (k, kc)) in risk_keys.iter().zip(risk_ci_keys).enumerate() {
        let key = |s: &str| -> &'static str { Box::leak(format!("risk_{s}").into_boxed_str()) };
        display.push((key(k), fmt::pct1(h.risk[i].0)));
        display.push((key(kc), pct1_ci(h.risk[i].1)));
    }
    display.extend([
        ("work_vs_unchecked", fmt::pct(h.work_vs_unchecked)),
        ("wrong_vs_unchecked", fmt::pct2(h.wrong_vs_unchecked.0)),
        ("wrong_vs_unchecked_ci", fmt::range(h.wrong_vs_unchecked.1, fmt::pct2)),
        ("work_vs_cold_01", fmt::x(h.work_vs_cold[0].0)),
        ("work_vs_cold_01_ci", x_ci(h.work_vs_cold[0].1)),
        ("work_vs_cold_10", fmt::x(h.work_vs_cold[1].0)),
        ("work_vs_cold_10_ci", x_ci(h.work_vs_cold[1].1)),
        ("cold_crossing", cold_crossing_line(&cold)),
        ("nodes2_work_vs_cold_01", fmt::x(two_vs_cold[0].0)),
        ("nodes2_work_vs_cold_01_ci", x_ci(two_vs_cold[0].1)),
        ("nodes2_work_vs_cold_10", fmt::x(two_vs_cold[1].0)),
        ("nodes2_work_vs_cold_10_ci", x_ci(two_vs_cold[1].1)),
        ("voyager", fmt::x(interp(&ground, VOYAGER_YEARS))),
        ("sweep_work_range", x_ci(range(&all_work))),
        ("sweep_fewer_range", fmt::range((1.0 - wrong_hi, 1.0 - wrong_lo), fmt::pct)),
    ]);

    let dir = Path::new(env!("CARGO_MANIFEST_DIR")).join("docs");
    std::fs::create_dir_all(&dir).expect("create docs/");
    let page = Page {
        generated: &generated,
        headline: &headline,
        line: &line,
        cold_aging: base.cold_aging,
        rows: &rows,
        default,
        ground: &ground,
        cold: &cold,
        display: &display,
    };
    std::fs::write(dir.join("data.json"), data_json(&page)).expect("write data.json");
    println!("wrote {}", dir.join("data.json").display());
    for t in [&LIGHT, &DARK] {
        for (stem, svg) in [
            ("output", output_chart(t, &line, years, base.cold_aging)),
            ("sensitivity", sensitivity_chart(t, &rows, (default.0, default.1))),
            ("ground", ground_chart(t, &ground)),
            ("cold-aging", cold_chart(t, &cold, headline.work_vs_tmr.0, headline.simplex_vs_tmr)),
        ] {
            let path = dir.join(format!("{stem}-{}.svg", t.name));
            std::fs::write(&path, svg).expect("write chart");
            println!("wrote {}", path.display());
        }
    }

    println!();
    println!("generated {} at {} on {} with {}", generated.date, generated.commit, generated.host, generated.rustc);
    println!("plotted to year {years}");
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
    println!();
    for (k, v) in &display {
        println!("{k:<26} {v}");
    }
}
