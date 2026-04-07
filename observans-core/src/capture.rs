use crate::camera_inventory::camera_device_candidates;
use crate::config::Config;
use crate::metrics::SharedMetrics;
use crate::probe::{probe_dshow, probe_v4l2, resolve_params_from_probe, ResolvedCaptureParams};
use crate::runtime::resolve_ffmpeg_for_current_process;
use anyhow::{anyhow, Result};
use observans_bus::{ClientGate, FrameSender};
use std::io::Read;
use std::path::PathBuf;
use std::process::{ChildStderr, Command, Stdio};
use std::sync::mpsc::RecvTimeoutError;
use std::sync::Arc;
use std::thread;
use std::time::Duration;
use tracing::{info, warn};

// ---------------------------------------------------------------------------
// Public entry-point
// ---------------------------------------------------------------------------

/// Spawns the capture supervisor thread.
///
/// Lifecycle:
/// 1. Park — wait for the first viewer via `gate`.
/// 2. Run  — launch ffmpeg, forward frames to `tx`.
/// 3. Stop — kill ffmpeg when the last viewer disconnects, go to step 1.
///
/// If ffmpeg crashes while viewers are still connected, it is restarted with
/// an exponential back-off (1–5 s).
pub fn start_capture(
    config: Config,
    tx: FrameSender,
    metrics: SharedMetrics,
    gate: Arc<ClientGate>,
) -> thread::JoinHandle<()> {
    thread::spawn(move || loop {
        gate.wait_for_clients();
        info!("viewer connected – starting capture pipeline");

        let mut retry_delay_secs = 1u64;
        let mut first = true;

        loop {
            // Bail out immediately if all viewers left (e.g. while we slept).
            if gate.client_count() == 0 {
                break;
            }

            if !first {
                metrics.note_restart();
                thread::sleep(Duration::from_secs(retry_delay_secs));
                // Re-check after sleep — viewer may have left during the wait.
                if gate.client_count() == 0 {
                    break;
                }
            }
            first = false;

            match run_capture_session(&config, &tx, &metrics, &gate) {
                CaptureEnd::Idle => break, // clean stop, go back to parking
                CaptureEnd::Error(err) => {
                    warn!("capture session ended: {:#}", err.error);
                    retry_delay_secs = if err.frames_sent > 0 {
                        1
                    } else {
                        (retry_delay_secs + 1).min(5)
                    };
                }
            }
        }

        info!("all viewers gone – capture pipeline stopped");
    })
}

// ---------------------------------------------------------------------------
// Session result
// ---------------------------------------------------------------------------

enum CaptureEnd {
    /// Stopped cleanly because the last viewer disconnected.
    Idle,
    /// ffmpeg exited unexpectedly or failed to start.
    Error(CaptureFailure),
}

// ---------------------------------------------------------------------------
// Session — try capture attempts in priority order
// ---------------------------------------------------------------------------

fn run_capture_session(
    config: &Config,
    tx: &FrameSender,
    metrics: &SharedMetrics,
    gate: &Arc<ClientGate>,
) -> CaptureEnd {
    let devices = match resolve_device_candidates(config) {
        Ok(d) => d,
        Err(e) => return CaptureEnd::Error(capture_failure(0, e)),
    };
    let ffmpeg = ffmpeg_binary();
    let mut device_failures = Vec::new();

    for device in devices {
        let params = probe_camera(config, &device, &ffmpeg);
        metrics.set_stream_input(params.device.clone());

        let attempts = build_capture_attempts(config, &params);
        let mut startup_failures = Vec::new();

        for attempt in attempts {
            let suffix = if attempt.label == "primary" {
                String::new()
            } else {
                format!(" ({})", attempt.label)
            };
            info!(
                "starting capture: {} on {}{}",
                config.capture_format(),
                params.device,
                suffix,
            );

            match run_capture_attempt(&ffmpeg, config, tx, metrics, &attempt, gate) {
                CaptureEnd::Idle => return CaptureEnd::Idle,
                CaptureEnd::Error(err) if err.frames_sent == 0 => {
                    startup_failures.push(format!("{}: {}", attempt.label, err.error));
                }
                end => return end,
            }
        }

        device_failures.push(format!(
            "{}: {}",
            params.device,
            startup_failures.join(" | ")
        ));
    }

    let scope = if config.device == "auto" {
        "auto device candidates"
    } else {
        "configured device"
    };

    CaptureEnd::Error(capture_failure(
        0,
        anyhow!(
            "all capture attempts failed for {} ({}): {}",
            scope,
            config.capture_format(),
            device_failures.join(" || ")
        ),
    ))
}

