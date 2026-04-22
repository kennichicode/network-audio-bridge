use std::fs::{File, OpenOptions};
use std::io::Write;
use std::path::PathBuf;
use std::sync::Mutex;
use std::sync::OnceLock;

const MAX_LOG_BYTES: u64 = 10 * 1024 * 1024;

static LOGGER: OnceLock<Mutex<Option<File>>> = OnceLock::new();

fn log_path() -> Option<PathBuf> {
    let home = std::env::var_os("HOME").or_else(|| std::env::var_os("USERPROFILE"))?;
    let mut p = PathBuf::from(home);
    p.push(".nab");
    if std::fs::create_dir_all(&p).is_err() {
        return None;
    }
    p.push("log.txt");
    Some(p)
}

fn rotate_if_needed(path: &PathBuf) {
    if let Ok(meta) = std::fs::metadata(path) {
        if meta.len() > MAX_LOG_BYTES {
            let mut old = path.clone();
            old.set_extension("txt.1");
            let _ = std::fs::rename(path, old);
        }
    }
}

pub fn init(role: &str) {
    let file = log_path().and_then(|p| {
        rotate_if_needed(&p);
        OpenOptions::new().create(true).append(true).open(&p).ok()
    });
    let _ = LOGGER.set(Mutex::new(file));
    log(&format!("=== {} started ===", role));
}

pub fn log(msg: &str) {
    let cell = match LOGGER.get() {
        Some(c) => c,
        None => return,
    };
    let Ok(mut guard) = cell.lock() else { return };
    if let Some(f) = guard.as_mut() {
        let now = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_secs())
            .unwrap_or(0);
        let _ = writeln!(f, "[{}] {}", now, msg);
        let _ = f.flush();
    }
}
