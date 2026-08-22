# Recorder Preview and Precise Clip Selection

Status: proposed implementation design

This design covers the next Recorder phase:

- Show the rolling MIDI buffer as an activity timeline on a phone.
- Select an exact time range with touch-friendly handles.
- Preview that exact range through the computer's existing MIDI sound source.
- Save the same prepared range as a standard MIDI file.

It deliberately does not turn the monitor into a DAW.

## Product Decisions

### Preview runs on the computer

The phone is a remote control. MIDI preview is emitted by the monitor server and
heard through the computer's existing Garritan, Reaper, or standalone piano
setup.

The MVP does not synthesize piano audio in the browser because that would add
sample assets, licensing, mobile audio restrictions, memory use, and a preview
sound that differs from the user's real piano setup.

### Default preview output is `Keyestra MIDI`

Preview defaults to the MIDI output whose name matches the monitored
`Keyestra MIDI` input. This requires no new MIDI Loopback 2.0 pair and works with the sound
source the user already configured.

Sending preview back through that virtual port also makes it return to the
monitor input. To keep playback from contaminating the rolling recording:

- Preview sets an atomic `capture_suspended` gate.
- The MIDI input callback may continue updating visual Monitor state, but skips
  `Recorder::ingest` while the gate is set.
- Preview is rejected while a Take is actively recording.
- Every preview exit path sends cleanup messages.
- Capture resumes roughly 150 ms after cleanup, allowing virtual-port tail
  messages to drain.
- The UI clearly says that rolling capture is paused during preview.

A separately configured `Keyestra Playback` output can be added later for advanced
routing. It is not an MVP requirement.

### Preview and export share one clip preparation path

Both operations consume the same `PreparedClip`. This is the central invariant:
the range the user hears must have the same boundaries and MIDI state as the
range written to disk.

### All new ranges are half-open

New APIs and clip preparation use:

```text
[start_us, end_us)
```

An event at `start_us` is included. An event at `end_us` is excluded, and
generated cleanup is placed at `end_us`. Adjacent clips therefore do not
duplicate a boundary event.

## User Flow

1. Open the existing Monitor page on the phone.
2. Expand `编辑片段` in the Recorder card.
3. The initial selection is:
   - the stopped Take, when one exists; otherwise
   - the latest 60 seconds within the available rolling buffer.
4. Choose a view scale: `30 秒`, `2 分钟`, `5 分钟`, or `全部`.
5. Drag the start handle, end handle, or the selected region.
6. Read exact start, end, duration, Note On count, and event count.
7. Select either boundary and use `−1s`, `−100ms`, `+100ms`, or `+1s` for fine
   adjustment.
8. Tap `试听`. The computer sound source plays the selected range and the phone
   shows a moving playhead.
9. Tap `停止` to end early, or let the range finish.
10. Tap `保存此片段`. The new `.mid` appears in the existing download list.

Changing a boundary while preview is active stops preview first. A server
restart or an expired rolling-buffer range invalidates the old selection rather
than silently saving different material.

## Time and Session Model

Recorder events keep their current monotonic `at_us` time relative to Recorder
startup.

Add these values:

```text
session_id         Identifies this Recorder process lifetime
revision           Increments for every recordable event
now_us             Current Recorder monotonic time
available_start_us Time of the oldest retained event
available_end_us   Current now_us
```

`session_id` can combine startup epoch time and process ID; no random-number
dependency is required. It prevents a phone page opened before a server restart
from applying stale coordinates to a new buffer.

JavaScript `Number` can safely represent these relative microsecond values for
the expected process lifetime. The UI displays milliseconds, but all requests
retain microsecond coordinates.

## Timeline Model

Do not send all raw MIDI bytes to the phone. Return bounded aggregate bins for
the visible window. Each bin contains:

```json
{
  "startUs": 120000000,
  "endUs": 120500000,
  "eventCount": 34,
  "noteOnCount": 8,
  "velocityAverage": 71,
  "velocityMaximum": 104,
  "minimumNote": 43,
  "maximumNote": 81,
  "pedalDownAtEnd": true
}
```

