use crate::camera_inventory::first_camera_device;
use crate::config::Config;
use crate::metrics::SharedMetrics;
use crate::runtime::resolve_ffmpeg_for_current_process;
use anyhow::{anyhow, bail, Result};
use observans_bus::FrameSender;
use std::io::Read;
use std::path::PathBuf;
use std::process::{ChildStderr, Command, Stdio};
use std::thread;
use std::time::Duration;
use tracing::{info, warn};

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

fn run_capture_session(
    config: &Config,
    tx: &FrameSender,
    metrics: &SharedMetrics,
) -> CaptureResult<()> {
    let device = resolve_device(config).map_err(|error| capture_failure(0, error))?;
    let ffmpeg = ffmpeg_binary();
    metrics.set_stream_input(device.clone());

    let attempts = build_capture_attempts(config, &device);
    let mut startup_failures = Vec::new();
    for attempt in attempts {
        let attempt_suffix = if attempt.label == "requested mode" {
            String::new()
        } else {
            format!(" ({})", attempt.label)
        };
        info!(
            "starting capture with {} on {}{}",
            config.capture_format(),
            device,
            attempt_suffix
        );

        match run_capture_attempt(&ffmpeg, config, tx, metrics, &attempt) {
            Ok(()) => return Ok(()),
            Err(error) if error.frames_sent == 0 => {
                startup_failures.push(format!("{}: {}", attempt.label, error.error));
            }
            Err(error) => return Err(error),
        }
    }

    let details = startup_failures.join(" | ");
    Err(capture_failure(
        0,
        anyhow!(
            "failed to start capture on {device} with {}: {details}",
            config.capture_format()
        ),
    ))
}

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
        .map_err(|error| capture_failure(0, anyhow!(error).context("failed to spawn ffmpeg")))?;

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
    let mut buffer = [0_u8; 8192];
    let mut frames_sent = 0_u64;

    loop {
        let read = stdout.read(&mut buffer).map_err(|error| {
            capture_failure(
                frames_sent,
                anyhow!(error).context("failed to read ffmpeg output"),
            )
        })?;
        if read == 0 {
            break;
        }

        for frame in parser.push(&buffer[..read]) {
            let (width, height) = jpeg_dimensions(&frame).unwrap_or((config.width, config.height));
            metrics.note_frame(frame.len(), width, height);
            frames_sent += 1;
            let _ = tx.send(frame);
        }
    }

    let status = child.wait().map_err(|error| {
        capture_failure(
            frames_sent,
            anyhow!(error).context("failed to wait for ffmpeg"),
        )
    })?;
    let stderr_output = finish_stderr_collector(stderr_reader);

    let error = if stderr_output.is_empty() {
        anyhow!("ffmpeg exited with status {status}")
    } else {
        anyhow!("ffmpeg exited with status {status}: {stderr_output}")
    };
    Err(capture_failure(frames_sent, error))
}

fn build_capture_attempts(config: &Config, device: &str) -> Vec<CaptureAttempt> {
    let normalized_device = normalize_device_for_platform(config, device);
    match config.capture_format() {
        "dshow" => build_dshow_attempts(config, &normalized_device),
        "v4l2" => build_v4l2_attempts(config, &normalized_device),
        _ => vec![CaptureAttempt {
            label: "requested mode".to_string(),
            args: build_ffmpeg_args(config, &normalized_device, CaptureProfile::requested_mode()),
        }],
    }
}

// FIX: added mjpeg-first attempt and dedup; dshow COM crashes (0xc0000005) were caused by
// -fflags nobuffer / -flags low_delay which are now omitted for dshow (see build_ffmpeg_args).
fn build_dshow_attempts(config: &Config, device: &str) -> Vec<CaptureAttempt> {
    let attempts = vec![
        // Try mjpeg input format first — most webcams expose it and it avoids
        // colour-space negotiation that causes "Could not set video options".
        CaptureAttempt {
            label: "mjpeg input".to_string(),
            args: build_ffmpeg_args(
                config,
                device,
                CaptureProfile {
                    include_framerate: true,
                    include_video_size: true,
                    input_format: Some("mjpeg"),
                },
            ),
        },
        CaptureAttempt {
            label: "requested mode".to_string(),
            args: build_ffmpeg_args(config, device, CaptureProfile::requested_mode()),
        },
        CaptureAttempt {
            label: "driver size fallback".to_string(),
            args: build_ffmpeg_args(
                config,
                device,
                CaptureProfile {
                    include_framerate: true,
                    include_video_size: false,
                    input_format: None,
                },
            ),
        },
        CaptureAttempt {
            label: "driver defaults".to_string(),
            args: build_ffmpeg_args(
                config,
                device,
                CaptureProfile {
                    include_framerate: false,
                    include_video_size: false,
                    input_format: None,
                },
            ),
        },
    ];
    // Deduplicate so identical arg-lists are not run twice.
    dedup_attempts(attempts)
}

