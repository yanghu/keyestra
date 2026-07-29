use std::collections::{BTreeMap, VecDeque};
use std::sync::Mutex;
use std::time::Instant;

use serde::Serialize;

use crate::metronome::GridTiming;

const CHORD_WINDOW_US: u64 = 52_000;
const RECENT_HIT_LIMIT: usize = 32;

#[derive(Debug, Clone, Copy, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct PracticeSettings {
    pub bpm: u16,
    pub beats_per_bar: u8,
    pub subdivision: u8,
    pub target_rounds: u8,
    pub count_in_bars: u8,
    pub hit_window_ms: f64,
}

impl Default for PracticeSettings {
    fn default() -> Self {
        Self {
            bpm: 80,
            beats_per_bar: 4,
            subdivision: 2,
            target_rounds: 12,
            count_in_bars: 1,
            hit_window_ms: 45.0,
        }
    }
}

impl PracticeSettings {
    pub fn normalized(mut self) -> Self {
        self.bpm = self.bpm.clamp(30, 240);
        self.beats_per_bar = self.beats_per_bar.clamp(1, 12);
        self.subdivision = self.subdivision.clamp(1, 4);
        self.target_rounds = self.target_rounds.clamp(1, 64);
        self.count_in_bars = self.count_in_bars.clamp(1, 4);
        self.hit_window_ms = self.hit_window_ms.clamp(10.0, 120.0);
        self
    }

    fn slots_per_round(self) -> u64 {
        self.beats_per_bar as u64 * self.subdivision as u64
    }

    fn total_slots(self) -> u64 {
        self.slots_per_round() * self.target_rounds as u64
    }

