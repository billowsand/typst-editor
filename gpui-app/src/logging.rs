//! A small local log (rotated), crash reports, and the diagnostics text
//! users can copy into a bug report. Document contents are never logged:
//! only events, paths and error messages.

use std::io::Write;
use std::path::PathBuf;
use std::sync::Mutex;

/// Version string baked in at build time (see `build.rs`).
pub const VERSION: &str = env!("CARGO_PKG_VERSION");
pub const COMMIT: &str = env!("ZIJIAN_COMMIT");
pub const BUILD_DATE: &str = env!("ZIJIAN_BUILD_DATE");

const MAX_LOG_BYTES: u64 = 1024 * 1024;

static LOG: Mutex<Option<PathBuf>> = Mutex::new(None);

pub fn log_dir() -> PathBuf {
    crate::recovery::data_dir().join("logs")
}

fn log_file() -> PathBuf {
    log_dir().join("zijian.log")
}

/// Start logging and install the crash-report panic hook.
pub fn init() {
    let _ = std::fs::create_dir_all(log_dir());
    rotate();
    *LOG.lock().unwrap_or_else(|e| e.into_inner()) = Some(log_file());
    info(&format!(
        "start 字笺 {VERSION} ({COMMIT}, {BUILD_DATE}) on {} {}",
        std::env::consts::OS,
        std::env::consts::ARCH
    ));

    let default_hook = std::panic::take_hook();
    std::panic::set_hook(Box::new(move |panic| {
        let report = format!(
            "字笺 {VERSION} ({COMMIT}, {BUILD_DATE})\n{} {}\n\n{panic}\n\n{}",
            std::env::consts::OS,
            std::env::consts::ARCH,
            std::backtrace::Backtrace::force_capture()
        );
        let name = format!("crash-{}.txt", chrono::Local::now().format("%Y%m%d-%H%M%S"));
        let _ = std::fs::write(log_dir().join(name), &report);
        error(&format!("panic: {panic}"));
        default_hook(panic);
    }));
}

/// Keep one previous log when the current one grows past the limit.
fn rotate() {
    let file = log_file();
    if std::fs::metadata(&file).is_ok_and(|m| m.len() > MAX_LOG_BYTES) {
        let _ = std::fs::rename(&file, log_dir().join("zijian.1.log"));
    }
}

fn write(level: &str, message: &str) {
    let guard = LOG.lock().unwrap_or_else(|e| e.into_inner());
    let Some(path) = guard.as_ref() else {
        return;
    };
    if let Ok(mut file) = std::fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(path)
    {
        let _ = writeln!(
            file,
            "{} {level} {message}",
            chrono::Local::now().format("%Y-%m-%d %H:%M:%S%.3f")
        );
    }
}

pub fn info(message: &str) {
    write("INFO ", message);
}

pub fn warn(message: &str) {
    write("WARN ", message);
}

pub fn error(message: &str) {
    write("ERROR", message);
}

/// Text for "复制诊断信息": version, platform, data paths and the tail of
/// the log. No document text.
pub fn diagnostics(font_serif: &str, font_mono: &str) -> String {
    let tail = std::fs::read_to_string(log_file())
        .map(|log| {
            let lines: Vec<&str> = log.lines().collect();
            lines[lines.len().saturating_sub(60)..].join("\n")
        })
        .unwrap_or_default();
    let crashes = std::fs::read_dir(log_dir())
        .map(|dir| {
            dir.filter_map(|e| e.ok())
                .filter(|e| e.file_name().to_string_lossy().starts_with("crash-"))
                .count()
        })
        .unwrap_or(0);
    format!(
        "字笺 · Typst {VERSION}\ncommit {COMMIT}\nbuilt {BUILD_DATE}\nplatform {} {}\n\
         fonts {font_serif} / {font_mono}\ndata {}\ncrash reports {crashes}\n\n--- log (last 60 lines) ---\n{tail}\n",
        std::env::consts::OS,
        std::env::consts::ARCH,
        crate::recovery::data_dir().display(),
    )
}
