use serde::Deserialize;
use std::fs;
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};
use sysinfo::Components;

const BATTERY_REFRESH_INTERVAL: Duration = Duration::from_secs(30);

#[derive(Debug, Clone)]
pub struct SensorReading {
    pub temp_c: Option<f32>,
    pub battery_pct: Option<i32>,
    pub battery_status: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct BatteryReading {
    percent: i32,
    status: String,
}

#[derive(Debug)]
pub struct SensorSampler {
    components: Components,
    battery_cache: Option<BatteryReading>,
    last_battery_poll: Option<Instant>,
}

impl SensorSampler {
    pub fn new() -> Self {
        Self {
            components: Components::new_with_refreshed_list(),
            battery_cache: None,
            last_battery_poll: None,
        }
    }

    pub fn sample(&mut self) -> SensorReading {
        let temp_c = sample_temperature(&mut self.components);
        self.refresh_battery_if_due();

        SensorReading {
            temp_c,
            battery_pct: self.battery_cache.as_ref().map(|reading| reading.percent),
            battery_status: self
                .battery_cache
                .as_ref()
                .map(|reading| reading.status.clone()),
        }
    }

    fn refresh_battery_if_due(&mut self) {
        let should_refresh = self
            .last_battery_poll
            .map(|last_poll| last_poll.elapsed() >= BATTERY_REFRESH_INTERVAL)
            .unwrap_or(true);

        if !should_refresh {
            return;
        }

        self.last_battery_poll = Some(Instant::now());
        if let Some(battery) = read_battery() {
            self.battery_cache = Some(battery);
        }
    }
}

fn sample_temperature(components: &mut Components) -> Option<f32> {
    components.refresh(false);
    pick_component_temperature(components).or_else(platform_temperature_fallback)
}

fn pick_component_temperature(components: &Components) -> Option<f32> {
    components
        .iter()
        .filter_map(|component| component.temperature())
        .filter_map(sanitize_temperature)
        .max_by(|left, right| left.total_cmp(right))
}

fn sanitize_temperature(temp_c: f32) -> Option<f32> {
    if temp_c.is_finite() && (0.0..140.0).contains(&temp_c) {
        Some((temp_c * 10.0).round() / 10.0)
    } else {
        None
    }
}

#[cfg(target_os = "linux")]
fn platform_temperature_fallback() -> Option<f32> {
    read_linux_temperature_root(Path::new("/sys/class/thermal"))
}

#[cfg(not(target_os = "linux"))]
fn platform_temperature_fallback() -> Option<f32> {
    None
}

#[cfg(target_os = "linux")]
fn read_battery() -> Option<BatteryReading> {
    read_linux_battery_root(Path::new("/sys/class/power_supply"))
}

#[cfg(target_os = "windows")]
fn read_battery() -> Option<BatteryReading> {
    read_windows_battery()
}

#[cfg(not(any(target_os = "linux", target_os = "windows")))]
fn read_battery() -> Option<BatteryReading> {
    None
}

#[cfg(target_os = "linux")]
fn read_linux_temperature_root(root: &Path) -> Option<f32> {
    let mut hottest = None;

    for entry in fs::read_dir(root).ok()? {
        let path = entry.ok()?.path();
        let file_name = path.file_name()?.to_str()?;
        if !file_name.starts_with("thermal_zone") {
            continue;
        }

        let temp = read_trimmed(path.join("temp"))
            .and_then(|raw| parse_linux_temperature(&raw))
            .and_then(sanitize_temperature);

        if let Some(temp) = temp {
            hottest = Some(match hottest {
                Some(current) if current > temp => current,
                _ => temp,
            });
        }
    }

    hottest
}

#[cfg(target_os = "linux")]
fn parse_linux_temperature(raw: &str) -> Option<f32> {
    let raw = raw.trim().parse::<f32>().ok()?;
    let temp_c = if raw > 200.0 { raw / 1000.0 } else { raw };
    Some(temp_c)
}

#[cfg(target_os = "linux")]
fn read_linux_battery_root(root: &Path) -> Option<BatteryReading> {
    let mut candidates = battery_candidate_dirs(root);
    candidates.sort();
    candidates
        .into_iter()
        .find_map(|battery_dir| read_linux_battery_dir(&battery_dir))
}

#[cfg(target_os = "linux")]
fn battery_candidate_dirs(root: &Path) -> Vec<PathBuf> {
    let mut candidates = Vec::new();

    if let Ok(entries) = fs::read_dir(root) {
        for entry in entries.flatten() {
            let path = entry.path();
            if !path.is_dir() {
                continue;
            }

            let file_name = path.file_name().and_then(|name| name.to_str()).unwrap_or("");
            let type_name = read_trimmed(path.join("type")).unwrap_or_default();
            if file_name.starts_with("BAT") || type_name.eq_ignore_ascii_case("battery") {
                candidates.push(path);
            }
        }
    }

    candidates
}

#[cfg(target_os = "linux")]
fn read_linux_battery_dir(path: &Path) -> Option<BatteryReading> {
    let percent = read_trimmed(path.join("capacity"))
        .and_then(|raw| raw.parse::<i32>().ok())
        .or_else(|| compute_linux_battery_percent(path))?
        .clamp(0, 100);

    let status = read_trimmed(path.join("status"))
        .map(|raw| normalize_linux_battery_status(&raw))
        .unwrap_or_else(|| "unknown".to_string());

    Some(BatteryReading { percent, status })
}

#[cfg(target_os = "linux")]
fn compute_linux_battery_percent(path: &Path) -> Option<i32> {
    let now = read_numeric(path.join("energy_now"))
        .or_else(|| read_numeric(path.join("charge_now")))
        .or_else(|| read_numeric(path.join("current_now")))?;
    let full = read_numeric(path.join("energy_full"))
        .or_else(|| read_numeric(path.join("charge_full")))
        .or_else(|| read_numeric(path.join("current_max")))?;

    if full <= 0.0 {
        return None;
    }

    Some(((now / full) * 100.0).round() as i32)
}

#[cfg(target_os = "linux")]
fn normalize_linux_battery_status(raw: &str) -> String {
    match raw.trim().to_ascii_lowercase().as_str() {
        "charging" => "charging".to_string(),
        "discharging" => "discharging".to_string(),
        "full" => "full".to_string(),
        "not charging" => "plugged".to_string(),
        "unknown" => "unknown".to_string(),
        other => other.to_string(),
    }
}

#[cfg(target_os = "windows")]
fn read_windows_battery() -> Option<BatteryReading> {
    let output = std::process::Command::new("powershell.exe")
        .args([
            "-NoProfile",
            "-NonInteractive",
            "-Command",
            "$b = Get-CimInstance Win32_Battery -ErrorAction SilentlyContinue | Select-Object -First 1 EstimatedChargeRemaining, BatteryStatus; if ($null -eq $b) { 'null' } else { $b | ConvertTo-Json -Compress }",
        ])
        .output()
        .ok()?;

    if !output.status.success() {
        return None;
    }

    parse_windows_battery_json(&String::from_utf8_lossy(&output.stdout))
}

#[cfg_attr(not(any(test, target_os = "windows")), allow(dead_code))]
fn parse_windows_battery_json(raw: &str) -> Option<BatteryReading> {
    let trimmed = raw.trim().trim_start_matches('\u{feff}');
    if trimmed.is_empty() || trimmed.eq_ignore_ascii_case("null") {
        return None;
    }

    let payload = match serde_json::from_str::<WindowsBatteryEnvelope>(trimmed).ok()? {
        WindowsBatteryEnvelope::One(payload) => payload,
        WindowsBatteryEnvelope::Many(payloads) => payloads.into_iter().next()?,
    };

    Some(BatteryReading {
        percent: payload.estimated_charge_remaining?.clamp(0, 100),
        status: normalize_windows_battery_status(payload.battery_status),
    })
}

#[cfg_attr(not(any(test, target_os = "windows")), allow(dead_code))]
fn normalize_windows_battery_status(code: Option<i32>) -> String {
    match code.unwrap_or_default() {
        1 => "discharging".to_string(),
        2 => "plugged".to_string(),
        3 => "full".to_string(),
        4 | 5 => "low".to_string(),
        6 | 7 | 8 | 9 => "charging".to_string(),
        11 => "partially charged".to_string(),
        _ => "unknown".to_string(),
    }
}

#[cfg_attr(not(any(test, target_os = "windows")), allow(dead_code))]
#[derive(Debug, Deserialize)]
#[serde(untagged)]
enum WindowsBatteryEnvelope {
    One(WindowsBatteryPayload),
    Many(Vec<WindowsBatteryPayload>),
}

#[cfg_attr(not(any(test, target_os = "windows")), allow(dead_code))]
#[derive(Debug, Deserialize)]
struct WindowsBatteryPayload {
    #[serde(rename = "EstimatedChargeRemaining")]
    estimated_charge_remaining: Option<i32>,
    #[serde(rename = "BatteryStatus")]
    battery_status: Option<i32>,
}

fn read_trimmed(path: PathBuf) -> Option<String> {
    Some(fs::read_to_string(path).ok()?.trim().to_string())
}

#[cfg(target_os = "linux")]
fn read_numeric(path: PathBuf) -> Option<f64> {
    read_trimmed(path)?.parse::<f64>().ok()
}

#[cfg(test)]
mod tests {
    use super::{
        normalize_windows_battery_status, parse_windows_battery_json, read_trimmed,
        sanitize_temperature, BatteryReading,
    };
    #[cfg(target_os = "linux")]
    use super::{
        normalize_linux_battery_status, parse_linux_temperature, read_linux_battery_root,
        read_linux_temperature_root,
    };
    use std::fs;
    use std::path::{Path, PathBuf};
    use std::time::{SystemTime, UNIX_EPOCH};

