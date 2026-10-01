//! Logger writing to stderr and to an in-memory ring shown in the GUI.

use std::collections::VecDeque;
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
        let mut buffer = self.buffer.lock().unwrap();
        if buffer.len() >= MAX_LINES {
            buffer.pop_front();
        }
        buffer.push_back(LogLine { seconds, level: record.level(), target: record.target().to_owned(), message });
    }

    fn flush(&self) {}
}

pub fn init(verbose: bool) -> LogBuffer {
    let buffer = BUFFER.get_or_init(|| Arc::new(Mutex::new(VecDeque::new()))).clone();
    let level = if verbose { LevelFilter::Debug } else { LevelFilter::Info };
    let logger = Logger { start: Instant::now(), level, buffer: buffer.clone() };
    if log::set_boxed_logger(Box::new(logger)).is_ok() {
        log::set_max_level(LevelFilter::Debug);
    }
    buffer
}
