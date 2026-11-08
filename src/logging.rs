use std::any::Any;
use std::fs::File;
use std::io;
use std::io::Write;
use std::path::Path;
#[cfg(feature = "gui")]
use std::sync::MutexGuard;
use std::sync::{Arc, LazyLock, Mutex, OnceLock};

use tracing::info;
use tracing::level_filters::LevelFilter;
use tracing_subscriber::fmt::MakeWriter;
use tracing_subscriber::prelude::*;
use tracing_subscriber::{EnvFilter, Registry, reload};

/// Lines are dropped from the front of the log buffer once it grows past this
const LOG_BUFFER_MAX_LINES: usize = 100000;
/// How many of the most recent lines survive when the log buffer is trimmed
const LOG_BUFFER_KEEP_LINES: usize = 10000;

/// Everything logged to the console, kept around for the GUI's log view and the crash log
static LOG_BUFFER: LazyLock<Mutex<Vec<String>>> = LazyLock::new(|| Mutex::new(Vec::new()));
/// Notified whenever lines are added to [`LOG_BUFFER`]
static LOG_NOTIFY: LazyLock<tokio::sync::Notify> = LazyLock::new(tokio::sync::Notify::new);
/// Changes the filter of the console log, set up by [`tracing_init`]
static CONSOLE_FILTER: OnceLock<reload::Handle<EnvFilter, Registry>> = OnceLock::new();

/// The lines logged to the console so far, oldest first.
///
/// Logging blocks while the guard is held, so don't hold on to it for long.
#[cfg(feature = "gui")]
pub fn lines() -> MutexGuard<'static, Vec<String>> {
    LOG_BUFFER.lock().unwrap()
}

/// Waits until new lines have been logged
#[cfg(feature = "gui")]
pub async fn wait_for_new_lines() {
    LOG_NOTIFY.notified().await;
}

/// What the console logs when nothing more verbose was asked for: the app's own info logs, plus
/// warnings from the libraries that decode and capture traffic. Those warnings (missed packets,
/// KCP errors) are often the only sign of why a capture got stuck, so they belong in screenshots.
///
/// Targets match by prefix and the longest one wins, so `reliquary=warn` doesn't override
/// `reliquary_archiver=info`.
const DEFAULT_DIRECTIVES: &str = "reliquary_archiver=info,reliquary=warn,pktmon=warn";

/// One step up from [`DEFAULT_DIRECTIVES`]. The libraries' info logs include every decoded
/// command and every new conversation, which is what diagnosing a stuck capture needs.
#[cfg(any(feature = "gui", test))]
const DEBUG_DIRECTIVES: &str = "reliquary_archiver=debug,reliquary=info,pktmon=info";

/// The console filter for a level picked in the GUI. The capture libraries stay one level
/// quieter than the app, other crates stay quiet, and trace shows everything.
#[cfg(feature = "gui")]
fn directives_for_level(level: LevelFilter) -> String {
    match level {
        LevelFilter::INFO => DEFAULT_DIRECTIVES.to_string(),
        LevelFilter::DEBUG => DEBUG_DIRECTIVES.to_string(),
        level => level.to_string(),
    }
}

fn console_filter(directives: &str) -> EnvFilter {
    EnvFilter::builder().parse_lossy(directives)
}

/// Changes which levels are logged to the console and the log buffer
#[cfg(feature = "gui")]
pub fn set_console_level(level: LevelFilter) {
    let Some(handle) = CONSOLE_FILTER.get() else {
        return;
    };

    let filter = console_filter(&directives_for_level(level));
    let result = handle.modify(|f| *f = filter);
    if let Err(e) = result {
        tracing::error!("Failed to change log level: {}", e);
    }
}

struct VecWriter;

impl io::Write for VecWriter {
    fn write(&mut self, buf: &[u8]) -> io::Result<usize> {
        let str = String::from_utf8_lossy(buf);
        let lines = str.lines().map(|s| s.to_string());
        {
            let mut buffer = LOG_BUFFER.lock().unwrap();
            buffer.extend(lines);

            // Limit the buffer size to prevent unbounded memory growth
            // (assuming an average of ~100 bytes per log line, the buffer peaks at around 10MB of memory)
            if buffer.len() > LOG_BUFFER_MAX_LINES {
                let range = 0..buffer.len() - LOG_BUFFER_KEEP_LINES;
                buffer.drain(range);
            }
        }
        LOG_NOTIFY.notify_one();
        Ok(buf.len())
    }

    fn flush(&mut self) -> io::Result<()> {
        Ok(())
    }
}

struct DualWriter<A: io::Write, B: io::Write> {
    m: Arc<Mutex<(A, B)>>,
}

impl<A: io::Write, B: io::Write> DualWriter<A, B> {
    fn new(a: A, b: B) -> Self {
        Self {
            m: Arc::new(Mutex::new((a, b))),
        }
    }
}

impl<A: io::Write, B: io::Write> io::Write for DualWriter<A, B> {
    fn write(&mut self, buf: &[u8]) -> io::Result<usize> {
        let mut m = self.m.lock().unwrap();
        m.0.write_all(buf)?;
        m.1.write(buf)
    }

