use std::path::Path;
use std::process::Command;
use tracing::debug;

// ---------------------------------------------------------------------------
// Public types
// ---------------------------------------------------------------------------

/// A single camera mode: format + resolution + maximum framerate.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CameraMode {
    /// Normalised format name: "mjpeg", "yuyv422", "bgr24", "h264", …
    pub format: String,
    pub width: u32,
    pub height: u32,
    /// Maximum framerate supported for this format+resolution.
    pub fps_max: u32,
}

impl CameraMode {
    /// True for hardware-compressed streams (no CPU decode needed on ingress).
    pub fn is_compressed(&self) -> bool {
        matches!(self.format.as_str(), "mjpeg" | "h264")
    }

    /// Scoring used during mode selection.
    ///
    /// Priority order (descending):
    ///   1. Compressed format (MJPEG / H264)
    ///   2. Resolution (pixel count)
    ///   3. Framerate
    fn score(&self) -> u64 {
        let compression_bonus: u64 = if self.is_compressed() {
            1_000_000_000_000
        } else {
            0
        };
        let pixels: u64 = (self.width as u64) * (self.height as u64);
        let fps: u64 = self.fps_max as u64;
        compression_bonus + pixels * 1_000 + fps
    }
}

/// Result of probing a single camera device.
#[derive(Debug, Clone, Default)]
pub struct ProbeResult {
    pub modes: Vec<CameraMode>,
}

/// The resolved parameters Observans will use for a capture session.
#[derive(Debug, Clone)]
pub struct ResolvedCaptureParams {
    /// Resolved device string (may differ from config.device when "auto" is
    /// expanded or when the platform normalises the name).
    pub device: String,
    pub width: u32,
    pub height: u32,
    pub fps: u32,
    /// None means "let FFmpeg negotiate"; Some("mjpeg") / Some("yuyv422") /
    /// Some("bgr24") means pass `-input_format <value>` to FFmpeg.
    pub input_format: Option<String>,
    /// Whether the probe found real capability data for this device.
    pub probed: bool,
}

// ---------------------------------------------------------------------------
// Public probe entry-points
// ---------------------------------------------------------------------------

/// Probe a V4L2 device.
///
/// Tries `v4l2-ctl --list-formats-ext` first; falls back to
/// `ffmpeg -f v4l2 -list_formats all` if v4l2-ctl is unavailable or returns
/// nothing useful.
pub fn probe_v4l2(device: &str, ffmpeg_path: Option<&Path>) -> ProbeResult {
    if let Some(modes) = probe_v4l2_via_v4l2ctl(device) {
        if !modes.is_empty() {
            debug!("v4l2 probe via v4l2-ctl: {} modes found", modes.len());
            return ProbeResult { modes };
        }
    }

    if let Some(ffmpeg) = ffmpeg_path {
        if let Some(modes) = probe_v4l2_via_ffmpeg(device, ffmpeg) {
            debug!("v4l2 probe via ffmpeg: {} modes found", modes.len());
            return ProbeResult { modes };
        }
    }

    debug!("v4l2 probe returned no modes for {device}");
    ProbeResult::default()
}

/// Probe a DirectShow (Windows) device.
///
/// Runs `ffmpeg -f dshow -list_options true -i video=<device>`.
pub fn probe_dshow(device: &str, ffmpeg_path: &Path) -> ProbeResult {
    // Normalise: strip "video=" prefix for the ffmpeg -i argument if needed.
    let ffmpeg_input = dshow_input_name(device);
    let modes = probe_dshow_via_ffmpeg(&ffmpeg_input, ffmpeg_path).unwrap_or_default();
    debug!(
        "dshow probe for '{}': {} modes found",
        ffmpeg_input,
        modes.len()
    );
    ProbeResult { modes }
}

// ---------------------------------------------------------------------------
// Mode selection
// ---------------------------------------------------------------------------

