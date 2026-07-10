# Mobile Monitor Plan

The mobile monitor should feel like a practice dashboard, not a narrow desktop
monitor page. A phone on the piano should answer the most important musical
questions at a glance.

## Current Direction

- Mobile uses its own DOM in `src/monitor_web/index.html`.
- Desktop keeps the fuller monitor layout.
- Mobile starts directly at practice feedback; the app title, input name, and
  message count are hidden to save vertical space.
- Current mobile order:
  1. Current feedback
  2. Recent velocity
  3. Current chord
  4. Velocity stability
  5. Details

## Notes From Design Discussion

- Raw MIDI and event logs are low priority on mobile. Keep them hidden or
  collapsed unless explicitly needed.
- Recent velocity should stay compressed by chord group. A chord should produce
  one bar based on the group average, not one bar per note.
- Current chord balance is high priority because it answers whether a chord was
  even and at what dynamic level.
- Velocity stability may not always be useful, because recent history may
  contain mixed material that the player is not trying to evaluate. Keep it
  lower priority until a better practice-mode model exists.
- Dynamics labels matter. Velocity numbers are clear, but `pp`, `p`, `mp`, `mf`,
  `f`, and `ff` make the feedback more musical.

## Candidate Next Features

- Practice modes:
  - Free: observe current playing without a target.
  - Steady: aim for a selected dynamic such as `p`, `mp`, or `mf`.
  - Crescendo: compare recent velocity against a rising target.
  - Diminuendo: compare recent velocity against a falling target.
  - Chord: prioritize chord balance and per-note dynamic differences.
- Target dynamics:
  - Show current target, such as `target mp` or `p -> mf`.
  - Add a target band or target line to the recent velocity view.
  - Make thresholds configurable later if the default dynamic ranges do not
    match the piano/curve feel.
- MIDI remote control:
  - Use edge keys to change mode or target without touching the phone.
  - Candidate defaults:
    - Lowest edge key: previous mode
    - Highest edge key: next mode
    - Adjacent edge keys: lower/raise target dynamic
  - Control notes should not count as practice notes or affect trend/stability.
  - Add a guard against accidental triggers, such as only allowing outermost
    notes or requiring a short hold/double-tap.
- Mobile chord view:
  - Keep overall dynamic and balance score visible.
  - Highlight the most uneven note if spread is large.
  - Consider showing per-note dynamics only when it does not crowd the view.
- Recent velocity view:
  - Keep one bar per chord group.
  - Consider overlaying target bands/lines once practice modes exist.
  - Consider smoothing or window controls if the trend feels too jumpy.

## Implementation Reminders

- The monitor web assets live in `src/monitor_web/` and are embedded into the
  release binary with `include_str!`.
- Keep `/`, `/assets/monitor.css`, `/assets/monitor.js`, `/state`, `/events`,
  and `/seed-log` routes working.
- For normal validation, run:
  - `cargo fmt --check`
  - `cargo test`
- For monitor-server or release-package changes, also run:
  - `cargo build --release --bin fp10-map --bin fp10-map-tray --bin fp10-monitor-server`
- If release binaries are running, Windows may lock `target/release/*.exe`.
  Stop the running `fp10-*` processes before deploying over the release build.
