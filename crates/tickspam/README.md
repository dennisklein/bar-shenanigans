# tickspam

Tick spam simulator for Beyond All Reason (BAR). It answers two questions
for the Armada Tick (`armflea`) from the Bot Lab (`armlab`):

1. **Configuration.** How many Ticks per minute do `L` labs with `N`
   construction turrets ("nanos", `armnanotc`) make? This is the question of
   the video [Most Efficient Tick Spam Configuration](https://www.youtube.com/watch?v=95fFokFMv3w)
   (BetterStrategy).
2. **Opener.** Which commander build order gives the most Ticks at minute 3
   on a given map and start position?

```
BAR checkout (Lua unit defs)  --bar-data-->  Catalog (costs, build power)
map spec (Lua)                --bar-data-->  Scenario (start, metal spots, wind)
                                                  |
                         +------------------------+------------------------+
                         |                        |                        |
                   tickspam configs         tickspam search          tickspam replay
                   (labs x nanos table)     (beam search)            (Lua plan file)
                                                  |                        |
                                                  +-----> opener.json <----+
                                                          (opener-gui)
```

## Usage

```sh
# lab and nano table, resources unlimited
cargo run --release -p tickspam -- configs --bar ../Beyond-All-Reason

# best commander build for the most Ticks at 180 s
cargo run --release -p tickspam -- search --bar ../Beyond-All-Reason \
    --map data/maps/all_that_glitters.lua --start P1 --out tickspam.json

# replay a hand-written plan
cargo run --release -p tickspam -- replay --bar ../Beyond-All-Reason \
    --map data/maps/all_that_glitters.lua --start P1 \
    --plan crates/tickspam/plans/lab_first.lua --out lab_first.json
```

Open the `opener.json` file in the opener browser (`crates/opener-gui`).

Options:

| Option | Default | Meaning |
|---|---|---|
| `--mode fight\|guard` | `fight` | How nanos assist: fight/patrol helps the nearest unit under construction, guard follows one lab. |
| `--horizon S` | `180` | Time in seconds at which the Ticks are counted. |
| `--width N` | `128` | Beam width of the search. |
| `--max-labs N`, `--max-nanos N` | `4`, `8` | Limits of the search. |
| `--labs N`, `--nanos N`, `--seconds S` | `4`, `8`, `180` | Size and measure time of the `configs` table. |

A plan file is Lua:

```lua
return {
  name = "lab first",
  actions = { "lab", "mex", "mex", "wind", "wind", "wind" },
}
```

The actions are `mex` (nearest free metal spot), `wind`, `solar`, `lab` and
`nano` (next to the lab with the fewest nanos). After the last action the
commander guards the first lab.

## How the model works

The simulation steps at the engine rate of 30 frames per second and copies
the Recoil engine rules that decide the Tick rate:

```
frame:   0        15       30       45       60  ...
         |--------|--------|--------|--------|
lab:     slow update (SU) once per 15 frames, at the lab's own phase
Tick k:  ====== built ======|                         (lab + nanos)
                            ^ complete
lab:                           ^ next SU: queue Tick k+1
pad:                        ~~~~^ Tick k has left the pad: start Tick k+1
nano:                                ^ next SU: assist Tick k+1   (fight)
nano:                                         ^ one more SU        (guard)
```

- A lab queues the next unit only in its own slow update
  (`CFactoryCAI::SlowUpdate`). Each Tick thus waits 1 to 15 frames.
- A nano joins a new Tick in its next slow update on fight/patrol
  (`CBuilderCAI::ExecuteFight`), or one slow update later on guard
  (`CBuilderCAI::ExecuteGuard` pushes a repair order and returns).
- A Tick needs 800 build power. A lab has 150, a nano 200. With many
  nanos a Tick is complete in less than one slow update, so the waits
  above dominate and the Tick rate comes in steps of 0.5 s per Tick.
- Build power is all or nothing per builder and frame, in unit order, if
  the team has the resources (`CUnit::AddBuildPower`).
- Income comes in half-second chunks at each unit's slow update. A mex
  first pays its 3 E/s upkeep.
- The commander starts a structure one slow update after it is in range
  (build animation), and walks at its full speed in a straight line.

The slow update phase of a unit depends on its place in the engine's unit
list, which other players' units also change. `configs` thus gives the
mean over 15 phase offsets, and the minimum and maximum.

The values that are not unit def values are in `Rules` (`src/sim.rs`):
pad clear time (8 frames, estimate), lab open animation (10 frames),
build animation (1 slow update).

## Results

### Configurations (`results/configs_fight.json`, `results/configs_guard.json`)

Resources unlimited, Ticks counted for 180 s after 20 s warm-up.
Efficiency is the Tick rate divided by the rate if all build power worked
all the time.

| Labs | Nanos | Ticks/min (fight) | Efficiency (fight) | Ticks/min (guard) | Efficiency (guard) |
|---|---|---|---|---|---|
| 1 | 0 | 10.7 | 95% | 10.7 | 95% |
| 1 | 1 | 20.5 | 78% | 20.0 | 76% |
| 2 | 3 | 60.2 | 89% | 44.6 | 66% |
| 4 | 6 | 117.0 | 87% | 89.2 | 66% |
| 4 | 8 | 124.1 | 75% | 98.3 | 60% |

- Labs without nanos use 95% of their build power. Per build power a lab
  costs more metal (3.3 M) but less energy (6.3 E) than a nano (1.2 M,
  16 E).
- 2 labs with 3 nanos is a local best (89%). The video's comments name the
  same setup as best, and say 4 labs with 6 nanos was worse. The model
  agrees in direction; the difference is smaller in the model.
- Nanos on fight/patrol make 20 to 35% more Ticks than nanos on guard.
- Ticks use 300 energy each: 60 Ticks per minute need 300 E/s.

The video frames could not be read here (YouTube needs a sign-in from the
build server), so the video's spreadsheet numbers are not compared yet.

### Opener: All That Glitters, start P1, Ticks at 180 s

`openers/all_that_glitters_tickspam.json` (`mise run tickspam:search`).
Wind 0 to 16 gives 11.9 E/s per turbine on average.

| Plan | Commander builds | Ticks at 180 s |
|---|---|---|
| Beam search | wind x3, lab, mex, wind x2, mex, solar, mex, wind | **46** |
| Lab first (baseline) | lab, wind x2, mex x2, wind x4 | 36 |
| Eco first (baseline) | mex x2, wind x3, lab, wind x2, nano | 25 |

- The lab is complete at 36 s, the first Tick at 42 s. After its last
  build (85 s) the commander guards the lab: 150 + 300 build power.
- No nano: a nano costs 3200 E, more than 10 Ticks, and does not pay back
  in 3 minutes. At 300 s the search still uses no nano (89 Ticks).
- The search gives the same result with beam width 128 and 512, and with
  fight or guard nanos.

## Limits of the model

- No enemy, no unit losses, no reclaim. Ticks stay alive.
- Wind is the map's average all the time.
- The base layout is fixed and compact; the commander walks straight.
- Nanos on fight help only units under construction (no repair of damaged
  units).
- The pad clear time is an estimate, not a measurement.