impl ProbeResult {
    /// Select the best mode given preferred constraints.
    ///
    /// Algorithm:
    ///   • Among modes whose resolution ≤ preferred AND fps ≤ preferred,
    ///     pick the highest-scoring one.
    ///   • If none fit inside constraints, pick the highest-scoring overall
    ///     (camera can't meet the requested settings; let FFmpeg or the driver
    ///     downscale / drop frames).
    pub fn best_mode(
        &self,
        preferred_width: u32,
        preferred_height: u32,
        preferred_fps: u32,
    ) -> Option<&CameraMode> {
        if self.modes.is_empty() {
            return None;
        }

        let preferred_pixels = (preferred_width as u64) * (preferred_height as u64);

        // Modes that fit inside the preferred envelope.
        let fitting: Vec<&CameraMode> = self
            .modes
            .iter()
            .filter(|m| {
                let pixels = (m.width as u64) * (m.height as u64);
                pixels <= preferred_pixels && m.fps_max <= preferred_fps
            })
            .collect();

        if !fitting.is_empty() {
            return fitting.into_iter().max_by_key(|m| m.score());
        }

        // Nothing fits — pick the overall best and hope FFmpeg adapts.
        self.modes.iter().max_by_key(|m| m.score())
    }
}

// ---------------------------------------------------------------------------
// Resolution into ResolvedCaptureParams
// ---------------------------------------------------------------------------

/// Build `ResolvedCaptureParams` from a probe result (or config fallback).
///
/// Called in `capture.rs` after the device has been resolved.
pub fn resolve_params_from_probe(
    probe: &ProbeResult,
    device: &str,
    preferred_width: u32,
    preferred_height: u32,
    preferred_fps: u32,
    user_input_format: &str,
) -> ResolvedCaptureParams {
    // If the user pinned an input format ("mjpeg", "yuyv422", …) honour it.
    let user_pinned = user_input_format != "auto";

    if user_pinned {
        // User knows what they want; skip probe-based selection.
        return ResolvedCaptureParams {
            device: device.to_string(),
            width: preferred_width,
            height: preferred_height,
            fps: preferred_fps,
            input_format: Some(user_input_format.to_string()),
            probed: false,
        };
    }

    if let Some(best) = probe.best_mode(preferred_width, preferred_height, preferred_fps) {
        let chosen_fps = best.fps_max.min(preferred_fps);
        let input_format = if best.is_compressed() {
            Some(best.format.clone())
        } else {
            // Raw format: pass it so FFmpeg doesn't guess wrong pixel format.
            Some(best.format.clone())
        };

        tracing::info!(
            "probe selected: {}  {}×{}  {}fps  (preferred {}×{}@{})",
            best.format,
            best.width,
            best.height,
            chosen_fps,
            preferred_width,
            preferred_height,
            preferred_fps
        );

        ResolvedCaptureParams {
            device: device.to_string(),
            width: best.width,
            height: best.height,
            fps: chosen_fps,
            input_format,
            probed: true,
        }
    } else {
        // Probe returned nothing — fall back to config values.
        tracing::warn!("camera probe returned no modes for {device}; using config defaults");
        ResolvedCaptureParams {
            device: device.to_string(),
            width: preferred_width,
            height: preferred_height,
            fps: preferred_fps,
            input_format: None,
            probed: false,
        }
    }
}

// ---------------------------------------------------------------------------
// v4l2-ctl parsing
// ---------------------------------------------------------------------------

fn probe_v4l2_via_v4l2ctl(device: &str) -> Option<Vec<CameraMode>> {
    let output = Command::new("v4l2-ctl")
        .args(["-d", device, "--list-formats-ext"])
        .output()
        .ok()?;

    if !output.status.success() && output.stdout.is_empty() {
        return None;
    }

    let text = String::from_utf8_lossy(&output.stdout);
    let modes = parse_v4l2ctl_formats(&text);
    Some(modes)
}

