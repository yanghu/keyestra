-- Keyestra Piano Compare bootstrap for REAPER 7.
--
-- Run this once from REAPER's Actions > Show action list > New action > Load
-- ReaScript. It creates or updates a tagged Piano Compare project, saves a
-- canonical project and project template, and writes Keyestra's piano config
-- only when that config does not already exist.

local _, current_project_path = reaper.EnumProjects(-1, "")
if current_project_path:lower():match("keyestra piano live%.rpp$") then
  local source = debug.getinfo(1, "S").source:sub(2)
  local directory = source:match("^(.*[\\/])") or ""
  dofile(directory .. "keyestra-piano-live-simplify.lua")
  return
end

local PROJECT_SECTION = "KEYESTRA"
local PROJECT_MARKER_KEY = "PIANO_COMPARE"
local TRACK_ID_KEY = "P_EXT:KEYESTRA_PIANO_ID"
local MIDI_INPUT_NAMES = { "Keyestra MIDI", "FP10 Mapped" }

-- Preset names must exactly match REAPER's FX preset dropdown. Leave them
-- empty until the desired third-party state has been saved as a host preset.
local PIANOS = {
  {
    id = "cfx",
    name = "Garritan CFX",
    active = true,
    fx_candidates = {
      "VST: CFX Concert Grand (Garritan)",
      "CFX Concert Grand (Garritan)",
      "VST: CFX Lite (Garritan)",
      "CFX Lite (Garritan)",
    },
    preset = "",
    color = { 194, 111, 45 },
  },
  {
    id = "sk-ex",
    name = "Pianoteq SK-EX",
    active = false,
    fx_candidates = {
      "VST3: Pianoteq 9 (Modartt)",
      "VST3: Pianoteq 9",
      "VST3: Pianoteq (Modartt)",
      "VST3: Pianoteq",
      "VST: Pianoteq 9 (Modartt)",
      "VST: Pianoteq 9",
    },
    preset = "",
    color = { 47, 128, 124 },
  },
  {
    id = "steinway-d",
    name = "Pianoteq Steinway D",
    active = false,
    fx_candidates = {
      "VST3: Pianoteq 9 (Modartt)",
      "VST3: Pianoteq 9",
      "VST3: Pianoteq (Modartt)",
      "VST3: Pianoteq",
      "VST: Pianoteq 9 (Modartt)",
      "VST: Pianoteq 9",
    },
    preset = "",
    color = { 71, 104, 160 },
  },
  {
    id = "280vc",
    name = "Pianoteq Bösendorfer 280VC",
    active = false,
    fx_candidates = {
      "VST3: Pianoteq 9 (Modartt)",
      "VST3: Pianoteq 9",
      "VST3: Pianoteq (Modartt)",
      "VST3: Pianoteq",
      "VST: Pianoteq 9 (Modartt)",
      "VST: Pianoteq 9",
    },
    preset = "",
    color = { 117, 83, 145 },
  },
}

local function path_join(left, right)
  local separator = package.config:sub(1, 1)
  return left .. separator .. right
end

local function find_midi_input()
  local fallback_index = nil
  local fallback_name = nil
  for index = 0, reaper.GetMaxMidiInputs() - 1 do
    local present, name = reaper.GetMIDIInputName(index, "")
    if present then
      for priority, desired in ipairs(MIDI_INPUT_NAMES) do
        if name:lower() == desired:lower() then
          if priority == 1 then
            return index, name
          end
          fallback_index = index
          fallback_name = name
        end
      end
    end
  end
  return fallback_index, fallback_name
end

local function track_id(track)
  local _, value = reaper.GetSetMediaTrackInfo_String(track, TRACK_ID_KEY, "", false)
  return value
end

local function track_name(track)
  local _, value = reaper.GetSetMediaTrackInfo_String(track, "P_NAME", "", false)
  return value
end