For detailed windows of at most five minutes, the client may request bounded
Note On markers:

```json
{
  "atUs": 120234500,
  "channel": 0,
  "note": 60,
  "velocity": 82
}
```

Limits:

- 50 to 1200 timeline bins.
- At most 5000 Note On markers.
- Set `notesTruncated: true` when the marker limit is reached.
- Do not send SysEx contents or raw event bytes to the browser.

## HTTP API

The current synchronous, thread-per-connection HTTP server remains. No async
runtime, WebSocket, or web framework is needed.

New mutations use `POST`, an empty body, query parameters, and:

```http
X-Keyestra-Control: 1
```

GET endpoints remain read-only. The server does not enable CORS.

### Timeline

```http
GET /recorder/timeline?sessionId=...&startUs=...&endUs=...&bins=600&notes=1
```

On the first request, all range parameters may be omitted and the server returns
the default window.

```json
{
  "ok": true,
  "schemaVersion": 1,
  "sessionId": "1722190000000-1234",
  "revision": 9214,
  "generatedAtUs": 3471000000,
  "buffer": {
    "availableStartUs": 120000000,
    "availableEndUs": 3471000000,
    "limitUs": 3600000000,
    "eventCount": 9214
  },
  "range": {
    "startUs": 3171000000,
    "endUs": 3471000000
  },
  "take": {
    "startUs": 3200000000,
    "endUs": 3400000000,
    "recording": false
  },
  "bins": [],
  "notes": [],
  "notesTruncated": false,
  "stateComplete": true
}
```

`stateComplete: false` means the rolling buffer may have discarded channel
state that occurred before its oldest retained event.

Read-only timeline requests may clamp a requested display window to the current
buffer. Save and preview mutations must never clamp silently.

### Save an exact range

```http
POST /recorder/save-range?sessionId=...&startUs=...&endUs=...
X-Keyestra-Control: 1
Content-Length: 0
```

```json
{
  "ok": true,
  "recording": {
    "name": "keyestra-1722190123456.mid",
    "size": 18432,
    "modifiedMs": 1722190123456,
    "startUs": 3200000000,
    "endUs": 3400000000,
    "durationUs": 200000000
  },
  "warnings": []
}
```

### Preview status and output list

```http
GET /recorder/preview
```

```json
{
  "ok": true,
  "outputs": ["Keyestra MIDI", "Keyestra Playback"],
  "preview": {
    "state": "stopped",
    "previewId": null,
    "sessionId": null,
    "selectionStartUs": null,
    "selectionEndUs": null,
    "positionUs": null,
    "outputName": "Keyestra MIDI",
    "captureSuspended": false,
    "error": null
  }
}
```

### Start or seek preview

```http
POST /recorder/preview?action=play&sessionId=...&startUs=...&endUs=...&positionUs=...&outputName=Keyestra%20MIDI
X-Keyestra-Control: 1
```

When `positionUs` is omitted, playback starts at `startUs`. Seeking safely stops
the old preview and prepares a new clip beginning at the requested position.

### Stop preview

```http
POST /recorder/preview?action=stop&previewId=42
X-Keyestra-Control: 1
```

The ID prevents a stale phone page from stopping a newer preview started by
another page.

### Error shape

```json
{
  "ok": false,
  "error": {
    "code": "range_expired",
    "message": "Selected range is no longer in the rolling buffer",
    "details": {
      "availableStartUs": 130000000
    }
  }
}
```

Required error codes:

- `invalid_parameter`
- `invalid_range`
- `stale_session`
- `range_expired`
- `range_empty`
- `take_in_progress`
- `preview_replaced`
- `output_not_found`
- `output_connect_failed`
- `preview_failed`
- `state_lock_failed`
- `internal_error`

