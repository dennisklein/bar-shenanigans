# opener-gui

A desktop browser for build orders in the `opener.json` format
(see [`opener-format`](../opener-format)). It shows a simulated opener
step by step: who builds what, where, and how the economy reacts.

## Run

```sh
cargo run --release -p opener-gui -- path/to/opener.json
```

Without an argument, it opens `openers/all_that_glitters_eco.json`, or else
the example in `crates/opener-format/examples/opener.json`. You can also
type a path in the file field and press **Load**.

On Linux the GUI needs X11 or Wayland and `libxkbcommon-x11`.

## What you see

| Area | Content |
| --- | --- |
| Top bar | Play/pause, jump to start, step to the previous or next event, speed, time slider, objective and score |
| Left | Build queue. Grey: not ordered yet. Blue: builder is moving. Yellow: under construction. White: done. Red: unfinished at the horizon |
| Centre | Map with metal spots, build footprints and builders |
| Bottom | Metal and energy: stored, income and usage, with a cursor at the current time |

## Controls

- Click a row in the queue or a building on the map: jump to the start of that step.
- Click or drag in a curve: seek to that time.
- Map: drag to pan, scroll to zoom, **Fit map** to reset.
- **step** buttons jump to the previous or next start or finish of any step.

## Layout

- `model.rs`: step state at a time `t`, colours, time format.
- `map.rs`: map drawing, pan and zoom, hit testing.
- `app.rs`: window layout, playback, queue table and curves.
