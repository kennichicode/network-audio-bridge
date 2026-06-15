local log_path = "/tmp/nab_tap_reload.log"

local function log(line)
  local file = io.open(log_path, "a")
  if file then
    file:write(os.date("%Y-%m-%d %H:%M:%S"), " ", line, "\n")
    file:close()
  end
end

local master = reaper.GetMasterTrack(0)
if not master then
  log("error=no_master_track")
  return
end

local before = reaper.TrackFX_GetCount(master)
log("before_master_fx_count=" .. tostring(before))

for i = before - 1, 0, -1 do
  local ok, name = reaper.TrackFX_GetFXName(master, i, "")
  local lower = ok and string.lower(name) or ""
  if string.find(lower, "nab tap", 1, true) or string.find(lower, "nab_tap", 1, true) then
    log("deleting_fx_index=" .. tostring(i) .. " name=" .. name)
    reaper.TrackFX_Delete(master, i)
  end
end

local idx = reaper.TrackFX_AddByName(master, "VST3: NAB Tap (Kenichi Kawabata)", false, -1)
log("added_fx_index=" .. tostring(idx))

if idx >= 0 then
  reaper.TrackFX_SetEnabled(master, idx, true)
  reaper.TrackFX_SetOffline(master, idx, false)
  local _, added_name = reaper.TrackFX_GetFXName(master, idx, "")
  log("added_fx_name=" .. added_name)
  log("added_fx_offline=" .. tostring(reaper.TrackFX_GetOffline(master, idx)))
  reaper.TrackFX_Show(master, idx, 1)
  reaper.TrackFX_Show(master, idx, 3)
else
  log("error=add_failed")
end

reaper.UpdateArrange()