    #[cfg(target_os = "linux")]
    #[test]
    fn reads_linux_battery_capacity_and_status() {
        let root = temp_dir("battery-capacity");
        let battery = root.join("BAT0");
        fs::create_dir_all(&battery).unwrap();
        fs::write(battery.join("type"), "Battery").unwrap();
        fs::write(battery.join("capacity"), "87").unwrap();
        fs::write(battery.join("status"), "Charging").unwrap();

        let reading = read_linux_battery_root(&root).unwrap();
        assert_eq!(
            reading,
            BatteryReading {
                percent: 87,
                status: "charging".to_string(),
            }
        );

        cleanup(&root);
    }

    #[cfg(target_os = "linux")]
    #[test]
    fn reads_linux_battery_ratio_without_capacity_file() {
        let root = temp_dir("battery-ratio");
        let battery = root.join("BAT1");
        fs::create_dir_all(&battery).unwrap();
        fs::write(battery.join("type"), "Battery").unwrap();
        fs::write(battery.join("charge_now"), "48").unwrap();
        fs::write(battery.join("charge_full"), "96").unwrap();
        fs::write(battery.join("status"), "Discharging").unwrap();

        let reading = read_linux_battery_root(&root).unwrap();
        assert_eq!(
            reading,
            BatteryReading {
                percent: 50,
                status: "discharging".to_string(),
            }
        );

        cleanup(&root);
    }

