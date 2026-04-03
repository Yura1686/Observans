use crate::camera_inventory::CameraInfo;
use crate::config::Config;
use crate::logs::{LogLevel, SharedLogBuffer};
use crate::metrics::{MetricsSnapshot, SharedMetrics};
use crate::shutdown::Shutdown;
use anyhow::{anyhow, Result};
use crossterm::cursor::{Hide, MoveTo, Show};
use crossterm::event::{poll, read, Event, KeyCode, KeyEventKind, KeyModifiers};
use crossterm::style::{Attribute, Color, Print, ResetColor, SetAttribute, SetForegroundColor};
use crossterm::terminal::{
    disable_raw_mode, enable_raw_mode, size, Clear, ClearType, EnterAlternateScreen,
    LeaveAlternateScreen,
};
use crossterm::{execute, queue};
use std::io::{self, IsTerminal, Write};
use std::net::{IpAddr, UdpSocket};
use std::thread;
use std::time::Duration;

const DEFAULT_WIDTH: usize = 96;
const MENU_WIDTH: usize = 88;
const MIN_WIDTH: usize = 56;
const LOG_LINES: usize = 10;

#[derive(Clone)]
struct MenuItem {
    label: String,
    value: String,
    sublabel: String,
    details: String,
}

#[derive(Clone)]
pub struct DashboardContext {
    pub config: Config,
    pub metrics: SharedMetrics,
    pub logs: SharedLogBuffer,
    pub shutdown: Shutdown,
}

pub fn terminal_is_interactive() -> bool {
    std::io::stdin().is_terminal() && std::io::stdout().is_terminal()
}

pub fn choose_camera(cameras: &[CameraInfo]) -> Result<Option<String>> {
    let mut items = cameras
        .iter()
        .map(|camera| MenuItem {
            label: camera.name.clone(),
            value: camera.device.clone(),
            sublabel: format!("backend: {}   id: {}", camera.backend, camera.device),
            details: format!(
                "Camera name : {}\r\nCapture id  : {}\r\nBackend     : {}\r\nNotes       : {}",
                camera.name, camera.device, camera.backend, camera.details
            ),
        })
        .collect::<Vec<_>>();

    items.push(MenuItem {
        label: "Auto detect".to_string(),
        value: "auto".to_string(),
        sublabel: "resolve the first available camera at runtime".to_string(),
        details: "Camera name : Automatic selection\r\nCapture id  : auto\r\nBackend     : runtime probe\r\nNotes       : uses the first working camera and applies safer fallbacks on startup".to_string(),
    });

    run_picker(&items)
}

pub fn spawn_dashboard(context: DashboardContext) -> Option<thread::JoinHandle<()>> {
    if !terminal_is_interactive() {
        return None;
    }

    Some(thread::spawn(move || {
        if let Err(error) = run_dashboard(context.clone()) {
            context.logs.push(
                LogLevel::Warn,
                "TUI",
                format!("dashboard renderer exited: {error:#}"),
            );
        }
    }))
}

fn run_picker(items: &[MenuItem]) -> Result<Option<String>> {
    if items.is_empty() {
        return Ok(None);
    }

    let mut stdout = io::stdout();
    let _guard = TerminalGuard::enter(&mut stdout)?;
    let mut cursor = 0usize;

    loop {
        render_picker(&mut stdout, items, cursor)?;
        stdout.flush()?;

        if let Event::Key(key) = read()? {
            if key.kind != KeyEventKind::Press {
                continue;
            }

            match key.code {
                KeyCode::Char('c') if key.modifiers.contains(KeyModifiers::CONTROL) => {
                    return Err(anyhow!("camera picker aborted by Ctrl+C"));
                }
                KeyCode::Up => {
                    cursor = if cursor == 0 {
                        items.len() - 1
                    } else {
                        cursor - 1
                    };
                }
                KeyCode::Down => cursor = (cursor + 1) % items.len(),
                KeyCode::Enter => return Ok(Some(items[cursor].value.clone())),
                KeyCode::Esc | KeyCode::Char('q') | KeyCode::Char('Q') => return Ok(None),
                KeyCode::Char(ch) if ch.is_ascii_digit() => {
                    let index = ch.to_digit(10).unwrap_or_default() as usize;
                    if (1..=items.len()).contains(&index) {
                        cursor = index - 1;
                    }
                }
                _ => {}
            }
        }
    }
}

