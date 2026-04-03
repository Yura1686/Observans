pub mod bootstrap;
pub mod camera_inventory;
pub mod capture;
pub mod config;
pub mod metrics;
pub mod platform;
pub mod probe;
pub mod runtime;
pub mod tui;

pub use bootstrap::patch_args_for_camera_selection;
pub use camera_inventory::{enumerate_cameras, CameraInfo};
pub use capture::start_capture;
pub use config::Config;
pub use metrics::{spawn_system_sampler, MetricsSnapshot, SharedMetrics};
pub use probe::{probe_dshow, probe_v4l2, CameraMode, ProbeResult, ResolvedCaptureParams};