// ---------------------------------------------------------------------------
// Camera probe → ResolvedCaptureParams
// ---------------------------------------------------------------------------

fn probe_camera(config: &Config, device: &str, ffmpeg: &PathBuf) -> ResolvedCaptureParams {
    match config.capture_format() {
        "v4l2" => {
            let probe = probe_v4l2(device, Some(ffmpeg.as_path()));
            resolve_params_from_probe(
                &probe,
                device,
                config.width,
                config.height,
                config.fps,
                &config.input_format,
            )
        }
        "dshow" => {
            // Normalise device name before probing.
            let probe_device = if device.starts_with("video=") {
                device[6..].to_string()
            } else {
                device.to_string()
            };
            let probe = probe_dshow(&probe_device, ffmpeg.as_path());
            resolve_params_from_probe(
                &probe,
                device,
                config.width,
                config.height,
                config.fps,
                &config.input_format,
            )
        }
        _ => ResolvedCaptureParams {
            device: device.to_string(),
            width: config.width,
            height: config.height,
            fps: config.fps,
            input_format: None,
            probed: false,
        },
    }
}

// ---------------------------------------------------------------------------
// Attempt construction
// ---------------------------------------------------------------------------

fn build_capture_attempts(config: &Config, params: &ResolvedCaptureParams) -> Vec<CaptureAttempt> {
    match config.capture_format() {
        "dshow" => build_dshow_attempts(config, params),
        "v4l2" => build_v4l2_attempts(config, params),
        _ => vec![CaptureAttempt {
            label: "primary".into(),
            args: ffmpeg_args(config, params, &ProfileHint::Exact),
        }],
    }
}

/// Hint passed to `ffmpeg_args` to select which parameters to include.
enum ProfileHint {
    /// Use exactly the resolved params (format + size + fps).
    Exact,
    /// Skip the input_format override (let the driver negotiate).
    NoInputFormat,
    /// Skip video_size (let the driver pick resolution).
    NoSize,
    /// Skip both video_size and fps.
    DriverDefaults,
}

fn build_v4l2_attempts(config: &Config, params: &ResolvedCaptureParams) -> Vec<CaptureAttempt> {
    let mut attempts = vec![
        CaptureAttempt {
            label: "primary".into(),
            args: ffmpeg_args(config, params, &ProfileHint::Exact),
        },
        CaptureAttempt {
            label: "no input_format".into(),
            args: ffmpeg_args(config, params, &ProfileHint::NoInputFormat),
        },
        CaptureAttempt {
            label: "driver defaults".into(),
            args: ffmpeg_args(config, params, &ProfileHint::DriverDefaults),
        },
    ];
    dedup_attempts(&mut attempts);
    attempts
}

// FIX: -fflags nobuffer / -flags low_delay trigger STATUS_ACCESS_VIOLATION
// (0xc0000005) inside the dshow COM layer on Windows. Omitted for dshow.
fn build_dshow_attempts(config: &Config, params: &ResolvedCaptureParams) -> Vec<CaptureAttempt> {
    let mut attempts = vec![
        CaptureAttempt {
            label: "primary".into(),
            args: ffmpeg_args(config, params, &ProfileHint::Exact),
        },
        CaptureAttempt {
            label: "no input_format".into(),
            args: ffmpeg_args(config, params, &ProfileHint::NoInputFormat),
        },
        CaptureAttempt {
            label: "driver size".into(),
            args: ffmpeg_args(config, params, &ProfileHint::NoSize),
        },
        CaptureAttempt {
            label: "driver defaults".into(),
            args: ffmpeg_args(config, params, &ProfileHint::DriverDefaults),
        },
    ];
    dedup_attempts(&mut attempts);
    attempts
}

// ---------------------------------------------------------------------------
// FFmpeg argument builder
// ---------------------------------------------------------------------------

