use crate::camera_inventory::CameraInfo;
use anyhow::Result;

pub fn patch_args_for_camera_selection<E, P>(
    args: Vec<String>,
    interactive: bool,
    enumerate: E,
    pick: P,
) -> Result<Vec<String>>
where
    E: FnOnce() -> Result<Vec<CameraInfo>>,
    P: FnOnce(&[CameraInfo]) -> Result<Option<String>>,
{
    if has_device_flag(&args) || has_flag(&args, "--no-camera-select") {
        return Ok(args);
    }

    if !interactive {
        return Ok(args);
    }

    let cameras = match enumerate() {
        Ok(cameras) => cameras,
        Err(_) => return Ok(args),
    };
    if cameras.is_empty() {
        return Ok(patch_device(args, "auto"));
    }

    match pick(&cameras).unwrap_or(None) {
        Some(device) => Ok(patch_device(args, &device)),
        None => Ok(args),
    }
}

fn has_device_flag(args: &[String]) -> bool {
    args.iter()
        .skip(1)
        .any(|arg| arg == "--device" || arg.starts_with("--device="))
}

fn has_flag(args: &[String], needle: &str) -> bool {
    args.iter().skip(1).any(|arg| arg == needle)
}

fn patch_device(mut args: Vec<String>, device: &str) -> Vec<String> {
    args.push("--device".to_string());
    args.push(device.to_string());
    args
}

#[cfg(test)]
mod tests {
    use super::patch_args_for_camera_selection;
    use crate::camera_inventory::CameraInfo;

    fn sample_camera() -> CameraInfo {
        CameraInfo {
            device: "/dev/video7".to_string(),
            name: "Camera".to_string(),
            backend: "v4l2".to_string(),
            details: "test".to_string(),
        }
    }

    #[test]
    fn skips_when_device_is_already_present() {
        let args = vec!["observans".into(), "--device".into(), "auto".into()];
        let patched = patch_args_for_camera_selection(
            args.clone(),
            true,
            || Ok(vec![sample_camera()]),
            |_| Ok(Some("/dev/video7".into())),
        )
        .unwrap();

        assert_eq!(patched, args);
    }

    #[test]
    fn skips_when_not_interactive() {
        let args = vec!["observans".into()];
        let patched = patch_args_for_camera_selection(
            args.clone(),
            false,
            || Ok(vec![sample_camera()]),
            |_| Ok(Some("/dev/video7".into())),
        )
        .unwrap();

        assert_eq!(patched, args);
    }

    #[test]
    fn falls_back_to_auto_when_no_cameras_are_found() {
        let args = vec!["observans".into()];
        let patched =
            patch_args_for_camera_selection(args, true, || Ok(Vec::new()), |_| Ok(None)).unwrap();

        assert_eq!(patched, vec!["observans", "--device", "auto"]);
    }

    #[test]
    fn injects_selected_camera_into_final_args() {
        let patched = patch_args_for_camera_selection(
            vec!["observans".into()],
            true,
            || Ok(vec![sample_camera()]),
            |_| Ok(Some("/dev/video7".into())),
        )
        .unwrap();

        assert_eq!(patched, vec!["observans", "--device", "/dev/video7"]);
    }

    #[test]
    fn keeps_startup_alive_when_camera_enumeration_fails() {
        let patched = patch_args_for_camera_selection(
            vec!["observans".into()],
            true,
            || Err(anyhow::anyhow!("ffmpeg probe failed")),
            |_| Ok(Some("/dev/video7".into())),
        )
        .unwrap();

        assert_eq!(patched, vec!["observans"]);
    }

    #[test]
    fn keeps_startup_alive_when_picker_fails() {
        let patched = patch_args_for_camera_selection(
            vec!["observans".into()],
            true,
            || Ok(vec![sample_camera()]),
            |_| Err(anyhow::anyhow!("tui failed")),
        )
        .unwrap();

        assert_eq!(patched, vec!["observans"]);
    }
}
