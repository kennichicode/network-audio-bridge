local result_path = "/tmp/nab_tap_reaper_selftest_result.txt"
local result = assert(io.open(result_path, "w"))

local function read_number(path)
  local file = io.open(path, "r")
  if not file then
    return nil
  end
  local value = tonumber(file:read("*a"))
  file:close()
  return value
end

local duration_seconds = read_number("/tmp/nab_tap_selftest_seconds.txt")
  or tonumber(os.getenv("NAB_TAP_SELFTEST_SECONDS") or "12")
  or 12
local track_volume = read_number("/tmp/nab_tap_selftest_volume.txt") or 0.1

local function log(line)
  result:write(line .. "\n")
  result:flush()
end

local old_project_srate = reaper.GetSetProjectInfo(0, "PROJECT_SRATE", 0, false)
local old_project_srate_use = reaper.GetSetProjectInfo(0, "PROJECT_SRATE_USE", 0, false)
log("old_project_srate=" .. tostring(old_project_srate))
log("old_project_srate_use=" .. tostring(old_project_srate_use))

reaper.GetSetProjectInfo(0, "PROJECT_SRATE", 96000, true)
reaper.GetSetProjectInfo(0, "PROJECT_SRATE_USE", 1, true)

local master = reaper.GetMasterTrack(0)
local old_master_volume = reaper.GetMediaTrackInfo_Value(master, "D_VOL")
log("old_master_volume=" .. tostring(old_master_volume))
reaper.SetMediaTrackInfo_Value(master, "D_VOL", 1.0)
log("selftest_master_volume=1.0")

local tap_fx = reaper.TrackFX_AddByName(master, "VST3: NAB Tap (Kenichi Kawabata)", false, 0)
local added_tap = false
if tap_fx < 0 then
  tap_fx = reaper.TrackFX_AddByName(master, "NAB Tap", false, 0)
end
if tap_fx < 0 then
  tap_fx = reaper.TrackFX_AddByName(master, "VST3: NAB Tap (Kenichi Kawabata)", false, -1)
  added_tap = tap_fx >= 0
end
if tap_fx < 0 then
  tap_fx = reaper.TrackFX_AddByName(master, "NAB Tap", false, -1)
  added_tap = tap_fx >= 0
end
if added_tap and tap_fx > 0 then
  reaper.TrackFX_CopyToTrack(master, tap_fx, master, 0, true)
  tap_fx = 0
  log("moved_added_tap_to_master_fx_0=1")
end
log("master_nab_tap_fx=" .. tostring(tap_fx))
log("added_tap=" .. tostring(added_tap))

reaper.InsertTrackAtIndex(0, true)
local track = reaper.GetTrack(0, 0)
reaper.GetSetMediaTrackInfo_String(track, "P_NAME", "NAB Tap Selftest Tone", true)
reaper.SetOnlyTrackSelected(track)

reaper.SetMediaTrackInfo_Value(track, "D_VOL", track_volume)
local inserted = reaper.InsertMedia("/tmp/nab_tap_selftest_tone.wav", 0)
log("inserted_audio_media=" .. tostring(inserted))
local item = reaper.GetSelectedMediaItem(0, 0)
if item then
  reaper.SetMediaItemInfo_Value(item, "B_LOOPSRC", 1)
  reaper.SetMediaItemInfo_Value(item, "D_LENGTH", duration_seconds)
  log("looped_item_length=" .. tostring(duration_seconds))
end
reaper.UpdateArrange()

reaper.OnPlayButton()
local started = reaper.time_precise()

local function finish()
  reaper.OnStopButton()
  log("transport_stopped=1")
  if track then
    reaper.DeleteTrack(track)
    log("deleted_test_track=1")
  end
  if added_tap and tap_fx >= 0 then
    reaper.TrackFX_Delete(master, tap_fx)
    log("deleted_added_tap=1")
  else
    log("left_existing_tap=1")
  end
  reaper.GetSetProjectInfo(0, "PROJECT_SRATE", old_project_srate, true)
  reaper.GetSetProjectInfo(0, "PROJECT_SRATE_USE", old_project_srate_use, true)
  reaper.SetMediaTrackInfo_Value(master, "D_VOL", old_master_volume)
  log("restored_master_volume=" .. tostring(old_master_volume))
  log("restored_project_srate=" .. tostring(old_project_srate))
  log("restored_project_srate_use=" .. tostring(old_project_srate_use))
  reaper.UpdateArrange()
  result:close()
end

local function tick()
  if reaper.time_precise() - started >= duration_seconds then
    finish()
    return
  end
  reaper.defer(tick)
end

tick()
