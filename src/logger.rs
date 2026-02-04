use std::fmt::Arguments;
use std::sync::{Mutex, OnceLock};

struct Logger {
    lines: Vec<String>,
}

static LOGGER: OnceLock<Mutex<Logger>> = OnceLock::new();

pub fn init_logger() {
    let _ = LOGGER.set(Mutex::new(Logger { lines: Vec::new() }));
}

fn push_line(line: &str) {
    if let Some(lock) = LOGGER.get() {
        if let Ok(mut logger) = lock.lock() {
            logger.lines.push(line.to_string());
        }
    }
}

pub fn log_stdout_fmt(args: Arguments) {
    let line = args.to_string();
    println!("{}", line);
    push_line(&line);
}

pub fn log_stderr_fmt(args: Arguments) {
    let line = args.to_string();
    eprintln!("{}", line);
    push_line(&line);
}

pub fn collected_log() -> String {
    if let Some(lock) = LOGGER.get() {
        if let Ok(logger) = lock.lock() {
            return logger.lines.join("\n");
        }
    }
    String::new()
}

#[macro_export]
macro_rules! log_out {
    ($($arg:tt)*) => {
        $crate::logger::log_stdout_fmt(format_args!($($arg)*))
    };
}

#[macro_export]
macro_rules! log_err {
    ($($arg:tt)*) => {
        $crate::logger::log_stderr_fmt(format_args!($($arg)*))
    };
}
