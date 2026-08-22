-- Keyestra Pianoteq single-preset test for REAPER 7.
--
-- This script deliberately changes only the currently open project's Pianoteq
-- instance. It does NOT write or overwrite a REAPER/Pianoteq named preset.
-- After listening and checking the result, use the FX window's preset menu to
-- save it as a NEW test preset. One Undo restores the complete prior state.
--
-- Install with Actions > Show action list > New action > Load ReaScript.

local project = select(1, reaper.EnumProjects(-1, ""))

local function lower(value)
  return (value or ""):lower()
end

local function find_track(name)
  local wanted = lower(name)
  for index = 0, reaper.CountTracks(project) - 1 do
    local track = reaper.GetTrack(project, index)
    local _, track_name = reaper.GetTrackName(track, "")
    if lower(track_name) == wanted then return track end
  end
  return nil
end

local function find_pianoteq_fx(track)
  local matches = {}
  for fx = 0, reaper.TrackFX_GetCount(track) - 1 do
    local _, name = reaper.TrackFX_GetFXName(track, fx, "")
    if lower(name):find("pianoteq", 1, true) then table.insert(matches, fx) end
  end
  if #matches == 1 then return matches[1] end
  return nil, matches
end

local function matching_parameters(track, fx, aliases)
  local exact, partial = {}, {}
  for parameter = 0, reaper.TrackFX_GetNumParams(track, fx) - 1 do
    local _, name = reaper.TrackFX_GetParamName(track, fx, parameter, "")
    local normalized_name = lower(name):gsub("%s+", " ")
    for _, alias in ipairs(aliases) do
      if normalized_name == alias then table.insert(exact, parameter); break end
      if normalized_name:find(alias, 1, true) then table.insert(partial, parameter); break end
    end
  end
  if #exact == 1 then return exact[1] end
  if #exact == 0 and #partial == 1 then return partial[1] end
  return nil, (#exact > 0 and exact or partial)
end

local function parameter_description(track, fx, parameter)
  local _, name = reaper.TrackFX_GetParamName(track, fx, parameter, "")
  local normalized = reaper.TrackFX_GetParamNormalized(track, fx, parameter)
  local _, formatted = reaper.TrackFX_GetFormattedParamValue(track, fx, parameter, "")
  return string.format("%s = %.6f (%s)", name, normalized, formatted)
end

local function describe_candidates(track, fx, candidates)
  local values = {}
  for _, parameter in ipairs(candidates or {}) do
    table.insert(values, parameter_description(track, fx, parameter))
  end
  return #values > 0 and table.concat(values, "\n") or "none"
end

local function valid_normalized(text)
  local value = tonumber(text)
  return value and value >= 0 and value <= 1 and value or nil
end

-- This list has no side effects and lets the script use the exact spelling
-- exposed by the installed Pianoteq VST/VST3 version.
local function report_parameter_names(track, fx)
  local lines = { "Pianoteq parameters on this system:" }
  for parameter = 0, reaper.TrackFX_GetNumParams(track, fx) - 1 do
    local _, name = reaper.TrackFX_GetParamName(track, fx, parameter, "")
    if lower(name):find("dynamic", 1, true) or lower(name):find("volume", 1, true) then
      table.insert(lines, string.format("%d: %s", parameter, parameter_description(track, fx, parameter)))
    end
  end
  reaper.ShowConsoleMsg(table.concat(lines, "\n") .. "\n")
end

local ok, values = reaper.GetUserInputs(
  "Pianoteq 单个 Preset 试验",
  4,
  "轨道名称,源 REAPER FX preset（可留空）,Dynamics（0..1）,Volume（0..1）",
  "Pianoteq Live,,,"
)
if not ok then return end

local track_name, source_preset, dynamics_text, volume_text = values:match("^(.-),(.-),(.-),(.-)$")
local dynamics, volume = valid_normalized(dynamics_text), valid_normalized(volume_text)
if not dynamics or not volume then
  reaper.MB("Dynamics 和 Volume 必须是 0 到 1 的数值。\n没有做任何修改。", "Pianoteq 单个 Preset 试验", 0)
  return
end

local track = find_track(track_name)
if not track then
  reaper.MB("找不到轨道：" .. track_name .. "\n没有做任何修改。", "Pianoteq 单个 Preset 试验", 0)
  return
end

local fx, fx_candidates = find_pianoteq_fx(track)
if not fx then
  reaper.MB("该轨道上必须恰好有一个名称含 Pianoteq 的 FX。\n匹配数：" .. #(fx_candidates or {}), "Pianoteq 单个 Preset 试验", 0)
  return
end

report_parameter_names(track, fx)
local dynamics_parameter, dynamics_candidates = matching_parameters(track, fx, { "dynamics", "dynamic range" })
local volume_parameter, volume_candidates = matching_parameters(track, fx, { "volume", "master volume" })
if not dynamics_parameter or not volume_parameter then
  reaper.MB(
    "无法唯一识别 Dynamics 或 Volume；没有做任何修改。\n\nDynamics 候选：\n"
      .. describe_candidates(track, fx, dynamics_candidates)
      .. "\n\nVolume 候选：\n" .. describe_candidates(track, fx, volume_candidates)
      .. "\n\n完整诊断已写到 View > Show console output。",
    "Pianoteq 单个 Preset 试验", 0
  )
  return
end

reaper.Undo_BeginBlock2(project)
reaper.PreventUIRefresh(1)
local success = true
if source_preset ~= "" then success = reaper.TrackFX_SetPreset(track, fx, source_preset) end
if success then
  reaper.TrackFX_SetParamNormalized(track, fx, dynamics_parameter, dynamics)
  reaper.TrackFX_SetParamNormalized(track, fx, volume_parameter, volume)
end
reaper.PreventUIRefresh(-1)
reaper.TrackList_AdjustWindows(false)
reaper.UpdateArrange()

if not success then
  reaper.Undo_EndBlock2(project, "未找到 Pianoteq 源 preset（未修改）", -1)
  reaper.MB("REAPER FX preset 不存在：" .. source_preset .. "\n没有保留修改。", "Pianoteq 单个 Preset 试验", 0)
  return
end

reaper.Undo_EndBlock2(project, "Pianoteq 单个 preset：Dynamics + Volume", -1)
reaper.MB(
  "已在当前项目的 Pianoteq 上修改：\n\n"
    .. parameter_description(track, fx, dynamics_parameter)
    .. "\n" .. parameter_description(track, fx, volume_parameter)
    .. "\n\n请试听并在 FX 窗口的 preset 菜单选择“Save preset as...”，另存为新的 TEST 预设。\n"
    .. "若不满意，按 Ctrl+Z 即可完整还原。",
  "Pianoteq 单个 Preset 试验", 0
)
