-- Keyestra fixed-MIDI piano benchmark bootstrap for REAPER 7.
--
-- Run this from the saved Keyestra Piano Compare project. Select the
-- maestro-v3.0.0.csv file when prompted. The script copies four source MIDI
-- files into REAPER's media directory, creates one MIDI source track routed to
-- every tagged piano, adds four fixed excerpts as regions, and saves a separate
-- Keyestra Piano Benchmark project. The original Piano Compare project remains
-- the reusable live-playing project.

local PROJECT_SECTION = "KEYESTRA"
local PIANO_COMPARE_KEY = "PIANO_COMPARE"
local BENCHMARK_KEY = "PIANO_BENCHMARK"
local TRACK_ID_KEY = "P_EXT:KEYESTRA_PIANO_ID"
local SOURCE_TRACK_KEY = "P_EXT:KEYESTRA_BENCHMARK_SOURCE"
local SOURCE_TRACK_NAME = "Benchmark MIDI (MAESTRO)"
local REGION_PREFIX = "[Keyestra Benchmark] "
local PROJECT_BPM = 120
local SETUP_LEAD_SECONDS = 0.05
local RELEASE_TAIL_SECONDS = 4.0
local BETWEEN_REGIONS_SECONDS = 2.0

-- MAESTRO v3.0.0 stores these Disklavier performances at a fixed 120 BPM;
-- expressive timing is encoded directly in the MIDI event positions.
local CLIPS = {
  {
    id = "A",
    title = "Chopin Op. 9 No. 2 — lyrical / p–mf",
    relative_path = "2011/MIDI-Unprocessed_06_R3_2011_MID--AUDIO_R3-D3_05_Track05_wav.midi",
    copied_name = "A-chopin-op9-no2.midi",
    source_start = 0.0,
    duration = 34.75,
    initial_cc = {},
    color = { 58, 132, 195 },
  },
  {
    id = "B",
    title = "Debussy Des pas sur la neige — pp / resonance / decay",
    relative_path = "2018/MIDI-Unprocessed_Recital16_MID--AUDIO_16_R1_2018_wav--3.midi",
    copied_name = "B-debussy-des-pas-sur-la-neige.midi",
    source_start = 29.10,
    duration = 29.40,
    initial_cc = { [64] = 127, [67] = 127 },
    color = { 92, 153, 101 },
  },
  {
    id = "C",
    title = "Chopin Op. 48 No. 1 — ff / bass / projection",
    relative_path = "2015/MIDI-Unprocessed_R1_D1-1-8_mid--AUDIO-from_mp3_01_R1_2015_wav--2.midi",
    copied_name = "C-chopin-op48-no1-climax.midi",
    source_start = 160.40,
    duration = 30.40,
    initial_cc = { [67] = 127 },
    color = { 197, 91, 72 },
  },
  {
    id = "D",
    title = "Scriabin Op. 8 No. 10 — speed / attack / clarity",
    relative_path = "2011/MIDI-Unprocessed_15_R1_2011_MID--AUDIO_R1-D6_09_Track09_wav.midi",
    copied_name = "D-scriabin-op8-no10.midi",
    source_start = 57.58,
    duration = 25.00,
    initial_cc = { [64] = 27, [67] = 127 },
    color = { 146, 103, 180 },
  },
}

local function path_join(left, right)
  local separator = package.config:sub(1, 1)
  local normalized = right:gsub("[/\\]", separator)
  return left .. separator .. normalized
end

local function file_exists(path)
  local file = io.open(path, "rb")
  if not file then return false end
  file:close()
  return true
end

local function copy_file(source_path, destination_path)
  local source, source_error = io.open(source_path, "rb")
  if not source then return false, source_error end
  local destination, destination_error = io.open(destination_path, "wb")
  if not destination then
    source:close()
    return false, destination_error
  end
  while true do
    local chunk = source:read(1024 * 1024)
    if not chunk then break end
    local ok, write_error = destination:write(chunk)
    if not ok then
      source:close()
      destination:close()
      return false, write_error
    end
  end
  source:close()
  destination:close()
  return true, nil
end

local function track_extension_value(track, key)
  local _, value = reaper.GetSetMediaTrackInfo_String(track, key, "", false)
  return value
end

