use crate::platform::require_current_platform;
use anyhow::Result;
use std::collections::BTreeSet;
use std::fs;
use std::path::Path;
use std::process::{Command, Output};

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CameraInfo {
    pub device: String,
    pub name: String,
    pub backend: String,
    pub details: String,
}

pub fn enumerate_cameras(ffmpeg_path: Option<&Path>) -> Result<Vec<CameraInfo>> {
    let platform_name = require_current_platform()?;
    let cameras = match platform_name {
        "linux" => enumerate_linux_cameras()?,
        "windows" => enumerate_ffmpeg_cameras(platform_name, ffmpeg_path)?,
        _ => unreachable!(),
    };

    Ok(dedup_cameras(cameras))
}

pub fn first_camera_device(ffmpeg_path: Option<&Path>) -> Option<String> {
    camera_device_candidates(ffmpeg_path).into_iter().next()
}

pub fn camera_device_candidates(ffmpeg_path: Option<&Path>) -> Vec<String> {
    enumerate_cameras(ffmpeg_path)
        .ok()
        .map(|cameras| cameras.into_iter().map(|camera| camera.device).collect())
        .unwrap_or_default()
}

fn dedup_cameras(cameras: Vec<CameraInfo>) -> Vec<CameraInfo> {
    let mut seen = BTreeSet::new();
    cameras
        .into_iter()
        .filter(|camera| seen.insert((camera.device.clone(), camera.name.clone())))
        .collect()
}

fn enumerate_linux_cameras() -> Result<Vec<CameraInfo>> {
    let listed = Command::new("v4l2-ctl").arg("--list-devices").output();
    if let Ok(output) = listed {
        let parsed = parse_v4l2_devices(&String::from_utf8_lossy(&output.stdout));
        if !parsed.is_empty() {
            return Ok(parsed);
        }
    }

    Ok(scan_linux_video_devices())
}

fn enumerate_ffmpeg_cameras(
    platform_name: &str,
    ffmpeg_path: Option<&Path>,
) -> Result<Vec<CameraInfo>> {
    let ffmpeg_bin = ffmpeg_path
        .map(Path::to_path_buf)
        .unwrap_or_else(|| Path::new("ffmpeg").to_path_buf());
    let primary_args = match platform_name {
        "windows" => vec![
            "-hide_banner",
            "-list_devices",
            "true",
            "-f",
            "dshow",
            "-i",
            "dummy",
        ],
        _ => unreachable!(),
    };
    let mut cameras = parse_ffmpeg_device_list(
        platform_name,
        &run_ffmpeg_inventory(&ffmpeg_bin, &primary_args)?,
    );

    if cameras.is_empty() && platform_name == "windows" {
        let fallback_args = ["-hide_banner", "-sources", "dshow"];
        cameras = parse_ffmpeg_device_list(
            platform_name,
            &run_ffmpeg_inventory(&ffmpeg_bin, &fallback_args)?,
        );
    }

    Ok(cameras)
}

fn run_ffmpeg_inventory(ffmpeg_bin: &Path, args: &[&str]) -> Result<String> {
    let output = Command::new(ffmpeg_bin).args(args).output()?;
    Ok(ffmpeg_output_text(&output))
}

fn ffmpeg_output_text(output: &Output) -> String {
    let stderr = String::from_utf8_lossy(&output.stderr);
    let stdout = String::from_utf8_lossy(&output.stdout);

    match (stderr.is_empty(), stdout.is_empty()) {
        (true, false) => stdout.into_owned(),
        (false, true) => stderr.into_owned(),
        (false, false) => format!("{stderr}\n{stdout}"),
        (true, true) => String::new(),
    }
}

pub fn parse_v4l2_devices(text: &str) -> Vec<CameraInfo> {
    let mut cameras = Vec::new();
    let mut current_name = String::new();
    let mut current_devices = Vec::new();

    let mut flush = |name: &mut String, devices: &mut Vec<String>| {
        if name.is_empty() {
            devices.clear();
            return;
        }

        if let Some(device) = devices
            .iter()
            .find(|entry| is_capture_device(entry))
            .cloned()
        {
            cameras.push(CameraInfo {
                device,
                name: name.clone(),
                backend: "v4l2".to_string(),
                details: "linux camera".to_string(),
            });
        }

        name.clear();
        devices.clear();
    };

    for raw in text.lines() {
        if raw.trim().is_empty() {
            continue;
        }

        if raw.starts_with('\t') || raw.starts_with("    ") {
            let candidate = raw.trim().to_string();
            if candidate.starts_with("/dev/video") {
                current_devices.push(candidate);
            }
            continue;
        }

        flush(&mut current_name, &mut current_devices);
        current_name = raw.trim().trim_end_matches(':').to_string();
    }

    flush(&mut current_name, &mut current_devices);
    cameras
}

pub fn parse_ffmpeg_device_list(platform_name: &str, text: &str) -> Vec<CameraInfo> {
    match platform_name {
        "windows" => {
            let mut cameras = parse_dshow_devices(text);
            cameras.extend(parse_dshow_sources(text));
            dedup_cameras(cameras)
        }
        _ => Vec::new(),
    }
}

