use anyhow::{bail, Result};

pub fn current_platform() -> &'static str {
    std::env::consts::OS
}

pub fn require_supported_platform(platform_name: &str) -> Result<&'static str> {
    match platform_name {
        "linux" => Ok("linux"),
        "windows" => Ok("windows"),
        _ => bail!("Observans supports only Linux and Windows. Current platform: {platform_name}"),
    }
}

pub fn require_current_platform() -> Result<&'static str> {
    require_supported_platform(current_platform())
}

pub fn capture_format_for(platform_name: &str) -> Result<&'static str> {
    Ok(match require_supported_platform(platform_name)? {
        "linux" => "v4l2",
        "windows" => "dshow",
        _ => unreachable!(),
    })
}

pub fn default_device_for(platform_name: &str) -> Result<&'static str> {
    Ok(match require_supported_platform(platform_name)? {
        "linux" => "/dev/video0",
        "windows" => "video=Integrated Camera",
        _ => unreachable!(),
    })
}

#[cfg(test)]
mod tests {
    use super::{capture_format_for, default_device_for, require_supported_platform};

    #[test]
    fn uses_expected_capture_formats() {
        assert_eq!(capture_format_for("linux").unwrap(), "v4l2");
        assert_eq!(capture_format_for("windows").unwrap(), "dshow");
    }

    #[test]
    fn uses_expected_default_devices() {
        assert_eq!(default_device_for("linux").unwrap(), "/dev/video0");
        assert_eq!(
            default_device_for("windows").unwrap(),
            "video=Integrated Camera"
        );
    }

    #[test]
    fn rejects_unsupported_platforms() {
        let error = require_supported_platform("macos").unwrap_err().to_string();
        assert!(error.contains("Linux and Windows"));
        assert!(error.contains("macos"));
        assert!(capture_format_for("macos").is_err());
        assert!(default_device_for("macos").is_err());
    }
}