Use HTTP 400 for invalid parameters, 404 for an output that does not exist, 409
for session/range/state conflicts, and 500 for internal errors.

## Canonical Clip Preparation

Introduce a pure `midi_clip` module. It scans the snapshot and produces a
`PreparedClip` used by both SMF encoding and preview scheduling.

### State immediately before the start

Scan events strictly before `start_us` and reconstruct state independently for
each channel:

- Bank Select CC 0 and CC 32.
- Program Change.
- The latest value for CC 0 through 119.
- Pitch Bend.
- Channel Pressure.
- Physically held notes and their Note On velocity.
- Sustain state.
- Notes whose keys were released while sustain remained down.
- Channels encountered in the snapshot.

CC 120 through 127 are Channel Mode messages. Preserve them when they occur
inside the selected range, but do not restore them as persistent start state.

SysEx inside the range is preserved. The MVP does not infer which older SysEx
messages should be repeated at the start.

Preamble ordering:

1. Bank Select.
2. Program Change.
3. Other CC state.
4. Pitch Bend and Channel Pressure.
5. Sustain state.
6. Synthetic Note On for keys still physically held at the boundary.

Events exactly at `start_us` are real in-range events and must not be duplicated
by the state scan.

### Notes crossing the start boundary

Use one fixed `musical` policy:

- If a key remains physically held at the boundary, synthesize Note On at clip
  time zero using its original velocity.
- If the key was released earlier and only sustain keeps the sound alive, do not
  retrigger it.

MIDI cannot recreate the decay phase of a sustained piano sample. Retriggering
such a note introduces a false new attack, which is more misleading than
omitting its remaining tail. Return a `sustained_notes_not_recreated` warning.

Track repeated Note On messages for the same channel and pitch with a count.
Synthesize at most one effective active note, using the latest active velocity.
Treat Note On velocity zero as Note Off.

### End boundary and cleanup

Include original events only while `at_us < end_us`. At exactly `end_us`:

1. Emit Note Off for notes still active.
2. Emit sustain off (`CC64=0`) on involved channels.
3. Emit All Notes Off (`CC123=0`) on involved channels.
4. End the MIDI track.

Emergency Preview stop may additionally emit All Sound Off (`CC120=0`).
Exported files do not include CC120 during normal completion.

Before preview starts, clean the channels that will be used so an earlier
abnormal exit cannot leave stuck state.

When a selection touches a rolling-buffer left edge with incomplete earlier
state, allow the operation but return a `state_incomplete` warning.

### Timing precision

Keep the current SMF timing:

- 1000 ticks per quarter note.
- 500,000 microseconds per quarter note.
- 500 microseconds per tick.

No rhythmic quantization is introduced.

## Mobile Interaction

Use a compact inline editor, not a DAW-style piano roll.

Components:

- Overview activity rail combining note density and velocity.
- Dimmed area outside the selection.
- Start and end handles with at least 44 by 44 CSS-pixel touch targets.
- Draggable selected region.
- Independent preview playhead.
- Start, end, and duration readouts.
- `30s`, `2m`, `5m`, and `全部` view presets.
- `调整开始` and `调整结束` focus controls.
- `−1s`, `−100ms`, `+100ms`, and `+1s` fine adjustments.
- `试听`, `停止`, and `保存此片段` primary actions.

Implement the timeline with Pointer Events and `setPointerCapture`. Each handle
uses `role="slider"`, keyboard arrow support, and appropriate ARIA values. Do
not layer two native range inputs.

Rules:

- Handles do not cross.
- Minimum range is 100 ms.
- Dragging updates local browser state only.
- Refresh selection statistics after pointer release.
- Stop preview before changing the range.
- If the rolling start overtakes the selection, clamp the UI and explicitly say
  that the oldest part left the buffer.
- A changed `session_id` clears the old selection and reports a Monitor restart.
- The server always stops and cleans up at `end_us`, even if the phone sleeps or
  disconnects.

The MVP uses preset zoom levels rather than pinch zoom.

