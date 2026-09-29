# dusk

A Monte Carlo of redundancy policies on hardware that only ever decays. Part of
[Mru](https://mru.space).

Classic triple modular redundancy has a fixed threshold. Three processors vote,
two can still detect a disagreement, and one cannot outvote anything, so the
system stops. On a mission with no repairs and no ground team in reach, that
throws away the last processor's whole remaining life.

The Mru whitepaper proposes a **shrinking quorum** instead: vote while three
processors are alive, compare while two are, and self-check on one. This
program asks what that buys and what it costs, on the same simulated hardware.

**Interactive version: [dusk.mru.space](https://dusk.mru.space)**

## Results

<picture>
  <source media="(prefers-color-scheme: dark)" srcset="docs/output-dark.svg">
  <img src="docs/output-light.svg" alt="Mean useful output over mission time for the three policies. Fixed TMR falls to zero first. Standby simplex holds near half rate. The shrinking quorum tracks TMR while it can vote, then continues at half rate, enclosing the largest area." width="760">
</picture>

The shrinking quorum matches fixed TMR while three processors survive. It
pulls ahead as TMR starts to stop, and it keeps working long after. The shaded
area is the number that matters: total useful work across the whole decline.

Default parameters, 2,000 missions, seed 1:

```
policy              useful      p10      p50      p90      service   wrong days
                    mean y                                  mean y   mean / run
fixed TMR             99.2     37.0     92.3    170.2         99.3        0.000
standby simplex       82.4     38.7     77.4    132.5        165.0        0.586
shrinking quorum     132.0     63.5    126.2    207.4        165.0        0.236

shrinking quorum vs fixed TMR: 1.33x the useful work in total (95% CI 1.31x to 1.35x); per mission median 1.26x (p10 1.00x, p90 2.33x)
shrinking quorum vs standby simplex: 160% of the useful work, 40% of the wrong results (95% CI 37% to 43%)
break-even: fixed TMR comes out ahead only if one wrong result costs more than 139 years of useful work (95% CI 128 to 152)
```

- **Against fixed TMR:** 1.33x the useful work (95% CI 1.31x to 1.35x). Never
  less on any single mission, because the two policies are identical until TMR
  stops. The cost is a small number of wrong results in the single-processor
  tail, which TMR never lives long enough to produce.
- **The price of those wrong results:** TMR comes out ahead only if one wrong
  result costs more than **139 years** of useful work (95% CI 128 to 152).
- **Against standby simplex:** the same service life, 1.6x the useful work, and
  60% fewer wrong results (95% CI 57% to 63%), because it votes for as long as
  it has hardware to vote with.

Intervals are from a paired bootstrap: 2,000 resamples of whole missions.

### When the ground can help

Real spacecraft do not run TMR alone. When a string fails, a ground team
diagnoses it and commands a fallback by hand. `--ground` adds that baseline:
TMR that, once it loses its majority, is switched to self-checking simplex by
the ground after some delay, as long as ground support still exists.

<picture>
  <source media="(prefers-color-scheme: dark)" srcset="docs/ground-dark.svg">
  <img src="docs/ground-light.svg" alt="The shrinking quorum's useful work relative to TMR with a ground-commanded fallback, by years of ground support. 1.33x with no ground team, 1.24x at Voyager's 49 years, falling to 1.00x as support lasts past about 200 years." width="760">
</picture>

```
2000 missions, seed 1; shrinking quorum's useful work vs TMR + ground fallback

ground support ends        1 day   30 days  180 days   95% CI at 30 days
at launch                  1.33x     1.33x     1.33x   1.31x to 1.35x
after 10 years             1.33x     1.33x     1.33x   1.31x to 1.34x
after 25 years             1.31x     1.31x     1.31x   1.29x to 1.33x
after 50 years             1.24x     1.24x     1.24x   1.22x to 1.25x
after 100 years            1.11x     1.11x     1.11x   1.10x to 1.11x
after 200 years            1.01x     1.01x     1.01x   1.01x to 1.01x
never                      1.00x     1.00x     1.00x   1.00x to 1.00x
```

- **Answer time barely matters.** A fallback 1 day or 6 months after it is
  needed gives the same result to two decimals. The lost days are small next
  to decades of single-processor life.
- **What matters is whether anyone is still there.** With ground support that
  never ends, autonomy adds nothing to throughput: the ground does the same
  thing by hand. The whole gain comes from the years after support ends. At
  Voyager's 49 years so far, it is still 1.24x.

These years are relative to the hardware: with a 125-year Weibull scale, the
median processor lives about 98 years. Shorter-lived hardware moves the curve
left.

<picture>
  <source media="(prefers-color-scheme: dark)" srcset="docs/sensitivity-dark.svg">
  <img src="docs/sensitivity-light.svg" alt="For each of six uncertain parameters, the range of the shrinking quorum useful work relative to fixed TMR (1.11x to 1.66x overall) and its wrong results relative to standby simplex (19% to 53% overall)." width="760">
</picture>

`--sweep` varies each parameter the model cannot pin down. Across every row,
the shrinking quorum returned **1.11x to 1.66x** the useful work of fixed TMR
and **47% to 81% fewer** wrong results than standby simplex. The work gain is
largest with early, random failures (Weibull shape 1) and smallest with sharp
wear-out (shape 4), where all three processors tend to die close together and
there is little tail left to use.

<details>
<summary>Full sweep output (the numbers behind the chart)</summary>

```
1000 missions per row, seed 1; unlisted parameters at their defaults

varied                        work vs TMR  wrong/mission wrong vs simplex
shape 1                             1.55x          0.398            53%
shape 1.5                           1.33x          0.243            40%
shape 2.5                           1.18x          0.141            26%
shape 4                             1.11x          0.098            19%
p_corr 0                            1.35x          0.264            41%
p_corr 0.1                          1.33x          0.243            40%
p_corr 0.3                          1.28x          0.182            35%
p_corr 0.6                          1.22x          0.121            30%
spares 0                            1.33x          0.243            40%
spares 1                            1.35x          0.220            35%
spares 2                            1.34x          0.221            34%
spares 4                            1.34x          0.214            31%
p_upset 1e-5                        1.33x          0.001            20%
p_upset 1e-4                        1.33x          0.028            44%
p_upset 1e-3                        1.33x          0.243            40%
p_upset 1e-2                        1.33x          2.475            40%
coverage 0.9                        1.33x          2.482            40%
coverage 0.99                       1.33x          0.243            40%
coverage 0.999                      1.33x          0.033            52%
self-check throughput 0.25          1.17x          0.243            40%
self-check throughput 0.5           1.33x          0.243            40%
self-check throughput 1             1.66x          0.243            40%
```

At `p_upset 1e-5` there is about one wrong result in 1,000 missions, so the
20% in that row is noise, not a finding.

</details>

## Running it

```sh
cargo run --release                 # the default scenario
cargo run --release -- --spares 2   # add salvaged processors
cargo run --release -- --sweep      # sensitivity across uncertain parameters
cargo run --release -- --ground     # against TMR with a ground team
cargo run --release -- --help       # every option
cargo run --release --bin charts    # redraw the charts in docs/
cargo test --release
```

No dependencies. The default run takes under a second; the sweep and the
ground table a few seconds each.
Results depend only on the seed, never on the thread count.

The charts are plain SVG, written by `src/bin/charts.rs` from the same seeds as
the tables above, in a light and a dark version styled after
[mru.space](https://mru.space). Nothing is drawn by hand.

The page at [dusk.mru.space](https://dusk.mru.space) is `docs/index.html`,
served by GitHub Pages. It draws the same charts live from `docs/data.json`,
which `charts` also writes, so the page and the README never disagree.

## The model

**Processors die** on a Weibull lifetime. Shape 1.5 and a 125-year scale give
a median life of about 98 years.

**Deaths can be correlated.** With probability `p_corr`, a death also kills one
other processor in service that day, and that death can do the same. This is a
stand-in for a shared thermal or power fault, or one particle shower hitting
neighbours.

**Spares are salvaged.** With `--spares N`, N more processors belong to
instruments. Each joins the pool when its instrument dies, but it has aged in
the same radiation since launch, so it can die before it is ever used. Both
TMR and the shrinking quorum use spares; simplex swaps them in as standbys.

**Upsets corrupt a day's result.** Each processor, each day, has a `p_upset`
chance of a transient fault that gets past EDAC and scrubbing. That is far
rarer than the raw bit-flip rate, and it is the number that matters here.

**The policies:**

| Live processors | Fixed TMR | Standby simplex | Shrinking quorum |
|---|---|---|---|
| 3 or more | vote, masks one upset | one self-checks | vote, masks one upset |
| 2 | compare, loses the day on any upset | one self-checks | compare, loses the day on any upset |
| 1 | **stops for good** | self-checks | self-checks |

TMR + ground fallback behaves like fixed TMR, except that on one processor it
idles until the ground's command arrives and then self-checks. If support has
already ended when TMR loses its majority, no command comes and it stops.

A self-checking processor catches a fraction `coverage` of its upsets and loses
those days. The rest become **wrong results**: output nobody knew was bad. It
also runs at `selfcheck_throughput` of full rate, because checking yourself
means doing the work twice.

**The comparison is paired.** Every policy runs against the same fault history
and the same upsets, drawn from keyed hashes rather than a shared stream. Any
difference between policies is the policy.

## What this does not claim

The parameters are **illustrative, not calibrated.** They are plausible
orders of magnitude, not values fitted to flight data or radiation test
results. That is why the sweep exists: the claim is the shape of the trade,
and the sweep shows where it holds.

Other simplifications:

- A day is the unit of work. Nothing inside a day is modelled.
- Two or more upsets in a vote are always caught as a split. A real voter can
  be fooled by identical wrong answers, so this slightly flatters every voting
  policy equally.
- Power, thermal limits, and duty cycling are not modelled. Every processor
  runs every day.
- The ground team in `--ground` is perfect: it always diagnoses correctly and
  its command always works. Real anomaly response is slower and less certain,
  so this baseline flatters the ground.

## Questions or contributions

Issues and PRs aren't open to the public on this repo, but we'd love to hear from
you. Email [contact@mru.space](mailto:contact@mru.space).

## Licence

Code under [Apache License 2.0](./LICENSE). The **Mru** name and mark are
trademarks; see [TRADEMARK.md](./TRADEMARK.md).
