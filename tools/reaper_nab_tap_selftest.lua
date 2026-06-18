local result_path = "/tmp/nab_tap_reaper_selftest_result.txt"
local result = assert(io.open(result_path, "w"))
local duration_seconds = tonumber(os.getenv("NAB_TAP_SELFTEST_SECONDS") or "12") or 12

local function log(line)
  result:write(line .. "\n")
  result:flush()
end

reaper.GetSetProjectInfo(0, "PROJECT_SRATE", 96000, true)
reaper.GetSetProjectInfo(0, "PROJECT_SRATE_USE", 1, true)

local master = reaper.GetMasterTrack(0)
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
log("master_nab_tap_fx=" .. tostring(tap_fx))
log("added_tap=" .. tostring(added_tap))

reaper.InsertTrackAtIndex(0, true)
local track = reaper.GetTrack(0, 0)
reaper.GetSetMediaTrackInfo_String(track, "P_NAME", "NAB Tap Selftest Tone", true)
reaper.SetOnlyTrackSelected(track)

reaper.SetMediaTrackInfo_Value(track, "D_VOL", 0.1)
local inserted = reaper.InsertMedia("/tmp/nab_tap_selftest_tone.wav", 0)
log("inserted_audio_media=" .. tostring(inserted))
reaper.UpdateArrange()

reaper.GetSetProjectInfo(0, "RENDER_SETTINGS", 0, true)
reaper.GetSetProjectInfo(0, "RENDER_BOUNDSFLAG", 0, true)
reaper.GetSetProjectInfo(0, "RENDER_STARTPOS", 0.0, true)
reaper.GetSetProjectInfo(0, "RENDER_ENDPOS", duration_seconds, true)
reaper.GetSetProjectInfo(0, "RENDER_CHANNELS", 2, true)
reaper.GetSetProjectInfo(0, "RENDER_SRATE", 48000, true)
reaper.GetSetProjectInfo(0, "RENDER_TAILFLAG", 0, true)
reaper.GetSetProjectInfo(0, "RENDER_ADDTOPROJ", 0, true)
reaper.GetSetProjectInfo_String(0, "RENDER_FILE", "/tmp", true)
reaper.GetSetProjectInfo_String(0, "RENDER_PATTERN", "nab_tap_selftest_render", true)
reaper.GetSetProjectInfo_String(0, "RENDER_FORMAT", "evaw", true)
reaper.Main_SaveProjectEx(0, "/tmp/nab_tap_reaper_selftest.rpp", 8)
log("saved_project=/tmp/nab_tap_reaper_selftest.rpp")

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