fn run_dashboard(context: DashboardContext) -> Result<()> {
    let mut stdout = io::stdout();
    let _guard = TerminalGuard::enter(&mut stdout)?;

    loop {
        render_dashboard(&mut stdout, &context)?;
        stdout.flush()?;

        if context.shutdown.is_triggered() {
            break;
        }

        if poll(Duration::from_millis(150))? {
            match read()? {
                Event::Key(key)
                    if key.kind == KeyEventKind::Press
                        && matches!(key.code, KeyCode::Char('c'))
                        && key.modifiers.contains(KeyModifiers::CONTROL) =>
                {
                    context
                        .logs
                        .push(LogLevel::Wait, "SYS", "Ctrl+C received - shutting down");
                    context.shutdown.trigger();
                    break;
                }
                Event::Key(key)
                    if key.kind == KeyEventKind::Press
                        && matches!(key.code, KeyCode::Char('q') | KeyCode::Esc) =>
                {
                    context
                        .logs
                        .push(LogLevel::Wait, "SYS", "dashboard quit requested");
                    context.shutdown.trigger();
                    break;
                }
                _ => {}
            }
        }
    }

    Ok(())
}

fn render_picker(stdout: &mut io::Stdout, items: &[MenuItem], cursor: usize) -> Result<()> {
    let width = menu_width();
    let selected = &items[cursor];
    queue!(stdout, MoveTo(0, 0), Clear(ClearType::All))?;

    draw_banner(
        stdout,
        width,
        "OBSERVANS CLI CAMERA CONTROL",
        "ASCII / ANSI camera picker",
    )?;
    draw_section_title(stdout, width, "CAMERA INVENTORY")?;
    draw_status_line(
        stdout,
        width,
        "[....]",
        Color::DarkGrey,
        "TUI",
        &format!(
            "{} cameras discovered - use arrows or 1..9 to select - ENTER to continue",
            items.len().saturating_sub(1)
        ),
    )?;

    for (index, item) in items.iter().enumerate() {
        let prefix = if index == cursor { ">" } else { " " };
        let color = if index == cursor {
            Color::Green
        } else {
            Color::White
        };
        let label = fit(&item.label, width.saturating_sub(20));
        let line = format!(
            " {prefix} [{:>2}] {:<28} {}",
            index + 1,
            label,
            item.sublabel
        );
        draw_inner_line(
            stdout,
            width,
            &pad_to_width(&line, width.saturating_sub(4)),
            color,
        )?;
    }

    draw_section_end(stdout, width)?;
    draw_section_title(stdout, width, "SELECTION PREVIEW")?;
    for line in selected.details.lines() {
        draw_inner_line(
            stdout,
            width,
            &fit(line, width.saturating_sub(6)),
            Color::Cyan,
        )?;
    }
    draw_section_end(stdout, width)?;
    draw_section_title(stdout, width, "CONTROLS")?;
    draw_inner_line(
        stdout,
        width,
        "UP/DOWN  move   ENTER  confirm   Q / ESC  skip camera picker",
        Color::DarkGrey,
    )?;
    draw_section_end(stdout, width)?;

    Ok(())
}

fn render_dashboard(stdout: &mut io::Stdout, context: &DashboardContext) -> Result<()> {
    let width = dashboard_width();
    let metrics = context.metrics.snapshot();
    let logs = context.logs.snapshot(LOG_LINES);
    let urls = stream_urls(&context.config);

    queue!(stdout, MoveTo(0, 0), Clear(ClearType::All))?;
    draw_banner(
        stdout,
        width,
        "OBSERVANS CLI CONTROL PLANE",
        "live stream status / metrics / errors / logs",
    )?;

    draw_section_title(stdout, width, "STREAM ENDPOINTS")?;
    for url in &urls {
        draw_status_line(stdout, width, "[++++]", Color::Green, "WEB", url)?;
    }
    draw_status_line(
        stdout,
        width,
        "[....]",
        Color::DarkGrey,
        "CFG",
        &format!("camera request: {}", context.config.device),
    )?;
    draw_section_end(stdout, width)?;

    draw_section_title(stdout, width, "LIVE TELEMETRY")?;
    for line in dashboard_metrics(&metrics, logs.warn_count, logs.error_count) {
        draw_inner_line(stdout, width, &line, Color::White)?;
    }
    draw_section_end(stdout, width)?;

    draw_section_title(stdout, width, "EVENT FEED")?;
    if logs.entries.is_empty() {
        draw_status_line(
            stdout,
            width,
            "[~~~~]",
            Color::Yellow,
            "SYS",
            "waiting for runtime events...",
        )?;
    } else {
        for entry in &logs.entries {
            let message = fit(
                &format!(
                    "{} {:<3} {}",
                    entry.timestamp,
                    entry.tag,
                    entry.message.replace('\n', " ")
                ),
                width.saturating_sub(22),
            );
            draw_status_line(
                stdout,
                width,
                entry.level.token(),
                token_color(entry.level),
                &entry.tag,
                &message,
            )?;
        }
    }
    draw_section_end(stdout, width)?;

    draw_section_title(stdout, width, "HOTKEYS")?;
    draw_inner_line(
        stdout,
        width,
        "CTRL+C  graceful shutdown    Q / ESC  exit dashboard and stop server",
        Color::DarkGrey,
    )?;
    draw_section_end(stdout, width)?;

    Ok(())
}