local function collect_piano_tracks(project)
  local tracks = {}
  local ids = {}
  for index = 0, reaper.CountTracks(project) - 1 do
    local track = reaper.GetTrack(project, index)
    local id = track_extension_value(track, TRACK_ID_KEY)
    if id ~= "" then
      if ids[id] then return nil, "Duplicate Keyestra piano ID: " .. id end
      ids[id] = true
      table.insert(tracks, track)
    end
  end
  if #tracks == 0 then
    return nil, "No tagged Keyestra piano tracks were found. Run this from the saved Piano Compare project."
  end
  return tracks, nil
end

local function validate_project_tempo(project)
  local count = reaper.CountTempoTimeSigMarkers(project)
  for index = 0, count - 1 do
    local present, _, _, _, bpm = reaper.GetTempoTimeSigMarker(project, index)
    if present and math.abs(bpm - PROJECT_BPM) > 0.0001 then
      return false, "The project contains a tempo marker that is not 120 BPM. Use the clean Piano Compare project so MAESTRO timing is not altered."
    end
  end
  return true, nil
end

local function find_or_create_source_track(project)
  for index = 0, reaper.CountTracks(project) - 1 do
    local track = reaper.GetTrack(project, index)
    if track_extension_value(track, SOURCE_TRACK_KEY) == "1" then return track end
  end
  reaper.InsertTrackInProject(project, 0, 0)
  return reaper.GetTrack(project, 0)
end

local function clear_source_track(track)
  for index = reaper.CountTrackMediaItems(track) - 1, 0, -1 do
    reaper.DeleteTrackMediaItem(track, reaper.GetTrackMediaItem(track, index))
  end
  for index = reaper.GetTrackNumSends(track, 0) - 1, 0, -1 do
    reaper.RemoveTrackSend(track, 0, index)
  end
end

local function configure_source_track(track)
  reaper.GetSetMediaTrackInfo_String(track, "P_NAME", SOURCE_TRACK_NAME, true)
  reaper.GetSetMediaTrackInfo_String(track, SOURCE_TRACK_KEY, "1", true)
  reaper.SetMediaTrackInfo_Value(track, "B_MAINSEND", 0)
  reaper.SetMediaTrackInfo_Value(track, "I_RECARM", 0)
  reaper.SetMediaTrackInfo_Value(track, "I_RECMON", 0)
  reaper.SetMediaTrackInfo_Value(track, "I_MIDIHWOUT", -1)
  reaper.SetTrackColor(track, reaper.ColorToNative(92, 92, 92) | 0x1000000)
end

local function route_source_to_pianos(source_track, piano_tracks)
  for _, piano_track in ipairs(piano_tracks) do
    local send_index = reaper.CreateTrackSend(source_track, piano_track)
    reaper.SetTrackSendInfo_Value(source_track, 0, send_index, "I_SRCCHAN", -1)
    reaper.SetTrackSendInfo_Value(source_track, 0, send_index, "I_MIDIFLAGS", 0)
    reaper.SetTrackSendInfo_Value(source_track, 0, send_index, "B_MUTE", 0)
  end
end

local function add_controller_item(track, position, values, name)
  local item = reaper.CreateNewMIDIItemInProj(track, position, position + 0.04, false)
  local take = reaper.GetActiveTake(item)
  local ppq = reaper.MIDI_GetPPQPosFromProjTime(take, position + 0.001)
  for channel = 0, 15 do
    for _, controller in ipairs({ 64, 66, 67, 123 }) do
      reaper.MIDI_InsertCC(take, false, false, ppq, 0xB0, channel, controller, 0)
    end
  end
  local initialized_ppq = reaper.MIDI_GetPPQPosFromProjTime(take, position + 0.010)
  for controller, value in pairs(values) do
    reaper.MIDI_InsertCC(take, false, false, initialized_ppq, 0xB0, 0, controller, value)
  end
  reaper.MIDI_Sort(take)
  reaper.GetSetMediaItemTakeInfo_String(take, "P_NAME", name, true)
  reaper.SetMediaItemInfo_Value(item, "B_LOOPSRC", 0)
  reaper.SetMediaItemInfo_Value(item, "C_BEATATTACHMODE", 0)
  return item
end

