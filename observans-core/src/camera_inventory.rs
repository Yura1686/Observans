use crate::platform::require_current_platform;
use anyhow::Result;
use std::collections::BTreeSet;
use std::fs;
use std::path::Path;
use std::process::Command;

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
    enumerate_cameras(ffmpeg_path)
        .ok()
        .and_then(|cameras| cameras.into_iter().next().map(|camera| camera.device))
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
    let args = match platform_name {
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

    let output = Command::new(ffmpeg_bin).args(args).output()?;
    let text = String::from_utf8_lossy(&output.stderr);

    Ok(parse_ffmpeg_device_list(platform_name, &text))
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
        "windows" => parse_dshow_devices(text),
        _ => Vec::new(),
    }
}

fn parse_dshow_devices(text: &str) -> Vec<CameraInfo> {
    let mut in_video_section = false;
    let mut cameras = Vec::new();

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
        if !in_video_section || lower.contains("alternative name") {
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

fn extract_quoted(line: &str) -> Option<&str> {
    let start = line.find('"')?;
    let rest = &line[start + 1..];
    let end = rest.find('"')?;
    Some(&rest[..end])
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
    use super::{first_camera_device, parse_ffmpeg_device_list, parse_v4l2_devices};
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
        assert_eq!(cameras.len(), 1);
        assert_eq!(cameras[0].device, "video=Integrated Camera");
    }

    #[test]
    fn first_camera_device_returns_none_when_probe_fails() {
        assert_eq!(
            first_camera_device(Some(Path::new("/definitely/missing/ffmpeg"))),
            None
        );
    }
}
