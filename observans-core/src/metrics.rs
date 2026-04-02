use crate::config::Config;
use chrono::Local;
use serde::Serialize;
use std::sync::{Arc, Mutex, RwLock};
use std::time::{Duration, Instant};
use sysinfo::System;

#[derive(Debug, Clone, Serialize)]
pub struct MetricsSnapshot {
    pub time: String,
    pub date: String,
    pub cpu: f32,
    pub ram_pct: f32,
    pub ram_used_mb: u64,
    pub ram_total_mb: u64,
    pub temp: f32,
    pub batt: i32,
    pub batt_status: String,
    pub hostname: String,
    pub platform_name: String,
    pub capture_backend: String,
    pub clients: usize,
    pub uptime: String,
    pub res: String,
    pub fps_actual: f32,
    pub fps_target: u32,
    pub stream_pipeline: String,
    pub stream_input: String,
    pub frame_age_ms: i64,
    pub queue_drops: u64,
    pub avg_frame_kb: f32,
    pub restarts: u64,
}

#[derive(Debug)]
struct FrameStats {
    window_started: Instant,
    frames_in_window: u32,
    last_frame_at: Option<Instant>,
    avg_frame_kb: f32,
}

#[derive(Debug)]
struct MetricsInner {
    snapshot: RwLock<MetricsSnapshot>,
    frame_stats: Mutex<FrameStats>,
    started_at: Instant,
}

#[derive(Clone, Debug)]
pub struct SharedMetrics {
    inner: Arc<MetricsInner>,
}

impl SharedMetrics {
    pub fn new(config: &Config) -> Self {
        let now = Local::now();
        let snapshot = MetricsSnapshot {
            time: now.format("%H:%M:%S").to_string(),
            date: now.format("%d.%m.%Y").to_string(),
            cpu: 0.0,
            ram_pct: 0.0,
            ram_used_mb: 0,
            ram_total_mb: 0,
            temp: -1.0,
            batt: -1,
            batt_status: "unavailable".to_string(),
            hostname: System::host_name().unwrap_or_else(|| "localhost".to_string()),
            platform_name: config.platform_name().to_string(),
            capture_backend: config.capture_backend_label(),
            clients: 0,
            uptime: "00:00:00".to_string(),
            res: format!("{}x{}", config.width, config.height),
            fps_actual: 0.0,
            fps_target: config.fps,
            stream_pipeline: "ffmpeg-cli -> mjpeg -> broadcast".to_string(),
            stream_input: "awaiting camera".to_string(),
            frame_age_ms: -1,
            queue_drops: 0,
            avg_frame_kb: 0.0,
            restarts: 0,
        };

        Self {
            inner: Arc::new(MetricsInner {
                snapshot: RwLock::new(snapshot),
                frame_stats: Mutex::new(FrameStats {
                    window_started: Instant::now(),
                    frames_in_window: 0,
                    last_frame_at: None,
                    avg_frame_kb: 0.0,
                }),
                started_at: Instant::now(),
            }),
        }
    }

    pub fn snapshot(&self) -> MetricsSnapshot {
        let mut snapshot = self
            .inner
            .snapshot
            .read()
            .expect("metrics read lock poisoned")
            .clone();
        snapshot.uptime = format_uptime(self.inner.started_at.elapsed());
        snapshot.frame_age_ms = self.frame_age_ms();
        snapshot.avg_frame_kb = self
            .inner
            .frame_stats
            .lock()
            .expect("frame stats lock poisoned")
            .avg_frame_kb;
        snapshot
    }

