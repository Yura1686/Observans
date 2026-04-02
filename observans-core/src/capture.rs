use crate::camera_inventory::first_camera_device;
use crate::config::Config;
use crate::metrics::SharedMetrics;
use crate::runtime::resolve_ffmpeg_for_current_process;
use anyhow::{bail, Context, Result};
use observans_bus::FrameSender;
use std::io::Read;
use std::path::PathBuf;
use std::process::{Command, Stdio};
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
        loop {
            if !first_attempt {
                metrics.note_restart();
                thread::sleep(Duration::from_secs(1));
            }

            if let Err(error) = run_capture_session(&config, &tx, &metrics) {
                warn!("capture session ended: {error:#}");
            }

            first_attempt = false;
        }
    })
}

fn run_capture_session(config: &Config, tx: &FrameSender, metrics: &SharedMetrics) -> Result<()> {
    let device = resolve_device(config)?;
    let ffmpeg = ffmpeg_binary();

    metrics.set_stream_input(device.clone());
    info!(
        "starting capture with {} on {}",
        config.capture_format(),
        device
    );

    let mut child = Command::new(ffmpeg)
        .args(build_ffmpeg_args(config, &device))
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .spawn()
        .context("failed to spawn ffmpeg")?;

    let mut stdout = child.stdout.take().context("ffmpeg stdout missing")?;
    let mut parser = JpegStreamParser::default();
    let mut buffer = [0_u8; 8192];

    loop {
        let read = stdout
            .read(&mut buffer)
            .context("failed to read ffmpeg output")?;
        if read == 0 {
            break;
        }

        for frame in parser.push(&buffer[..read]) {
            metrics.note_frame(frame.len(), config.width, config.height);
            let _ = tx.send(frame);
        }
    }

    let status = child.wait().context("failed to wait for ffmpeg")?;
    bail!("ffmpeg exited with status {status}")
}

fn build_ffmpeg_args(config: &Config, device: &str) -> Vec<String> {
    let mut args = vec![
        "-hide_banner".to_string(),
        "-loglevel".to_string(),
        "error".to_string(),
        "-fflags".to_string(),
        "nobuffer".to_string(),
        "-f".to_string(),
        config.capture_format().to_string(),
        "-framerate".to_string(),
        config.fps.to_string(),
        "-video_size".to_string(),
        format!("{}x{}", config.width, config.height),
    ];

    if config.capture_format() == "v4l2" && config.input_format != "auto" {
        args.push("-input_format".to_string());
        args.push(config.input_format.clone());
    }

    args.push("-i".to_string());
    args.push(normalize_device_for_platform(config, device));
    args.extend([
        "-an".to_string(),
        "-vf".to_string(),
        format!("fps={}", config.fps),
        "-q:v".to_string(),
        "5".to_string(),
        "-f".to_string(),
        "image2pipe".to_string(),
        "-vcodec".to_string(),
        "mjpeg".to_string(),
        "-".to_string(),
    ]);
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

    Ok(config.platform_default_device().to_string())
}

fn ffmpeg_binary() -> PathBuf {
    resolve_ffmpeg_for_current_process(std::env::consts::OS)
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
    use super::JpegStreamParser;
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
