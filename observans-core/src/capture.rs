use crate::camera_inventory::first_camera_device;
use crate::config::Config;
use crate::metrics::SharedMetrics;
use crate::probe::{
    probe_dshow, probe_v4l2, resolve_params_from_probe, ResolvedCaptureParams,
};
use crate::runtime::resolve_ffmpeg_for_current_process;
use anyhow::{anyhow, bail, Result};
use observans_bus::FrameSender;
use std::io::Read;
use std::path::PathBuf;
use std::process::{ChildStderr, Command, Stdio};
use std::thread;
use std::time::Duration;
use tracing::{info, warn};

// ---------------------------------------------------------------------------
// Public entry-point
// ---------------------------------------------------------------------------

pub fn start_capture(
    config: Config,
    tx: FrameSender,
    metrics: SharedMetrics,
) -> thread::JoinHandle<()> {
    thread::spawn(move || {
        let mut first_attempt = true;
        let mut retry_delay_secs = 1_u64;
        loop {
            if !first_attempt {
                metrics.note_restart();
                thread::sleep(Duration::from_secs(retry_delay_secs));
            }

            match run_capture_session(&config, &tx, &metrics) {
                Ok(()) => retry_delay_secs = 1,
                Err(error) => {
                    warn!("capture session ended: {:#}", error.error);
                    retry_delay_secs = if error.frames_sent > 0 {
                        1
                    } else {
                        (retry_delay_secs + 1).min(5)
                    };
                }
            }

            first_attempt = false;
        }
    })
}

// ---------------------------------------------------------------------------
// Session
// ---------------------------------------------------------------------------