fn ffmpeg_args(config: &Config, params: &ResolvedCaptureParams, hint: &ProfileHint) -> Vec<String> {
    let is_dshow = config.capture_format() == "dshow";
    let is_v4l2_mjpeg = config.capture_format() == "v4l2"
        && params.input_format.as_deref() == Some("mjpeg")
        && !matches!(
            hint,
            ProfileHint::NoInputFormat | ProfileHint::DriverDefaults
        );

    let mut args = vec![
        "-hide_banner".into(),
        "-nostdin".into(),
        "-loglevel".into(),
        "warning".into(),
    ];

    // Low-latency flags — safe on v4l2, crash-prone on dshow COM.
    if !is_dshow {
        args.extend([
            "-fflags".into(),
            "nobuffer".into(),
            "-flags".into(),
            "low_delay".into(),
        ]);
    }

    args.extend([
        "-thread_queue_size".into(),
        "4".into(),
        "-f".into(),
        config.capture_format().into(),
    ]);

    if is_dshow {
        args.extend(["-rtbufsize".into(), "128M".into()]);
    }

    // Framerate
    if !matches!(hint, ProfileHint::DriverDefaults) {
        args.extend(["-framerate".into(), params.fps.to_string()]);
    }

    // Video size
    if !matches!(hint, ProfileHint::NoSize | ProfileHint::DriverDefaults) {
        args.extend([
            "-video_size".into(),
            format!("{}x{}", params.width, params.height),
        ]);
    }

    // Input format (pixel/codec format from probe).
    //
    // v4l2  → -input_format <fmt>                 (kernel V4L2 ioctl-level selection)
    // dshow → -pixel_format <fmt>  for raw        (bgr24, yuyv422, …)
    //         -vcodec <fmt>        for compressed (mjpeg, h264)
    //
    // `-input_format` is a v4l2-specific option; passing it to dshow causes
    // "Unrecognized option 'input_format'" and FFmpeg refuses to start.
    let include_fmt = params.input_format.is_some()
        && !matches!(
            hint,
            ProfileHint::NoInputFormat | ProfileHint::DriverDefaults
        );
    if include_fmt {
        if let Some(fmt) = &params.input_format {
            if is_dshow {
                let is_compressed = matches!(fmt.as_str(), "mjpeg" | "h264");
                if is_compressed {
                    args.extend(["-vcodec".into(), fmt.clone()]);
                } else {
                    args.extend(["-pixel_format".into(), fmt.clone()]);
                }
            } else {
                args.extend(["-input_format".into(), fmt.clone()]);
            }
        }
    }

    // Input device.
    // dshow requires the "video=<name>" prefix in the -i argument.
    let ffmpeg_input = if is_dshow {
        if params.device.starts_with("video=") {
            params.device.clone()
        } else {
            format!("video={}", params.device)
        }
    } else {
        params.device.clone()
    };
    args.extend(["-i".into(), ffmpeg_input]);

    // Audio suppression + output
    args.extend(["-an".into(), "-flush_packets".into(), "1".into()]);

    // For v4l2 MJPEG we can pass the stream straight through — no decode/re-encode.
    if is_v4l2_mjpeg {
        args.extend([
            "-c:v".into(),
            "copy".into(),
            "-f".into(),
            "mjpeg".into(),
            "pipe:1".into(),
        ]);
    } else {
        args.extend([
            "-threads".into(),
            "1".into(),
            "-q:v".into(),
            "7".into(),
            "-f".into(),
            "image2pipe".into(),
            "-c:v".into(),
            "mjpeg".into(),
            "pipe:1".into(),
        ]);
    }

    args
}

// ---------------------------------------------------------------------------
// Run one attempt — on-demand: stops cleanly when no viewers remain
// ---------------------------------------------------------------------------