## Rust Module Boundaries

### `src/recorder.rs`

Owns:

- Monotonic clock, `session_id`, and `revision`.
- Rolling `VecDeque`.
- Take state.
- Recorder snapshots and timeline entry point.
- Exact range save entry point.
- Atomic file writes and recording list.
- `capture_suspended` state exposed to the UI.

Proposed methods:

```rust
timeline(request) -> Result<TimelineSnapshot>
prepare_range(session_id, start_us, end_us) -> Result<PreparedClip>
save_range(session_id, start_us, end_us) -> Result<SavedRecordingResult>
take_is_recording() -> bool
```

### `src/midi_clip.rs`

Pure logic:

- `TimedMidiEvent`.
- Half-open range selection.
- Channel-state reconstruction.
- Active note and sustain state machines.
- Start preamble and end cleanup.
- `PreparedClip`.
- SMF encoding.

Most boundary tests belong here.

### `src/midi_preview.rs`

Owns:

- MIDI output enumeration and selection.
- A dedicated scheduling thread.
- `Play`, `Stop`, and `Shutdown` commands.
- The MIDI output connection, exclusively on the scheduling thread.
- Preview ID, state, position, and errors.
- Cleanup and capture-gate lifecycle.

Use `Instant` deadlines plus `recv_timeout` so Stop can interrupt a wait. Do not
sleep or send MIDI while holding a Recorder mutex.

### `src/bin/keyestra-monitor.rs`

Only:

- Parse method, path, query, and required control header.
- Validate API parameters.
- Map typed errors to HTTP status and JSON.
- Coordinate Take and Preview state.
- Serve the existing assets.

Do not add Tokio or a web framework.

### JSON

Add `serde_json` as the one new direct dependency. The new nested API and typed
error responses are large enough that hand-written JSON would create avoidable
escaping and schema bugs. Existing `serde` remains in use.

## Concurrency and Performance

- The MIDI callback performs only a constant-time gate check and normal ingest.
- Copy the requested event snapshot while holding the Recorder lock; perform
  aggregation, clip preparation, SMF encoding, file I/O, and MIDI output after
  releasing it.
- Allow one global preview at a time.
- A new Play safely replaces the previous Play.
- Serialize Take/Preview control transitions with a short-lived control mutex.
- Define and preserve one lock order.
- Timeline bins are capped at 1200 and Note On markers at 5000.
- Refresh an open timeline about every two seconds.
- Poll preview status about every 500 ms and animate between server
  synchronizations in the browser.
- A preview uses an event snapshot, so later rolling-buffer trimming cannot
  change playback already in progress.
- Do not introduce a database or event index until real one-hour performance
  measurements justify it.

All capture-gate paths must be release-safe. Prefer an RAII guard or equivalent
worker-owned lifecycle so output-connection errors and normal completion cannot
leave recording suspended.

## Security

The monitor binds to the LAN. This design improves local control without
claiming full authentication:

- All new mutations are POST.
- Require `X-Keyestra-Control: 1`.
- Do not enable CORS or respond permissively to cross-origin preflight.
- Validate query length, numbers, range limits, bin counts, and output names.
- Keep the existing safe recording-name validation.
- Escape all JSON and HTML output.
- Add `X-Content-Type-Options: nosniff` and
  `Referrer-Policy: no-referrer`.

PIN authentication and internet-facing access remain out of scope.

## Test Matrix

### `midi_clip` unit tests

- Exact `[start,end)` boundaries.
- Invalid, reversed, and empty ranges.
- Note On exactly at start is not duplicated.
- Note Off exactly at end is replaced by cleanup behavior.
- Notes crossing start and end.
- Note On velocity zero.
- Repeated same-note counts.
- Multiple channels.
- Sustain pressed before start and released inside the range.
- Sustain-held released notes are not retriggered and produce a warning.
- CC, Bank Select, Program Change, Pitch Bend, and Channel Pressure restore in
  the correct order.
