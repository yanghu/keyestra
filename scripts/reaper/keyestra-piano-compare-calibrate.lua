-- Keyestra Piano Compare loudness calibration for REAPER 7.
--
-- Load this file once from Actions > Show action list > New action > Load
-- ReaScript, then run it with the canonical Keyestra Piano Compare project
-- active. The script measures every KEYESTRA_PIANO_ID group with REAPER's
-- native dry-run renderer, proposes constant track-fader trims, and applies
-- them only after confirmation.

local TITLE = "Keyestra Piano Compare Calibration"
local PROJECT_SECTION = "KEYESTRA"
local PROJECT_MARKER_KEY = "PIANO_COMPARE"
local TRACK_ID_KEY = "P_EXT:KEYESTRA_PIANO_ID"
local CALIBRATION_VERSION = "1"

local BLOCK_SECONDS = 4.2
local VELOCITIES = { 40, 55, 70, 85, 100, 115 }
local FIRST_CHORD = { 48, 55, 60, 64, 67, 72 }
local SECOND_CHORD = { 50, 57, 62, 65, 69, 74 }
local TAIL_SECONDS = 5.0

local project = 0
local saved = nil
local temporary_items = {}
local cleanup_complete = false
local ui_refresh_suppressed = false

local function fail(message)
  error(message, 0)
end

local function track_name(track)
  local _, name = reaper.GetSetMediaTrackInfo_String(track, "P_NAME", "", false)
  return name ~= "" and name or "(unnamed track)"
end

local function track_piano_id(track)
  local _, value = reaper.GetSetMediaTrackInfo_String(track, TRACK_ID_KEY, "", false)
  return value
end

local function db_from_gain(gain)
  if gain <= 0 then
    return -150.0
  end
  return 20.0 * math.log(gain) / math.log(10.0)
end

local function gain_from_db(db)
  return 10.0 ^ (db / 20.0)
end

local function format_db(value)
  return string.format("%+.2f dB", value)
end

local function find_piano_groups()
  local by_id = {}
  local ordered = {}
  for index = 0, reaper.CountTracks(project) - 1 do
    local track = reaper.GetTrack(project, index)
    local id = track_piano_id(track)
    if id ~= "" then
      local group = by_id[id]
      if not group then
        group = { id = id, tracks = {}, names = {} }
        by_id[id] = group
        table.insert(ordered, group)
      end
      table.insert(group.tracks, track)
      table.insert(group.names, track_name(track))
    end
  end
  for _, group in ipairs(ordered) do
    group.name = table.concat(group.names, " + ")
  end
  return ordered
end

local NUMERIC_RENDER_KEYS = {
  "RENDER_SETTINGS",
  "RENDER_BOUNDSFLAG",
  "RENDER_STARTPOS",
  "RENDER_ENDPOS",
  "RENDER_CHANNELS",
  "RENDER_SRATE",
  "RENDER_TAILFLAG",
  "RENDER_TAILMS",
  "RENDER_NORMALIZE",
  "RENDER_DELAY",
}

local function capture_state()
  local state = {
    tracks = {},
    render = {},
    cursor = reaper.GetCursorPositionEx(project),
  }
  for index = 0, reaper.CountTracks(project) - 1 do
    local track = reaper.GetTrack(project, index)
    state.tracks[track] = {
      selected = reaper.GetMediaTrackInfo_Value(track, "I_SELECTED"),
      mute = reaper.GetMediaTrackInfo_Value(track, "B_MUTE"),
      solo = reaper.GetMediaTrackInfo_Value(track, "I_SOLO"),
    }
  end
  for _, key in ipairs(NUMERIC_RENDER_KEYS) do
    state.render[key] = reaper.GetSetProjectInfo(project, key, 0, false)
  end
  return state
end

local function restore_state()
  if cleanup_complete then
    return
  end
  if not saved and #temporary_items == 0 then
    cleanup_complete = true
    return
  end
  for index = #temporary_items, 1, -1 do
    local entry = temporary_items[index]
    if reaper.ValidatePtr2(project, entry.track, "MediaTrack*") and
        reaper.ValidatePtr2(project, entry.item, "MediaItem*") then
      reaper.DeleteTrackMediaItem(entry.track, entry.item)
    end
  end
  temporary_items = {}

  if saved then
    for track, state in pairs(saved.tracks) do
      if reaper.ValidatePtr2(project, track, "MediaTrack*") then
        reaper.SetMediaTrackInfo_Value(track, "B_MUTE", state.mute)
        reaper.SetMediaTrackInfo_Value(track, "I_SOLO", state.solo)
        reaper.SetTrackSelected(track, state.selected ~= 0)
      end
    end
    for _, key in ipairs(NUMERIC_RENDER_KEYS) do
      reaper.GetSetProjectInfo(project, key, saved.render[key], true)
    end
    reaper.SetEditCurPos2(project, saved.cursor, false, false)
  end

  if ui_refresh_suppressed then
    reaper.PreventUIRefresh(-1)
    ui_refresh_suppressed = false
  end
  reaper.TrackList_AdjustWindows(false)
  reaper.UpdateArrange()
  cleanup_complete = true
