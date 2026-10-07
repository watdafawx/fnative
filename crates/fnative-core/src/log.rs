//! A plain log file: FNATIVE_LOG if set (the launcher sets it), else fnative.log beside the game's exe.

use std::io::Write;
use std::sync::Mutex;

static LOCK: Mutex<()> = Mutex::new(());

fn path() -> std::path::PathBuf {
    if let Ok(p) = std::env::var("FNATIVE_LOG") {
        return p.into();
    }
    std::env::current_exe()
        .ok()
        .and_then(|p| p.parent().map(|d| d.join("fnative.log")))
        .unwrap_or_else(|| "fnative.log".into())
}

pub fn line(msg: &str) {
    let _guard = LOCK.lock();
    if let Ok(mut f) = std::fs::OpenOptions::new().create(true).append(true).open(path()) {
        let t = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_secs_f64())
            .unwrap_or(0.0);
        let _ = writeln!(f, "{t:.3} {msg}");
    }
}