fn build_v4l2_attempts(config: &Config, device: &str) -> Vec<CaptureAttempt> {
    let mut attempts = Vec::new();

    if config.input_format == "auto" {
        attempts.push(CaptureAttempt {
            label: "preferred mjpeg input".to_string(),
            args: build_ffmpeg_args(
                config,
                device,
                CaptureProfile {
                    include_framerate: true,
                    include_video_size: true,
                    input_format: Some("mjpeg"),
                },
            ),
        });
    }

    attempts.push(CaptureAttempt {
        label: "requested mode".to_string(),
        args: build_ffmpeg_args(
            config,
            device,
            CaptureProfile {
                include_framerate: true,
                include_video_size: true,
                input_format: (config.input_format != "auto")
                    .then_some(config.input_format.as_str()),
            },
        ),
    });

    if config.input_format == "auto" {
        attempts.push(CaptureAttempt {
            label: "driver defaults".to_string(),
            args: build_ffmpeg_args(
                config,
                device,
                CaptureProfile {
                    include_framerate: false,
                    include_video_size: false,
                    input_format: None,
                },
            ),
        });
    }

    dedup_attempts(attempts)
}

fn dedup_attempts(attempts: Vec<CaptureAttempt>) -> Vec<CaptureAttempt> {
    let mut unique = Vec::new();
    for attempt in attempts {
        if unique
            .iter()
            .all(|existing: &CaptureAttempt| existing.args != attempt.args)
        {
            unique.push(attempt);
        }
    }
    unique
}

fn build_ffmpeg_args(config: &Config, device: &str, profile: CaptureProfile<'_>) -> Vec<String> {
    let is_dshow = config.capture_format() == "dshow";

    let mut args = vec![
        "-hide_banner".to_string(),
        "-nostdin".to_string(),
        "-loglevel".to_string(),
        "warning".to_string(),
    ];

    // FIX: -fflags nobuffer and -flags low_delay trigger STATUS_ACCESS_VIOLATION
    // (exit 0xc0000005) inside the dshow COM layer on Windows. These flags are
    // safe and beneficial for v4l2 but must be omitted for dshow.
    if !is_dshow {
        args.push("-fflags".to_string());
        args.push("nobuffer".to_string());
        args.push("-flags".to_string());
        args.push("low_delay".to_string());
    }

    args.extend([
        "-thread_queue_size".to_string(),
        "4".to_string(),
        "-f".to_string(),
        config.capture_format().to_string(),
    ]);

    if is_dshow {
        args.push("-rtbufsize".to_string());
        args.push("128M".to_string());
    }

    if profile.include_framerate {
        args.push("-framerate".to_string());
        args.push(config.fps.to_string());
    }

    if profile.include_video_size {
        args.push("-video_size".to_string());
        args.push(format!("{}x{}", config.width, config.height));
    }

    if let Some(input_format) = profile.input_format {
        args.push("-input_format".to_string());
        args.push(input_format.to_string());
    }

    args.push("-i".to_string());
    args.push(device.to_string());
    args.push("-an".to_string());
    args.push("-flush_packets".to_string());
    args.push("1".to_string());

    if config.capture_format() == "v4l2" && profile.input_format == Some("mjpeg") {
        args.extend([
            "-c:v".to_string(),
            "copy".to_string(),
            "-f".to_string(),
            "mjpeg".to_string(),
            "pipe:1".to_string(),
        ]);
    } else {
        args.extend([
            "-threads".to_string(),
            "1".to_string(),
            "-q:v".to_string(),
            "7".to_string(),
            "-f".to_string(),
            "image2pipe".to_string(),
            "-c:v".to_string(),
            "mjpeg".to_string(),
            "pipe:1".to_string(),
        ]);
    }
    args
}

fn normalize_device_for_platform(config: &Config, device: &str) -> String {
    match config.capture_format() {
        "dshow" if !device.starts_with("video=") => format!("video={device}"),
        _ => device.to_string(),
    }
}

fn resolve_device(config: &Config) -> Result<String> {
    if config.device != "auto" {
        return Ok(config.device.clone());
    }

    let ffmpeg_path = ffmpeg_binary();
    if let Some(device) = first_camera_device(Some(ffmpeg_path.as_path())) {
        return Ok(device);
    }

    if config.capture_format() == "v4l2" {
        return Ok(config.platform_default_device().to_string());
    }

    bail!(
        "could not auto-resolve a Windows camera. Re-run with --device \"Camera Name\" or check the bundled ffmpeg device list"
    )
}

