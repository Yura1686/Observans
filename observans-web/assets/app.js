const params = new URLSearchParams(location.search);
const streamSource = document.getElementById("stream-source");
const streamFrame = document.getElementById("stream-frame");
const placeholder = document.getElementById("stream-placeholder");
const stage = document.getElementById("stream-stage");
const stageCtx = stage.getContext("2d", { alpha: false, desynchronized: true });
const statusLine = document.getElementById("status-line");
const fullscreenBtn = document.getElementById("fullscreen-btn");
const recordingDot = document.getElementById("recording-dot");
const recordingState = document.getElementById("recording-state");
const recordingTime = document.getElementById("recording-time");
const recordBtn = document.getElementById("record-btn");
const stopBtn = document.getElementById("stop-btn");
const saveBtn = document.getElementById("save-btn");
const backendPill = document.getElementById("backend-pill");
const frameSizePill = document.getElementById("frame-size-pill");
const restartPill = document.getElementById("restart-pill");

let streamAlive = false;
let lastMetricsOk = 0;
let reconnectTimer = null;
let renderLoopId = 0;
let lastMetrics = null;
let mediaRecorder = null;
let recordedChunks = [];
let recordedBlob = null;
let recordedUrl = null;
let recordedMimeType = "video/webm";
let recordStartedAt = 0;
let recordTicker = null;

function withTs(path) {
  const query = new URLSearchParams(params);
  query.set("t", Date.now().toString());
  const encoded = query.toString();
  return encoded ? `${path}?${encoded}` : path;
}

function clamp(value, min, max) {
  return Math.max(min, Math.min(max, value));
}

function setStatus(label, ok = null) {
  statusLine.textContent = label;
  statusLine.classList.remove("good", "bad");
  if (ok === true) statusLine.classList.add("good");
  if (ok === false) statusLine.classList.add("bad");
}

function setLiveState(live) {
  streamAlive = live;
  streamFrame.classList.toggle("is-live", live);
  placeholder.setAttribute("aria-hidden", live ? "true" : "false");
}

function connectStream() {
  if (reconnectTimer) {
    clearTimeout(reconnectTimer);
    reconnectTimer = null;
  }

  setLiveState(false);
  setStatus("CONNECTING", false);
  streamSource.src = withTs("/stream");
  updateRecordingUi();
}

function ensureStageSize() {
  const nextWidth = streamSource.naturalWidth || 1280;
  const nextHeight = streamSource.naturalHeight || 720;
  if (stage.width !== nextWidth || stage.height !== nextHeight) {
    stage.width = nextWidth;
    stage.height = nextHeight;
  }
}

function renderFrame() {
  renderLoopId = requestAnimationFrame(renderFrame);
  if (!stageCtx || !streamSource.complete || streamSource.naturalWidth === 0) return;
  ensureStageSize();
  stageCtx.drawImage(streamSource, 0, 0, stage.width, stage.height);
}

function startRenderLoop() {
  if (renderLoopId) return;
  renderLoopId = requestAnimationFrame(renderFrame);
}

function stopRenderLoop() {
  if (!renderLoopId) return;
  cancelAnimationFrame(renderLoopId);
  renderLoopId = 0;
}

function setBarFill(id, percent) {
  const fill = document.getElementById(id);
  if (!fill) return;
  fill.style.width = `${clamp(percent, 0, 100)}%`;
}

function setBatteryFill(percent) {
  const fill = document.getElementById("battery-fill");
  fill.style.height = `${clamp(percent, 0, 100)}%`;
}

function formatDuration(ms) {
  const totalSeconds = Math.max(0, Math.floor(ms / 1000));
  const minutes = String(Math.floor(totalSeconds / 60)).padStart(2, "0");
  const seconds = String(totalSeconds % 60).padStart(2, "0");
  return `${minutes}:${seconds}`;
}

function updateRecordingClock() {
  if (!recordStartedAt) {
    recordingTime.textContent = "00:00";
    return;
  }

  recordingTime.textContent = formatDuration(Date.now() - recordStartedAt);
}

function pickRecordingMimeType() {
  if (!window.MediaRecorder || !MediaRecorder.isTypeSupported) {
    return "";
  }

  const candidates = ["video/webm;codecs=vp9", "video/webm;codecs=vp8", "video/webm"];
  for (const candidate of candidates) {
    if (MediaRecorder.isTypeSupported(candidate)) {
      return candidate;
    }
  }

  return "";
}

function clearRecordedClip() {
  recordedBlob = null;
  recordedChunks = [];
  if (recordedUrl) {
    URL.revokeObjectURL(recordedUrl);
    recordedUrl = null;
  }
}

