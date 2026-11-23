//! The app's log, `flitskaart.log` in the data folder: every error the app
//! shows, panics with a backtrace, the recording steps, and whatever libraries
//! print to stderr (ALSA, PipeWire, the audio streams). Each line also goes to
//! the terminal. Once the file passes 1 MB it is moved to `flitskaart.log.1`
//! and a new one starts.

use std::fmt::Display;
use std::fs::{self, File, OpenOptions};
use std::io::{BufRead as _, BufReader, Write as _};
use std::os::fd::FromRawFd as _;
use std::path::Path;
use std::sync::{Mutex, OnceLock};

const MAX_SIZE: u64 = 1_000_000;

struct Log {
    file: File,
    /// The real stderr, kept before stderr is routed into the log.
    terminal: Option<File>,
}

static LOG: OnceLock<Mutex<Log>> = OnceLock::new();

/// Opens the log, routes stderr into it and logs panics. Call once, first thing.
pub fn init(path: &Path) {
    if let Some(dir) = path.parent() {
        let _ = fs::create_dir_all(dir);
    }
    if fs::metadata(path).is_ok_and(|meta| meta.len() > MAX_SIZE) {
        let _ = fs::rename(path, path.with_extension("log.1"));
    }
    let file = match OpenOptions::new().create(true).append(true).open(path) {
        Ok(file) => file,
        Err(err) => {
            eprintln!("Kan het logboek {} niet openen: {err}", path.display());
            return;
        }
    };
    // SAFETY: dup returns a new descriptor that only this File owns.
    let terminal = match unsafe { libc::dup(libc::STDERR_FILENO) } {
        -1 => None,
        fd => Some(unsafe { File::from_raw_fd(fd) }),
    };
    if LOG.set(Mutex::new(Log { file, terminal })).is_err() {
        return;
    }
    capture_stderr();
    std::panic::set_hook(Box::new(|info| {
        let thread = std::thread::current();
        let backtrace = std::backtrace::Backtrace::force_capture();
        write("PANIC", &format!("in thread '{}': {info}\n{backtrace}", thread.name().unwrap_or("naamloos")));
    }));
    let args: Vec<String> = std::env::args().skip(1).collect();
    info(format!("Gestart: versie {}{}", env!("CARGO_PKG_VERSION"), if args.is_empty() { String::new() } else { format!(", {}", args.join(" ")) }));
}

/// Logs an error from `area` (such as "opname") and returns the text, for showing it.
pub fn error(area: &str, text: impl Display) -> String {
    let text = text.to_string();
    write("ERROR", &format!("[{area}] {text}"));
    text
}

pub fn info(text: impl Display) {
    write("INFO", &text.to_string());
}

fn write(level: &str, text: &str) {
    let stamp = chrono::Local::now().format("%Y-%m-%d %H:%M:%S%.3f");
    let line = format!("{stamp} {level:<5} {}\n", text.trim_end().replace('\n', "\n    "));
    let Some(log) = LOG.get() else {
        eprint!("{line}");
        return;
    };
    let mut log = log.lock().unwrap_or_else(|poisoned| poisoned.into_inner());
    let _ = log.file.write_all(line.as_bytes());
    if let Some(terminal) = &mut log.terminal {
        let _ = terminal.write_all(line.as_bytes());
    }
}

/// Points stderr at a pipe and logs every line that arrives on it.
fn capture_stderr() {
    let mut fds = [0; 2];
    // SAFETY: plain descriptor calls; each descriptor is closed or owned below.
    unsafe {
        if libc::pipe(fds.as_mut_ptr()) != 0 {
            return;
        }
        if libc::dup2(fds[1], libc::STDERR_FILENO) == -1 {
            libc::close(fds[0]);
            libc::close(fds[1]);
            return;
        }
        libc::close(fds[1]);
    }
    let reader = unsafe { File::from_raw_fd(fds[0]) };
    let _ = std::thread::Builder::new().name("stderr-log".into()).spawn(move || {
        // Read bytes, not text: a library may print anything, and stopping would
        // leave stderr blocked once the pipe fills.
        let mut reader = BufReader::new(reader);
        let mut line = Vec::new();
        loop {
            line.clear();
            match reader.read_until(b'\n', &mut line) {
                Ok(0) => break,
                Ok(_) => {
                    let text = String::from_utf8_lossy(&line);
                    if !text.trim().is_empty() {
                        write("STDERR", &text);
                    }
                }
                Err(err) if err.kind() == std::io::ErrorKind::Interrupted => {}
                Err(_) => break,
            }
        }
    });
}
