pub fn current_platform() -> &'static str {
    std::env::consts::OS
}

pub fn capture_format_for(platform_name: &str) -> &'static str {
    match platform_name {
        "linux" => "v4l2",
        "windows" => "dshow",
        "macos" => "avfoundation",
        _ => "unknown",
    }
}

pub fn default_device_for(platform_name: &str) -> &'static str {
    match platform_name {
        "linux" => "/dev/video0",
        "windows" => "video=Integrated Camera",
        "macos" => "0",
        _ => "auto",
    }
}

#[cfg(test)]
mod tests {
    use super::{capture_format_for, default_device_for};

    #[test]
    fn uses_expected_capture_formats() {
        assert_eq!(capture_format_for("linux"), "v4l2");
        assert_eq!(capture_format_for("windows"), "dshow");
        assert_eq!(capture_format_for("macos"), "avfoundation");
    }

    #[test]
    fn uses_expected_default_devices() {
        assert_eq!(default_device_for("linux"), "/dev/video0");
        assert_eq!(default_device_for("windows"), "video=Integrated Camera");
        assert_eq!(default_device_for("macos"), "0");
    }
}
