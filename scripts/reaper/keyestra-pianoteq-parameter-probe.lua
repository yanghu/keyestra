-- Read-only Pianoteq VST parameter probe for REAPER 7.
-- It does not open the plug-in UI and does not change the project or FX state.
-- Run it, then open View > Show console output and copy the result.

local project = select(1, reaper.EnumProjects(-1, ""))
local ok, track_name = reaper.GetUserInputs(
  "Pianoteq VST 参数探测（只读）", 1, "轨道名称", "Pianoteq Live"
)
if not ok then return end

local track
for index = 0, reaper.CountTracks(project) - 1 do
  local candidate = reaper.GetTrack(project, index)
  local _, name = reaper.GetTrackName(candidate, "")
  if name:lower() == track_name:lower() then track = candidate; break end
end
if not track then
  reaper.MB("找不到轨道：" .. track_name, "Pianoteq VST 参数探测", 0)
  return
end

local fx_matches = {}
for fx = 0, reaper.TrackFX_GetCount(track) - 1 do
  local _, name = reaper.TrackFX_GetFXName(track, fx, "")
  if name:lower():find("pianoteq", 1, true) then table.insert(fx_matches, fx) end
end
if #fx_matches ~= 1 then
  reaper.MB("该轨道上必须恰好有一个名称含 Pianoteq 的 FX。匹配数：" .. #fx_matches, "Pianoteq VST 参数探测", 0)
  return
end

local fx = fx_matches[1]
local lines = { "Pianoteq exposed REAPER/VST parameters:" }
for parameter = 0, reaper.TrackFX_GetNumParams(track, fx) - 1 do
  local _, name = reaper.TrackFX_GetParamName(track, fx, parameter, "")
  local lower = name:lower()
  if lower:find("freeze", 1, true)
    or lower:find("dynamic", 1, true)
    or lower:find("volume", 1, true) then
    local normalized = reaper.TrackFX_GetParamNormalized(track, fx, parameter)
    local _, formatted = reaper.TrackFX_GetFormattedParamValue(track, fx, parameter, "")
    table.insert(lines, string.format("%d\t%s\t%.6f\t%s", parameter, name, normalized, formatted))
  end
end
if #lines == 1 then table.insert(lines, "(No matching parameters were exposed.)") end
reaper.ShowConsoleMsg(table.concat(lines, "\n") .. "\n")
reaper.MB("探测完成，没有修改任何设置。\n请打开 View > Show console output，复制结果。", "Pianoteq VST 参数探测", 0)
