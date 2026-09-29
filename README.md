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

shrinking quorum vs fixed TMR: 1.33x the useful work in total; per mission median 1.26x (p10 1.00x, p90 2.33x)
shrinking quorum vs standby simplex: 160% of the useful work, 40% of the wrong results
```

- **Against fixed TMR:** 1.33x the useful work. Never less on any single
  mission, because the two policies are identical until TMR stops. The cost is
  a small number of wrong results in the single-processor tail, which TMR never
  lives long enough to produce.
- **Against standby simplex:** the same service life, 1.6x the useful work, and
  60% fewer wrong results, because it votes for as long as it has hardware to
  vote with.

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
cargo run --release -- --help       # every option
cargo run --release --bin charts    # redraw the charts in docs/
cargo test --release
```

No dependencies. The default run takes under a second, the sweep a few seconds.
Results depend only on the seed, never on the thread count.

The charts are plain SVG, written by `src/bin/charts.rs` from the same seeds as
the tables above, in a light and a dark version styled after
[mru.space](https://mru.space). Nothing is drawn by hand.

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
- Fixed TMR stopping at one processor is the textbook policy. Real spacecraft
  often have a ground-commanded fallback to one string. The point of the
  shrinking quorum is to make that fallback autonomous, with defined
  behaviour and a measured cost, for missions where the ground cannot help.

## Questions or contributions

Issues and PRs aren't open to the public on this repo, but we'd love to hear from
you — say hi on X at [@mruspace](https://x.com/mruspace) or email
[contact@mru.space](mailto:contact@mru.space).

## Licence

Code under [Apache License 2.0](./LICENSE). The **Mru** name and mark are
trademarks; see [TRADEMARK.md](./TRADEMARK.md).
