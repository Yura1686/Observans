use crate::camera_inventory::CameraInfo;
use anyhow::Result;
use crossterm::cursor::{Hide, MoveToColumn, MoveUp, Show};
use crossterm::event::{read, Event, KeyCode};
use crossterm::style::{Color, Stylize};
use crossterm::terminal::{disable_raw_mode, enable_raw_mode, size, Clear, ClearType};
use crossterm::{execute, queue};
use std::cmp::min;
use std::io::{self, Write};

const DEFAULT_WIDTH: usize = 72;
const MIN_WIDTH: usize = 48;

#[derive(Clone)]
struct MenuItem {
    label: String,
    value: String,
    sublabel: String,
}

pub fn choose_camera(cameras: &[CameraInfo]) -> Result<Option<String>> {
    let mut items = cameras
        .iter()
        .map(|camera| MenuItem {
            label: camera.name.clone(),
            value: camera.device.clone(),
            sublabel: format!("{}  {}", camera.backend, camera.device),
        })
        .collect::<Vec<_>>();

    items.push(MenuItem {
        label: "Auto-detect".to_string(),
        value: "auto".to_string(),
        sublabel: "resolve the first working camera at runtime".to_string(),
    });

    println!();
    println!(
        "{}",
        "  +--[ CAMERA SETUP ]--------------------------------------------------+"
            .with(Color::Cyan)
    );
    println!("  {}", "Scanning for cameras...".with(Color::DarkGrey));

    run_menu("Select capture device", &items)
}

fn run_menu(title: &str, items: &[MenuItem]) -> Result<Option<String>> {
    if items.is_empty() {
        return Ok(None);
    }

    let mut stdout = io::stdout();
    let mut cursor = 0usize;

    enable_raw_mode()?;
    execute!(stdout, Hide)?;

    let result = (|| -> Result<Option<String>> {
        let mut lines = render_menu(&mut stdout, title, items, cursor)?;
        stdout.flush()?;

        loop {
            if let Event::Key(key) = read()? {
                match key.code {
                    KeyCode::Up => {
                        cursor = if cursor == 0 {
                            items.len() - 1
                        } else {
                            cursor - 1
                        }
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

            clear_rendered_menu(&mut stdout, lines)?;
            lines = render_menu(&mut stdout, title, items, cursor)?;
            stdout.flush()?;
        }
    })();

    if let Err(error) = clear_rendered_menu(&mut stdout, rendered_line_count(items.len())) {
        let _ = execute!(stdout, Show);
        let _ = disable_raw_mode();
        return Err(error);
    }

    execute!(stdout, Show)?;
    disable_raw_mode()?;

    match &result? {
        Some(choice) if choice == "auto" => {
            println!(
                "  {}  {}",
                "[....]".with(Color::DarkGrey),
                "Auto-detect enabled - device resolved at runtime".with(Color::DarkGrey)
            );
            Ok(Some(choice.clone()))
        }
        Some(choice) => {
            println!(
                "  {}  {} {}",
                "[++++]".with(Color::Green),
                "Selected:".with(Color::DarkGrey),
                choice.as_str().with(Color::Cyan)
            );
            Ok(Some(choice.clone()))
        }
        None => {
            println!(
                "  {}  {}",
                "[....]".with(Color::DarkGrey),
                "Camera picker skipped - continuing with CLI/default device".with(Color::DarkGrey)
            );
            Ok(None)
        }
    }
}

fn clear_rendered_menu(stdout: &mut io::Stdout, lines: usize) -> Result<()> {
    queue!(stdout, MoveUp(lines as u16))?;
    for _ in 0..lines {
        queue!(stdout, MoveToColumn(0), Clear(ClearType::CurrentLine))?;
        newline(stdout)?;
    }
    queue!(stdout, MoveUp(lines as u16), MoveToColumn(0))?;
    Ok(())
}

fn render_menu(
    stdout: &mut io::Stdout,
    title: &str,
    items: &[MenuItem],
    cursor: usize,
) -> Result<usize> {
    let width = menu_width();
    let inner = width.saturating_sub(4);

    write_line(stdout, &format!("  {}", fit(title, inner).white().bold()))?;
    write_line(
        stdout,
        &format!(
            "  {}",
            fit("UP/DOWN move  ENTER select  Q skip", inner).dark_grey()
        ),
    )?;
    write_line(
        stdout,
        &format!("{}", format!("  +{}+", "-".repeat(inner)).with(Color::Cyan)),
    )?;

    for (index, item) in items.iter().enumerate() {
        let label_width = item_label_width(inner);
        let prefix = if index == cursor { ">" } else { " " };
        let number = format!("[{:>2}]", index + 1);
        let label = fit(&item.label, label_width);
        let sublabel_max = inner.saturating_sub(1 + 2 + number.len() + 2 + label_width + 2);
        let sublabel = fit(&item.sublabel, sublabel_max);
        let row = format!(" {prefix}  {number}  {:label_width$}  {sublabel}", label);
        let padded = pad_to_width(&row, inner);

        if index == cursor {
            write_line(
                stdout,
                &format!(
                    "{}{}{}",
                    "  |".with(Color::Cyan),
                    padded.white().bold().on_dark_grey(),
                    "|".with(Color::Cyan)
                ),
            )?;
        } else {
            write_line(
                stdout,
                &format!(
                    "{}{}{}",
                    "  |".with(Color::Cyan),
                    padded.with(Color::White),
                    "|".with(Color::Cyan)
                ),
            )?;
        }
    }

    write_line(
        stdout,
        &format!("{}", format!("  +{}+", "-".repeat(inner)).with(Color::Cyan)),
    )?;

    Ok(rendered_line_count(items.len()))
}

fn rendered_line_count(item_count: usize) -> usize {
    item_count + 4
}

fn menu_width() -> usize {
    let terminal_width = size().map(|(cols, _)| cols as usize).unwrap_or(80);
    let safe_width = terminal_width.saturating_sub(2);
    safe_width.clamp(MIN_WIDTH, DEFAULT_WIDTH)
}

fn item_label_width(inner: usize) -> usize {
    min(28, inner.saturating_sub(20)).max(12)
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

fn write_line(stdout: &mut io::Stdout, line: &str) -> Result<()> {
    write!(stdout, "{line}")?;
    newline(stdout)
}

fn newline(stdout: &mut io::Stdout) -> Result<()> {
    write!(stdout, "\r\n")?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::{fit, pad_to_width};

    #[test]
    fn fit_truncates_without_wrapping() {
        assert_eq!(fit("abcdefghijklmnopqrstuvwxyz", 8), "abcdefgh");
    }

    #[test]
    fn pad_to_width_fills_remaining_space() {
        assert_eq!(pad_to_width("cam", 6), "cam   ");
    }
}