    fn count_in_slots(self) -> u64 {
        self.slots_per_round() * self.count_in_bars as u64
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum PracticeStatus {
    Idle,
    CountIn,
    Running,
    Paused,
    Complete,
}

impl PracticeStatus {
    fn as_str(self) -> &'static str {
        match self {
            Self::Idle => "idle",
            Self::CountIn => "countIn",
            Self::Running => "running",
            Self::Paused => "paused",
            Self::Complete => "complete",
        }
    }

    fn accepts_hits(self) -> bool {
        matches!(self, Self::CountIn | Self::Running)
    }
}

#[derive(Debug, Clone)]
struct SlotResult {
    error_ms: f64,
    extras: u16,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct PracticeHit {
    pub slot: u64,
    pub round: u8,
    pub beat: u8,
    pub division: u8,
    pub error_ms: f64,
    pub within_window: bool,
    pub extra: bool,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct PracticeFeedback {
    pub label: String,
    pub detail: String,
    pub bias_ms: f64,
    pub spread_ms: f64,
    pub hit_rate: u8,
    pub missed: u64,
    pub extras: u64,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct PracticeRound {
    pub round: u8,
    pub bias_ms: f64,
    pub spread_ms: f64,
    pub hit_rate: u8,
    pub missed: u64,
    pub extras: u64,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct PracticeSnapshot {
    pub status: String,
    pub settings: PracticeSettings,
    pub count_in_remaining: u64,
    pub current_round: u8,
    pub completed_rounds: u8,
    pub feedback: PracticeFeedback,
    pub recent: Vec<PracticeHit>,
    pub rounds: Vec<PracticeRound>,
}

#[derive(Debug)]
struct PracticeState {
    status: PracticeStatus,
    settings: PracticeSettings,
    generation: Option<u64>,
    segment_start_slot: u64,
    settled_slots: u64,
    current_slot: u64,
    count_in_remaining: u64,
    slots: BTreeMap<u64, SlotResult>,
    recent: VecDeque<PracticeHit>,
    last_arrival_us: Option<u64>,
}

impl Default for PracticeState {
    fn default() -> Self {
        Self {
            status: PracticeStatus::Idle,
            settings: PracticeSettings::default(),
            generation: None,
            segment_start_slot: 0,
            settled_slots: 0,
            current_slot: 0,
            count_in_remaining: 0,
            slots: BTreeMap::new(),
            recent: VecDeque::new(),
            last_arrival_us: None,
        }
    }
}

pub struct Practice {
    clock: Instant,
    state: Mutex<PracticeState>,
}

impl Practice {
    pub fn new() -> Self {
        Self {
            clock: Instant::now(),
            state: Mutex::new(PracticeState::default()),
        }
    }

    pub fn start(&self, settings: PracticeSettings) {
        if let Ok(mut state) = self.state.lock() {
            let settings = settings.normalized();
            *state = PracticeState {
                status: PracticeStatus::CountIn,
                settings,
                count_in_remaining: settings.count_in_slots(),
                ..PracticeState::default()
            };
        }
    }

    pub fn pause(&self, timing: Option<GridTiming>) {
        if let Ok(mut state) = self.state.lock() {
            state.refresh(timing);
            if matches!(
                state.status,
                PracticeStatus::CountIn | PracticeStatus::Running
            ) {
                state.segment_start_slot = state.settled_slots;
                state.generation = None;
                state.status = PracticeStatus::Paused;
                state.last_arrival_us = None;
            }
        }
    }

    pub fn resume(&self) {
        if let Ok(mut state) = self.state.lock() {
            if state.status == PracticeStatus::Paused {
                state.generation = None;
                state.status = PracticeStatus::CountIn;
                state.count_in_remaining = state.settings.count_in_slots();
                state.last_arrival_us = None;
            }
        }
    }

    pub fn finish(&self, timing: Option<GridTiming>) {
        if let Ok(mut state) = self.state.lock() {
            state.refresh(timing);
            if state.status != PracticeStatus::Idle {
                state.status = PracticeStatus::Complete;
            }
        }
    }

    pub fn reset(&self) {
        if let Ok(mut state) = self.state.lock() {
            *state = PracticeState::default();
        }
    }

    pub fn ingest(&self, message: &[u8], timing: Option<GridTiming>) {
        if message.len() < 3 || message[0] & 0xf0 != 0x90 || message[2] == 0 {
            return;
        }
        let arrival_us = self.clock.elapsed().as_micros().min(u64::MAX as u128) as u64;
        if let Ok(mut state) = self.state.lock() {
            state.ingest_hit_at(arrival_us, timing);
        }
    }

    pub fn snapshot(&self, timing: Option<GridTiming>) -> PracticeSnapshot {
        self.state
            .lock()
            .map(|mut state| {
                state.refresh(timing);
                state.snapshot()
            })
            .unwrap_or_else(|_| PracticeState::default().snapshot())
    }

    pub fn to_json(&self, timing: Option<GridTiming>) -> String {
        serde_json::to_string(&self.snapshot(timing))
            .unwrap_or_else(|_| "{\"status\":\"error\"}".to_string())
    }
}

impl PracticeState {
    fn timing_matches_settings(&self, timing: GridTiming) -> bool {
        timing.bpm == self.settings.bpm
            && timing.beats_per_bar == self.settings.beats_per_bar
            && timing.subdivision == self.settings.subdivision
    }

    fn bind_generation(&mut self, timing: GridTiming) -> bool {
        if !self.timing_matches_settings(timing) {
            return false;
        }
        if self.generation == Some(timing.generation) {
            return true;
        }
        if self.generation.is_some() {
            return false;
        }
        self.generation = Some(timing.generation);
        true
    }

    fn session_slot_for(&self, grid_index: u64) -> Option<u64> {
        let local = grid_index.checked_sub(self.settings.count_in_slots())?;
        Some(self.segment_start_slot.saturating_add(local))
    }

    fn refresh(&mut self, timing: Option<GridTiming>) {
        if !self.status.accepts_hits() {
            return;
        }
        let Some(timing) = timing else {
            return;
        };
        if !self.bind_generation(timing) {
            return;
        }

        let count_in_slots = self.settings.count_in_slots();
        if timing.grid_index < count_in_slots {
            self.status = PracticeStatus::CountIn;
            self.count_in_remaining = count_in_slots.saturating_sub(timing.grid_index);
            return;
        }
        self.status = PracticeStatus::Running;
        self.count_in_remaining = 0;

        let current = self
            .segment_start_slot
            .saturating_add(timing.grid_index - count_in_slots);
        self.current_slot = current.min(self.settings.total_slots());

        let settled_local_count = if timing.phase_ms >= self.effective_window_ms(timing) {
            timing.grid_index.saturating_add(1)
        } else {
            timing.grid_index
        };
        if let Some(settled_local) = settled_local_count.checked_sub(count_in_slots) {
            self.settled_slots = self
                .settled_slots
                .max(self.segment_start_slot.saturating_add(settled_local))
                .min(self.settings.total_slots());
        }

        if self.settled_slots >= self.settings.total_slots() {
            self.status = PracticeStatus::Complete;
        }
    }

    fn effective_window_ms(&self, timing: GridTiming) -> f64 {
        self.settings.hit_window_ms.min(timing.interval_ms * 0.35)
    }

    fn ingest_hit_at(&mut self, arrival_us: u64, timing: Option<GridTiming>) {
        if !self.status.accepts_hits() {
            return;
        }
        let Some(timing) = timing else {
            return;
        };
        self.refresh(Some(timing));
        if self.generation != Some(timing.generation)
            || !self.timing_matches_settings(timing)
            || !self.status.accepts_hits()
            || self.status == PracticeStatus::CountIn
        {
            return;
        }
        if self
            .last_arrival_us
            .is_some_and(|last| arrival_us.saturating_sub(last) < CHORD_WINDOW_US)
        {
            return;
        }
        self.last_arrival_us = Some(arrival_us);
        let Some(slot) = self.session_slot_for(timing.nearest_grid_index) else {
            return;
        };
        if slot >= self.settings.total_slots() {
            self.status = PracticeStatus::Complete;
            return;
        }

        let slots_per_round = self.settings.slots_per_round();
        let within_window = timing.error_ms.abs() <= self.effective_window_ms(timing);
        let extra = if let Some(existing) = self.slots.get_mut(&slot) {
            existing.extras = existing.extras.saturating_add(1);
            true
        } else {
            self.slots.insert(
                slot,
                SlotResult {
                    error_ms: timing.error_ms,
                    extras: 0,
                },
            );
            false
        };
        self.recent.push_front(PracticeHit {
            slot,
            round: (slot / slots_per_round) as u8 + 1,
            beat: ((slot % slots_per_round) / self.settings.subdivision as u64) as u8 + 1,
            division: (slot % self.settings.subdivision as u64) as u8 + 1,
            error_ms: round_one(timing.error_ms),
            within_window,
            extra,
        });
        self.recent.truncate(RECENT_HIT_LIMIT);
    }

    fn snapshot(&self) -> PracticeSnapshot {
        let slots_per_round = self.settings.slots_per_round();
        let completed_rounds =
            (self.settled_slots / slots_per_round).min(self.settings.target_rounds as u64) as u8;
        let current_round = if self.status == PracticeStatus::Idle {
            0
        } else {
            (self.current_slot / slots_per_round)
                .min(self.settings.target_rounds.saturating_sub(1) as u64) as u8
                + 1
        };
        let count_in_remaining = self.count_in_remaining;
        let rounds = (0..completed_rounds)
            .map(|round| self.round_summary(round))
            .collect();
        PracticeSnapshot {
            status: self.status.as_str().to_string(),
            settings: self.settings,
            count_in_remaining,
            current_round,
            completed_rounds,
            feedback: self.feedback(self.settled_slots),
            recent: self.recent.iter().cloned().collect(),
            rounds,
        }
    }

    fn round_summary(&self, round: u8) -> PracticeRound {
        let start = round as u64 * self.settings.slots_per_round();
        let end = start + self.settings.slots_per_round();
        let results = self
            .slots
            .range(start..end)
            .map(|(_, value)| value)
            .collect::<Vec<_>>();
        let feedback = feedback_for(
            &results,
            self.settings.slots_per_round(),
            self.settings.hit_window_ms,
        );
        PracticeRound {
            round: round + 1,
            bias_ms: feedback.bias_ms,
            spread_ms: feedback.spread_ms,
            hit_rate: feedback.hit_rate,
            missed: feedback.missed,
            extras: feedback.extras,
        }
    }

    fn feedback(&self, settled_slots: u64) -> PracticeFeedback {
        let results = self
            .slots
            .range(..settled_slots)
            .map(|(_, value)| value)
            .collect::<Vec<_>>();
        feedback_for(&results, settled_slots, self.settings.hit_window_ms)
    }
}

fn feedback_for(
    results: &[&SlotResult],
    expected_slots: u64,
    hit_window_ms: f64,
) -> PracticeFeedback {
    let mut errors = results
        .iter()
        .map(|result| result.error_ms)
        .collect::<Vec<_>>();
    errors.sort_by(f64::total_cmp);
    let bias_ms = median(&errors);
    let spread_ms = match (errors.first(), errors.last()) {
        (Some(first), Some(last)) => last - first,
        _ => 0.0,
    };
    let within = errors
        .iter()
        .filter(|error| error.abs() <= hit_window_ms)
        .count() as u64;
    let missed = expected_slots.saturating_sub(results.len() as u64);
    let extras = results
        .iter()
        .map(|result| result.extras as u64)
        .sum::<u64>();
    let hit_rate = if expected_slots == 0 {
        0
    } else {
        ((within * 100) / expected_slots).min(100) as u8
    };
    let (label, detail) = if errors.is_empty() {
        ("等待输入", "跟着节拍器每格弹一下")
    } else if errors.len() < 4 {
        ("继续弹", "再弹几下就能判断整体倾向")
    } else if bias_ms > 12.0 {
        ("整体偏慢", "圆点在目标拍点之后")
    } else if bias_ms < -12.0 {
        ("整体偏快", "圆点在目标拍点之前")
    } else if spread_ms > 70.0 {
        ("落点不稳定", "平均接近拍点，但前后波动较大")
    } else {
        ("基本稳定", "落点集中在目标拍点附近")
    };
    PracticeFeedback {
        label: label.to_string(),
        detail: detail.to_string(),
        bias_ms: round_one(bias_ms),
        spread_ms: round_one(spread_ms),
        hit_rate,
        missed,
        extras,
    }
}

fn median(values: &[f64]) -> f64 {
    match values.len() {
        0 => 0.0,
        length if length % 2 == 1 => values[length / 2],
        length => (values[length / 2 - 1] + values[length / 2]) / 2.0,
    }
}

fn round_one(value: f64) -> f64 {
    (value * 10.0).round() / 10.0
}

#[cfg(test)]
mod tests {
    use super::*;

    fn timing(grid: u64, nearest: u64, error_ms: f64, phase_ms: f64) -> GridTiming {
        timing_meter(4, 2, grid, nearest, error_ms, phase_ms)
    }

    fn timing_meter(
        beats_per_bar: u8,
        subdivision: u8,
        grid: u64,
        nearest: u64,
        error_ms: f64,
        phase_ms: f64,
    ) -> GridTiming {
        GridTiming {
            generation: 7,
            grid_index: grid,
            nearest_grid_index: nearest,
            error_ms,
            phase_ms,
            interval_ms: 250.0,
            bpm: 120,
            beats_per_bar,
            subdivision,
        }
    }

    #[test]
    fn count_in_is_not_scored_and_late_hits_are_reported() {
        let mut state = PracticeState {
            status: PracticeStatus::CountIn,
            settings: PracticeSettings {
                bpm: 120,
                beats_per_bar: 4,
                subdivision: 2,
                target_rounds: 2,
                count_in_bars: 1,
                hit_window_ms: 45.0,
            },
            ..PracticeState::default()
        };
        state.ingest_hit_at(0, Some(timing(3, 3, 18.0, 18.0)));
        assert!(state.slots.is_empty());

        for index in 0..4 {
            let grid = 8 + index;
            state.ingest_hit_at(
                100_000 + index * 100_000,
                Some(timing(grid, grid, 22.0, 22.0)),
            );
        }
        state.refresh(Some(timing(12, 12, 0.0, 60.0)));
        let feedback = state.snapshot().feedback;
        assert_eq!(feedback.label, "整体偏慢");
        assert_eq!(feedback.bias_ms, 22.0);
    }

    #[test]
    fn notes_inside_chord_window_count_as_one_hit() {
        let mut state = PracticeState {
            status: PracticeStatus::Running,
            settings: PracticeSettings {
                bpm: 120,
                beats_per_bar: 4,
                subdivision: 2,
                count_in_bars: 1,
                ..PracticeSettings::default()
            },
            generation: Some(7),
            ..PracticeState::default()
        };
        state.ingest_hit_at(100_000, Some(timing(8, 8, 5.0, 5.0)));
        state.ingest_hit_at(130_000, Some(timing(8, 8, 8.0, 8.0)));
        assert_eq!(state.slots.len(), 1);
        assert_eq!(state.slots.get(&0).unwrap().extras, 0);
    }

    #[test]
    fn completed_round_includes_missing_and_extra_hits() {
        let mut state = PracticeState {
            status: PracticeStatus::Running,
            settings: PracticeSettings {
                bpm: 120,
                beats_per_bar: 2,
                subdivision: 2,
                target_rounds: 1,
                count_in_bars: 1,
                hit_window_ms: 45.0,
            },
            generation: Some(7),
            ..PracticeState::default()
        };
        state.ingest_hit_at(100_000, Some(timing_meter(2, 2, 4, 4, -10.0, 240.0)));
        state.ingest_hit_at(200_000, Some(timing_meter(2, 2, 5, 5, 10.0, 10.0)));
        state.ingest_hit_at(300_000, Some(timing_meter(2, 2, 5, 5, 12.0, 12.0)));
        state.refresh(Some(timing_meter(2, 2, 8, 8, 0.0, 60.0)));
        let snapshot = state.snapshot();
        assert_eq!(snapshot.status, "complete");
        assert_eq!(snapshot.rounds[0].missed, 2);
        assert_eq!(snapshot.rounds[0].extras, 1);
    }

    #[test]
    fn resume_restores_a_full_count_in() {
        let practice = Practice::new();
        {
            let mut state = practice.state.lock().unwrap();
            state.status = PracticeStatus::Paused;
            state.settings = PracticeSettings {
                bpm: 120,
                beats_per_bar: 4,
                subdivision: 3,
                count_in_bars: 1,
                ..PracticeSettings::default()
            };
            state.count_in_remaining = 0;
        }
        practice.resume();
        assert_eq!(practice.snapshot(None).count_in_remaining, 12);
    }
}