fn probe_v4l2_via_ffmpeg(device: &str, ffmpeg_path: &Path) -> Option<Vec<CameraMode>> {
    let output = Command::new(ffmpeg_path)
        .args([
            "-hide_banner",
            "-f",
            "v4l2",
            "-list_formats",
            "all",
            "-i",
            device,
        ])
        .output()
        .ok()?;

    // FFmpeg writes format list to stderr even on "error opening input" exit.
    let text = String::from_utf8_lossy(&output.stderr);
    let modes = parse_ffmpeg_v4l2_formats(&text);
    Some(modes)
}

/// Parse the output of `v4l2-ctl -d /dev/videoN --list-formats-ext`.
///
/// Example:
/// ```text
/// [0]: 'YUYV' (YUYV 4:2:2)
///         Size: Discrete 640x480
///                 Interval: Discrete 0.033s (30.000 fps)
///                 Interval: Discrete 0.067s (15.000 fps)
///         Size: Discrete 1280x720
///                 Interval: Discrete 0.100s (10.000 fps)
/// [1]: 'MJPG' (Motion-JPEG, compressed)
///         Size: Discrete 1280x720
///                 Interval: Discrete 0.033s (30.000 fps)
/// ```
pub fn parse_v4l2ctl_formats(text: &str) -> Vec<CameraMode> {
    let mut modes: Vec<CameraMode> = Vec::new();
    let mut current_format: Option<String> = None;
    let mut current_size: Option<(u32, u32)> = None;

    for line in text.lines() {
        let trimmed = line.trim();

        // Format header: [N]: 'FOURCC' (description)
        if trimmed.starts_with('[') {
            if let Some(fmt) = extract_v4l2ctl_fourcc(trimmed) {
                current_format = Some(normalise_format(&fmt));
                current_size = None;
            }
            continue;
        }

        // Size line: "Size: Discrete WxH"
        if let Some(rest) = trimmed.strip_prefix("Size: Discrete ") {
            current_size = parse_wxh(rest.split_whitespace().next().unwrap_or(""));
            continue;
        }

        // Interval line: "Interval: Discrete 0.033s (30.000 fps)"
        if trimmed.starts_with("Interval: Discrete ") {
            if let (Some(fmt), Some((w, h))) = (&current_format, current_size) {
                if let Some(fps) = extract_fps_from_interval(trimmed) {
                    upsert_mode(&mut modes, fmt.clone(), w, h, fps);
                }
            }
        }
    }

    modes
}

/// Parse the stderr of `ffmpeg -f v4l2 -list_formats all -i /dev/videoN`.
///
/// Example lines:
/// ```text
/// [in#0 @ 0x…] Raw       :     yuyv422 :           YUYV 4:2:2 : 640x480 640x360 1280x720
/// [in#0 @ 0x…] Compressed:       mjpeg :          Motion-JPEG : 640x480 1280x720
/// ```
pub fn parse_ffmpeg_v4l2_formats(text: &str) -> Vec<CameraMode> {
    let mut modes: Vec<CameraMode> = Vec::new();

    for line in text.lines() {
        // Strip "[in#0 @ 0x…] " prefix.
        let content = match line.find("] ") {
            Some(idx) => line[idx + 2..].trim(),
            None => continue,
        };

        // Expect 4 colon-separated fields: type : fmt_id : description : resolutions
        let parts: Vec<&str> = content.splitn(4, ':').collect();
        if parts.len() < 4 {
            continue;
        }

        let fmt_id = parts[1].trim();
        if fmt_id.is_empty() {
            continue;
        }
        let format = normalise_format(fmt_id);

        // Resolutions field: "640x480 640x360 1280x720"
        // FFmpeg doesn't report per-resolution fps here; assume 30 as safe default.
        for token in parts[3].split_whitespace() {
            if let Some((w, h)) = parse_wxh(token) {
                upsert_mode(&mut modes, format.clone(), w, h, 30);
            }
        }
    }

    modes
}

// ---------------------------------------------------------------------------
// dshow parsing
// ---------------------------------------------------------------------------