fn ffmpeg_binary() -> PathBuf {
    resolve_ffmpeg_for_current_process(std::env::consts::OS)
}

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

#[derive(Debug, Clone, Copy)]
struct CaptureProfile<'a> {
    include_framerate: bool,
    include_video_size: bool,
    input_format: Option<&'a str>,
}

impl CaptureProfile<'_> {
    fn requested_mode() -> Self {
        Self {
            include_framerate: true,
            include_video_size: true,
            input_format: None,
        }
    }
}

fn capture_failure(frames_sent: u64, error: impl Into<anyhow::Error>) -> CaptureFailure {
    CaptureFailure {
        frames_sent,
        error: error.into(),
    }
}

fn spawn_stderr_collector(stderr: ChildStderr) -> thread::JoinHandle<String> {
    thread::spawn(move || {
        let mut reader = stderr;
        let mut bytes = Vec::new();
        let _ = reader.read_to_end(&mut bytes);
        format_ffmpeg_stderr(&bytes)
    })
}

fn finish_stderr_collector(stderr_reader: thread::JoinHandle<String>) -> String {
    stderr_reader.join().unwrap_or_default()
}

fn format_ffmpeg_stderr(bytes: &[u8]) -> String {
    const MAX_LINES: usize = 4;
    const MAX_CHARS: usize = 600;

    let mut message = String::from_utf8_lossy(bytes)
        .lines()
        .map(str::trim)
        .filter(|line| !line.is_empty())
        .rev()
        .take(MAX_LINES)
        .collect::<Vec<_>>()
        .into_iter()
        .rev()
        .collect::<Vec<_>>()
        .join(" | ");

    if message.len() > MAX_CHARS {
        message.truncate(MAX_CHARS.saturating_sub(3));
        message.push_str("...");
    }

    message
}

fn jpeg_dimensions(frame: &[u8]) -> Option<(u32, u32)> {
    if frame.len() < 4 || frame[0] != 0xFF || frame[1] != 0xD8 {
        return None;
    }

    let mut index = 2;
    while index + 8 < frame.len() {
        if frame[index] != 0xFF {
            index += 1;
            continue;
        }

        while index < frame.len() && frame[index] == 0xFF {
            index += 1;
        }
        if index >= frame.len() {
            return None;
        }

        let marker = frame[index];
        index += 1;

        match marker {
            0xD8 | 0xD9 | 0x01 | 0xD0..=0xD7 => continue,
            0xC0..=0xC3 | 0xC5..=0xC7 | 0xC9..=0xCB | 0xCD..=0xCF => {
                if index + 6 >= frame.len() {
                    return None;
                }
                let height = u16::from_be_bytes([frame[index + 3], frame[index + 4]]) as u32;
                let width = u16::from_be_bytes([frame[index + 5], frame[index + 6]]) as u32;
                return Some((width, height));
            }
            _ => {
                if index + 1 >= frame.len() {
                    return None;
                }
                let segment_len = u16::from_be_bytes([frame[index], frame[index + 1]]) as usize;
                if segment_len < 2 || index + segment_len > frame.len() {
                    return None;
                }
                index += segment_len;
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

            let frame = self.buffer.drain(..end + 2).collect::<Vec<_>>();
            frames.push(frame);
        }

        frames
    }
}

fn find_marker(buffer: &[u8], marker: &[u8], from: usize) -> Option<usize> {
    buffer
        .windows(marker.len())
        .enumerate()
        .skip(from)
        .find_map(|(index, window)| (window == marker).then_some(index))
}

fn trim_prefix_noise(buffer: &mut Vec<u8>) {
    if buffer.len() > 1 {
        let keep_from = buffer.len().saturating_sub(1);
        buffer.drain(..keep_from);
    }
}

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
        fs::create_dir_all(bundled.parent().expect("bundled parent")).unwrap();
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
        fs::create_dir_all(executable.parent().expect("binary parent")).unwrap();
        fs::create_dir_all(bundled.parent().expect("bundled parent")).unwrap();
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
        fs::create_dir_all(bundled.parent().expect("bundled parent")).unwrap();
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
            .expect("system time before epoch")
            .as_nanos();
        let dir = std::env::temp_dir().join(format!("observans-{label}-{}-{nonce}", process::id()));
        fs::create_dir_all(&dir).unwrap();
        dir
    }
}