local function find_track(project, piano)
  local matching_name = nil
  local matching_name_count = 0
  for index = 0, reaper.CountTracks(project) - 1 do
    local track = reaper.GetTrack(project, index)
    if track_id(track) == piano.id then
      return track, false
    end
    if track_name(track):lower() == piano.name:lower() then
      matching_name = track
      matching_name_count = matching_name_count + 1
    end
  end
  if matching_name_count == 1 then
    return matching_name, false
  end
  reaper.InsertTrackInProject(project, reaper.CountTracks(project), 0)
  return reaper.GetTrack(project, reaper.CountTracks(project) - 1), true
end

local function find_or_add_fx(track, candidates)
  for _, candidate in ipairs(candidates) do
    local index = reaper.TrackFX_AddByName(track, candidate, false, 0)
    if index >= 0 then
      return index, candidate, false
    end
  end
  for _, candidate in ipairs(candidates) do
    local index = reaper.TrackFX_AddByName(track, candidate, false, 1)
    if index >= 0 then
      return index, candidate, true
    end
  end
  return -1, nil, false
end

local function configure_track(track, piano, midi_index)
  reaper.GetSetMediaTrackInfo_String(track, "P_NAME", piano.name, true)
  reaper.GetSetMediaTrackInfo_String(track, TRACK_ID_KEY, piano.id, true)
  reaper.SetMediaTrackInfo_Value(track, "I_RECINPUT", 4096 + (midi_index << 5))
  reaper.SetMediaTrackInfo_Value(track, "I_RECARM", 1)
  reaper.SetMediaTrackInfo_Value(track, "I_RECMON", 1)
  reaper.SetMediaTrackInfo_Value(track, "I_RECMODE", 0)
  reaper.SetMediaTrackInfo_Value(track, "B_MAINSEND", 1)
  reaper.SetMediaTrackInfo_Value(track, "I_MIDIHWOUT", -1)
  reaper.SetMediaTrackInfo_Value(track, "I_FXEN", 1)
  reaper.SetMediaTrackInfo_Value(track, "B_MUTE", piano.active and 0 or 1)
  reaper.SetTrackSelected(track, false)

  -- Disable anticipative FX for these live-monitored instruments while
  -- preserving other performance flags.
  local performance_flags = math.floor(reaper.GetMediaTrackInfo_Value(track, "I_PERFFLAGS"))
  reaper.SetMediaTrackInfo_Value(track, "I_PERFFLAGS", performance_flags | 2)
  reaper.SetTrackColor(track, reaper.ColorToNative(table.unpack(piano.color)) | 0x1000000)

  local fx_index, fx_name, added = find_or_add_fx(track, piano.fx_candidates)
  local preset_loaded = false
  if fx_index >= 0 then
    reaper.TrackFX_SetEnabled(track, fx_index, true)
    if piano.preset ~= "" then
      preset_loaded = reaper.TrackFX_SetPreset(track, fx_index, piano.preset)
    end
  end
  return fx_index, fx_name, added, preset_loaded
end

local function write_keyestra_config_if_missing(config_path)
  local existing = io.open(config_path, "rb")
  if existing then
    existing:close()
    return false, nil
  end

  local file, error_message = io.open(config_path, "wb")
  if not file then
    return false, error_message
  end
  file:write([=[
[reaper]
address = "127.0.0.1:8080"
timeout_ms = 800

[pianoteq_vst]
piano_id = "sk-ex"
track = "Pianoteq SK-EX"
fx = 1
osc_address = "127.0.0.1:9000"

[[piano]]
id = "cfx"
name = "Garritan CFX"
tracks = ["Garritan CFX"]

[[piano]]
id = "sk-ex"
name = "Pianoteq SK-EX"
tracks = ["Pianoteq SK-EX"]

[[piano]]
id = "steinway-d"
name = "Pianoteq Steinway D"
tracks = ["Pianoteq Steinway D"]

[[piano]]
id = "280vc"
name = "Pianoteq Bösendorfer 280VC"
tracks = ["Pianoteq Bösendorfer 280VC"]
]=])
  file:close()
  return true, nil
end