fn run_capture_attempt(
    ffmpeg: &PathBuf,
    config: &Config,
    tx: &FrameSender,
    metrics: &SharedMetrics,
    attempt: &CaptureAttempt,
    gate: &Arc<ClientGate>,
) -> CaptureEnd {
    let mut child = match Command::new(ffmpeg)
        .args(&attempt.args)
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
    {
        Ok(c) => c,
        Err(e) => {
            return CaptureEnd::Error(capture_failure(
                0,
                anyhow!(e).context("failed to spawn ffmpeg"),
            ));
        }
    };

    let stdout = match child.stdout.take() {
        Some(s) => s,
        None => {
            return CaptureEnd::Error(capture_failure(0, anyhow!("ffmpeg stdout missing")));
        }
    };
    let stderr = match child.stderr.take() {
        Some(s) => s,
        None => {
            return CaptureEnd::Error(capture_failure(0, anyhow!("ffmpeg stderr missing")));
        }
    };

    let stderr_reader = spawn_stderr_collector(stderr);

    // Offload the blocking stdout read to a dedicated thread so the main loop
    // can poll the client gate with a timeout without stalling frame delivery.
    let (frame_tx, frame_rx) = std::sync::mpsc::channel::<Vec<u8>>();
    let reader = thread::spawn(move || {
        let mut stdout = stdout;
        let mut parser = JpegStreamParser::default();
        let mut buf = [0_u8; 8192];
        loop {
            match stdout.read(&mut buf) {
                Ok(0) | Err(_) => break,
                Ok(n) => {
                    for frame in parser.push(&buf[..n]) {
                        if frame_tx.send(frame).is_err() {
                            return;
                        }
                    }
                }
            }
        }
    });

    let mut frames_sent = 0u64;
    let mut stopped_idle = false;

    // How often to check the gate even when frames are flowing.
    // This is the maximum delay between a viewer leaving and ffmpeg being killed.
    const GATE_CHECK_INTERVAL: u64 = 10;

    loop {
        match frame_rx.recv_timeout(Duration::from_millis(200)) {
            Ok(frame) => {
                let (w, h) = jpeg_dimensions(&frame).unwrap_or((config.width, config.height));
                metrics.note_frame(frame.len(), w, h);
                frames_sent += 1;
                let _ = tx.send(frame);

                // Check gate periodically even when frames are flowing.
                // Without this check, if the client disconnects between frames
                // the timeout branch may never be reached while ffmpeg keeps
                // the camera open.
                if frames_sent % GATE_CHECK_INTERVAL == 0 && gate.client_count() == 0 {
                    let _ = child.kill();
                    stopped_idle = true;
                    break;
                }
            }
            Err(RecvTimeoutError::Timeout) => {
                if gate.client_count() == 0 {
                    let _ = child.kill();
                    stopped_idle = true;
                    break;
                }
            }
            Err(RecvTimeoutError::Disconnected) => break, // ffmpeg exited naturally
        }
    }

    // Drop the receiver BEFORE joining the reader thread.
    // This unblocks the reader's `frame_tx.send()` call so it exits without
    // waiting for the OS pipe buffer to be fully drained.
    drop(frame_rx);

    // Join the reader — it exits quickly because its sender gets Err.
    reader.join().ok();

    // Reap the child. This is the call that makes the kernel release all file
    // descriptors held by the ffmpeg process, including the camera device.
    // Without wait() the process becomes a zombie and the camera stays locked.
    let _ = child.wait();

    let stderr_out = finish_stderr_collector(stderr_reader);

    if stopped_idle {
        info!("ffmpeg reaped – camera released");
        return CaptureEnd::Idle;
    }

    let err = if stderr_out.is_empty() {
        anyhow!("ffmpeg exited unexpectedly")
    } else {
        anyhow!("ffmpeg: {stderr_out}")
    };
    CaptureEnd::Error(capture_failure(frames_sent, err))
}

// ---------------------------------------------------------------------------
// Device resolution
// ---------------------------------------------------------------------------

fn resolve_device_candidates(config: &Config) -> Result<Vec<String>> {
    if config.device != "auto" {
        return Ok(vec![config.device.clone()]);
    }

    let ffmpeg_path = ffmpeg_binary();
    let devices = camera_device_candidates(Some(ffmpeg_path.as_path()));
    if !devices.is_empty() {
        return Ok(devices);
    }

    let fallback = config.platform_default_device().to_string();
    warn!(
        "camera auto-resolve returned no devices; falling back to {}",
        fallback
    );
    Ok(vec![fallback])
}

fn ffmpeg_binary() -> PathBuf {
    resolve_ffmpeg_for_current_process(std::env::consts::OS)
}

// ---------------------------------------------------------------------------
// Support types
// ---------------------------------------------------------------------------

#[derive(Debug)]
struct CaptureFailure {
    frames_sent: u64,
    error: anyhow::Error,
}

#[derive(Debug, Clone)]
struct CaptureAttempt {
    label: String,
    args: Vec<String>,
}

fn capture_failure(frames_sent: u64, error: impl Into<anyhow::Error>) -> CaptureFailure {
    CaptureFailure {
        frames_sent,
        error: error.into(),
    }
}