fn dashboard_metrics(metrics: &MetricsSnapshot, warn_count: u64, error_count: u64) -> Vec<String> {
    let frame_age = if metrics.frame_age_ms >= 0 {
        format!("{} ms", metrics.frame_age_ms)
    } else {
        "--".to_string()
    };

    vec![
        format!(
            " host      : {:<24} platform   : {}",
            fit(&metrics.hostname, 24),
            metrics.platform_name
        ),
        format!(
            " backend   : {:<24} input      : {}",
            fit(&metrics.capture_backend, 24),
            fit(&metrics.stream_input, 28)
        ),
        format!(
            " clients   : {:<24} uptime     : {}",
            metrics.clients, metrics.uptime
        ),
        format!(
            " video     : {:<24} fps        : {:.1} / {}",
            metrics.res, metrics.fps_actual, metrics.fps_target
        ),
        format!(
            " cpu / ram : {:>5.1}% / {:>5.1}%{:>9}frame age  : {}",
            metrics.cpu, metrics.ram_pct, "", frame_age
        ),
        format!(
            " queue     : drops {:<16} avg frame  : {:.1} KB",
            metrics.queue_drops, metrics.avg_frame_kb
        ),
        format!(
            " restarts  : {:<24} warnings   : {}   errors: {}",
            metrics.restarts, warn_count, error_count
        ),
    ]
}

fn stream_urls(config: &Config) -> Vec<String> {
    let mut urls = vec![format!("http://127.0.0.1:{}/", config.port)];

    if let Some(ip) = primary_local_ip() {
        urls.push(format!("http://{}:{}/", ip, config.port));
    }

    urls.push(format!(
        "stream endpoint: http://localhost:{}/stream",
        config.port
    ));
    urls
}

fn primary_local_ip() -> Option<IpAddr> {
    let socket = UdpSocket::bind("0.0.0.0:0").ok()?;
    socket.connect("8.8.8.8:80").ok()?;
    Some(socket.local_addr().ok()?.ip())
}

fn token_color(level: LogLevel) -> Color {
    match level {
        LogLevel::Info => Color::DarkGrey,
        LogLevel::Ok => Color::Green,
        LogLevel::Wait => Color::Yellow,
        LogLevel::Warn => Color::Yellow,
        LogLevel::Error => Color::Red,
    }
}

fn draw_banner(stdout: &mut io::Stdout, width: usize, title: &str, subtitle: &str) -> Result<()> {
    let bar = format!("+{}+", "=".repeat(width.saturating_sub(2)));
    queue!(
        stdout,
        SetForegroundColor(Color::Cyan),
        Print(format!("{bar}\r\n")),
        ResetColor
    )?;
    draw_inner_line(
        stdout,
        width,
        &center(title, width.saturating_sub(4)),
        Color::White,
    )?;
    draw_inner_line(
        stdout,
        width,
        &center(subtitle, width.saturating_sub(4)),
        Color::DarkGrey,
    )?;
    queue!(
        stdout,
        SetForegroundColor(Color::Cyan),
        Print(format!("{bar}\r\n\r\n")),
        ResetColor
    )?;
    Ok(())
}

fn draw_section_title(stdout: &mut io::Stdout, width: usize, title: &str) -> Result<()> {
    let used = 7 + title.len();
    let line = format!(
        "+--[ {} ]{}+",
        title,
        "-".repeat(width.saturating_sub(used + 1))
    );
    queue!(
        stdout,
        SetForegroundColor(Color::Cyan),
        Print(format!("{}\r\n", fit(&line, width))),
        ResetColor
    )?;
    Ok(())
}