local function main()
  local project = 0
  local marker_present = select(1, reaper.GetProjExtState(project, PROJECT_SECTION, PROJECT_MARKER_KEY)) > 0
  if reaper.CountTracks(project) > 0 and not marker_present then
    reaper.MB(
      "This project is not tagged as a Keyestra Piano Compare project.\n\n" ..
      "Open a new empty project tab and run the bootstrap again. Existing tracks were not changed.",
      "Keyestra Piano Compare",
      0
    )
    return
  end

  local midi_index, midi_name = find_midi_input()
  if not midi_index then
    reaper.MB(
      "No Keyestra MIDI input was found. Create/enable 'Keyestra MIDI' in loopMIDI, then run this script again.",
      "Keyestra Piano Compare",
      0
    )
    return
  end

  local missing_fx = {}
  local preset_failures = {}
  local created_count = 0
  local added_fx_count = 0

  reaper.Undo_BeginBlock2(project)
  reaper.PreventUIRefresh(1)
  for _, piano in ipairs(PIANOS) do
    local track, created = find_track(project, piano)
    if created then created_count = created_count + 1 end
    local fx_index, _, added, preset_loaded = configure_track(track, piano, midi_index)
    if fx_index < 0 then
      table.insert(missing_fx, piano.name)
    elseif added then
      added_fx_count = added_fx_count + 1
    end
    if piano.preset ~= "" and not preset_loaded then
      table.insert(preset_failures, piano.name .. " → " .. piano.preset)
    end
  end
  reaper.SetProjExtState(project, PROJECT_SECTION, PROJECT_MARKER_KEY, "1")
  reaper.SetProjExtState(project, PROJECT_SECTION, "SCHEMA_VERSION", "1")
  reaper.SetProjExtState(project, PROJECT_SECTION, "MIDI_INPUT", midi_name)
  reaper.PreventUIRefresh(-1)
  reaper.TrackList_AdjustWindows(false)
  reaper.UpdateArrange()
  reaper.Undo_EndBlock2(project, "Bootstrap Keyestra Piano Compare", -1)

  local resource_path = reaper.GetResourcePath()
  local project_directory = path_join(path_join(resource_path, "Projects"), "Keyestra")
  local template_directory = path_join(resource_path, "ProjectTemplates")
  reaper.RecursiveCreateDirectory(project_directory, 0)
  reaper.RecursiveCreateDirectory(template_directory, 0)
  local project_path = path_join(project_directory, "Keyestra Piano Compare.rpp")
  local template_path = path_join(template_directory, "Keyestra Piano Compare.RPP")
  reaper.Main_SaveProjectEx(project, project_path, 8)
  reaper.Main_SaveProjectEx(project, template_path, 0)

  local appdata = os.getenv("APPDATA") or resource_path
  local keyestra_directory = path_join(appdata, "keyestra")
  reaper.RecursiveCreateDirectory(keyestra_directory, 0)
  local config_path = path_join(keyestra_directory, "reaper-pianos.toml")
  local config_written, config_error = write_keyestra_config_if_missing(config_path)

  local lines = {
    "Piano Compare bootstrap finished.",
    "",
    "MIDI input: " .. midi_name,
    "Tracks created: " .. created_count,
    "Plug-ins added: " .. added_fx_count,
    "Project: " .. project_path,
    "Template: " .. template_path,
    config_written and ("Keyestra config created: " .. config_path)
      or ("Keyestra config preserved: " .. config_path),
  }
  if config_error then
    table.insert(lines, "Keyestra config error: " .. config_error)
  end
  if #missing_fx > 0 then
    table.insert(lines, "")
    table.insert(lines, "Plug-in not found (track is ready; install/rescan, then rerun):")
    for _, name in ipairs(missing_fx) do table.insert(lines, "• " .. name) end
  end
  if #preset_failures > 0 then
    table.insert(lines, "")
    table.insert(lines, "REAPER preset not found:")
    for _, name in ipairs(preset_failures) do table.insert(lines, "• " .. name) end
  end
  table.insert(lines, "")
  table.insert(lines, "Open each third-party plug-in once to confirm licensing, sample paths, and its internal sound state, then save the project again.")
  reaper.MB(table.concat(lines, "\n"), "Keyestra Piano Compare", 0)
end

main()