    #[cfg(target_os = "linux")]
    #[test]
    fn reads_hottest_linux_thermal_zone() {
        let root = temp_dir("thermal-root");
        let zone0 = root.join("thermal_zone0");
        let zone1 = root.join("thermal_zone1");
        fs::create_dir_all(&zone0).unwrap();
        fs::create_dir_all(&zone1).unwrap();
        fs::write(zone0.join("temp"), "41000").unwrap();
        fs::write(zone1.join("temp"), "58750").unwrap();

        let hottest = read_linux_temperature_root(&root).unwrap();
        assert!((hottest - 58.8).abs() < 0.11);

        cleanup(&root);
    }

    #[cfg(target_os = "linux")]
    #[test]
    fn normalizes_linux_battery_status_strings() {
        assert_eq!(normalize_linux_battery_status("Charging"), "charging");
        assert_eq!(normalize_linux_battery_status("Not charging"), "plugged");
    }

    #[cfg(target_os = "linux")]
    #[test]
    fn parses_linux_temperature_units() {
        assert_eq!(parse_linux_temperature("42000"), Some(42.0));
        assert_eq!(parse_linux_temperature("39.5"), Some(39.5));
    }

    #[test]
    fn parses_windows_battery_json_payload() {
        let reading = parse_windows_battery_json(
            r#"{"EstimatedChargeRemaining":73,"BatteryStatus":6}"#,
        )
        .unwrap();
        assert_eq!(
            reading,
            BatteryReading {
                percent: 73,
                status: "charging".to_string(),
            }
        );
    }

    #[test]
    fn parses_windows_battery_json_array_payload() {
        let reading = parse_windows_battery_json(
            r#"[{"EstimatedChargeRemaining":92,"BatteryStatus":3}]"#,
        )
        .unwrap();
        assert_eq!(reading.percent, 92);
        assert_eq!(reading.status, "full");
    }

    #[test]
    fn handles_missing_windows_battery_payload() {
        assert_eq!(parse_windows_battery_json("null"), None);
        assert_eq!(parse_windows_battery_json(""), None);
    }

    #[test]
    fn normalizes_windows_battery_status_codes() {
        assert_eq!(normalize_windows_battery_status(Some(1)), "discharging");
        assert_eq!(normalize_windows_battery_status(Some(2)), "plugged");
        assert_eq!(normalize_windows_battery_status(Some(9)), "charging");
        assert_eq!(normalize_windows_battery_status(Some(99)), "unknown");
    }

    #[test]
    fn discards_invalid_temperatures() {
        assert_eq!(sanitize_temperature(f32::NAN), None);
        assert_eq!(sanitize_temperature(-4.0), None);
        assert_eq!(sanitize_temperature(180.0), None);
        assert_eq!(sanitize_temperature(63.27), Some(63.3));
    }

    #[test]
    fn reads_trimmed_file_content() {
        let root = temp_dir("trimmed");
        let file = root.join("value.txt");
        fs::create_dir_all(&root).unwrap();
        fs::write(&file, "  value  \n").unwrap();

        assert_eq!(read_trimmed(file), Some("value".to_string()));
        cleanup(&root);
    }

    fn temp_dir(label: &str) -> PathBuf {
        let nonce = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        std::env::temp_dir().join(format!("observans-sensors-{label}-{nonce}"))
    }

    fn cleanup(path: &Path) {
        let _ = fs::remove_dir_all(path);
    }
}