/// Remove attempts with identical arg-lists to avoid running the same command twice.
fn dedup_attempts(attempts: &mut Vec<CaptureAttempt>) {
    let mut seen: Vec<Vec<String>> = Vec::new();
    attempts.retain(|a| {
        if seen.iter().any(|s| *s == a.args) {
            false
        } else {
            seen.push(a.args.clone());
            true
        }
    });
}

// ---------------------------------------------------------------------------
// Stderr collector
// ---------------------------------------------------------------------------

fn spawn_stderr_collector(stderr: ChildStderr) -> thread::JoinHandle<String> {
    thread::spawn(move || {
        let mut r = stderr;
        let mut bytes = Vec::new();
        let _ = r.read_to_end(&mut bytes);
        format_ffmpeg_stderr(&bytes)
    })
}

fn finish_stderr_collector(h: thread::JoinHandle<String>) -> String {
    h.join().unwrap_or_default()
}

fn format_ffmpeg_stderr(bytes: &[u8]) -> String {
    const MAX_LINES: usize = 4;
    const MAX_CHARS: usize = 600;

    let mut msg = String::from_utf8_lossy(bytes)
        .lines()
        .map(str::trim)
        .filter(|l| !l.is_empty())
        .rev()
        .take(MAX_LINES)
        .collect::<Vec<_>>()
        .into_iter()
        .rev()
        .collect::<Vec<_>>()
        .join(" | ");

    if msg.len() > MAX_CHARS {
        msg.truncate(MAX_CHARS.saturating_sub(3));
        msg.push_str("...");
    }
    msg
}

// ---------------------------------------------------------------------------
// JPEG stream parser
// ---------------------------------------------------------------------------

fn jpeg_dimensions(frame: &[u8]) -> Option<(u32, u32)> {
    if frame.len() < 4 || frame[0] != 0xFF || frame[1] != 0xD8 {
        return None;
    }
    let mut i = 2;
    while i + 8 < frame.len() {
        if frame[i] != 0xFF {
            i += 1;
            continue;
        }
        while i < frame.len() && frame[i] == 0xFF {
            i += 1;
        }
        if i >= frame.len() {
            return None;
        }
        let marker = frame[i];
        i += 1;
        match marker {
            0xD8 | 0xD9 | 0x01 | 0xD0..=0xD7 => continue,
            0xC0..=0xC3 | 0xC5..=0xC7 | 0xC9..=0xCB | 0xCD..=0xCF => {
                if i + 6 >= frame.len() {
                    return None;
                }
                let h = u16::from_be_bytes([frame[i + 3], frame[i + 4]]) as u32;
                let w = u16::from_be_bytes([frame[i + 5], frame[i + 6]]) as u32;
                return Some((w, h));
            }
            _ => {
                if i + 1 >= frame.len() {
                    return None;
                }
                let seg_len = u16::from_be_bytes([frame[i], frame[i + 1]]) as usize;
                if seg_len < 2 || i + seg_len > frame.len() {
                    return None;
                }
                i += seg_len;
            }
        }
    }
    None
}

#[derive(Default)]
struct JpegStreamParser {
    buffer: Vec<u8>,
}

impl JpegStreamParser {
    fn push(&mut self, chunk: &[u8]) -> Vec<Vec<u8>> {
        self.buffer.extend_from_slice(chunk);
        let mut frames = Vec::new();
        loop {
            let Some(start) = find_marker(&self.buffer, &[0xFF, 0xD8], 0) else {
                trim_prefix_noise(&mut self.buffer);
                break;
            };
            if start > 0 {
                self.buffer.drain(..start);
            }
            let Some(end) = find_marker(&self.buffer, &[0xFF, 0xD9], 2) else {
                break;
            };
            frames.push(self.buffer.drain(..end + 2).collect());
        }
        frames
    }
}

fn find_marker(buf: &[u8], marker: &[u8], from: usize) -> Option<usize> {
    buf.windows(marker.len())
        .enumerate()
        .skip(from)
        .find_map(|(i, w)| (w == marker).then_some(i))
}

