use std::collections::{BTreeMap, BTreeSet};

use anyhow::Result;
use serde::Serialize;

pub const MIDI_TICKS_PER_QUARTER: u16 = 1_000;
const MIDI_US_PER_TICK: u64 = 500;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TimedMidiEvent {
    pub at_us: u64,
    pub message: Vec<u8>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ClipEvent {
    pub offset_us: u64,
    pub message: Vec<u8>,
    pub generated: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum ClipWarning {
    StateIncomplete,
    SustainedNotesNotRecreated,
}

#[derive(Debug, Clone)]
pub struct PreparedClip {
    pub selection_start_us: u64,
    pub selection_end_us: u64,
    pub events: Vec<ClipEvent>,
    pub channels: BTreeSet<u8>,
    pub warnings: Vec<ClipWarning>,
    pub original_event_count: usize,
}

#[derive(Debug, Clone, Default)]
struct HeldNote {
    count: u16,
    velocity: u8,
}

#[derive(Debug, Clone)]
struct ChannelState {
    cc: [Option<u8>; 120],
    program: Option<u8>,
    pitch_bend: Option<[u8; 2]>,
    pressure: Option<u8>,
    held: BTreeMap<u8, HeldNote>,
    sustained: BTreeSet<u8>,
    sustain_down: bool,
}

impl Default for ChannelState {
    fn default() -> Self {
        Self {
            cc: [None; 120],
            program: None,
            pitch_bend: None,
            pressure: None,
            held: BTreeMap::new(),
            sustained: BTreeSet::new(),
            sustain_down: false,
        }
    }
}

pub fn prepare_clip(
    snapshot: &[TimedMidiEvent],
    start_us: u64,
    end_us: u64,
    state_complete: bool,
) -> Result<PreparedClip> {
    if start_us >= end_us {
        anyhow::bail!("range start must be before range end");
    }

    let mut states = BTreeMap::<u8, ChannelState>::new();
    let mut channels = BTreeSet::new();
    for event in snapshot.iter().filter(|event| event.at_us < start_us) {
        apply_message(&event.message, &mut states, &mut channels);
    }

    let mut events = Vec::new();
    let mut warnings = Vec::new();
    if !state_complete {
        warnings.push(ClipWarning::StateIncomplete);
    }
    if states.values().any(|state| !state.sustained.is_empty()) {
        warnings.push(ClipWarning::SustainedNotesNotRecreated);
    }

    for (&channel, state) in &states {
        if let Some(value) = state.cc[0] {
            push_generated(&mut events, 0, vec![0xB0 | channel, 0, value]);
        }
        if let Some(value) = state.cc[32] {
            push_generated(&mut events, 0, vec![0xB0 | channel, 32, value]);
        }
        if let Some(program) = state.program {
            push_generated(&mut events, 0, vec![0xC0 | channel, program]);
        }
        for (controller, value) in state.cc.iter().enumerate() {
            if matches!(controller, 0 | 32 | 64) {
                continue;
            }
            if let Some(value) = value {
                push_generated(
                    &mut events,
                    0,
                    vec![0xB0 | channel, controller as u8, *value],
                );
            }
        }
        if let Some([least, most]) = state.pitch_bend {
            push_generated(&mut events, 0, vec![0xE0 | channel, least, most]);
        }
        if let Some(pressure) = state.pressure {
            push_generated(&mut events, 0, vec![0xD0 | channel, pressure]);
        }
        if state.sustain_down {
            push_generated(&mut events, 0, vec![0xB0 | channel, 64, 127]);
        }
        for (&note, held) in &state.held {
            if held.count > 0 {
                push_generated(&mut events, 0, vec![0x90 | channel, note, held.velocity]);
            }
        }
    }

    let original_event_count = snapshot
        .iter()
        .filter(|event| event.at_us >= start_us && event.at_us < end_us)
        .count();
    for event in snapshot
        .iter()
        .filter(|event| event.at_us >= start_us && event.at_us < end_us)
    {
        if let Some(channel) = message_channel(&event.message) {
            channels.insert(channel);
        }
        events.push(ClipEvent {
            offset_us: event.at_us.saturating_sub(start_us),
            message: event.message.clone(),
            generated: false,
        });
        apply_message(&event.message, &mut states, &mut channels);
    }

    let duration_us = end_us - start_us;
    for channel in channels.iter().copied() {
        if let Some(state) = states.get(&channel) {
            for (&note, held) in &state.held {
                if held.count > 0 {
                    push_generated(&mut events, duration_us, vec![0x80 | channel, note, 0]);
                }
            }
        }
    }
    for channel in channels.iter().copied() {
        push_generated(&mut events, duration_us, vec![0xB0 | channel, 64, 0]);
        push_generated(&mut events, duration_us, vec![0xB0 | channel, 123, 0]);
    }

    Ok(PreparedClip {
        selection_start_us: start_us,
        selection_end_us: end_us,
        events,
        channels,
        warnings,
        original_event_count,
    })
}

fn push_generated(events: &mut Vec<ClipEvent>, offset_us: u64, message: Vec<u8>) {
    events.push(ClipEvent {
        offset_us,
        message,
        generated: true,
    });
}

fn apply_message(
    message: &[u8],
    states: &mut BTreeMap<u8, ChannelState>,
    channels: &mut BTreeSet<u8>,
) {
    let Some(status) = message.first().copied() else {
        return;
    };
    if !(0x80..=0xEF).contains(&status) {
        return;
    }
    let channel = status & 0x0F;
    channels.insert(channel);
    let state = states.entry(channel).or_default();
    match status & 0xF0 {
        0x80 if message.len() >= 3 => note_off(state, message[1]),
        0x90 if message.len() >= 3 && message[2] == 0 => note_off(state, message[1]),
        0x90 if message.len() >= 3 => {
            let held = state.held.entry(message[1]).or_default();
            held.count = held.count.saturating_add(1);
            held.velocity = message[2];
            state.sustained.remove(&message[1]);
        }
        0xB0 if message.len() >= 3 => {
            let controller = message[1];
            if controller < 120 {
                state.cc[controller as usize] = Some(message[2]);
            }
            if controller == 64 {
                let down = message[2] >= 64;
                if state.sustain_down && !down {
                    state.sustained.clear();
                }
                state.sustain_down = down;
            } else if controller == 120 || controller == 123 {
                state.held.clear();
                state.sustained.clear();
            }
        }
        0xC0 if message.len() >= 2 => state.program = Some(message[1]),
        0xD0 if message.len() >= 2 => state.pressure = Some(message[1]),
        0xE0 if message.len() >= 3 => state.pitch_bend = Some([message[1], message[2]]),
        _ => {}
    }
}

fn note_off(state: &mut ChannelState, note: u8) {
    if let Some(held) = state.held.get_mut(&note) {
        held.count = held.count.saturating_sub(1);
        if held.count == 0 {
            state.held.remove(&note);
            if state.sustain_down {
                state.sustained.insert(note);
            } else {
                state.sustained.remove(&note);
            }
        }
    }
}

fn message_channel(message: &[u8]) -> Option<u8> {
    let status = *message.first()?;
    (status < 0xF0).then_some(status & 0x0F)
}

pub fn is_note_on(message: &[u8]) -> bool {
    message.len() >= 3 && message[0] & 0xF0 == 0x90 && message[2] > 0
}

pub fn encode_smf(clip: &PreparedClip) -> Result<Vec<u8>> {
    let mut track = Vec::with_capacity(clip.events.len().saturating_mul(5).saturating_add(128));
    track.extend_from_slice(&[0x00, 0xFF, 0x51, 0x03, 0x07, 0xA1, 0x20]);
    let track_name = b"FP10 Mapped";
    track.extend_from_slice(&[0x00, 0xFF, 0x03, track_name.len() as u8]);
    track.extend_from_slice(track_name);

    let mut previous_tick = 0;
    let mut written_events = 0;
    for event in &clip.events {
        let Some(encoded) = encode_midi_message(&event.message) else {
            continue;
        };
        let tick = us_to_tick(event.offset_us);
        write_variable_length(&mut track, tick.saturating_sub(previous_tick))?;
        track.extend_from_slice(&encoded);
        previous_tick = tick;
        written_events += 1;
    }
    if clip.original_event_count == 0 || written_events == 0 {
        anyhow::bail!("The selected range has no exportable MIDI events");
    }

    let end_tick = us_to_tick(clip.selection_end_us - clip.selection_start_us);
    write_variable_length(&mut track, end_tick.saturating_sub(previous_tick))?;
    track.extend_from_slice(&[0xFF, 0x2F, 0x00]);

    let mut file = Vec::with_capacity(track.len().saturating_add(22));
    file.extend_from_slice(b"MThd");
    file.extend_from_slice(&6_u32.to_be_bytes());
    file.extend_from_slice(&0_u16.to_be_bytes());
    file.extend_from_slice(&1_u16.to_be_bytes());
    file.extend_from_slice(&MIDI_TICKS_PER_QUARTER.to_be_bytes());
    file.extend_from_slice(b"MTrk");
    file.extend_from_slice(&(track.len() as u32).to_be_bytes());
    file.extend_from_slice(&track);
    Ok(file)
}

fn encode_midi_message(message: &[u8]) -> Option<Vec<u8>> {
    let status = *message.first()?;
    match status {
        0x80..=0xEF => {
            let expected = if matches!(status & 0xF0, 0xC0 | 0xD0) {
                2
            } else {
                3
            };
            (message.len() >= expected).then(|| message[..expected].to_vec())
        }
        0xF0 | 0xF7 => {
            let payload = &message[1..];
            let mut encoded = vec![status];
            write_variable_length(&mut encoded, payload.len() as u64).ok()?;
            encoded.extend_from_slice(payload);
            Some(encoded)
        }
        _ => None,
    }
}

fn us_to_tick(us: u64) -> u64 {
    us.saturating_add(MIDI_US_PER_TICK / 2) / MIDI_US_PER_TICK
}

pub fn write_variable_length(out: &mut Vec<u8>, value: u64) -> Result<()> {
    if value > 0x0FFF_FFFF {
        anyhow::bail!("MIDI delta time is too large");
    }
    let mut value = value as u32;
    let mut buffer = [0_u8; 4];
    let mut index = 3;
    buffer[index] = (value & 0x7F) as u8;
    while {
        value >>= 7;
        value > 0
    } {
        index -= 1;
        buffer[index] = ((value & 0x7F) as u8) | 0x80;
    }
    out.extend_from_slice(&buffer[index..]);
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn event(at_us: u64, message: &[u8]) -> TimedMidiEvent {
        TimedMidiEvent {
            at_us,
            message: message.to_vec(),
        }
    }

    #[test]
    fn range_is_half_open_and_start_event_is_not_duplicated() {
        let clip = prepare_clip(
            &[event(100, &[0x90, 60, 90]), event(200, &[0x80, 60, 0])],
            100,
            200,
            true,
        )
        .unwrap();
        assert_eq!(clip.original_event_count, 1);
        assert_eq!(
            clip.events
                .iter()
                .filter(|item| item.message == [0x90, 60, 90])
                .count(),
            1
        );
        assert!(!clip
            .events
            .iter()
            .any(|item| !item.generated && item.message == [0x80, 60, 0]));
    }

    #[test]
    fn held_note_crossing_start_is_recreated_and_cleaned_up() {
        let clip = prepare_clip(&[event(10, &[0x91, 64, 77])], 20, 40, true).unwrap();
        assert_eq!(clip.events[0].message, [0x91, 64, 77]);
        assert!(clip
            .events
            .iter()
            .any(|item| item.offset_us == 20 && item.message == [0x81, 64, 0]));
    }

    #[test]
    fn sustained_released_note_is_not_retriggered() {
        let clip = prepare_clip(
            &[
                event(1, &[0xB0, 64, 127]),
                event(2, &[0x90, 60, 80]),
                event(3, &[0x80, 60, 0]),
                event(20, &[0xB0, 64, 0]),
            ],
            10,
            30,
            true,
        )
        .unwrap();
        assert!(clip
            .warnings
            .contains(&ClipWarning::SustainedNotesNotRecreated));
        assert!(!clip
            .events
            .iter()
            .any(|item| item.generated && item.message[0] & 0xF0 == 0x90));
    }

    #[test]
    fn preamble_has_canonical_order_and_omits_channel_mode_state() {
        let clip = prepare_clip(
            &[
                event(1, &[0xB0, 7, 99]),
                event(2, &[0xE0, 1, 65]),
                event(3, &[0xC0, 4]),
                event(4, &[0xB0, 32, 2]),
                event(5, &[0xB0, 0, 1]),
                event(6, &[0xD0, 50]),
                event(7, &[0xB0, 123, 0]),
                event(20, &[0x90, 60, 1]),
            ],
            10,
            30,
            true,
        )
        .unwrap();
        let messages: Vec<_> = clip
            .events
            .iter()
            .take_while(|item| item.offset_us == 0)
            .map(|item| item.message.as_slice())
            .collect();
        assert_eq!(
            messages,
            vec![
                &[0xB0, 0, 1][..],
                &[0xB0, 32, 2],
                &[0xC0, 4],
                &[0xB0, 7, 99],
                &[0xE0, 1, 65],
                &[0xD0, 50]
            ]
        );
        assert!(!messages.iter().any(|message| **message == [0xB0, 123, 0]));
    }

    #[test]
    fn repeated_notes_use_latest_velocity_and_counted_note_offs() {
        let clip = prepare_clip(
            &[
                event(1, &[0x90, 60, 40]),
                event(2, &[0x90, 60, 90]),
                event(3, &[0x80, 60, 0]),
                event(20, &[0x80, 60, 0]),
            ],
            10,
            30,
            true,
        )
        .unwrap();
        assert_eq!(clip.events[0].message, [0x90, 60, 90]);
        assert!(clip
            .events
            .iter()
            .any(|item| !item.generated && item.message == [0x80, 60, 0]));
    }

    #[test]
    fn note_on_velocity_zero_is_a_note_off() {
        let clip = prepare_clip(
            &[
                event(1, &[0x90, 60, 80]),
                event(2, &[0x90, 60, 0]),
                event(20, &[0x91, 64, 70]),
            ],
            10,
            30,
            true,
        )
        .unwrap();
        assert!(!clip
            .events
            .iter()
            .any(|item| item.generated && item.message == [0x90, 60, 80]));
    }

    #[test]
    fn cleanup_finishes_note_offs_before_pedal_and_all_notes_off() {
        let clip = prepare_clip(
            &[event(1, &[0x90, 60, 80]), event(2, &[0x91, 64, 70])],
            0,
            10,
            true,
        )
        .unwrap();
        let cleanup: Vec<_> = clip
            .events
            .iter()
            .filter(|item| item.offset_us == 10)
            .map(|item| item.message.clone())
            .collect();
        assert_eq!(
            cleanup,
            vec![
                vec![0x80, 60, 0],
                vec![0x81, 64, 0],
                vec![0xB0, 64, 0],
                vec![0xB0, 123, 0],
                vec![0xB1, 64, 0],
                vec![0xB1, 123, 0],
            ]
        );
    }

    #[test]
    fn malformed_messages_do_not_panic_and_sysex_is_range_limited() {
        let clip = prepare_clip(
            &[
                event(1, &[]),
                event(2, &[0x90]),
                event(10, &[0xF0, 1, 2, 0xF7]),
                event(20, &[0xF0, 3, 0xF7]),
            ],
            10,
            20,
            true,
        )
        .unwrap();
        assert_eq!(clip.original_event_count, 1);
        assert!(clip.events.iter().any(|item| item.message[0] == 0xF0));
    }

    #[test]
    fn incomplete_state_is_warned_and_smf_uses_expected_division() {
        let clip = prepare_clip(&[event(10, &[0x90, 60, 100])], 10, 510, false).unwrap();
        assert!(clip.warnings.contains(&ClipWarning::StateIncomplete));
        let file = encode_smf(&clip).unwrap();
        assert_eq!(&file[..4], b"MThd");
        assert_eq!(&file[12..14], &[0x03, 0xE8]);
        assert!(file.ends_with(&[0xFF, 0x2F, 0x00]));
    }

    #[test]
    fn variable_length_encoding_matches_midi_examples() {
        for (value, expected) in [
            (0, vec![0x00]),
            (127, vec![0x7F]),
            (128, vec![0x81, 0x00]),
            (16_383, vec![0xFF, 0x7F]),
            (16_384, vec![0x81, 0x80, 0x00]),
        ] {
            let mut actual = Vec::new();
            write_variable_length(&mut actual, value).unwrap();
            assert_eq!(actual, expected);
        }
    }
}