    fn flush(&mut self) -> io::Result<()> {
        let mut m = self.m.lock().unwrap();
        m.0.flush()?;
        m.1.flush()
    }
}

impl<'a, A: io::Write, B: io::Write> MakeWriter<'a> for DualWriter<A, B> {
    type Writer = DualWriter<A, B>;

    fn make_writer(&'a self) -> Self::Writer {
        DualWriter { m: self.m.clone() }
    }
}

pub fn tracing_init(verbose: u8, log_path: Option<&Path>) {
    tracing_log::LogTracer::init().unwrap();

    // RUST_LOG replaces the defaults entirely when it's set
    let directives = std::env::var(EnvFilter::DEFAULT_ENV)
        .ok()
        .filter(|directives| !directives.is_empty())
        .unwrap_or_else(|| {
            match verbose {
                0 => DEFAULT_DIRECTIVES,
                1 => "info",
                2 => "debug",
                _ => "trace",
            }
            .to_string()
        });

    let (filter, handle) = reload::Layer::new(console_filter(&directives));
    let console_log = tracing_subscriber::fmt::layer()
        .with_ansi(false)
        .with_writer(DualWriter::new(VecWriter, io::stdout()))
        .with_filter(filter);

    CONSOLE_FILTER.set(handle).ok();

    let file_log = log_path.map(|log_path| {
        let log_file = File::create(log_path).unwrap();
        tracing_subscriber::fmt::layer()
            .json()
            .with_writer(Mutex::new(log_file))
            .with_filter(LevelFilter::TRACE)
    });

    let subscriber = Registry::default().with(console_log).with(file_log);

    tracing::subscriber::set_global_default(subscriber).expect("unable to set up logging");
}

/// Writes the buffered log and the panic message to crashlog.txt
pub fn write_crashlog(payload: &(dyn Any + Send)) {
    let Ok(mut file) = File::create("crashlog.txt") else {
        return;
    };

    let log = match LOG_BUFFER.try_lock() {
        Ok(buffer) => buffer.join("\n"),
        Err(_) => "failed to lock log buffer".to_string(),
    };

    let panic_message = if let Some(s) = payload.downcast_ref::<&str>() {
        s
    } else if let Some(s) = payload.downcast_ref::<String>() {
        s.as_str()
    } else {
        "panic: unknown payload type"
    };

    if write!(file, "{log}\n\n{panic_message}").is_ok() {
        info!("wrote crashlog to crashlog.txt");
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn default_directives_parse() {
        // parse_lossy drops invalid directives silently, so check them strictly here
        EnvFilter::builder().parse(DEFAULT_DIRECTIVES).unwrap();
        EnvFilter::builder().parse(DEBUG_DIRECTIVES).unwrap();
    }

    /// Logs one event per target and level through `directives` and returns what got through
    fn logged_through(directives: &str) -> String {
        let lines = Arc::new(Mutex::new(Vec::<u8>::new()));
        let writer = {
            let lines = lines.clone();
            move || SharedWriter(lines.clone())
        };

        let subscriber = Registry::default().with(
            tracing_subscriber::fmt::layer()
                .with_ansi(false)
                .with_writer(writer)
                .with_filter(console_filter(directives)),
        );

        tracing::subscriber::with_default(subscriber, || {
            tracing::debug!(target: "reliquary_archiver::session", "app debug");
            tracing::info!(target: "reliquary_archiver::session", "app info");
            tracing::info!(target: "reliquary::network", "library info");
            tracing::warn!(target: "reliquary::network::kcp", "library warning");
            tracing::info!(target: "pktmon::realtime", "pktmon info");
            tracing::warn!(target: "pktmon::realtime", "pktmon warning");
            tracing::warn!(target: "hyper", "unrelated warning");
        });

        String::from_utf8(lines.lock().unwrap().clone()).unwrap()
    }

    #[test]
    fn default_directives_show_library_warnings_only() {
        let output = logged_through(DEFAULT_DIRECTIVES);
        assert!(output.contains("app info"));
        assert!(output.contains("library warning"));
        assert!(output.contains("pktmon warning"));
        assert!(!output.contains("app debug"));
        assert!(!output.contains("library info"));
        assert!(!output.contains("pktmon info"));
        assert!(!output.contains("unrelated warning"));
    }

    #[test]
    fn debug_directives_show_library_info() {
        let output = logged_through(DEBUG_DIRECTIVES);
        assert!(output.contains("app debug"));
        assert!(output.contains("library info"));
        assert!(output.contains("pktmon info"));
        assert!(!output.contains("unrelated warning"));
    }

    struct SharedWriter(Arc<Mutex<Vec<u8>>>);

    impl io::Write for SharedWriter {
        fn write(&mut self, buf: &[u8]) -> io::Result<usize> {
            self.0.lock().unwrap().extend_from_slice(buf);
            Ok(buf.len())
        }

        fn flush(&mut self) -> io::Result<()> {
            Ok(())
        }
    }
}
