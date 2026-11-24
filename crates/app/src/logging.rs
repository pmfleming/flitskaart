//! The app's log, `flitskaart.log` in the data folder: every error the app
//! shows, panics with a backtrace, the recording steps, and whatever libraries
//! print to stderr (ALSA, PipeWire, the audio streams). Each line also goes to
//! the terminal. Once the file passes 1 MB it is moved to `flitskaart.log.1`
//! and a new one starts.

use std::fmt::Display;
use std::fs::{self, File, OpenOptions};
use std::io::{self, BufRead as _, BufReader, Read, Write as _};
use std::os::fd::AsFd as _;
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
    let file = match open(path) {
        Ok(file) => file,
        Err(err) => return eprintln!("Kan het logboek {} niet openen: {err}", path.display()),
    };
    let terminal = io::stderr().as_fd().try_clone_to_owned().ok().map(File::from);
    if LOG.set(Mutex::new(Log { file, terminal })).is_err() {
        return;
    }
    if let Err(err) = capture_stderr() {
        error("logboek", format!("stderr wordt niet gelogd: {err}"));
    }
    std::panic::set_hook(Box::new(|info| {
        let backtrace = std::backtrace::Backtrace::force_capture();
        let thread = std::thread::current().name().unwrap_or("naamloos").to_string();
        write("PANIC", &format!("in thread '{thread}': {info}\n{backtrace}"));
    }));
    let args: Vec<String> = std::env::args().skip(1).collect();
    info(format!("Gestart: versie {} {}", env!("CARGO_PKG_VERSION"), args.join(" ")));
}

/// Opens the log for appending, first moving a full one aside.
fn open(path: &Path) -> io::Result<File> {
    if let Some(dir) = path.parent() {
        fs::create_dir_all(dir)?;
    }
    if fs::metadata(path).is_ok_and(|meta| meta.len() > MAX_SIZE) {
        fs::rename(path, path.with_extension("log.1"))?;
    }
    OpenOptions::new().create(true).append(true).open(path)
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

/// Something that looks wrong but isn't shown to the learner, such as a
/// microphone that stopped delivering sound partway through a recording.
pub fn warn(text: impl Display) {
    write("WARN", &text.to_string());
}

fn write(level: &str, text: &str) {
    let stamp = chrono::Local::now().format("%Y-%m-%d %H:%M:%S%.3f");
    let line = format!("{stamp} {level:<5} {}\n", text.trim_end().replace('\n', "\n    "));
    let Some(log) = LOG.get() else { return eprint!("{line}") };
    let mut log = log.lock().unwrap_or_else(|poisoned| poisoned.into_inner());
    let _ = log.file.write_all(line.as_bytes());
    if let Some(terminal) = &mut log.terminal {
        let _ = terminal.write_all(line.as_bytes());
    }
}

/// Points stderr at a pipe and logs every line that arrives on it.
fn capture_stderr() -> io::Result<()> {
    let (reader, writer) = io::pipe()?;
    std::thread::Builder::new().name("stderr-log".into()).spawn(move || forward_lines(reader, |line| write("STDERR", line)))?;
    rustix::stdio::dup2_stderr(writer)?;
    Ok(())
}

/// Hands each non-blank line `reader` yields to `emit` until it closes. Reads bytes,
/// not text: a library may print anything, and stopping would block stderr once the pipe fills.
fn forward_lines(reader: impl Read, mut emit: impl FnMut(&str)) {
    let mut reader = BufReader::new(reader);
    let mut line = Vec::new();
    while reader.read_until(b'\n', &mut line).is_ok_and(|read| read > 0) {
        let text = String::from_utf8_lossy(&line);
        if !text.trim().is_empty() {
            emit(text.trim_end());
        }
        line.clear();
    }
}

#[cfg(test)]
mod tests {
    use super::forward_lines;

    #[test]
    fn lines_from_libraries_reach_the_log_even_when_not_utf8() {
        let mut lines = Vec::new();
        forward_lines(&b"ALSA lib pcm.c: fout\n\xff raw\n\nlast line without newline"[..], |line| lines.push(line.to_string()));
        assert_eq!(lines, ["ALSA lib pcm.c: fout", "\u{fffd} raw", "last line without newline"]);
    }
}