function updateRecordingUi() {
  const isRecording = mediaRecorder && mediaRecorder.state !== "inactive";
  const canRecord = streamAlive && stage.width > 0 && stage.height > 0 && !!window.MediaRecorder && !!stage.captureStream;

  recordBtn.disabled = isRecording || !canRecord;
  stopBtn.disabled = !isRecording;
  saveBtn.disabled = isRecording || !recordedBlob;

  recordBtn.classList.toggle("is-hidden", isRecording);
  stopBtn.classList.toggle("is-hidden", !isRecording);
  recordingDot.classList.toggle("live", isRecording);
  recordingState.textContent = isRecording
    ? "recording local clip"
    : (recordedBlob ? "clip ready to save" : "ready to capture");
}

function startRecording() {
  if (!window.MediaRecorder || !stage.captureStream || !stageCtx) {
    recordingState.textContent = "recording unsupported";
    setStatus("RECORDING UNSUPPORTED", false);
    return;
  }

  if (!streamAlive || stage.width === 0 || stage.height === 0) {
    recordingState.textContent = "waiting for live stream";
    setStatus("WAIT FOR LIVE STREAM", false);
    return;
  }

  clearRecordedClip();
  const preferredMimeType = pickRecordingMimeType();
  recordedMimeType = preferredMimeType || "video/webm";
  const targetFps = Math.max(12, Math.min(60, Math.round((lastMetrics && lastMetrics.fps_actual) || (lastMetrics && lastMetrics.fps_target) || 24)));
  const recordStream = stage.captureStream(targetFps);
  const options = preferredMimeType
    ? { mimeType: preferredMimeType, videoBitsPerSecond: 6000000 }
    : { videoBitsPerSecond: 6000000 };

  try {
    mediaRecorder = new MediaRecorder(recordStream, options);
  } catch (error) {
    recordingState.textContent = "recorder init failed";
    setStatus("COULD NOT START RECORDING", false);
    return;
  }

  mediaRecorder.ondataavailable = (event) => {
    if (event.data && event.data.size > 0) {
      recordedChunks.push(event.data);
    }
  };

  mediaRecorder.onstop = () => {
    recordedBlob = new Blob(recordedChunks, { type: mediaRecorder.mimeType || recordedMimeType });
    recordedMimeType = mediaRecorder.mimeType || recordedMimeType;
    recordStartedAt = 0;
    if (recordTicker) {
      clearInterval(recordTicker);
      recordTicker = null;
    }
    recordingTime.textContent = "00:00";
    setStatus("CLIP READY", true);
    updateRecordingUi();
    const tracks = mediaRecorder.stream ? mediaRecorder.stream.getTracks() : [];
    tracks.forEach((track) => track.stop());
  };

  mediaRecorder.start(1000);
  recordStartedAt = Date.now();
  updateRecordingClock();
  recordTicker = setInterval(updateRecordingClock, 250);
  setStatus("RECORDING", true);
  updateRecordingUi();
}

function stopRecording() {
  if (!mediaRecorder || mediaRecorder.state === "inactive") return;
  mediaRecorder.stop();
}

function saveRecording() {
  if (!recordedBlob) return;

  if (recordedUrl) {
    URL.revokeObjectURL(recordedUrl);
  }

  recordedUrl = URL.createObjectURL(recordedBlob);
  const link = document.createElement("a");
  const stamp = new Date().toISOString().replace(/[:T]/g, "-").slice(0, 19);
  const extension = recordedMimeType.includes("mp4") ? "mp4" : "webm";
  link.href = recordedUrl;
  link.download = `observans-${stamp}.${extension}`;
  document.body.appendChild(link);
  link.click();
  document.body.removeChild(link);
  setStatus("CLIP SAVED", true);
}

function titleCase(value) {
  return String(value || "--")
    .replace(/[_-]+/g, " ")
    .split(/\s+/)
    .filter(Boolean)
    .map((part) => part.charAt(0).toUpperCase() + part.slice(1))
    .join(" ");
}

function formatTemperature(temp) {
  if (temp < 0) return "N/A";
  return `${temp.toFixed(1)}°C`;
}

function thermalState(temp) {
  if (temp < 0) return "sensor unavailable";
  if (temp >= 80) return "thermal load high";
  if (temp >= 65) return "thermal load elevated";
  return "thermal envelope stable";
}

function toggleFullscreen() {
  if (!document.fullscreenElement) {
    streamFrame.requestFullscreen?.().catch(() => {});
  } else if (document.fullscreenElement === streamFrame) {
    document.exitFullscreen?.().catch(() => {});
  }
}

function syncFullscreenUi() {
  const active = document.fullscreenElement === streamFrame;
  document.body.classList.toggle("viewer-fullscreen", active);
  fullscreenBtn.setAttribute("aria-label", active ? "Exit fullscreen" : "Enter fullscreen");
}

streamSource.onload = () => {
  setLiveState(true);
  setStatus("LIVE", true);
  startRenderLoop();
  updateRecordingUi();
};

streamSource.onerror = () => {
  setLiveState(false);
  stopRenderLoop();
  setStatus("RECONNECTING", false);
  updateRecordingUi();
  reconnectTimer = setTimeout(connectStream, 1500);
};