end

local function add_chord(take, start_time, duration, notes, velocity)
  for note_index, pitch in ipairs(notes) do
    local note_start = start_time + (note_index - 1) * 0.035
    local start_ppq = reaper.MIDI_GetPPQPosFromProjTime(take, note_start)
    local end_ppq = reaper.MIDI_GetPPQPosFromProjTime(take, start_time + duration)
    reaper.MIDI_InsertNote(take, false, false, start_ppq, end_ppq, 0, pitch, velocity, true)
  end
end

local function add_sustain(take, time, value)
  local ppq = reaper.MIDI_GetPPQPosFromProjTime(take, time)
  reaper.MIDI_InsertCC(take, false, false, ppq, 0xB0, 0, 64, value)
end

local function create_calibration_item(track, start_time, musical_end)
  local item = reaper.CreateNewMIDIItemInProj(track, start_time, musical_end, false)
  if not item then
    fail("Could not create the temporary calibration MIDI item on " .. track_name(track) .. ".")
  end
  table.insert(temporary_items, { track = track, item = item })
  local take = reaper.GetActiveTake(item)
  if not take or not reaper.TakeIsMIDI(take) then
    fail("REAPER did not create a MIDI take on " .. track_name(track) .. ".")
  end

  reaper.MIDI_DisableSort(take)
  for velocity_index, velocity in ipairs(VELOCITIES) do
    local block_start = start_time + (velocity_index - 1) * BLOCK_SECONDS
    add_sustain(take, block_start + 0.05, 127)
    add_chord(take, block_start + 0.20, 1.35, FIRST_CHORD, velocity)
    add_sustain(take, block_start + 1.72, 0)
    add_sustain(take, block_start + 1.92, 127)
    add_chord(take, block_start + 2.05, 1.35, SECOND_CHORD, velocity)
    add_sustain(take, block_start + 3.58, 0)
  end
  reaper.MIDI_Sort(take)
  return item
end