local function add_external_midi_item(track, file_path, position, clip)
  local source = reaper.PCM_Source_CreateFromFileEx(file_path, true)
  if not source then return nil, "REAPER could not open " .. file_path end
  local item = reaper.AddMediaItemToTrack(track)
  local take = reaper.AddTakeToMediaItem(item)
  reaper.SetMediaItemTake_Source(take, source)
  reaper.SetMediaItemInfo_Value(item, "D_POSITION", position)
  reaper.SetMediaItemInfo_Value(item, "D_LENGTH", clip.duration)
  reaper.SetMediaItemInfo_Value(item, "B_LOOPSRC", 0)
  reaper.SetMediaItemInfo_Value(item, "C_BEATATTACHMODE", 0)
  reaper.SetMediaItemTakeInfo_Value(take, "D_STARTOFFS", clip.source_start)
  reaper.SetMediaItemTakeInfo_Value(take, "D_PLAYRATE", 1.0)
  reaper.GetSetMediaItemTakeInfo_String(take, "P_NAME", clip.id .. " — " .. clip.title, true)
  return item, nil
end

local function remove_old_regions(project)
  local _, marker_count, region_count = reaper.CountProjectMarkers(project)
  for enum_index = marker_count + region_count - 1, 0, -1 do
    local present, is_region, _, _, name, marker_id = reaper.EnumProjectMarkers3(project, enum_index)
    if present > 0 and is_region and name:sub(1, #REGION_PREFIX) == REGION_PREFIX then
      reaper.DeleteProjectMarker(project, marker_id, true)
    end
  end
end

local function write_attribution(media_directory)
  local path = path_join(media_directory, "ATTRIBUTION.txt")
  local file, error_message = io.open(path, "wb")
  if not file then return false, error_message end
  file:write(
    "Selected MIDI performances from MAESTRO Dataset v3.0.0\r\n" ..
    "https://magenta.withgoogle.com/datasets/maestro\r\n\r\n" ..
    "MAESTRO (MIDI and Audio Edited for Synchronous TRacks and Organization)\r\n" ..
    "contains virtuoso piano performances recorded on Yamaha Disklaviers at\r\n" ..
    "the International Piano-e-Competition. See the copied LICENSE file for\r\n" ..
    "the dataset's license and attribution terms.\r\n"
  )
  file:close()
  return true, nil
end

local function main()
  local project = 0
  local marker_present = select(1, reaper.GetProjExtState(project, PROJECT_SECTION, PIANO_COMPARE_KEY)) > 0
  if not marker_present then
    reaper.MB(
      "This is not a tagged Keyestra Piano Compare project.\n\nOpen the saved Piano Compare project and run the benchmark bootstrap again.",
      "Keyestra Piano Benchmark",
      0
    )
    return
  end
  local benchmark_present = select(1, reaper.GetProjExtState(project, PROJECT_SECTION, BENCHMARK_KEY)) > 0
  if not benchmark_present and reaper.IsProjectDirty(project) ~= 0 then
    reaper.MB(
      "Save the Piano Compare project first, then run this script again. This ensures the canonical live-playing project keeps the exact plug-in and calibration state used by the benchmark.",
      "Keyestra Piano Benchmark",
      0
    )
    return
  end

  local piano_tracks, piano_error = collect_piano_tracks(project)
  if not piano_tracks then
    reaper.MB(piano_error, "Keyestra Piano Benchmark", 0)
    return
  end
  local tempo_ok, tempo_error = validate_project_tempo(project)
  if not tempo_ok then
    reaper.MB(tempo_error, "Keyestra Piano Benchmark", 0)
    return
  end

  local selected, csv_path = reaper.GetUserFileNameForRead("", "Select maestro-v3.0.0.csv", "csv")
  if not selected then return end
  local dataset_root = csv_path:match("^(.*)[/\\][^/\\]+$")
  if not dataset_root then
    reaper.MB("Could not determine the MAESTRO dataset directory.", "Keyestra Piano Benchmark", 0)
    return
  end

  local missing = {}
  for _, clip in ipairs(CLIPS) do
    clip.source_path = path_join(dataset_root, clip.relative_path)
    if not file_exists(clip.source_path) then table.insert(missing, clip.relative_path) end
  end
  if #missing > 0 then
    reaper.MB(
      "The selected folder is missing required MAESTRO files:\n\n" .. table.concat(missing, "\n"),
      "Keyestra Piano Benchmark",
      0
    )
    return
  end

  local resource_path = reaper.GetResourcePath()
  local media_directory = path_join(path_join(path_join(resource_path, "Media"), "Keyestra"), "MAESTRO v3.0.0")
  reaper.RecursiveCreateDirectory(media_directory, 0)
  for _, clip in ipairs(CLIPS) do
    clip.copied_path = path_join(media_directory, clip.copied_name)
    local copied, copy_error = copy_file(clip.source_path, clip.copied_path)
    if not copied then
      reaper.MB("Could not copy " .. clip.source_path .. ":\n\n" .. tostring(copy_error), "Keyestra Piano Benchmark", 0)
      return
    end
  end
  for _, supporting_file in ipairs({ "LICENSE", "maestro-v3.0.0.csv" }) do
    local source_path = path_join(dataset_root, supporting_file)
    if file_exists(source_path) then
      local copied, copy_error = copy_file(source_path, path_join(media_directory, supporting_file))
      if not copied then
        reaper.MB("Could not copy " .. supporting_file .. ":\n\n" .. tostring(copy_error), "Keyestra Piano Benchmark", 0)
        return
      end
    end
  end
  local attributed, attribution_error = write_attribution(media_directory)
  if not attributed then
    reaper.MB("Could not write attribution: " .. tostring(attribution_error), "Keyestra Piano Benchmark", 0)
    return
  end

  reaper.Undo_BeginBlock2(project)
  reaper.PreventUIRefresh(1)
  reaper.SetCurrentBPM(project, PROJECT_BPM, true)
  local source_track = find_or_create_source_track(project)
  clear_source_track(source_track)
  configure_source_track(source_track)
  route_source_to_pianos(source_track, piano_tracks)
  remove_old_regions(project)

  local region_start = 0.0
  local first_region_end = nil
  for _, clip in ipairs(CLIPS) do
    add_controller_item(source_track, region_start, clip.initial_cc, clip.id .. " controller initialization")
    local item_start = region_start + SETUP_LEAD_SECONDS
    local item, item_error = add_external_midi_item(source_track, clip.copied_path, item_start, clip)
    if not item then
      reaper.PreventUIRefresh(-1)
      reaper.Undo_EndBlock2(project, "Create Keyestra Piano Benchmark", -1)
      reaper.MB(item_error, "Keyestra Piano Benchmark", 0)
      return
    end
    local item_end = item_start + clip.duration
    add_controller_item(source_track, item_end, {}, clip.id .. " pedal and note reset")
    local region_end = item_end + RELEASE_TAIL_SECONDS
    local color = reaper.ColorToNative(table.unpack(clip.color)) | 0x1000000
    reaper.AddProjectMarker2(project, true, region_start, region_end, REGION_PREFIX .. clip.id .. " — " .. clip.title, -1, color)
    if not first_region_end then first_region_end = region_end end
    region_start = region_end + BETWEEN_REGIONS_SECONDS
  end

  reaper.SetProjExtState(project, PROJECT_SECTION, BENCHMARK_KEY, "1")
  reaper.SetProjExtState(project, PROJECT_SECTION, "BENCHMARK_SCHEMA_VERSION", "1")
  reaper.GetSet_LoopTimeRange2(project, true, false, 0.0, first_region_end, false)
  reaper.SetEditCurPos2(project, 0.0, true, false)
  reaper.SetOnlyTrackSelected(source_track)
  reaper.PreventUIRefresh(-1)
  reaper.TrackList_AdjustWindows(false)
  reaper.UpdateArrange()
  reaper.Undo_EndBlock2(project, "Create Keyestra Piano Benchmark", -1)

  local project_directory = path_join(path_join(resource_path, "Projects"), "Keyestra")
  reaper.RecursiveCreateDirectory(project_directory, 0)
  local project_path = path_join(project_directory, "Keyestra Piano Benchmark.rpp")
  reaper.Main_SaveProjectEx(project, project_path, 8)

  reaper.MB(
    "Piano Benchmark is ready.\n\n" ..
    "Project: " .. project_path .. "\n" ..
    "Local media: " .. media_directory .. "\n" ..
    "Piano tracks routed: " .. #piano_tracks .. "\n\n" ..
    "Region A is selected as the loop range. Turn Repeat on and press Play. Use the phone's Piano A/B control during the four-second release tail so the newly selected piano receives the next pass from its beginning. " ..
    "The original Piano Compare project remains your live-playing project.",
    "Keyestra Piano Benchmark",
    0
  )
end

main()