fn draw_section_end(stdout: &mut io::Stdout, width: usize) -> Result<()> {
    let line = format!("+{}+", "-".repeat(width.saturating_sub(2)));
    queue!(
        stdout,
        SetForegroundColor(Color::Cyan),
        Print(format!("{line}\r\n\r\n")),
        ResetColor
    )?;
    Ok(())
}

fn draw_status_line(
    stdout: &mut io::Stdout,
    width: usize,
    token: &str,
    token_color: Color,
    tag: &str,
    message: &str,
) -> Result<()> {
    let content_width = width.saturating_sub(4);
    let tag = format!("{:<3}", fit(tag, 3));
    let message = fit(message, content_width.saturating_sub(12));
    queue!(
        stdout,
        SetForegroundColor(Color::Cyan),
        Print("| "),
        SetForegroundColor(token_color),
        SetAttribute(Attribute::Bold),
        Print(token),
        SetAttribute(Attribute::Reset),
        ResetColor,
        Print(" "),
        SetForegroundColor(Color::Blue),
        Print(tag),
        ResetColor,
        Print(" "),
        Print(pad_to_width(&message, content_width.saturating_sub(10))),
        SetForegroundColor(Color::Cyan),
        Print(" |\r\n"),
        ResetColor
    )?;
    Ok(())
}

fn draw_inner_line(stdout: &mut io::Stdout, width: usize, text: &str, color: Color) -> Result<()> {
    let padded = pad_to_width(text, width.saturating_sub(4));
    queue!(
        stdout,
        SetForegroundColor(Color::Cyan),
        Print("| "),
        SetForegroundColor(color),
        Print(padded),
        ResetColor,
        SetForegroundColor(Color::Cyan),
        Print(" |\r\n"),
        ResetColor
    )?;
    Ok(())
}

fn dashboard_width() -> usize {
    let terminal_width = size()
        .map(|(cols, _)| cols as usize)
        .unwrap_or(DEFAULT_WIDTH);
    terminal_width
        .saturating_sub(2)
        .clamp(MIN_WIDTH, DEFAULT_WIDTH)
}

fn menu_width() -> usize {
    let terminal_width = size().map(|(cols, _)| cols as usize).unwrap_or(MENU_WIDTH);
    terminal_width
        .saturating_sub(2)
        .clamp(MIN_WIDTH, MENU_WIDTH)
}

fn center(text: &str, width: usize) -> String {
    let text = fit(text, width);
    let len = text.chars().count();
    if len >= width {
        return text;
    }
    let left = (width - len) / 2;
    let right = width - len - left;
    format!("{}{}{}", " ".repeat(left), text, " ".repeat(right))
}

fn fit(text: &str, width: usize) -> String {
    if width == 0 {
        return String::new();
    }

    let mut out = String::new();
    for ch in text.chars() {
        if out.chars().count() >= width {
            break;
        }
        out.push(ch);
    }
    out
}

fn pad_to_width(text: &str, width: usize) -> String {
    let mut out = fit(text, width);
    let len = out.chars().count();
    if len < width {
        out.push_str(&" ".repeat(width - len));
    }
    out
}

struct TerminalGuard;

impl TerminalGuard {
    fn enter(stdout: &mut io::Stdout) -> Result<Self> {
        enable_raw_mode()?;
        execute!(stdout, EnterAlternateScreen, Hide, Clear(ClearType::All))?;
        Ok(Self)
    }
}

impl Drop for TerminalGuard {
    fn drop(&mut self) {
        let _ = disable_raw_mode();
        let _ = execute!(io::stdout(), Show, LeaveAlternateScreen, ResetColor);
    }
}

#[cfg(test)]
mod tests {
    use super::{center, fit, pad_to_width};

    #[test]
    fn fit_truncates_without_wrapping() {
        assert_eq!(fit("abcdefghijklmnopqrstuvwxyz", 8), "abcdefgh");
    }

    #[test]
    fn pad_to_width_fills_remaining_space() {
        assert_eq!(pad_to_width("abc", 5), "abc  ");
    }

    #[test]
    fn center_balances_padding() {
        assert_eq!(center("OBS", 7), "  OBS  ");
    }
}
