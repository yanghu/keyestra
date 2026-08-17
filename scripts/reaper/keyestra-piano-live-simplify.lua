-- Simplify the saved Keyestra Piano Live project to one CFX track and one
-- reusable Pianoteq VST track. The comparison project is deliberately untouched.

local project, project_path = reaper.EnumProjects(-1, "")
local expected_name = "Keyestra Piano Live.rpp"

if project_path:sub(-#expected_name):lower() ~= expected_name:lower() then
  reaper.MB(
    "Open " .. expected_name .. " before running this script.\n\nCurrent project:\n" .. project_path,
    "Keyestra Piano Live",
    0
  )
  return
end

local function track_by_name(name)
  for index = 0, reaper.CountTracks(project) - 1 do
    local track = reaper.GetTrack(project, index)
    local _, track_name = reaper.GetTrackName(track)
    if track_name:lower() == name:lower() then return track end
  end
  return nil
end

local function export_cfx_parameters(track)
  local appdata = os.getenv("APPDATA") or reaper.GetResourcePath()
  local directory = appdata .. "\\keyestra"
  reaper.RecursiveCreateDirectory(directory, 0)
  local file = io.open(directory .. "\\cfx-vst-parameters.txt", "wb")
  if not file then return end
  local fx_count = reaper.TrackFX_GetCount(track)
  for fx = 0, fx_count - 1 do
    local _, fx_name = reaper.TrackFX_GetFXName(track, fx, "")
    file:write(string.format("FX %d\t%s\n", fx + 1, fx_name))
    for parameter = 0, reaper.TrackFX_GetNumParams(track, fx) - 1 do
      local _, parameter_name = reaper.TrackFX_GetParamName(track, fx, parameter, "")
      local _, formatted = reaper.TrackFX_GetFormattedParamValue(track, fx, parameter, "")
      local value, minimum, maximum = reaper.TrackFX_GetParam(track, fx, parameter)
      file:write(string.format(
        "%d\t%s\t%.9f\t%.9f\t%.9f\t%s\n",
        parameter,
        parameter_name,
        value,
        minimum,
        maximum,
        formatted
      ))
    end
  end
  file:close()
end

local cfx = track_by_name("Garritan CFX")
local pianoteq = track_by_name("Pianoteq SK-EX") or track_by_name("Pianoteq Live")
if not cfx or not pianoteq then
  reaper.MB(
    "The Live project needs both 'Garritan CFX' and 'Pianoteq SK-EX' (or 'Pianoteq Live'). No changes were made.",
    "Keyestra Piano Live",
    0
  )
  return
end

reaper.Undo_BeginBlock2(project)
reaper.PreventUIRefresh(1)

for _, name in ipairs({ "Pianoteq Steinway D", "Pianoteq Bösendorfer 280VC" }) do
  local track = track_by_name(name)
  if track then reaper.DeleteTrack(track) end
end

reaper.GetSetMediaTrackInfo_String(pianoteq, "P_NAME", "Pianoteq Live", true)
reaper.SetMediaTrackInfo_Value(cfx, "B_MUTE", 1)
reaper.SetMediaTrackInfo_Value(pianoteq, "B_MUTE", 0)
reaper.SetOnlyTrackSelected(pianoteq)
export_cfx_parameters(cfx)
reaper.TrackList_AdjustWindows(false)
reaper.UpdateArrange()
reaper.PreventUIRefresh(-1)
reaper.Undo_EndBlock2(project, "Simplify Keyestra Piano Live", -1)
reaper.Main_SaveProject(project, false)

reaper.ShowConsoleMsg(
  "Keyestra Piano Live now contains one Garritan CFX track and one reusable Pianoteq Live track.\n"
)