fn trim_prefix_noise(buf: &mut Vec<u8>) {
    if buf.len() > 1 {
        let keep = buf.len().saturating_sub(1);
        buf.drain(..keep);
    }
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::{jpeg_dimensions, JpegStreamParser};
    use crate::runtime::{ffmpeg_executable_name, resolve_ffmpeg_binary};
    use std::fs;
    use std::path::PathBuf;
    use std::process;
    use std::time::{SystemTime, UNIX_EPOCH};

    #[test]
    fn extracts_jpeg_frames_from_stream() {
        let mut parser = JpegStreamParser::default();
        let stream = [
            0, 1, 0xFF, 0xD8, 9, 8, 7, 0xFF, 0xD9, 0xFF, 0xD8, 1, 0xFF, 0xD9,
        ];
        let frames = parser.push(&stream);
        assert_eq!(frames.len(), 2);
        assert_eq!(frames[0], vec![0xFF, 0xD8, 9, 8, 7, 0xFF, 0xD9]);
    }

    #[test]
    fn explicit_override_wins_over_bundle_lookup() {
        let path = resolve_ffmpeg_binary(
            Some(PathBuf::from("/tmp/custom-ffmpeg").into_os_string()),
            Some(PathBuf::from("/tmp/app/observans")),
            "linux",
        );
        assert_eq!(path, PathBuf::from("/tmp/custom-ffmpeg"));
    }

    #[test]
    fn bundled_linux_ffmpeg_is_preferred_when_present() {
        let temp = unique_temp_dir("linux-bundle");
        let executable = temp.join("observans");
        let bundled = temp.join("_observans_runtime/ffmpeg/bin/ffmpeg");
        fs::create_dir_all(bundled.parent().unwrap()).unwrap();
        fs::write(&executable, b"stub").unwrap();
        fs::write(&bundled, b"stub").unwrap();
        let resolved = resolve_ffmpeg_binary(None, Some(executable), "linux");
        assert_eq!(resolved, bundled);
        let _ = fs::remove_dir_all(temp);
    }

    #[test]
    fn bundled_ffmpeg_is_found_when_binary_is_under_runtime_bin() {
        let temp = unique_temp_dir("linux-hidden-binary");
        let executable = temp.join("_observans_runtime/bin/observans");
        let bundled = temp.join("_observans_runtime/ffmpeg/bin/ffmpeg");
        fs::create_dir_all(executable.parent().unwrap()).unwrap();
        fs::create_dir_all(bundled.parent().unwrap()).unwrap();
        fs::write(&executable, b"stub").unwrap();
        fs::write(&bundled, b"stub").unwrap();
        let resolved = resolve_ffmpeg_binary(None, Some(executable), "linux");
        assert_eq!(resolved, bundled);
        let _ = fs::remove_dir_all(temp);
    }

    #[test]
    fn bundled_windows_ffmpeg_is_preferred_when_present() {
        let temp = unique_temp_dir("windows-bundle");
        let executable = temp.join("observans.exe");
        let bundled = temp.join("_observans_runtime/ffmpeg/bin/ffmpeg.exe");
        fs::create_dir_all(bundled.parent().unwrap()).unwrap();
        fs::write(&executable, b"stub").unwrap();
        fs::write(&bundled, b"stub").unwrap();
        let resolved = resolve_ffmpeg_binary(None, Some(executable), "windows");
        assert_eq!(resolved, bundled);
        let _ = fs::remove_dir_all(temp);
    }

    #[test]
    fn falls_back_to_platform_binary_name_when_bundle_is_missing() {
        assert_eq!(
            resolve_ffmpeg_binary(None, None, "linux"),
            PathBuf::from("ffmpeg")
        );
        assert_eq!(
            resolve_ffmpeg_binary(
                None,
                Some(PathBuf::from("C:/Observans/observans.exe")),
                "windows"
            ),
            PathBuf::from("ffmpeg.exe")
        );
        assert_eq!(ffmpeg_executable_name("windows"), "ffmpeg.exe");
    }

    #[test]
    fn reads_jpeg_dimensions_from_frame_header() {
        let frame = [
            0xFF, 0xD8, 0xFF, 0xC0, 0x00, 0x11, 0x08, 0x02, 0xD0, 0x05, 0x00, 0x03, 0x01, 0x22,
            0x00, 0x02, 0x11, 0x01, 0x03, 0x11, 0x01, 0xFF, 0xD9,
        ];
        assert_eq!(jpeg_dimensions(&frame), Some((1280, 720)));
    }

    fn unique_temp_dir(label: &str) -> PathBuf {
        let nonce = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let dir = std::env::temp_dir().join(format!("observans-{label}-{}-{nonce}", process::id()));
        fs::create_dir_all(&dir).unwrap();
        dir
    }
}
