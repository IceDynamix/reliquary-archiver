use std::any::Any;
use std::fs::File;
use std::io;
use std::io::Write;
use std::path::Path;
use std::sync::{Arc, LazyLock, Mutex};

use tracing::info;
use tracing::level_filters::LevelFilter;
use tracing_subscriber::fmt::MakeWriter;
use tracing_subscriber::prelude::*;
use tracing_subscriber::{EnvFilter, Registry, reload};

/// Lines are dropped from the front of the log buffer once it grows past this
const LOG_BUFFER_MAX_LINES: usize = 100000;
/// How many of the most recent lines survive when the log buffer is trimmed
const LOG_BUFFER_KEEP_LINES: usize = 10000;

pub static LOG_BUFFER: LazyLock<Mutex<Vec<String>>> = LazyLock::new(|| Mutex::new(Vec::new()));
pub static LOG_NOTIFY: LazyLock<tokio::sync::Notify> = LazyLock::new(tokio::sync::Notify::new);

type VecLayerHandle = Box<dyn Fn(LevelFilter) + Send>;
pub static VEC_LAYER_HANDLE: LazyLock<Mutex<Option<VecLayerHandle>>> = LazyLock::new(|| Mutex::new(None));

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

    let env_filter = EnvFilter::builder()
        .with_default_directive(
            match verbose {
                0 => "reliquary_archiver=info",
                1 => "info",
                2 => "debug",
                _ => "trace",
            }
            .parse()
            .unwrap(),
        )
        .from_env_lossy();

    let (console_filter, handle) = reload::Layer::new(env_filter);
    let console_log = tracing_subscriber::fmt::layer()
        .with_ansi(false)
        .with_writer(DualWriter::new(VecWriter, io::stdout()))
        .with_filter(console_filter);

    *VEC_LAYER_HANDLE.lock().unwrap() = Some(Box::new(move |l| {
        let result = handle.modify(|f| {
            *f = EnvFilter::builder()
                .parse(match l {
                    LevelFilter::TRACE => "trace",
                    LevelFilter::DEBUG => "debug",
                    LevelFilter::INFO => "info",
                    LevelFilter::WARN => "warn",
                    LevelFilter::ERROR => "error",
                    _ => "off",
                })
                .unwrap();
        });

        if let Err(e) = result {
            tracing::error!("Failed to change log level: {}", e);
        }
    }));

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
