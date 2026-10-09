//! The build check at every start (fse-engine does the reading):
//! - factorio.exe (as loaded) and factorio.pdb must be the same build, else nothing is hooked (the game runs plain);
//! - what the core and the plugins need (`needs.json`, `plugins/*.needs.json`, the core's own Lua API list) is checked;
//! - the first start of a build caches its description in `cache/<build>/` and writes `report.txt` there: what it
//!   lacks and what moved since the build before (a game update shows its effect at once).

use std::path::PathBuf;
use std::sync::OnceLock;

use fse_engine as fe;
use windows_sys::Win32::System::LibraryLoader::GetModuleHandleW;

use crate::log;

static ENGINE: OnceLock<fe::Engine> = OnceLock::new();
static FRESH: OnceLock<bool> = OnceLock::new();

pub fn engine() -> Option<&'static fe::Engine> {
    ENGINE.get()
}

pub fn fresh() -> bool {
    FRESH.get().copied().unwrap_or(false)
}

fn home() -> PathBuf {
    std::env::var("FSE_HOME").map(PathBuf::from).unwrap_or_else(|_| {
        std::env::current_exe().ok().and_then(|p| p.parent().map(|d| d.to_path_buf())).unwrap_or_default()
    })
}

/// the running exe's build, from its CodeView record in memory
fn running_build() -> Result<fe::BuildId, String> {
    unsafe {
        let base = GetModuleHandleW(std::ptr::null()) as *const u8;
        let pe = *(base.add(0x3c) as *const u32) as usize;
        let size = *(base.add(pe + 24 + 56) as *const u32) as usize; // (OptionalHeader.SizeOfImage)
        fe::exe_build(std::slice::from_raw_parts(base, size), true)
    }
}

/// checks the build; Err stops fse from hooking anything
pub fn check(pdb: &std::path::Path, functions: &std::collections::HashMap<String, u32>, core: fe::Needs)
             -> Result<(), String> {
    let exe = running_build()?;
    let pdb_build = fe::pdb_build(pdb)?;
    if exe != pdb_build {
        return Err(format!("factorio.exe is build {exe} but factorio.pdb is {pdb_build} (a half-applied update?): \
                            nothing hooked"));
    }
    let h = home();
    let mut needs = core.clone();
    needs.merge(fe::Needs::read_folder(&h));
    // (the plugins folder: FSE_PLUGINS when set, as the plugins themselves are found)
    let plugins = std::env::var("FSE_PLUGINS").map(PathBuf::from).unwrap_or_else(|_| h.join("plugins"));
    needs.merge(fe::Needs::read_folder(&plugins));
    let cache = h.join("cache");
    let (e, fresh) = fe::load(&cache, pdb, functions, &needs)?;
    let missing = fe::check(&e, &needs);
    if fresh {
        let version = std::fs::metadata(std::env::current_exe().unwrap_or_default())
            .map(|m| format!("factorio.exe {} bytes", m.len())).unwrap_or_default();
        let text = fe::report(&cache, &e, &needs, &version);
        let path = cache.join(e.build.to_string()).join("report.txt");
        let _ = std::fs::write(&path, &text);
        log::line(&format!("new game build {}: report in {}", e.build, path.display()));
        for line in text.lines().skip(1) {
            log::line(&format!("  {line}"));
        }
    } else {
        log::line(&format!("game build {} (known)", e.build));
    }
    // what the core itself needs must be there; plugins only lose the parts they asked for
    let core_missing: Vec<&String> = missing.iter()
        .filter(|m| core.functions.iter().any(|f| m.starts_with(&format!("function {f}:")))).collect();
    if !core_missing.is_empty() {
        return Err(format!("this build lacks what the core needs: {core_missing:?}"));
    }
    for m in &missing {
        log::line(&format!("missing for a plugin: {m}"));
    }
    let _ = ENGINE.set(e);
    let _ = FRESH.set(fresh);
    Ok(())
}

/// a class field's offset, if the class and field were asked for in a needs file and this build has them
pub fn field_offset(class: &str, field: &str) -> Option<u64> {
    engine()?.classes.get(class)?.field(field).map(|f| f.offset)
}

pub fn class_size(class: &str) -> Option<u64> {
    engine()?.classes.get(class).filter(|c| !c.name.ends_with("(missing)")).map(|c| c.size)
}

/// the pdb's type index for reading engine objects, made on first use (~0.2 s)
struct TypesCell(std::sync::Mutex<fe::types::Types>);
// (only ever used under its mutex; the pdb data it points into is leaked, read-only, for the whole process)
unsafe impl Send for TypesCell {}
unsafe impl Sync for TypesCell {}
static TYPES: OnceLock<Result<TypesCell, String>> = OnceLock::new();

pub fn with_types<R>(f: impl FnOnce(&fe::types::Types) -> Result<R, String>) -> Result<R, String> {
    let cell = TYPES.get_or_init(|| {
        let t = std::time::Instant::now();
        let pdb = std::env::current_exe().map_err(|e| e.to_string())?.with_extension("pdb");
        let image = unsafe {
            let base = GetModuleHandleW(std::ptr::null()) as usize;
            let pe = *((base + 0x3c) as *const u32) as usize;
            (base, base + *((base + pe + 24 + 56) as *const u32) as usize)
        };
        let types = fe::types::Types::open(&pdb, image)?;
        log::line(&format!("type index read in {:.2?}", t.elapsed()));
        Ok(TypesCell(std::sync::Mutex::new(types)))
    });
    match cell {
        Ok(c) => f(&c.0.lock().unwrap()),
        Err(e) => Err(e.clone()),
    }
}