local function find_dry_run_action()
  local section = reaper.SectionFromUniqueID(0)
  if not section then
    return nil, "REAPER main action section was unavailable."
  end

  local best_command = nil
  local best_score = -1
  local candidates = {}
  local index = 0
  while true do
    local command, name = reaper.kbd_enumerateActions(section, index)
    if not command or command == 0 then
      break
    end
    local lower = (name or ""):lower()
    if lower:find("dry run render", 1, true) then
      table.insert(candidates, string.format("%d: %s", command, name))
      local score = 10
      if lower:find("no output", 1, true) then score = score + 10 end
      if lower:find("file:", 1, true) then score = score + 3 end
      if lower:find("selected items", 1, true) then score = score - 20 end
      if lower:find("selected tracks", 1, true) then score = score - 5 end
      if score > best_score then
        best_command = command
        best_score = score
      end
    end
    index = index + 1
  end

  if best_command then
    return best_command, nil
  end
  return nil, "No native 'Dry run render' action was found. Update REAPER 7 and try again." ..
      (#candidates > 0 and ("\nCandidates:\n" .. table.concat(candidates, "\n")) or "")
end

local function normalize_decimal(value)
  local normalized = value:gsub(",", ".")
  return normalized
end

local function number_after_label(text, labels)
  for segment in text:gmatch("[^\r\n;\t]+") do
    local lower = segment:lower()
    for _, label in ipairs(labels) do
      local first, last = lower:find(label, 1, true)
      if first then
        local after = segment:sub(last + 1)
        local value = after:match("[-+]?%d+[%,%.]?%d*")
        if value then
          return tonumber(normalize_decimal(value))
        end
      end
    end
  end
  return nil
end

local function parse_render_stats(raw, summary)
  local combined = (raw or "") .. "\n" .. (summary or "")
  local integrated = number_after_label(combined, {
    "lufs-i", "lufs_i", "lufsi", "integrated loudness", "integrated:",
  })
  local true_peak = number_after_label(combined, {
    "true peak", "true_peak", "truepeak", "dbtp",
  })
  if not true_peak then
    local before_dbtp = combined:match("([-+]?%d+[%,%.]?%d*)%s*[dD][bB][tT][pP]")
    if before_dbtp then
      true_peak = tonumber(normalize_decimal(before_dbtp))
    end
  end
  return integrated, true_peak
end

local function prepare_render(start_time, end_time)
  -- Selected tracks through master, custom bounds, stereo, no normalization.
  reaper.GetSetProjectInfo(project, "RENDER_SETTINGS", 128, true)
  reaper.GetSetProjectInfo(project, "RENDER_BOUNDSFLAG", 0, true)
  reaper.GetSetProjectInfo(project, "RENDER_STARTPOS", start_time, true)
  reaper.GetSetProjectInfo(project, "RENDER_ENDPOS", end_time, true)
  reaper.GetSetProjectInfo(project, "RENDER_CHANNELS", 2, true)
  reaper.GetSetProjectInfo(project, "RENDER_SRATE", 0, true)
  reaper.GetSetProjectInfo(project, "RENDER_TAILFLAG", 0, true)
  reaper.GetSetProjectInfo(project, "RENDER_TAILMS", 0, true)
  reaper.GetSetProjectInfo(project, "RENDER_NORMALIZE", 0, true)
  reaper.GetSetProjectInfo(project, "RENDER_DELAY", 0, true)
end

local function isolate_group(group, all_groups)
  for index = 0, reaper.CountTracks(project) - 1 do
    reaper.SetTrackSelected(reaper.GetTrack(project, index), false)
  end
  for _, candidate in ipairs(all_groups) do
    for _, track in ipairs(candidate.tracks) do
      reaper.SetMediaTrackInfo_Value(track, "I_SOLO", 0)
      reaper.SetMediaTrackInfo_Value(track, "B_MUTE", candidate == group and 0 or 1)
      if candidate == group then
        reaper.SetTrackSelected(track, true)
      end
    end
  end
  reaper.TrackList_AdjustWindows(false)
  reaper.UpdateArrange()
end

local function measure_group(group, all_groups, dry_run_command, start_time, render_end)
  isolate_group(group, all_groups)
  local _, raw = reaper.GetSetProjectInfo_String(
    project, "RENDER_STATS", tostring(dry_run_command), false)
  local _, summary = reaper.GetSetProjectInfo_String(project, "RENDER_STATS_SUMMARY", "", false)
  local integrated, true_peak = parse_render_stats(raw, summary)
  if not integrated then
    fail("REAPER completed the dry run but LUFS-I could not be read for " .. group.name ..
      ".\n\nRENDER_STATS:\n" .. (raw or "") .. "\n\nSummary:\n" .. (summary or ""))
  end
  if integrated < -70.0 then
    fail(group.name .. " measured " .. string.format("%.2f LUFS-I", integrated) ..
      ", which is effectively silent. Confirm that the instrument is loaded and produces audio.")
  end
  group.lufs_i = integrated
  group.true_peak = true_peak
  group.old_volumes = {}
  for _, track in ipairs(group.tracks) do
    table.insert(group.old_volumes, reaper.GetMediaTrackInfo_Value(track, "D_VOL"))
  end
end

local function build_report(groups, reference_lufs, spread)
  local lines = {
    "Native REAPER dry-run measurement completed.",
    "Reference: quietest measured piano (no piano is boosted).",
    "",
  }
  for _, group in ipairs(groups) do
    local correction = reference_lufs - group.lufs_i
    group.correction_db = correction
    local peak_text = group.true_peak and string.format("%+.2f dBTP", group.true_peak) or "n/a dBTP"
    local current_db = db_from_gain(group.old_volumes[1])
    local current_text = #group.old_volumes == 1 and format_db(current_db) or "group faders"
    local reference_text = math.abs(correction) < 0.005 and "  [reference]" or ""
    table.insert(lines, string.format(
      "%s\n  %.2f LUFS-I | %s | current %s | change %s%s",
      group.name, group.lufs_i, peak_text, current_text, format_db(correction), reference_text))
  end
  table.insert(lines, "")
  table.insert(lines, string.format("Measured spread: %.2f dB", spread))
  if spread > 18.0 then
    table.insert(lines, "WARNING: The spread exceeds 18 dB. A plug-in may be silent or misconfigured; automatic application is disabled.")
  else
    table.insert(lines, "Apply these constant corrections to the piano track faders?")
    table.insert(lines, "All tracks in a multi-track piano group receive the same correction.")
  end
  return table.concat(lines, "\n")
end

local function apply_corrections(groups, report)
  reaper.Undo_BeginBlock2(project)
  for _, group in ipairs(groups) do
    local factor = gain_from_db(group.correction_db)
    for index, track in ipairs(group.tracks) do
      reaper.SetMediaTrackInfo_Value(track, "D_VOL", group.old_volumes[index] * factor)
    end
  end
  reaper.SetProjExtState(project, PROJECT_SECTION, "CALIBRATION_VERSION", CALIBRATION_VERSION)
  reaper.SetProjExtState(project, PROJECT_SECTION, "CALIBRATION_REFERENCE", "quietest")
  reaper.SetProjExtState(project, PROJECT_SECTION, "CALIBRATION_REPORT", report)
  reaper.Undo_EndBlock2(project, "Apply Keyestra Piano Compare loudness calibration", -1)
  reaper.TrackList_AdjustWindows(false)
  reaper.UpdateArrange()
end

local function run()
  local marker_present = select(1, reaper.GetProjExtState(
    project, PROJECT_SECTION, PROJECT_MARKER_KEY)) > 0
  if not marker_present then
    fail("The active project is not tagged as a Keyestra Piano Compare project.")
  end

  local play_state = reaper.GetPlayStateEx(project)
  if (play_state & 4) ~= 0 then
    fail("Stop recording before running calibration.")
  end

  local groups = find_piano_groups()
  if #groups < 2 then
    fail("At least two tracks tagged with KEYESTRA_PIANO_ID are required.")
  end
  for _, group in ipairs(groups) do
    for _, track in ipairs(group.tracks) do
      if reaper.TrackFX_GetInstrument(track) < 0 then
        fail(track_name(track) .. " has no instrument FX. Load its piano plug-in before calibration.")
      end
    end
  end

  local dry_run_command, action_error = find_dry_run_action()
  if not dry_run_command then
    fail(action_error)
  end

  local intro = string.format(
    "Measure %d piano groups with a temporary %.1f-second pp-to-ff MIDI sequence?\n\n" ..
    "Release all notes and the sustain pedal first. REAPER will stop playback, temporarily change mute/selection states, and run one native dry render per piano. Those states and all render settings will be restored. No fader changes occur until you confirm the report.",
    #groups, #VELOCITIES * BLOCK_SECONDS + TAIL_SECONDS)
  if reaper.MB(intro, TITLE, 1) ~= 1 then
    return
  end

  reaper.OnStopButton()
  saved = capture_state()
  reaper.PreventUIRefresh(1)
  ui_refresh_suppressed = true

  local start_time = math.max(reaper.GetProjectLength(project) + 2.0, 2.0)
  local musical_end = start_time + #VELOCITIES * BLOCK_SECONDS
  local render_end = musical_end + TAIL_SECONDS
  for _, group in ipairs(groups) do
    for _, track in ipairs(group.tracks) do
      create_calibration_item(track, start_time, musical_end)
    end
  end
  prepare_render(start_time, render_end)
  reaper.PreventUIRefresh(-1)
  ui_refresh_suppressed = false

  for _, group in ipairs(groups) do
    measure_group(group, groups, dry_run_command, start_time, render_end)
  end

  local reference_lufs = groups[1].lufs_i
  local loudest_lufs = groups[1].lufs_i
  for _, group in ipairs(groups) do
    reference_lufs = math.min(reference_lufs, group.lufs_i)
    loudest_lufs = math.max(loudest_lufs, group.lufs_i)
  end
  local spread = loudest_lufs - reference_lufs
  local report = build_report(groups, reference_lufs, spread)

  restore_state()
  reaper.ClearConsole()
  reaper.ShowConsoleMsg(report .. "\n")

  if spread > 18.0 then
    reaper.MB(report, TITLE, 0)
    return
  end

  local answer = reaper.MB(report, TITLE, 3)
  if answer ~= 6 then
    reaper.MB("Measurement complete. No track faders were changed. The report remains in the ReaScript console.", TITLE, 0)
    return
  end

  apply_corrections(groups, report)
  if reaper.MB(
      "Calibration was applied as constant track-fader trims and can be undone with Ctrl+Z.\n\nSave the Piano Compare project now?",
      TITLE, 4) == 6 then
    reaper.Main_SaveProject(project, false)
    reaper.MB("Calibration applied and project saved. Finish with a blind A/B listening check; adjust only about +/-0.5 dB if needed.", TITLE, 0)
  else
    reaper.MB("Calibration applied but not saved. Press Ctrl+S after your blind A/B listening check.", TITLE, 0)
  end
end

local ok, message = xpcall(run, debug.traceback)
restore_state()
if not ok then
  reaper.MB("Calibration stopped safely. Temporary MIDI and changed measurement states were restored.\n\n" .. tostring(message), TITLE, 0)
end