fn probe_dshow_via_ffmpeg(ffmpeg_input: &str, ffmpeg_path: &Path) -> Option<Vec<CameraMode>> {
    let output = Command::new(ffmpeg_path)
        .args([
            "-hide_banner",
            "-f",
            "dshow",
            "-list_options",
            "true",
            "-i",
            ffmpeg_input,
        ])
        .output()
        .ok()?;

    let text = String::from_utf8_lossy(&output.stderr);
    Some(parse_dshow_options(&text))
}

/// Parse `ffmpeg -f dshow -list_options true -i video=<device>` stderr.
///
/// Example lines (after `[in#0 @ …] ` prefix):
/// ```text
///  Pin "Capture" (alternative pin name "Capture")
///   pixel_format=bgr24  min s=640x480 fps=30 max s=640x480 fps=30
///   pixel_format=yuyv422  min s=640x480 fps=30 max s=640x480 fps=30
///   vcodec=mjpeg  min s=1280x720 fps=30 max s=1280x720 fps=30
/// ```
pub fn parse_dshow_options(text: &str) -> Vec<CameraMode> {
    let mut modes: Vec<CameraMode> = Vec::new();

    for line in text.lines() {
        let content = match line.find("] ") {
            Some(idx) => line[idx + 2..].trim(),
            None => continue,
        };

        // Determine format from "pixel_format=X" or "vcodec=X".
        let format = if let Some(rest) = content.strip_prefix("pixel_format=") {
            normalise_format(rest.split_whitespace().next().unwrap_or(""))
        } else if let Some(rest) = content.strip_prefix("vcodec=") {
            normalise_format(rest.split_whitespace().next().unwrap_or(""))
        } else {
            continue;
        };

        if format.is_empty() {
            continue;
        }

        // Extract "max s=WxH fps=N"
        if let (Some(size), Some(fps)) = (
            extract_dshow_max_size(content),
            extract_dshow_max_fps(content),
        ) {
            upsert_mode(&mut modes, format, size.0, size.1, fps);
        }
    }

    modes
}

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

fn extract_v4l2ctl_fourcc(line: &str) -> Option<String> {
    // Line: "[0]: 'MJPG' (Motion-JPEG, compressed)"
    let start = line.find('\'')?;
    let rest = &line[start + 1..];
    let end = rest.find('\'')?;
    Some(rest[..end].to_string())
}

fn extract_fps_from_interval(line: &str) -> Option<u32> {
    // "Interval: Discrete 0.033s (30.000 fps)"
    let open = line.find('(')?;
    let rest = &line[open + 1..];
    let space = rest.find(' ')?;
    let fps_str = &rest[..space];
    let fps: f32 = fps_str.parse().ok()?;
    Some(fps.round() as u32)
}

fn extract_dshow_max_size(content: &str) -> Option<(u32, u32)> {
    // "… max s=640x480 fps=30"
    let idx = content.rfind("max s=")?;
    let rest = &content[idx + "max s=".len()..];
    parse_wxh(rest.split_whitespace().next()?)
}

fn extract_dshow_max_fps(content: &str) -> Option<u32> {
    // Find "fps=N" that comes after "max s="
    let max_idx = content.rfind("max s=")?;
    let rest = &content[max_idx..];
    let fps_idx = rest.find("fps=")?;
    let rest = &rest[fps_idx + "fps=".len()..];
    rest.split_whitespace().next()?.parse().ok()
}

/// Insert a new mode or update fps_max if the entry already exists.
fn upsert_mode(modes: &mut Vec<CameraMode>, format: String, w: u32, h: u32, fps: u32) {
    if let Some(existing) = modes
        .iter_mut()
        .find(|m| m.format == format && m.width == w && m.height == h)
    {
        if fps > existing.fps_max {
            existing.fps_max = fps;
        }
    } else {
        modes.push(CameraMode {
            format,
            width: w,
            height: h,
            fps_max: fps,
        });
    }
}

fn parse_wxh(s: &str) -> Option<(u32, u32)> {
    let (w, h) = s.split_once('x')?;
    Some((w.trim().parse().ok()?, h.trim().parse().ok()?))
}