fn run_capture_session(
    config: &Config,
    tx: &FrameSender,
    metrics: &SharedMetrics,
) -> CaptureResult<()> {
    let device = resolve_device(config).map_err(|e| capture_failure(0, e))?;
    let ffmpeg = ffmpeg_binary();

    // ---- Camera probe -------------------------------------------------------
    let params = probe_camera(config, &device, &ffmpeg);
    // -------------------------------------------------------------------------

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

        match run_capture_attempt(&ffmpeg, config, tx, metrics, &attempt) {
            Ok(()) => return Ok(()),
            Err(err) if err.frames_sent == 0 => {
                startup_failures.push(format!("{}: {}", attempt.label, err.error));
            }
            Err(err) => return Err(err),
        }
    }

    let details = startup_failures.join(" | ");
    Err(capture_failure(
        0,
        anyhow!(
            "all capture attempts failed for {} ({}): {details}",
            params.device,
            config.capture_format()
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
        // Unknown backend — use config as-is.
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
        // If the probe selected something unusual, also try without input_format.
        CaptureAttempt {
            label: "no input_format".into(),
            args: ffmpeg_args(config, params, &ProfileHint::NoInputFormat),
        },
        // Driver defaults — safest fallback.
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
        // Probed / best mode first.
        CaptureAttempt {
            label: "primary".into(),
            args: ffmpeg_args(config, params, &ProfileHint::Exact),
        },
        // Try without the pinned input_format.
        CaptureAttempt {
            label: "no input_format".into(),
            args: ffmpeg_args(config, params, &ProfileHint::NoInputFormat),
        },
        // Try without video_size.
        CaptureAttempt {
            label: "driver size".into(),
            args: ffmpeg_args(config, params, &ProfileHint::NoSize),
        },
        // Full driver defaults.
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
        && !matches!(hint, ProfileHint::NoInputFormat | ProfileHint::DriverDefaults);

    let mut args = vec![
        "-hide_banner".into(),
        "-nostdin".into(),
        "-loglevel".into(),
        "warning".into(),
    ];

    // Low-latency flags — safe on v4l2, crash-prone on dshow COM.
    if !is_dshow {
        args.extend(["-fflags".into(), "nobuffer".into(), "-flags".into(), "low_delay".into()]);
    }

    args.extend(["-thread_queue_size".into(), "4".into(), "-f".into(), config.capture_format().into()]);

    if is_dshow {
        args.extend(["-rtbufsize".into(), "128M".into()]);
    }

    // Framerate
    let include_fps = !matches!(hint, ProfileHint::DriverDefaults);
    if include_fps {
        args.extend(["-framerate".into(), params.fps.to_string()]);
    }

    // Video size
    let include_size = !matches!(hint, ProfileHint::NoSize | ProfileHint::DriverDefaults);
    if include_size {
        args.extend(["-video_size".into(), format!("{}x{}", params.width, params.height)]);
    }

    // Input format (pixel/codec format from probe)
    let include_fmt = params.input_format.is_some()
        && !matches!(hint, ProfileHint::NoInputFormat | ProfileHint::DriverDefaults);
    if include_fmt {
        if let Some(fmt) = &params.input_format {
            args.extend(["-input_format".into(), fmt.clone()]);
        }
    }

    // Input device
    args.extend(["-i".into(), params.device.clone()]);

    // Audio suppression + output
    args.extend(["-an".into(), "-flush_packets".into(), "1".into()]);

    // For v4l2 MJPEG we can pass the stream straight through — no decode/re-encode.
    if is_v4l2_mjpeg {
        args.extend(["-c:v".into(), "copy".into(), "-f".into(), "mjpeg".into(), "pipe:1".into()]);
    } else {
        args.extend([
            "-threads".into(), "1".into(),
            "-q:v".into(),    "7".into(),
            "-f".into(),      "image2pipe".into(),
            "-c:v".into(),    "mjpeg".into(),
            "pipe:1".into(),
        ]);
    }

    args
}

// ---------------------------------------------------------------------------
// Run one attempt
// ---------------------------------------------------------------------------

fn run_capture_attempt(
    ffmpeg: &PathBuf,
    config: &Config,
    tx: &FrameSender,
    metrics: &SharedMetrics,
    attempt: &CaptureAttempt,
) -> CaptureResult<()> {
    let mut child = Command::new(ffmpeg)
        .args(&attempt.args)
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .map_err(|e| capture_failure(0, anyhow!(e).context("failed to spawn ffmpeg")))?;

    let mut stdout = child
        .stdout
        .take()
        .ok_or_else(|| capture_failure(0, anyhow!("ffmpeg stdout missing")))?;
    let stderr = child
        .stderr
        .take()
        .ok_or_else(|| capture_failure(0, anyhow!("ffmpeg stderr missing")))?;

    let stderr_reader = spawn_stderr_collector(stderr);
    let mut parser = JpegStreamParser::default();
    let mut buf = [0_u8; 8192];
    let mut frames_sent = 0_u64;

    loop {
        let n = stdout.read(&mut buf).map_err(|e| {
            capture_failure(frames_sent, anyhow!(e).context("reading ffmpeg output"))
        })?;
        if n == 0 {
            break;
        }
        for frame in parser.push(&buf[..n]) {
            let (w, h) = jpeg_dimensions(&frame).unwrap_or((config.width, config.height));
            metrics.note_frame(frame.len(), w, h);
            frames_sent += 1;
            let _ = tx.send(frame);
        }
    }

    let status = child
        .wait()
        .map_err(|e| capture_failure(frames_sent, anyhow!(e).context("waiting for ffmpeg")))?;
    let stderr_out = finish_stderr_collector(stderr_reader);

    let err = if stderr_out.is_empty() {
        anyhow!("ffmpeg exited with {status}")
    } else {
        anyhow!("ffmpeg exited with {status}: {stderr_out}")
    };
    Err(capture_failure(frames_sent, err))
}

// ---------------------------------------------------------------------------
// Device resolution
// ---------------------------------------------------------------------------

fn resolve_device(config: &Config) -> Result<String> {
    if config.device != "auto" {
        return Ok(config.device.clone());
    }
    let ffmpeg_path = ffmpeg_binary();
    if let Some(dev) = first_camera_device(Some(ffmpeg_path.as_path())) {
        return Ok(dev);
    }
    if config.capture_format() == "v4l2" {
        return Ok(config.platform_default_device().to_string());
    }
    bail!("could not auto-resolve a Windows camera; re-run with --device \"Camera Name\"")
}

fn ffmpeg_binary() -> PathBuf {
    resolve_ffmpeg_for_current_process(std::env::consts::OS)
}

// ---------------------------------------------------------------------------
// Support types
// ---------------------------------------------------------------------------

type CaptureResult<T> = std::result::Result<T, CaptureFailure>;

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
    CaptureFailure { frames_sent, error: error.into() }
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
        assert_eq!(resolve_ffmpeg_binary(None, None, "linux"), PathBuf::from("ffmpeg"));
        assert_eq!(
            resolve_ffmpeg_binary(None, Some(PathBuf::from("C:/Observans/observans.exe")), "windows"),
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
        let dir = std::env::temp_dir()
            .join(format!("observans-{label}-{}-{nonce}", process::id()));
        fs::create_dir_all(&dir).unwrap();
        dir
    }
}