    pub fn note_frame(&self, frame_len: usize, width: u32, height: u32) {
        let now = Instant::now();
        let mut frame_stats = self
            .inner
            .frame_stats
            .lock()
            .expect("frame stats lock poisoned");
        frame_stats.frames_in_window += 1;
        frame_stats.last_frame_at = Some(now);
        let frame_kb = frame_len as f32 / 1024.0;
        frame_stats.avg_frame_kb = if frame_stats.avg_frame_kb == 0.0 {
            frame_kb
        } else {
            (frame_stats.avg_frame_kb * 0.88) + (frame_kb * 0.12)
        };

        let elapsed = frame_stats.window_started.elapsed().as_secs_f32();
        if elapsed >= 1.0 {
            let fps_actual = frame_stats.frames_in_window as f32 / elapsed;
            frame_stats.window_started = now;
            frame_stats.frames_in_window = 0;

            let mut snapshot = self
                .inner
                .snapshot
                .write()
                .expect("metrics write lock poisoned");
            snapshot.fps_actual = fps_actual;
        }

        let mut snapshot = self
            .inner
            .snapshot
            .write()
            .expect("metrics write lock poisoned");
        snapshot.res = format!("{width}x{height}");
        snapshot.avg_frame_kb = frame_stats.avg_frame_kb;
    }

    pub fn set_stream_input(&self, stream_input: impl Into<String>) {
        self.inner
            .snapshot
            .write()
            .expect("metrics write lock poisoned")
            .stream_input = stream_input.into();
    }

    pub fn set_clients(&self, clients: usize) {
        self.inner
            .snapshot
            .write()
            .expect("metrics write lock poisoned")
            .clients = clients;
    }

    pub fn note_queue_drop(&self, dropped: u64) {
        let mut snapshot = self
            .inner
            .snapshot
            .write()
            .expect("metrics write lock poisoned");
        snapshot.queue_drops += dropped;
    }

    pub fn note_restart(&self) {
        let mut snapshot = self
            .inner
            .snapshot
            .write()
            .expect("metrics write lock poisoned");
        snapshot.restarts += 1;
    }

    pub fn refresh_system(&self, system: &System) {
        let now = Local::now();
        let total_memory = system.total_memory();
        let used_memory = system.used_memory();
        let ram_pct = if total_memory == 0 {
            0.0
        } else {
            (used_memory as f32 / total_memory as f32) * 100.0
        };

        let mut snapshot = self
            .inner
            .snapshot
            .write()
            .expect("metrics write lock poisoned");
        snapshot.time = now.format("%H:%M:%S").to_string();
        snapshot.date = now.format("%d.%m.%Y").to_string();
        snapshot.cpu = system.global_cpu_usage();
        snapshot.ram_pct = ram_pct;
        snapshot.ram_used_mb = used_memory / 1024 / 1024;
        snapshot.ram_total_mb = total_memory / 1024 / 1024;
    }

    fn frame_age_ms(&self) -> i64 {
        let frame_stats = self
            .inner
            .frame_stats
            .lock()
            .expect("frame stats lock poisoned");
        frame_stats
            .last_frame_at
            .map(|instant| instant.elapsed().as_millis() as i64)
            .unwrap_or(-1)
    }
}

pub fn spawn_system_sampler(metrics: SharedMetrics) -> tokio::task::JoinHandle<()> {
    tokio::spawn(async move {
        let mut system = System::new_all();
        system.refresh_cpu_usage();

        let mut interval = tokio::time::interval(Duration::from_secs(1));
        loop {
            interval.tick().await;
            system.refresh_cpu_usage();
            system.refresh_memory();
            metrics.refresh_system(&system);
        }
    })
}

fn format_uptime(duration: Duration) -> String {
    let total_seconds = duration.as_secs();
    let hours = total_seconds / 3600;
    let minutes = (total_seconds % 3600) / 60;
    let seconds = total_seconds % 60;
    format!("{hours:02}:{minutes:02}:{seconds:02}")
}

#[cfg(test)]
mod tests {
    use super::SharedMetrics;
    use crate::config::Config;
    use clap::Parser;

    #[test]
    fn preserves_fallback_temperature_and_battery_values() {
        let config = Config::try_parse_from(["observans"]).unwrap();
        let metrics = SharedMetrics::new(&config);
        let snapshot = metrics.snapshot();

        assert_eq!(snapshot.temp, -1.0);
        assert_eq!(snapshot.batt, -1);
        assert_eq!(snapshot.batt_status, "unavailable");
    }
}