recordBtn.addEventListener("click", startRecording);
stopBtn.addEventListener("click", stopRecording);
saveBtn.addEventListener("click", saveRecording);
fullscreenBtn.addEventListener("click", toggleFullscreen);
document.addEventListener("fullscreenchange", syncFullscreenUi);

async function tick() {
  try {
    const response = await fetch(withTs("/metrics"), { cache: "no-store" });
    if (!response.ok) throw new Error("metrics failed");

    const metrics = await response.json();
    lastMetrics = metrics;
    lastMetricsOk = Date.now();

    const cpuPct = clamp(metrics.cpu || 0, 0, 100);
    const ramPct = clamp(metrics.ram_pct || 0, 0, 100);
    const tempAvailable = metrics.temp >= 0;
    const battAvailable = metrics.batt >= 0;
    const tempPct = tempAvailable ? clamp(metrics.temp, 0, 100) : 0;
    const frameAgeText = metrics.frame_age_ms >= 0 ? `${metrics.frame_age_ms} ms` : "--";
    const frameSizeText = metrics.avg_frame_kb > 0 ? `${metrics.avg_frame_kb.toFixed(1)} KB` : "--";
    const liveFpsText = `${metrics.fps_actual.toFixed(1)} / ${metrics.fps_target} fps`;
    const batteryText = battAvailable ? `${metrics.batt}%` : "N/A";
    const batteryStatus = battAvailable ? titleCase(metrics.batt_status || "unknown") : "unavailable";

    document.getElementById("clock").textContent = metrics.time;
    document.getElementById("date").textContent = metrics.date;
    document.getElementById("cpu").textContent = `${cpuPct.toFixed(1)}%`;
    document.getElementById("ram").textContent = `${ramPct.toFixed(1)}%`;
    document.getElementById("ram-sub").textContent = `${metrics.ram_used_mb} / ${metrics.ram_total_mb} MB`;
    document.getElementById("temp").textContent = formatTemperature(metrics.temp);
    document.getElementById("temp-sub").textContent = thermalState(metrics.temp);
    document.getElementById("batt").textContent = batteryText;
    document.getElementById("batt-sub").textContent = batteryStatus;
    document.getElementById("host").textContent = metrics.hostname;
    document.getElementById("host-sub").textContent = `${metrics.platform_name} / ${metrics.capture_backend}`;
    document.getElementById("clients").textContent = metrics.clients;
    document.getElementById("uptime").textContent = metrics.uptime;
    document.getElementById("res").textContent = metrics.res;
    document.getElementById("fps").textContent = liveFpsText;
    document.getElementById("frame-age").textContent = frameAgeText;
    document.getElementById("stream-input").textContent = metrics.stream_input;
    document.getElementById("stream-pipeline").textContent = metrics.stream_pipeline;
    document.getElementById("stream-meta").textContent = `Age ${frameAgeText} • Drops ${metrics.queue_drops} • Clients ${metrics.clients}`;
    document.getElementById("video-res").textContent = metrics.res;
    document.getElementById("video-meta-line").textContent =
      `${metrics.capture_backend} • ${metrics.stream_input} • ${frameSizeText} • RST ${metrics.restarts}`;

    backendPill.textContent = String(metrics.capture_backend || "--").toUpperCase();
    frameSizePill.textContent = frameSizeText;
    restartPill.textContent = `RST:${metrics.restarts}`;

    setBarFill("cpu-bar-fill", cpuPct);
    setBarFill("ram-bar-fill", ramPct);
    setBarFill("temp-bar-fill", tempPct);
    setBatteryFill(battAvailable ? metrics.batt : 0);

    if (!streamAlive && metrics.clients === 0) {
      setStatus("STANDBY", null);
    } else if (streamAlive && !(mediaRecorder && mediaRecorder.state !== "inactive")) {
      setStatus(`LIVE ${metrics.fps_actual.toFixed(1)} FPS`, true);
    }

    updateRecordingUi();
  } catch (error) {
    if (Date.now() - lastMetricsOk > 4000) {
      setStatus("TELEMETRY OFFLINE", false);
    }
  }
}

window.addEventListener("beforeunload", () => {
  if (mediaRecorder && mediaRecorder.state !== "inactive") {
    try {
      mediaRecorder.stop();
    } catch (error) {
    }
  }

  if (recordTicker) clearInterval(recordTicker);
  if (recordedUrl) URL.revokeObjectURL(recordedUrl);

  streamSource.src = "";
  stopRenderLoop();
});

document.addEventListener("visibilitychange", () => {
  if (document.hidden) {
    if (!mediaRecorder || mediaRecorder.state === "inactive") {
      streamSource.src = "";
      setLiveState(false);
      stopRenderLoop();
      updateRecordingUi();
    }
  } else {
    connectStream();
  }
});

connectStream();
setInterval(tick, 1000);
tick();
