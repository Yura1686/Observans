use chrono::Local;
use std::collections::VecDeque;
use std::sync::{Arc, Mutex};
use tracing::Level;

const DEFAULT_LOG_CAPACITY: usize = 160;

#[derive(Clone, Debug)]
pub struct SharedLogBuffer {
    inner: Arc<Mutex<LogState>>,
}

#[derive(Debug)]
struct LogState {
    entries: VecDeque<LogEntry>,
    capacity: usize,
    warn_count: u64,
    error_count: u64,
}

#[derive(Clone, Debug)]
pub struct LogSnapshot {
    pub entries: Vec<LogEntry>,
    pub warn_count: u64,
    pub error_count: u64,
}

#[derive(Clone, Debug)]
pub struct LogEntry {
    pub timestamp: String,
    pub level: LogLevel,
    pub tag: String,
    pub message: String,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum LogLevel {
    Info,
    Ok,
    Wait,
    Warn,
    Error,
}

impl SharedLogBuffer {
    pub fn new(capacity: usize) -> Self {
        let capacity = capacity.max(1);
        Self {
            inner: Arc::new(Mutex::new(LogState {
                entries: VecDeque::with_capacity(capacity),
                capacity,
                warn_count: 0,
                error_count: 0,
            })),
        }
    }

    pub fn push_tracing(&self, level: &Level, target: &str, message: impl Into<String>) {
        let message = message.into();
        let log_level = classify_level(level, &message);
        let tag = target_tag(target);
        self.push(log_level, tag, message);
    }

    pub fn push(&self, level: LogLevel, tag: impl Into<String>, message: impl Into<String>) {
        let mut state = self.inner.lock().expect("log buffer lock poisoned");
        let entry = LogEntry {
            timestamp: Local::now().format("%H:%M:%S").to_string(),
            level,
            tag: tag.into(),
            message: message.into(),
        };

        if matches!(entry.level, LogLevel::Warn) {
            state.warn_count += 1;
        }
        if matches!(entry.level, LogLevel::Error) {
            state.error_count += 1;
        }

        while state.entries.len() >= state.capacity {
            state.entries.pop_front();
        }
        state.entries.push_back(entry);
    }

    pub fn snapshot(&self, limit: usize) -> LogSnapshot {
        let state = self.inner.lock().expect("log buffer lock poisoned");
        let take = limit.min(state.entries.len());
        let start = state.entries.len().saturating_sub(take);
        LogSnapshot {
            entries: state.entries.iter().skip(start).cloned().collect(),
            warn_count: state.warn_count,
            error_count: state.error_count,
        }
    }
}

impl Default for SharedLogBuffer {
    fn default() -> Self {
        Self::new(DEFAULT_LOG_CAPACITY)
    }
}

impl LogLevel {
    pub fn token(self) -> &'static str {
        match self {
            LogLevel::Info => "[....]",
            LogLevel::Ok => "[++++]",
            LogLevel::Wait => "[~~~~]",
            LogLevel::Warn => "[!!!!]",
            LogLevel::Error => "[XXXX]",
        }
    }
}

fn classify_level(level: &Level, message: &str) -> LogLevel {
    match *level {
        Level::ERROR => LogLevel::Error,
        Level::WARN => LogLevel::Warn,
        Level::INFO => classify_info(message),
        _ => LogLevel::Info,
    }
}

fn classify_info(message: &str) -> LogLevel {
    let lower = message.to_ascii_lowercase();
    if lower.contains("waiting")
        || lower.contains("awaiting")
        || lower.contains("starting")
        || lower.contains("stopped")
    {
        LogLevel::Wait
    } else if lower.contains("listening")
        || lower.contains("connected")
        || lower.contains("selected")
        || lower.contains("released")
        || lower.contains("reaped")
    {
        LogLevel::Ok
    } else {
        LogLevel::Info
    }
}

fn target_tag(target: &str) -> &'static str {
    if target.contains("observans_web") {
        "WEB"
    } else if target.contains("capture") {
        "CAP"
    } else if target.contains("probe") {
        "PRB"
    } else if target.contains("metrics") {
        "MET"
    } else if target.contains("tui") {
        "TUI"
    } else {
        "SYS"
    }
}

#[cfg(test)]
mod tests {
    use super::{LogLevel, SharedLogBuffer};
    use tracing::Level;

    #[test]
    fn keeps_recent_logs_and_counts_issues() {
        let logs = SharedLogBuffer::new(2);
        logs.push(LogLevel::Info, "SYS", "first");
        logs.push(LogLevel::Warn, "CAP", "second");
        logs.push(LogLevel::Error, "WEB", "third");

        let snapshot = logs.snapshot(10);
        assert_eq!(snapshot.entries.len(), 2);
        assert_eq!(snapshot.warn_count, 1);
        assert_eq!(snapshot.error_count, 1);
        assert_eq!(snapshot.entries[0].message, "second");
        assert_eq!(snapshot.entries[1].message, "third");
    }

    #[test]
    fn derives_tls_style_tokens_from_tracing_levels() {
        let logs = SharedLogBuffer::new(8);
        logs.push_tracing(&Level::INFO, "observans_web", "observans web listening");
        logs.push_tracing(
            &Level::WARN,
            "observans_core::capture",
            "capture session ended",
        );

        let snapshot = logs.snapshot(8);
        assert_eq!(snapshot.entries[0].level.token(), "[++++]");
        assert_eq!(snapshot.entries[1].level.token(), "[!!!!]");
        assert_eq!(snapshot.entries[0].tag, "WEB");
        assert_eq!(snapshot.entries[1].tag, "CAP");
    }
}