fn normalise_format(raw: &str) -> String {
    match raw.to_lowercase().as_str() {
        "mjpg" | "mjpeg" => "mjpeg".to_string(),
        "yuyv" | "yuyv422" => "yuyv422".to_string(),
        "uyvy422" => "uyvy422".to_string(),
        "nv12" => "nv12".to_string(),
        "h264" => "h264".to_string(),
        "bgr24" | "bgr" => "bgr24".to_string(),
        "rgb24" | "rgb" => "rgb24".to_string(),
        other => other.to_string(),
    }
}

fn dshow_input_name(device: &str) -> String {
    if device.starts_with("video=") {
        device.to_string()
    } else {
        format!("video={device}")
    }
}

// ---------------------------------------------------------------------------
// Unit tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    // --- v4l2-ctl parsing ---

    #[test]
    fn parses_v4l2ctl_yuyv_and_mjpeg() {
        let text = "\
ioctl: VIDIOC_ENUM_FMT\n\
        Type: Video Capture\n\
\n\
        [0]: 'YUYV' (YUYV 4:2:2)\n\
                Size: Discrete 640x480\n\
                        Interval: Discrete 0.033s (30.000 fps)\n\
                        Interval: Discrete 0.067s (15.000 fps)\n\
                Size: Discrete 1280x720\n\
                        Interval: Discrete 0.100s (10.000 fps)\n\
        [1]: 'MJPG' (Motion-JPEG, compressed)\n\
                Size: Discrete 640x480\n\
                        Interval: Discrete 0.033s (30.000 fps)\n\
                Size: Discrete 1280x720\n\
                        Interval: Discrete 0.033s (30.000 fps)\n\
";
        let modes = parse_v4l2ctl_formats(text);

        // Should have: YUYV 640x480@30, YUYV 1280x720@10, MJPG 640x480@30, MJPG 1280x720@30
        assert_eq!(modes.len(), 4);

        let yuyv_hd = modes
            .iter()
            .find(|m| m.format == "yuyv422" && m.width == 1280)
            .unwrap();
        assert_eq!(yuyv_hd.fps_max, 10);

        let mjpeg_hd = modes
            .iter()
            .find(|m| m.format == "mjpeg" && m.width == 1280)
            .unwrap();
        assert_eq!(mjpeg_hd.fps_max, 30);
    }

    #[test]
    fn v4l2ctl_fps_max_is_highest_interval() {
        let text = "\
        [0]: 'YUYV' (YUYV 4:2:2)\n\
                Size: Discrete 640x480\n\
                        Interval: Discrete 0.067s (15.000 fps)\n\
                        Interval: Discrete 0.033s (30.000 fps)\n\
";
        let modes = parse_v4l2ctl_formats(text);
        assert_eq!(modes.len(), 1);
        assert_eq!(modes[0].fps_max, 30);
    }

    // --- ffmpeg v4l2 list_formats parsing ---

    #[test]
    fn parses_ffmpeg_v4l2_format_list() {
        let text = "\
[in#0 @ 0x5555] Raw       :     yuyv422 :           YUYV 4:2:2 : 640x480 1280x720\n\
[in#0 @ 0x5555] Compressed:       mjpeg :          Motion-JPEG : 640x480 1280x720\n\
";
        let modes = parse_ffmpeg_v4l2_formats(text);
        assert_eq!(modes.len(), 4);
        assert!(modes.iter().any(|m| m.format == "mjpeg" && m.width == 1280));
        assert!(modes
            .iter()
            .any(|m| m.format == "yuyv422" && m.width == 640));
    }

    // --- dshow parsing ---

    #[test]
    fn parses_dshow_pixel_formats() {
        let text = "\
[in#0 @ 000001] DirectShow video device options (from video devices)\n\
[in#0 @ 000001]  Pin \"Capture\" (alternative pin name \"Capture\")\n\
[in#0 @ 000001]   pixel_format=bgr24  min s=640x480 fps=30 max s=640x480 fps=30\n\
[in#0 @ 000001]   pixel_format=yuyv422  min s=640x480 fps=30 max s=640x480 fps=30\n\
";
        let modes = parse_dshow_options(text);
        assert_eq!(modes.len(), 2);
        assert!(modes
            .iter()
            .any(|m| m.format == "bgr24" && m.width == 640 && m.fps_max == 30));
        assert!(modes
            .iter()
            .any(|m| m.format == "yuyv422" && m.width == 640));
    }

    #[test]
    fn parses_dshow_vcodec_mjpeg() {
        let text = "\
[in#0 @ 000001]   vcodec=mjpeg  min s=1280x720 fps=30 max s=1280x720 fps=30\n\
";
        let modes = parse_dshow_options(text);
        assert_eq!(modes.len(), 1);
        assert_eq!(modes[0].format, "mjpeg");
        assert_eq!(modes[0].width, 1280);
        assert_eq!(modes[0].fps_max, 30);
    }

    // --- mode selection ---

    #[test]
    fn best_mode_prefers_mjpeg_over_yuyv() {
        let probe = ProbeResult {
            modes: vec![
                CameraMode {
                    format: "yuyv422".into(),
                    width: 1280,
                    height: 720,
                    fps_max: 10,
                },
                CameraMode {
                    format: "mjpeg".into(),
                    width: 1280,
                    height: 720,
                    fps_max: 30,
                },
            ],
        };
        let best = probe.best_mode(1280, 720, 30).unwrap();
        assert_eq!(best.format, "mjpeg");
        assert_eq!(best.fps_max, 30);
    }

    #[test]
    fn best_mode_falls_back_to_lower_res_when_preferred_unavailable() {
        // Windows-like: only 640x480 available, user wants 1280x720.
        let probe = ProbeResult {
            modes: vec![
                CameraMode {
                    format: "bgr24".into(),
                    width: 640,
                    height: 480,
                    fps_max: 30,
                },
                CameraMode {
                    format: "yuyv422".into(),
                    width: 640,
                    height: 480,
                    fps_max: 30,
                },
            ],
        };
        let best = probe.best_mode(1280, 720, 30).unwrap();
        assert_eq!(best.width, 640);
        assert_eq!(best.height, 480);
    }

    #[test]
    fn best_mode_caps_fps_to_preferred() {
        let probe = ProbeResult {
            modes: vec![CameraMode {
                format: "mjpeg".into(),
                width: 640,
                height: 480,
                fps_max: 60,
            }],
        };
        // Prefer 30fps.
        let params = resolve_params_from_probe(&probe, "/dev/video0", 640, 480, 30, "auto");
        assert_eq!(params.fps, 30);
    }

    #[test]
    fn resolve_params_honours_user_pinned_format() {
        let probe = ProbeResult {
            modes: vec![CameraMode {
                format: "mjpeg".into(),
                width: 1280,
                height: 720,
                fps_max: 30,
            }],
        };
        let params = resolve_params_from_probe(&probe, "/dev/video0", 1280, 720, 30, "yuyv422");
        assert_eq!(params.input_format, Some("yuyv422".into()));
        assert!(!params.probed);
    }

    #[test]
    fn resolve_params_falls_back_gracefully_on_empty_probe() {
        let probe = ProbeResult::default();
        let params = resolve_params_from_probe(&probe, "/dev/video0", 1280, 720, 30, "auto");
        assert_eq!(params.width, 1280);
        assert_eq!(params.fps, 30);
        assert!(!params.probed);
    }

    // --- normalise_format ---

    #[test]
    fn normalises_mjpg_to_mjpeg() {
        assert_eq!(normalise_format("MJPG"), "mjpeg");
        assert_eq!(normalise_format("mjpg"), "mjpeg");
        assert_eq!(normalise_format("mjpeg"), "mjpeg");
    }

    #[test]
    fn normalises_yuyv_variants() {
        assert_eq!(normalise_format("YUYV"), "yuyv422");
        assert_eq!(normalise_format("yuyv422"), "yuyv422");
    }
}