- CC 120 through 127 are not restored as state.
- SysEx is preserved only inside the range.
- Malformed messages do not panic.
- Cleanup contains pedal off and All Notes Off.
- SMF tick and variable-length delta encoding.
- Incomplete start-state warning.

### Timeline tests

- A boundary event belongs to exactly one bin.
- Empty windows.
- Bin totals.
- Velocity average and maximum.
- Pitch range.
- Pedal state across bins.
- Bin and marker limits.
- Expired ranges.
- Session mismatch.

### Preview tests

Abstract a `MidiSink` and use a fake sink:

- Preamble, body, and cleanup ordering.
- Stop interrupts and cleans up.
- New Play replaces old Play.
- Connection failure releases the capture gate.
- Normal completion releases the gate.
- Multiple-channel cleanup.
- Seek is equivalent to preparing from the new boundary.
- A stale Preview ID cannot stop a newer preview.

Avoid brittle exact wall-clock tests.

### API tests

- GET and POST method enforcement.
- Percent decoding.
- Required control header.
- HTTP status and error schema.
- Invalid numbers, oversized bins, and missing parameters.
- Stale sessions and expired selections.
- Repeated button requests.

### Browser and hardware validation

- 390 by 844, landscape mobile, and desktop layouts.
- Touch handles, selected-region drag, fine adjustment, and zoom.
- Rolling start overtaking the selection.
- Session change after Monitor restart.
- Preview progress and repeated taps.
- Network interruption and missing output.
- ARIA sliders and keyboard use.
- Windows MIDI Services MIDI Loopback 2.0 preview reaches the current sound source.
- Playback events do not re-enter Recorder.
- Active Take rejects preview.
- Sustain crossing a boundary and manual Stop do not leave stuck notes.
- iPhone Safari and Android Chrome.

Run the standard project validation:

```powershell
cargo fmt --check
cargo test
cargo build --release --bin keyestra --bin keyestra-tray --bin keyestra-monitor
```

## Implementation Sequence

Each phase is one reviewable, reversible commit.

1. `Refactor MIDI range preparation`
   - Add `midi_clip.rs`.
   - Move SMF encoding into it.
   - Adopt half-open ranges.
   - Implement state reconstruction and cleanup.
   - Add pure boundary tests.

2. `Add recorder timeline API`
   - Add session and revision.
   - Add bounded timeline aggregation.
   - Add typed JSON and errors.
   - Add GET timeline and POST save-range.

3. `Add server-side MIDI preview engine`
   - Add `midi_preview.rs`.
   - Add output enumeration and scheduling.
   - Default to `Keyestra MIDI`.
   - Add capture gate, Take exclusion, cleanup, and fake-sink tests.

4. `Add mobile clip selection editor`
   - Add overview, handles, selection drag, view presets, and fine adjustment.
   - Connect exact range save.
   - Handle expired ranges and restarted sessions.

5. `Connect preview transport`
   - Add Play, Stop, and seek.
   - Add playhead synchronization.
   - Show capture-paused and output errors clearly.

6. `Polish, document, and release`
   - Update README.
   - Run responsive browser validation.
   - Build and deploy release binaries.
   - Perform live MIDI and stuck-note checks.

Implement in this order. Clip-boundary semantics and tests must be stable before
real MIDI output or touch UI is added.

## Explicitly Deferred

- Audio from the phone speaker.
- Browser SoundFont, samples, or server audio streaming.
- Mandatory separate `Keyestra Playback` port.
- Playing live while preview continues recording.
- Parsing and previewing previously saved `.mid` files.
- Pause/resume, looping, speed changes, or tempo editing.
- Pinch zoom and automatic phrase detection.
- Piano-roll editing, velocity editing, or quantization.
- Audio waveforms or high-quality audio rendering.
- Tracks, mixing, plugins, or other DAW features.
- Crash-recoverable rolling journal.
- PIN, TLS, and internet access.
- Recording rename, tags, delete management, or cloud sync.
