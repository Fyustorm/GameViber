//! Logger writing to stderr, to a file (`gameviber.log` in the data directory,
//! the previous run's kept as `gameviber.previous.log`: what led to a crash
//! survives it, Windows' GUI having no stderr) and to an in-memory ring shown
//! in the GUI. Panics are logged too.

use std::collections::VecDeque;
use std::fs::File;
use std::io::Write;
use std::path::PathBuf;
use std::sync::{Arc, Mutex, OnceLock};
use std::time::Instant;

use log::{Level, LevelFilter, Log, Metadata, Record};

const MAX_LINES: usize = 2000;
/// Our own targets; everything else (buttplug, winit, wgpu...) is only shown from WARN up.
const OWN_TARGETS: [&str; 2] = ["gameviber", "mode"];

#[derive(Debug, Clone)]
pub struct LogLine {
    pub seconds: f64,
    pub level: Level,
    pub target: String,
    pub message: String,
}

pub type LogBuffer = Arc<Mutex<VecDeque<LogLine>>>;

struct Logger {
    start: Instant,
    level: LevelFilter,
    buffer: LogBuffer,
    file: Option<Mutex<File>>,
}

static BUFFER: OnceLock<LogBuffer> = OnceLock::new();

impl Log for Logger {
    fn enabled(&self, metadata: &Metadata) -> bool {
        let own = OWN_TARGETS.iter().any(|t| metadata.target().starts_with(t));
        metadata.level() <= if own { self.level } else { LevelFilter::Warn }
    }

    fn log(&self, record: &Record) {
        if !self.enabled(record.metadata()) {
            return;
        }
        let seconds = self.start.elapsed().as_secs_f64();
        let message = record.args().to_string();
        eprintln!("[{seconds:8.3} {:5}] {message}", record.level());
        // Unbuffered: every line is on disk before whatever comes next.
        if let Some(file) = &self.file {
            let _ = writeln!(file.lock().unwrap(), "[{seconds:8.3} {:5}] {message}", record.level());
        }
        let mut buffer = self.buffer.lock().unwrap();
        if buffer.len() >= MAX_LINES {
            buffer.pop_front();
        }
        buffer.push_back(LogLine { seconds, level: record.level(), target: record.target().to_owned(), message });
    }

    fn flush(&self) {}
}

/// This run's log file.
pub fn path() -> PathBuf {
    crate::platform::data_dir().join("gameviber.log")
}

/// A new log file, the previous one kept beside it.
fn open_file() -> Option<File> {
    let path = path();
    std::fs::create_dir_all(path.parent()?).ok()?;
    let _ = std::fs::rename(&path, path.with_file_name("gameviber.previous.log"));
    let mut file = File::create(&path).ok()?;
    let _ = writeln!(file, "GameViber {} on {}, started {}", env!("CARGO_PKG_VERSION"), std::env::consts::OS, crate::platform::local_time());
    Some(file)
}

pub fn init(verbose: bool) -> LogBuffer {
    let buffer = BUFFER.get_or_init(|| Arc::new(Mutex::new(VecDeque::new()))).clone();
    let level = if verbose { LevelFilter::Debug } else { LevelFilter::Info };
    let logger = Logger { start: Instant::now(), level, buffer: buffer.clone(), file: open_file().map(Mutex::new) };
    if log::set_boxed_logger(Box::new(logger)).is_ok() {
        log::set_max_level(LevelFilter::Debug);
        log_panics();
        crate::platform::log_crashes();
    }
    buffer
}

/// Panics, with the thread and where they happened, go to the log before the default hook.
fn log_panics() {
    let default = std::panic::take_hook();
    std::panic::set_hook(Box::new(move |info| {
        let thread = std::thread::current();
        let message = info.payload().downcast_ref::<&str>().copied().or_else(|| info.payload().downcast_ref::<String>().map(String::as_str)).unwrap_or("?");
        let place = info.location().map(|l| format!("{}:{}", l.file(), l.line())).unwrap_or_default();
        log::error!("panic in thread {}: {message} ({place})\n{}", thread.name().unwrap_or("?"), std::backtrace::Backtrace::force_capture());
        default(info);
    }));
}
