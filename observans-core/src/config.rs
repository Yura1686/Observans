use crate::bootstrap::patch_args_for_camera_selection;
use crate::camera_inventory::enumerate_cameras;
use crate::platform::{capture_format_for, default_device_for, require_current_platform};
use crate::runtime::resolve_ffmpeg_for_current_process;
use crate::tui::{choose_camera, terminal_is_interactive};
use anyhow::Result;
use clap::error::ErrorKind;
use clap::Parser;
use std::net::SocketAddr;

#[derive(Debug, Clone, Parser)]
#[command(
    name = "observans",
    about = "Linux and Windows camera streaming with a startup TUI"
)]
pub struct Config {
    #[arg(long, default_value_t = 8080)]
    pub port: u16,
    #[arg(
        long,
        default_value = "auto",
        help = "Capture device path or name. Use auto to resolve the first discovered camera."
    )]
    pub device: String,
    #[arg(long, default_value_t = 1280)]
    pub width: u32,
    #[arg(long, default_value_t = 720)]
    pub height: u32,
    #[arg(long, default_value_t = 30)]
    pub fps: u32,
    #[arg(
        long,
        default_value = "auto",
        value_parser = ["auto", "mjpeg", "yuyv422", "uyvy422", "nv12", "h264"]
    )]
    pub input_format: String,
    #[arg(long, default_value_t = false)]
    pub no_camera_select: bool,
}

impl Config {
    pub fn from_args_with_bootstrap<I>(args: I) -> Result<Self>
    where
        I: IntoIterator,
        I::Item: ToString,
    {
        let raw_args = args
            .into_iter()
            .map(|arg| arg.to_string())
            .collect::<Vec<_>>();
        let platform_name = require_current_platform()?;
        let interactive = terminal_is_interactive();
        let ffmpeg_path = resolve_ffmpeg_for_current_process(platform_name);
        let patched_args = patch_args_for_camera_selection(
            raw_args,
            interactive,
            || enumerate_cameras(Some(ffmpeg_path.as_path())),
            choose_camera,
        )?;

        match Self::try_parse_from(patched_args) {
            Ok(config) => Ok(config),
            Err(error)
                if matches!(
                    error.kind(),
                    ErrorKind::DisplayHelp | ErrorKind::DisplayVersion
                ) =>
            {
                error.print()?;
                std::process::exit(0);
            }
            Err(error) => Err(error.into()),
        }
    }

    pub fn platform_name(&self) -> &'static str {
        require_current_platform().expect("unsupported platform should be rejected during startup")
    }

    pub fn capture_format(&self) -> &'static str {
        capture_format_for(self.platform_name())
            .expect("unsupported platform should be rejected during startup")
    }

    pub fn platform_default_device(&self) -> &'static str {
        default_device_for(self.platform_name())
            .expect("unsupported platform should be rejected during startup")
    }

    pub fn bind_addr(&self) -> SocketAddr {
        SocketAddr::from(([0, 0, 0, 0], self.port))
    }

    pub fn capture_backend_label(&self) -> String {
        format!("ffmpeg-cli/{}", self.capture_format())
    }
}
