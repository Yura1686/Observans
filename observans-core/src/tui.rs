use crate::camera_inventory::CameraInfo;
use crate::config::Config;
use crate::logs::{LogLevel, SharedLogBuffer};
use crate::metrics::{MetricsSnapshot, SharedMetrics};
use crate::shutdown::Shutdown;
use anyhow::{anyhow, Result};
use crossterm::cursor::{Hide, MoveTo, Show};
use crossterm::event::{poll, read, Event, KeyCode, KeyEventKind, KeyModifiers};
use crossterm::style::{Color, Print, ResetColor, SetForegroundColor};
use crossterm::terminal::{
    disable_raw_mode, enable_raw_mode, size, BeginSynchronizedUpdate, Clear, ClearType,
    DisableLineWrap, EnableLineWrap, EndSynchronizedUpdate, EnterAlternateScreen,
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

#[derive(Clone)]
struct StyledLine {
    text: String,
    color: Color,
}

#[derive(Default)]
struct FramePainter {
    last_frame: Vec<String>,
}

pub fn terminal_is_interactive() -> bool {
    if cfg!(windows) {
        std::io::stdout().is_terminal()
    } else {
        std::io::stdin().is_terminal() && std::io::stdout().is_terminal()
    }
}

pub fn choose_camera(cameras: &[CameraInfo]) -> Result<Option<String>> {
    let mut items = cameras
        .iter()
        .map(|camera| MenuItem {
            label: camera.name.clone(),
            value: camera.device.clone(),
            sublabel: format!("backend: {}   id: {}", camera.backend, camera.device),
            details: format!(
                "Camera name : {}\nCapture id  : {}\nBackend     : {}\nNotes       : {}",
                camera.name, camera.device, camera.backend, camera.details
            ),
        })
        .collect::<Vec<_>>();

    items.push(MenuItem {
        label: "Auto detect".to_string(),
        value: "auto".to_string(),
        sublabel: "resolve the first available camera at runtime".to_string(),
        details: "Camera name : Automatic selection\nCapture id  : auto\nBackend     : runtime probe\nNotes       : uses the first working camera and applies safer fallbacks on startup".to_string(),
    });

    match run_picker(&items) {
        Ok(choice) => Ok(choice),
        Err(error) if is_ctrl_c_abort(&error) => Err(error),
        Err(_) => run_plain_picker(&items),
    }
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
    let mut painter = FramePainter::default();
    let mut cursor = 0usize;

    loop {
        painter.paint(&mut stdout, &picker_lines(items, cursor, menu_width()))?;
        stdout.flush()?;

        if let Event::Key(key) = read()? {
            if key.kind == KeyEventKind::Release {
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

fn run_plain_picker(items: &[MenuItem]) -> Result<Option<String>> {
    println!();
    println!("+==============================================================+");
    println!("|                 OBSERVANS CAMERA SELECTION                   |");
    println!("+==============================================================+");

    for (index, item) in items.iter().enumerate() {
        println!("  {:>2}. {}", index + 1, item.label);
        println!("      {}", item.sublabel);
    }
    println!();

    loop {
        print!("Select camera [1-{}], or Q to skip: ", items.len());
        io::stdout().flush()?;

        let mut input = String::new();
        io::stdin().read_line(&mut input)?;
        let trimmed = input.trim();

        if trimmed.eq_ignore_ascii_case("q") {
            return Ok(None);
        }

        if let Ok(index) = trimmed.parse::<usize>() {
            if (1..=items.len()).contains(&index) {
                return Ok(Some(items[index - 1].value.clone()));
            }
        }

        println!("Invalid selection. Please enter a number or Q.");
    }
}

fn run_dashboard(context: DashboardContext) -> Result<()> {
    let mut stdout = io::stdout();
    let _guard = TerminalGuard::enter(&mut stdout)?;
    let mut painter = FramePainter::default();

    loop {
        let lines = dashboard_lines(&context, dashboard_width());
        painter.paint(&mut stdout, &lines)?;
        stdout.flush()?;

        if context.shutdown.is_triggered() {
            break;
        }

        if poll(Duration::from_millis(200))? {
            match read()? {
                Event::Key(key)
                    if key.kind != KeyEventKind::Release
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
                    if key.kind != KeyEventKind::Release
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

fn picker_lines(items: &[MenuItem], cursor: usize, width: usize) -> Vec<StyledLine> {
    let selected = &items[cursor];
    let mut lines = Vec::new();

    push_banner(
        &mut lines,
        width,
        "OBSERVANS CLI CAMERA CONTROL",
        "ASCII / ANSI camera picker",
    );
    push_section_title(&mut lines, width, "CAMERA INVENTORY");
    push_status_line(
        &mut lines,
        width,
        LogLevel::Info,
        "TUI",
        &format!(
            "{} cameras discovered - use arrows or 1..9 to select - ENTER to continue",
            items.len().saturating_sub(1)
        ),
    );

    for (index, item) in items.iter().enumerate() {
        let marker = if index == cursor { ">" } else { " " };
        let number = format!("[{:>2}]", index + 1);
        let label = fit(&item.label, 28);
        let content = format!(" {marker} {number} {:<28} {}", label, item.sublabel);
        push_inner_line(
            &mut lines,
            width,
            &content,
            if index == cursor {
                Color::Green
            } else {
                Color::White
            },
        );
    }

    push_section_end(&mut lines, width);
    push_section_title(&mut lines, width, "SELECTION PREVIEW");
    for line in selected.details.lines() {
        push_inner_line(&mut lines, width, line, Color::Cyan);
    }
    push_section_end(&mut lines, width);
    push_section_title(&mut lines, width, "CONTROLS");
    push_inner_line(
        &mut lines,
        width,
        "UP/DOWN move   ENTER confirm   Q / ESC skip camera picker",
        Color::DarkGrey,
    );
    push_section_end(&mut lines, width);

    lines
}

fn dashboard_lines(context: &DashboardContext, width: usize) -> Vec<StyledLine> {
    let metrics = context.metrics.snapshot();
    let logs = context.logs.snapshot(LOG_LINES);
    let urls = stream_urls(&context.config);
    let mut lines = Vec::new();

    push_banner(
        &mut lines,
        width,
        "OBSERVANS CLI CONTROL PLANE",
        "live stream status / metrics / errors / logs",
    );

    push_section_title(&mut lines, width, "STREAM ENDPOINTS");
    for url in &urls {
        push_status_line(&mut lines, width, LogLevel::Ok, "WEB", url);
    }
    push_status_line(
        &mut lines,
        width,
        LogLevel::Info,
        "CFG",
        &format!("camera request: {}", context.config.device),
    );
    push_section_end(&mut lines, width);

    push_section_title(&mut lines, width, "LIVE TELEMETRY");
    for line in dashboard_metrics(&metrics, logs.warn_count, logs.error_count) {
        push_inner_line(&mut lines, width, &line, Color::White);
    }
    push_section_end(&mut lines, width);

    push_section_title(&mut lines, width, "EVENT FEED");
    if logs.entries.is_empty() {
        push_status_line(
            &mut lines,
            width,
            LogLevel::Wait,
            "SYS",
            "waiting for runtime events...",
        );
    } else {
        for entry in &logs.entries {
            let message = format!(
                "{} {:<3} {}",
                entry.timestamp,
                entry.tag,
                entry.message.replace('\n', " ")
            );
            push_status_line(&mut lines, width, entry.level, &entry.tag, &message);
        }
    }
    push_section_end(&mut lines, width);

    push_section_title(&mut lines, width, "HOTKEYS");
    push_inner_line(
        &mut lines,
        width,
        "CTRL+C graceful shutdown    Q / ESC exit dashboard and stop server",
        Color::DarkGrey,
    );
    push_section_end(&mut lines, width);

    lines
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
            " cpu / ram : {:>5.1}% / {:>5.1}%      frame age  : {}",
            metrics.cpu, metrics.ram_pct, frame_age
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

fn push_banner(lines: &mut Vec<StyledLine>, width: usize, title: &str, subtitle: &str) {
    let border = format!("+{}+", "=".repeat(width.saturating_sub(2)));
    lines.push(styled(border.clone(), Color::Cyan));
    lines.push(styled(
        framed(width, &center(title, width.saturating_sub(4))),
        Color::White,
    ));
    lines.push(styled(
        framed(width, &center(subtitle, width.saturating_sub(4))),
        Color::DarkGrey,
    ));
    lines.push(styled(border, Color::Cyan));
    lines.push(styled(String::new(), Color::Reset));
}

fn push_section_title(lines: &mut Vec<StyledLine>, width: usize, title: &str) {
    let prefix = format!("+--[ {} ]", title);
    let line = format!(
        "{}{}+",
        prefix,
        "-".repeat(width.saturating_sub(prefix.chars().count() + 1))
    );
    lines.push(styled(line, Color::Cyan));
}

fn push_section_end(lines: &mut Vec<StyledLine>, width: usize) {
    lines.push(styled(
        format!("+{}+", "-".repeat(width.saturating_sub(2))),
        Color::Cyan,
    ));
    lines.push(styled(String::new(), Color::Reset));
}

fn push_status_line(
    lines: &mut Vec<StyledLine>,
    width: usize,
    level: LogLevel,
    tag: &str,
    message: &str,
) {
    let body = format!("{} {:<3} {}", level.token(), fit(tag, 3), message);
    push_inner_line(lines, width, &body, token_color(level));
}

fn push_inner_line(lines: &mut Vec<StyledLine>, width: usize, text: &str, color: Color) {
    lines.push(styled(framed(width, text), color));
}

fn framed(width: usize, text: &str) -> String {
    format!("| {} |", pad_to_width(text, width.saturating_sub(4)))
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

fn token_color(level: LogLevel) -> Color {
    match level {
        LogLevel::Info => Color::Grey,
        LogLevel::Ok => Color::Green,
        LogLevel::Wait => Color::Yellow,
        LogLevel::Warn => Color::Yellow,
        LogLevel::Error => Color::Red,
    }
}

fn styled(text: String, color: Color) -> StyledLine {
    StyledLine { text, color }
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

fn is_ctrl_c_abort(error: &anyhow::Error) -> bool {
    error
        .to_string()
        .contains("camera picker aborted by Ctrl+C")
}

impl FramePainter {
    fn paint(&mut self, stdout: &mut io::Stdout, lines: &[StyledLine]) -> Result<()> {
        let current_frame = lines
            .iter()
            .map(|line| line.text.clone())
            .collect::<Vec<_>>();

        if current_frame == self.last_frame {
            return Ok(());
        }

        queue!(stdout, BeginSynchronizedUpdate, MoveTo(0, 0))?;
        for (row, line) in lines.iter().enumerate() {
            queue!(
                stdout,
                MoveTo(0, row as u16),
                SetForegroundColor(line.color),
                Print(&line.text),
                ResetColor,
                Clear(ClearType::UntilNewLine)
            )?;
        }

        for row in lines.len()..self.last_frame.len() {
            queue!(stdout, MoveTo(0, row as u16), Clear(ClearType::CurrentLine))?;
        }

        queue!(stdout, EndSynchronizedUpdate)?;
        self.last_frame = current_frame;
        Ok(())
    }
}

struct TerminalGuard {
    use_alternate_screen: bool,
}

impl TerminalGuard {
    fn enter(stdout: &mut io::Stdout) -> Result<Self> {
        enable_raw_mode()?;
        let use_alternate_screen = !cfg!(windows);

        if use_alternate_screen {
            execute!(
                stdout,
                EnterAlternateScreen,
                DisableLineWrap,
                Hide,
                Clear(ClearType::All),
                MoveTo(0, 0)
            )?;
        } else {
            execute!(
                stdout,
                DisableLineWrap,
                Hide,
                Clear(ClearType::All),
                MoveTo(0, 0)
            )?;
        }

        Ok(Self {
            use_alternate_screen,
        })
    }
}

impl Drop for TerminalGuard {
    fn drop(&mut self) {
        let _ = disable_raw_mode();

        if self.use_alternate_screen {
            let _ = execute!(
                io::stdout(),
                Show,
                EnableLineWrap,
                LeaveAlternateScreen,
                ResetColor
            );
        } else {
            let _ = execute!(
                io::stdout(),
                Show,
                EnableLineWrap,
                ResetColor,
                Clear(ClearType::All),
                MoveTo(0, 0)
            );
        }
    }
}

#[cfg(test)]
mod tests {
    use super::{center, fit, framed, pad_to_width};

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

    #[test]
    fn framed_lines_preserve_requested_width() {
        let line = framed(22, "hello");
        assert_eq!(line.chars().count(), 22);
        assert!(line.starts_with("| "));
        assert!(line.ends_with(" |"));
    }
}
