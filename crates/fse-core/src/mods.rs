//! The mods that come with fse (`<home>\mods\<name>`): put into the game's mods folder when it lacks them or has an
//! older version, before the game looks at its mods. Only when the loader is installed in the game.

use std::path::{Path, PathBuf};

use crate::log;

fn version(v: &str) -> Vec<u32> {
    v.split('.').map(|p| p.trim().parse().unwrap_or(0)).collect()
}

/// a mod's version from its info.json (folder) or file name (`name_1.2.3[.zip]`)
fn installed(entry: &Path) -> Option<(String, Vec<u32>)> {
    let file = entry.file_name()?.to_string_lossy().into_owned();
    let stem = file.strip_suffix(".zip").unwrap_or(&file);
    if let Some(info) = std::fs::read_to_string(entry.join("info.json")).ok() {
        return Some((field(&info, "name")?, version(&field(&info, "version")?)));
    }
    let (name, v) = stem.rsplit_once('_')?;
    Some((name.to_string(), version(v)))
}

/// a top-level string field of info.json (no full JSON parse needed for "name" and "version")
fn field(json: &str, key: &str) -> Option<String> {
    let rest = &json[json.find(&format!("\"{key}\""))? + key.len() + 2..];
    let rest = &rest[rest.find('"')? + 1..];
    Some(rest[..rest.find('"')?].to_string())
}

fn arg(name: &str) -> Option<String> {
    let args: Vec<String> = std::env::args().collect();
    args.iter().position(|a| a == name).and_then(|i| args.get(i + 1).cloned())
}

/// --mod-directory, else FACTORIO_MODS, else `<write-data>\mods` as the game's config says
fn mods_dir() -> Option<PathBuf> {
    if let Some(d) = arg("--mod-directory").or_else(|| std::env::var("FACTORIO_MODS").ok()) {
        return Some(d.into());
    }
    let exe_dir = std::env::current_exe().ok()?.parent()?.to_path_buf();
    let system = PathBuf::from(std::env::var("APPDATA").ok()?).join("Factorio");
    let expand = |s: &str| PathBuf::from(s.trim()
        .replace("__PATH__system-write-data__", &system.to_string_lossy())
        .replace("__PATH__executable__", &exe_dir.to_string_lossy()));
    let line = |text: &str, key: &str| text.lines().find_map(|l| l.trim().strip_prefix(key).map(str::to_string));
    let config = arg("--config").map(PathBuf::from).unwrap_or_else(|| {
        let cfg = std::fs::read_to_string(exe_dir.join("../../config-path.cfg")).unwrap_or_default();
        line(&cfg, "config-path=").map(|p| expand(&p)).unwrap_or_else(|| system.join("config")).join("config.ini")
    });
    let ini = std::fs::read_to_string(config).unwrap_or_default();
    Some(line(&ini, "write-data=").map(|p| expand(&p)).unwrap_or(system).join("mods"))
}

fn copy_dir(from: &Path, to: &Path) -> std::io::Result<()> {
    std::fs::create_dir_all(to)?;
    for e in std::fs::read_dir(from)? {
        let e = e?;
        if e.file_type()?.is_dir() {
            copy_dir(&e.path(), &to.join(e.file_name()))?;
        } else {
            std::fs::copy(e.path(), to.join(e.file_name()))?;
        }
    }
    Ok(())
}

pub fn sync() {
    let Ok(home) = std::env::var("FSE_HOME") else { return };
    let Ok(bundled) = std::fs::read_dir(Path::new(&home).join("mods")) else { return };
    let Some(dir) = mods_dir() else { return };
    let have: Vec<(String, Vec<u32>)> = std::fs::read_dir(&dir).into_iter().flatten().flatten()
        .filter_map(|e| installed(&e.path())).collect();
    for m in bundled.flatten() {
        let Some((name, v)) = installed(&m.path()) else { continue };
        if have.iter().any(|(n, hv)| *n == name && *hv >= v) {
            continue;
        }
        let dest = dir.join(&name);
        let _ = std::fs::remove_dir_all(&dest);
        match copy_dir(&m.path(), &dest) {
            Ok(()) => log::line(&format!("mod {name} {} put into {}", field_version(&v), dir.display())),
            Err(e) => log::line(&format!("mod {name}: copying into {} failed: {e}", dir.display())),
        }
    }
}

fn field_version(v: &[u32]) -> String {
    v.iter().map(u32::to_string).collect::<Vec<_>>().join(".")
}