fn parse_dshow_devices(text: &str) -> Vec<CameraInfo> {
    let mut in_video_section = false;
    let mut cameras: Vec<CameraInfo> = Vec::new();

    for line in text.lines() {
        let lower = line.to_ascii_lowercase();
        if lower.contains("directshow video devices") {
            in_video_section = true;
            continue;
        }
        if lower.contains("directshow audio devices") {
            in_video_section = false;
            continue;
        }
        if !in_video_section {
            continue;
        }

        if lower.contains("alternative name") {
            if let Some(name) = extract_quoted(line) {
                if let Some(index) = cameras.len().checked_sub(1) {
                    let camera_name = cameras[index].name.clone();
                    cameras[index].details = format!("windows camera (alternative device: {name})");
                    cameras.push(CameraInfo {
                        device: format!("video={name}"),
                        name: camera_name,
                        backend: "dshow".to_string(),
                        details: "windows camera alternative device".to_string(),
                    });
                }
            }
            continue;
        }

        if let Some(name) = extract_quoted(line) {
            cameras.push(CameraInfo {
                device: format!("video={name}"),
                name: name.to_string(),
                backend: "dshow".to_string(),
                details: "windows camera".to_string(),
            });
        }
    }

    cameras
}

fn parse_dshow_sources(text: &str) -> Vec<CameraInfo> {
    let mut cameras = Vec::new();

    for line in text.lines() {
        let Some(source) = extract_bracketed_line(line) else {
            continue;
        };
        if !source.to_ascii_lowercase().contains("(video)") {
            continue;
        }

        let name = strip_dshow_source_kind(source);
        if name.is_empty() {
            continue;
        }

        cameras.push(CameraInfo {
            device: format!("video={name}"),
            name: name.to_string(),
            backend: "dshow".to_string(),
            details: "windows camera".to_string(),
        });
    }

    cameras
}

fn extract_quoted(line: &str) -> Option<&str> {
    let start = line.find('"')?;
    let rest = &line[start + 1..];
    let end = rest.find('"')?;
    Some(&rest[..end])
}

fn extract_bracketed_line(line: &str) -> Option<&str> {
    let trimmed = line.trim();
    if !(trimmed.starts_with('[') && trimmed.ends_with(']')) {
        return None;
    }
    Some(&trimmed[1..trimmed.len() - 1])
}

fn strip_dshow_source_kind(source: &str) -> &str {
    source
        .rsplit_once(" (")
        .filter(|(_, suffix)| suffix.eq_ignore_ascii_case("video)"))
        .map(|(name, _)| name.trim())
        .unwrap_or_else(|| source.trim())
}

fn scan_linux_video_devices() -> Vec<CameraInfo> {
    let mut cameras = Vec::new();

    for index in 0..64 {
        let device = format!("/dev/video{index}");
        if !Path::new(&device).exists() || !is_capture_device(&device) {
            continue;
        }

        cameras.push(CameraInfo {
            name: sysfs_device_name(&device).unwrap_or_else(|| device.clone()),
            device,
            backend: "v4l2".to_string(),
            details: "linux camera".to_string(),
        });
    }

    cameras
}

fn sysfs_device_name(device: &str) -> Option<String> {
    let node = Path::new(device).file_name()?.to_string_lossy().to_string();
    let path = format!("/sys/class/video4linux/{node}/name");
    fs::read_to_string(path)
        .ok()
        .map(|name| name.trim().to_string())
}

fn is_capture_device(device: &str) -> bool {
    let node = match Path::new(device).file_name() {
        Some(value) => value.to_string_lossy().to_string(),
        None => return true,
    };
    let name_path = format!("/sys/class/video4linux/{node}/name");
    let Ok(name) = fs::read_to_string(name_path) else {
        return true;
    };
    let lower = name.to_ascii_lowercase();
    ![
        "metadata",
        " output",
        "m2m",
        "stateless",
        "stateful",
        "codec",
        "encoder",
        "decoder",
        "convert",
        "isp",
    ]
    .iter()
    .any(|keyword| lower.contains(keyword))
}

#[cfg(test)]
mod tests {
    use super::{camera_device_candidates, parse_ffmpeg_device_list, parse_v4l2_devices};
    use std::path::Path;

    #[test]
    fn parses_v4l2_inventory() {
        let text = "\
HD Pro Webcam C920 (usb-0000:00:14.0-8):\n\
\t/dev/video0\n\
\t/dev/video1\n\
\n";

        let cameras = parse_v4l2_devices(text);
        assert_eq!(cameras.len(), 1);
        assert_eq!(cameras[0].device, "/dev/video0");
        assert_eq!(cameras[0].backend, "v4l2");
    }

    #[test]
    fn parses_windows_ffmpeg_inventory() {
        let text = "\
[dshow @ 000001] DirectShow video devices\n\
[dshow @ 000001]  \"Integrated Camera\"\n\
[dshow @ 000001]  Alternative name \"@device_pnp_...\"\n";

        let cameras = parse_ffmpeg_device_list("windows", text);
        assert_eq!(cameras.len(), 2);
        assert_eq!(cameras[0].device, "video=Integrated Camera");
        assert_eq!(cameras[0].name, "Integrated Camera");
        assert_eq!(cameras[1].device, "video=@device_pnp_...");
    }

    #[test]
    fn parses_windows_ffmpeg_sources_inventory() {
        let text = "\
[Integrated Camera (video)]\n\
[Microphone Array (audio)]\n";

        let cameras = parse_ffmpeg_device_list("windows", text);
        assert_eq!(cameras.len(), 1);
        assert_eq!(cameras[0].device, "video=Integrated Camera");
        assert_eq!(cameras[0].name, "Integrated Camera");
    }

    #[test]
    fn camera_device_candidates_return_empty_when_inventory_fails() {
        assert!(camera_device_candidates(Some(Path::new("/definitely/missing/ffmpeg"))).is_empty());
    }
}
