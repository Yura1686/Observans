const params = new URLSearchParams(location.search);
const streamSource = document.getElementById("stream-source");
const stage = document.getElementById("stream-stage");
const stageCtx = stage.getContext("2d", { alpha: false, desynchronized: true });
const statusLine = document.getElementById("status-line");
const recordingDot = document.getElementById("recording-dot");
const recordingState = document.getElementById("recording-state");
const recordingTime = document.getElementById("recording-time");
const recordBtn = document.getElementById("record-btn");
const stopBtn = document.getElementById("stop-btn");
const saveBtn = document.getElementById("save-btn");

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

const cpuHistory = [];
const ramHistory = [];
const MAX_POINTS = 40;

function withTs(path) {
  const query = new URLSearchParams(params);
  query.set("t", Date.now().toString());
  const encoded = query.toString();
  return encoded ? `${path}?${encoded}` : path;
}

function setStatus(label, ok = null) {
  statusLine.textContent = label;
  statusLine.classList.remove("good", "bad");
  if (ok === true) statusLine.classList.add("good");
  if (ok === false) statusLine.classList.add("bad");
}

function connectStream() {
  if (reconnectTimer) {
    clearTimeout(reconnectTimer);
    reconnectTimer = null;
  }

  streamAlive = false;
  setStatus("connecting to twilight feed", false);
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

function pushHistory(arr, value) {
  arr.push(Math.max(0, Math.min(100, value)));
  while (arr.length > MAX_POINTS) arr.shift();
}

function drawGraph(svgId, values) {
  const svg = document.getElementById(svgId);
  if (!svg || values.length === 0) return;

  const width = 100;
  const height = 38;
  const step = width / Math.max(values.length - 1, 1);
  const points = values.map((value, index) => {
    const x = index * step;
    const y = height - (value / 100) * height;
    return `${x.toFixed(2)},${y.toFixed(2)}`;
  });

  const line = points.join(" ");
  const area = `0,${height} ${line} ${width},${height}`;
  svg.innerHTML =
    `<polygon points="${area}" fill="rgba(158,208,255,0.14)"></polygon>` +
    `<polyline points="${line}" fill="none" stroke="rgba(255, 220, 184, 0.96)" stroke-width="1.8" stroke-linejoin="round" stroke-linecap="round"></polyline>`;
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

  recordingDot.classList.toggle("live", isRecording);
  recordingState.textContent = isRecording
    ? "recording local clip"
    : (recordedBlob ? "clip ready to save" : "ready to capture");
}

function startRecording() {
  if (!window.MediaRecorder || !stage.captureStream || !stageCtx) {
    recordingState.textContent = "recording unsupported";
    setStatus("recording unsupported in this browser", false);
    return;
  }

  if (!streamAlive || stage.width === 0 || stage.height === 0) {
    recordingState.textContent = "waiting for live stream";
    setStatus("wait for the live stream before recording", false);
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
    setStatus("could not start local recording", false);
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
    setStatus("local clip ready to save", true);
    updateRecordingUi();
    const tracks = mediaRecorder.stream ? mediaRecorder.stream.getTracks() : [];
    tracks.forEach((track) => track.stop());
  };

  mediaRecorder.start(1000);
  recordStartedAt = Date.now();
  updateRecordingClock();
  recordTicker = setInterval(updateRecordingClock, 250);
  setStatus("recording local clip", true);
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
  setStatus("clip saved to local device", true);
}

streamSource.onload = () => {
  streamAlive = true;
  setStatus("observans live and stable", true);
  startRenderLoop();
  updateRecordingUi();
};

streamSource.onerror = () => {
  streamAlive = false;
  stopRenderLoop();
  setStatus("reconnecting to twilight feed", false);
  updateRecordingUi();
  reconnectTimer = setTimeout(connectStream, 1500);
};

recordBtn.addEventListener("click", startRecording);
stopBtn.addEventListener("click", stopRecording);
saveBtn.addEventListener("click", saveRecording);

async function tick() {
  try {
    const response = await fetch(withTs("/metrics"), { cache: "no-store" });
    if (!response.ok) throw new Error("metrics failed");

    const metrics = await response.json();
    lastMetrics = metrics;
    lastMetricsOk = Date.now();

    const tempText = metrics.temp > 0 ? `${metrics.temp.toFixed(1)}°C` : "N/A";
    const tempSub = metrics.temp > 0 ? "sensor active" : "unavailable on this platform";
    const battText = metrics.batt >= 0 ? `${metrics.batt}%` : "N/A";
    const frameAgeText = metrics.frame_age_ms >= 0 ? `${metrics.frame_age_ms} ms` : "--";
    const frameSizeText = metrics.avg_frame_kb > 0 ? `${metrics.avg_frame_kb.toFixed(1)} KB` : "--";

    document.getElementById("clock").textContent = metrics.time;
    document.getElementById("date").textContent = metrics.date;
    document.getElementById("cpu").textContent = `${metrics.cpu.toFixed(1)}%`;
    document.getElementById("ram").textContent = `${metrics.ram_pct.toFixed(1)}%`;
    document.getElementById("ram-sub").textContent = `${metrics.ram_used_mb} / ${metrics.ram_total_mb} MB`;
    document.getElementById("temp").textContent = tempText;
    document.getElementById("temp-sub").textContent = tempSub;
    document.getElementById("batt").textContent = battText;
    document.getElementById("batt-sub").textContent = String(metrics.batt_status || "--").toLowerCase();
    document.getElementById("host").textContent = metrics.hostname;
    document.getElementById("host-sub").textContent = `${metrics.platform_name} / ${metrics.capture_backend}`;
    document.getElementById("clients").textContent = metrics.clients;
    document.getElementById("uptime").textContent = `uptime ${metrics.uptime}`;
    document.getElementById("res").textContent = metrics.res;
    document.getElementById("fps").textContent = `${metrics.fps_actual.toFixed(1)} / ${metrics.fps_target} fps`;
    document.getElementById("stream-meta").textContent =
      `${metrics.stream_pipeline} | ${metrics.stream_input} | age ${frameAgeText} | drops ${metrics.queue_drops}`;
    document.getElementById("video-res").textContent = metrics.res;
    document.getElementById("video-meta-line").textContent =
      `${metrics.capture_backend} | ${frameSizeText} | restart ${metrics.restarts}`;

    pushHistory(cpuHistory, metrics.cpu);
    pushHistory(ramHistory, metrics.ram_pct);
    drawGraph("cpu-graph", cpuHistory);
    drawGraph("ram-graph", ramHistory);

    if (!streamAlive && metrics.clients === 0) {
      setStatus("system idle under the evening sky");
    } else if (streamAlive && !(mediaRecorder && mediaRecorder.state !== "inactive")) {
      setStatus(`live stream · ${metrics.fps_actual.toFixed(1)} fps · age ${frameAgeText}`, true);
    }

    updateRecordingUi();
  } catch (error) {
    if (Date.now() - lastMetricsOk > 4000) {
      setStatus("telemetry unavailable", false);
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
      streamAlive = false;
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

