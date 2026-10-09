//! What fse knows about a Factorio build, all from the factorio.pdb that ships with it:
//!
//! - **identity**: the pdb's GUID and age, and the same pair stamped in factorio.exe (its CodeView record). They must
//!   match: a half-applied update (new exe, old pdb) would put hooks at wrong addresses;
//! - **functions**: every public function symbol, name -> RVA;
//! - **class layouts**: for the classes something needs, their size, base classes and fields (offset, type).
//!
//! Class layouts take a few seconds to read (128 MB of type records), so they are cached per build in
//! `<cache>/<build>/engine.json`. `Needs` lists what fse and its plugins rely on; `check` says what a build lacks
//! and `diff` what moved since the previous build (written to `<cache>/<build>/report.txt` the first time a build is
//! seen, so a game update shows its effect at once).

pub mod types;

use std::collections::{BTreeMap, HashMap};
use std::fmt::Write as _;
use std::path::{Path, PathBuf};

use pdb::FallibleIterator;
use serde::{Deserialize, Serialize};

// ---- identity ----------------------------------------------------------------------------------------------------

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct BuildId {
    pub guid: String,
    pub age: u32,
}

impl std::fmt::Display for BuildId {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}-{}", self.guid, self.age)
    }
}

fn guid_string(b: &[u8]) -> String {
    // (the Windows GUID layout: three little-endian groups, then bytes as they are)
    let d1 = u32::from_le_bytes([b[0], b[1], b[2], b[3]]);
    let d2 = u16::from_le_bytes([b[4], b[5]]);
    let d3 = u16::from_le_bytes([b[6], b[7]]);
    let mut s = format!("{d1:08X}{d2:04X}{d3:04X}");
    for x in &b[8..16] {
        let _ = write!(s, "{x:02X}");
    }
    s
}

/// the build a pdb describes
pub fn pdb_build(pdb_path: &Path) -> Result<BuildId, String> {
    let file = std::fs::File::open(pdb_path).map_err(|e| format!("{}: {e}", pdb_path.display()))?;
    let mut pdb = pdb::PDB::open(file).map_err(|e| format!("{}: {e}", pdb_path.display()))?;
    let info = pdb.pdb_information().map_err(|e| e.to_string())?;
    let dbi = pdb.debug_information().map_err(|e| e.to_string())?;
    // (the pdb crate's GUID is already in the standard order: its text form, without dashes, matches the exe's)
    let guid = info.guid.to_string().replace('-', "").to_uppercase();
    Ok(BuildId { guid, age: dbi.age().unwrap_or(info.age) })
}

/// the build an exe image was linked with: its CodeView ("RSDS") debug record. `image` is the file's bytes
/// (`mapped = false`) or the module as loaded in memory (`mapped = true`).
pub fn exe_build(image: &[u8], mapped: bool) -> Result<BuildId, String> {
    let rd16 = |o: usize| image.get(o..o + 2).map(|b| u16::from_le_bytes([b[0], b[1]]) as usize);
    let rd32 = |o: usize| image.get(o..o + 4).map(|b| u32::from_le_bytes([b[0], b[1], b[2], b[3]]) as usize);
    let bad = || "not a PE image".to_string();
    let pe = rd32(0x3c).ok_or_else(bad)?;
    if image.get(pe..pe + 4) != Some(b"PE\0\0") {
        return Err(bad());
    }
    let n_sections = rd16(pe + 6).ok_or_else(bad)?;
    let opt = pe + 24;
    let opt_size = rd16(pe + 20).ok_or_else(bad)?;
    // PE32+: the data directories start 112 bytes into the optional header; entry 6 is the debug directory
    let dbg_rva = rd32(opt + 112 + 6 * 8).ok_or_else(bad)?;
    let dbg_size = rd32(opt + 112 + 6 * 8 + 4).ok_or_else(bad)?;
    let sections = opt + opt_size;
    let to_offset = |rva: usize| -> Option<usize> {
        if mapped {
            return Some(rva);
        }
        (0..n_sections).find_map(|i| {
            let s = sections + i * 40;
            let (vsize, va, raw_size, raw) = (rd32(s + 8)?, rd32(s + 12)?, rd32(s + 16)?, rd32(s + 20)?);
            (rva >= va && rva < va + vsize.max(raw_size)).then(|| rva - va + raw)
        })
    };
    let dir = to_offset(dbg_rva).ok_or("no debug directory")?;
    for i in 0..dbg_size / 28 {
        let e = dir + i * 28;
        if rd32(e + 12) != Some(2) {
            continue; // (IMAGE_DEBUG_TYPE_CODEVIEW)
        }
        let at = if mapped { rd32(e + 20) } else { rd32(e + 24) }.ok_or_else(bad)?;
        if image.get(at..at + 4) != Some(b"RSDS") {
            continue;
        }
        let guid = image.get(at + 4..at + 20).ok_or_else(bad)?;
        let age = rd32(at + 20).ok_or_else(bad)? as u32;
        return Ok(BuildId { guid: guid_string(guid), age });
    }
    Err("no CodeView record in the exe".into())
}

// ---- what is needed ----------------------------------------------------------------------------------------------

/// what fse or a plugin relies on: function names as the pdb has them (plain C names or MSVC-decorated C++
/// names), and class fields by class name. JSON: {"functions": [...], "classes": {"Inserter": ["field", ...]}}
#[derive(Clone, Debug, Default, Serialize, Deserialize)]
pub struct Needs {
    #[serde(default)]
    pub functions: Vec<String>,
    #[serde(default)]
    pub classes: BTreeMap<String, Vec<String>>,
}

impl Needs {
    pub fn merge(&mut self, other: Needs) {
        for f in other.functions {
            if !self.functions.contains(&f) {
                self.functions.push(f);
            }
        }
        for (c, fields) in other.classes {
            let mine = self.classes.entry(c).or_default();
            for f in fields {
                if !mine.contains(&f) {
                    mine.push(f);
                }
            }
        }
    }

    /// every `*.needs.json` in these folders, merged
    pub fn read_folder(dir: &Path) -> Needs {
        let mut out = Needs::default();
        if let Ok(entries) = std::fs::read_dir(dir) {
            let mut paths: Vec<PathBuf> = entries.flatten().map(|e| e.path())
                .filter(|p| p.to_string_lossy().ends_with(".needs.json")).collect();
            paths.sort();
            for p in paths {
                if let Some(n) = std::fs::read_to_string(&p).ok().and_then(|t| serde_json::from_str::<Needs>(&t).ok()) {
                    out.merge(n);
                }
            }
        }
        out
    }
}

// ---- what a build has --------------------------------------------------------------------------------------------

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Field {
    pub name: String,
    pub offset: u64,
    #[serde(rename = "type")]
    pub ty: String,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Class {
    pub name: String,
    pub size: u64,
    pub bases: Vec<(String, u64)>,
    pub fields: Vec<Field>,
}

impl Class {
    pub fn field(&self, name: &str) -> Option<&Field> {
        self.fields.iter().find(|f| f.name == name)
    }
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Engine {
    pub build: BuildId,
    /// RVAs of the needed functions (all functions are in `Functions`, read fresh: it takes 60 ms)
    pub functions: BTreeMap<String, u32>,
    pub classes: BTreeMap<String, Class>,
}

/// every public function symbol of a pdb, name -> RVA
pub fn functions(pdb_path: &Path) -> Result<HashMap<String, u32>, String> {
    let file = std::fs::File::open(pdb_path).map_err(|e| format!("{}: {e}", pdb_path.display()))?;
    let mut pdb = pdb::PDB::open(file).map_err(|e| e.to_string())?;
    let table = pdb.global_symbols().map_err(|e| e.to_string())?;
    let map = pdb.address_map().map_err(|e| e.to_string())?;
    let mut out = HashMap::with_capacity(120_000);
    let mut it = table.iter();
    while let Some(sym) = it.next().map_err(|e| e.to_string())? {
        if let Ok(pdb::SymbolData::Public(p)) = sym.parse() {
            if p.function {
                if let Some(r) = p.offset.to_rva(&map) {
                    out.insert(p.name.to_string().into_owned(), r.0);
                }
            }
        }
    }
    Ok(out)
}

/// a type as C++-ish text ("Entity*", "uint32_t", "std::vector<...>")
fn type_name(finder: &pdb::TypeFinder<'_>, idx: pdb::TypeIndex, depth: u32) -> String {
    if depth > 8 {
        return "...".into();
    }
    match finder.find(idx).and_then(|t| t.parse()) {
        Ok(pdb::TypeData::Primitive(p)) => {
            let base = match p.kind {
                pdb::PrimitiveKind::Void => "void", pdb::PrimitiveKind::Bool8 => "bool",
                pdb::PrimitiveKind::Char | pdb::PrimitiveKind::RChar | pdb::PrimitiveKind::I8 => "int8_t",
                pdb::PrimitiveKind::UChar | pdb::PrimitiveKind::U8 => "uint8_t",
                pdb::PrimitiveKind::WChar | pdb::PrimitiveKind::RChar16 => "char16_t",
                pdb::PrimitiveKind::I16 | pdb::PrimitiveKind::Short => "int16_t",
                pdb::PrimitiveKind::U16 | pdb::PrimitiveKind::UShort => "uint16_t",
                pdb::PrimitiveKind::I32 | pdb::PrimitiveKind::Long | pdb::PrimitiveKind::HRESULT => "int32_t",
                pdb::PrimitiveKind::U32 | pdb::PrimitiveKind::ULong => "uint32_t",
                pdb::PrimitiveKind::I64 | pdb::PrimitiveKind::Quad => "int64_t",
                pdb::PrimitiveKind::U64 | pdb::PrimitiveKind::UQuad => "uint64_t",
                pdb::PrimitiveKind::F32 => "float", pdb::PrimitiveKind::F64 => "double",
                _ => "?",
            };
            if p.indirection.is_some() { format!("{base}*") } else { base.to_string() }
        }
        Ok(pdb::TypeData::Pointer(p)) => format!("{}*", type_name(finder, p.underlying_type, depth + 1)),
        Ok(pdb::TypeData::Modifier(m)) => {
            let inner = type_name(finder, m.underlying_type, depth + 1);
            if m.constant { format!("const {inner}") } else { inner }
        }
        Ok(pdb::TypeData::Array(a)) => {
            let n = a.dimensions.last().copied().unwrap_or(0);
            format!("{}[{} bytes]", type_name(finder, a.element_type, depth + 1), n)
        }
        Ok(pdb::TypeData::Class(c)) => c.name.to_string().into_owned(),
        Ok(pdb::TypeData::Union(u)) => u.name.to_string().into_owned(),
        Ok(pdb::TypeData::Enumeration(e)) => e.name.to_string().into_owned(),
        Ok(pdb::TypeData::Bitfield(b)) => format!("{}:{}@{}", type_name(finder, b.underlying_type, depth + 1), b.length, b.position),
        Ok(pdb::TypeData::Procedure(_)) | Ok(pdb::TypeData::MemberFunction(_)) => "fn".into(),
        Ok(_) => "?".into(),
        Err(_) => format!("#{}", idx.0),
    }
}

/// layouts of the named classes (exact names as the pdb has them, "Inserter", "TransportLine"); classes not
/// found are left out. Reads all type records: seconds, hence the cache.
pub fn classes(pdb_path: &Path, wanted: &[String]) -> Result<BTreeMap<String, Class>, String> {
    let file = std::fs::File::open(pdb_path).map_err(|e| format!("{}: {e}", pdb_path.display()))?;
    let mut pdb = pdb::PDB::open(file).map_err(|e| e.to_string())?;
    let info = pdb.type_information().map_err(|e| e.to_string())?;
    let mut finder = info.finder();
    let mut found: BTreeMap<String, (u64, Option<pdb::TypeIndex>)> = BTreeMap::new();
    let mut it = info.iter();
    while let Some(t) = it.next().map_err(|e| e.to_string())? {
        finder.update(&it);
        if let Ok(pdb::TypeData::Class(c)) = t.parse() {
            if c.properties.forward_reference() {
                continue;
            }
            let name = c.name.to_string();
            if wanted.iter().any(|w| w.as_str() == name) && !found.contains_key(name.as_ref()) {
                found.insert(name.into_owned(), (c.size, c.fields));
            }
        }
    }
    let mut out = BTreeMap::new();
    for (name, (size, fields)) in found {
        let mut cls = Class { name: name.clone(), size, bases: Vec::new(), fields: Vec::new() };
        let mut next = fields;
        while let Some(idx) = next.take() {
            if let Ok(pdb::TypeData::FieldList(list)) = finder.find(idx).and_then(|t| t.parse()) {
                for f in list.fields {
                    match f {
                        pdb::TypeData::Member(m) => cls.fields.push(Field {
                            name: m.name.to_string().into_owned(), offset: m.offset,
                            ty: type_name(&finder, m.field_type, 0),
                        }),
                        pdb::TypeData::BaseClass(b) => cls.bases.push((type_name(&finder, b.base_class, 0), b.offset as u64)),
                        _ => {}
                    }
                }
                next = list.continuation;
            }
        }
        out.insert(name, cls);
    }
    Ok(out)
}

/// "?update@Inserter@@QEAAXXZ" -> "Inserter::update" (plain C names come back as they are)
pub fn undecorate(name: &str) -> String {
    if !name.starts_with('?') {
        return name.to_string();
    }
    let Ok(c) = std::ffi::CString::new(name) else { return name.to_string() };
    let mut buf = [0u8; 1024];
    let n = unsafe {
        windows_sys::Win32::System::Diagnostics::Debug::UnDecorateSymbolName(c.as_ptr() as *const u8, buf.as_mut_ptr(),
                                                                             buf.len() as u32, 0x1000)
    };
    if n == 0 { name.to_string() } else { String::from_utf8_lossy(&buf[..n as usize]).into_owned() }
}

/// names of the classes (complete definitions) containing `part`
pub fn class_names(pdb_path: &Path, part: &str, limit: usize) -> Result<Vec<(String, u64)>, String> {
    let file = std::fs::File::open(pdb_path).map_err(|e| format!("{}: {e}", pdb_path.display()))?;
    let mut pdb = pdb::PDB::open(file).map_err(|e| e.to_string())?;
    let info = pdb.type_information().map_err(|e| e.to_string())?;
    let mut out = BTreeMap::new();
    let mut it = info.iter();
    while let Some(t) = it.next().map_err(|e| e.to_string())? {
        if let Ok(pdb::TypeData::Class(c)) = t.parse() {
            if !c.properties.forward_reference() {
                let n = c.name.to_string();
                if n.contains(part) {
                    out.entry(n.into_owned()).or_insert(c.size);
                }
            }
        }
    }
    Ok(out.into_iter().take(limit).collect())
}

// ---- cache, check, diff ------------------------------------------------------------------------------------------

/// the engine description of the build `pdb_path` belongs to, for `needs`: from `<cache>/<build>/engine.json`
/// when it covers them, else read from the pdb and cached. Returns (engine, read_from_pdb)
pub fn load(cache: &Path, pdb_path: &Path, fns: &HashMap<String, u32>, needs: &Needs) -> Result<(Engine, bool), String> {
    let build = pdb_build(pdb_path)?;
    let dir = cache.join(build.to_string());
    let file = dir.join("engine.json");
    let functions: BTreeMap<String, u32> = needs.functions.iter()
        .filter_map(|f| fns.get(f).map(|r| (f.clone(), *r))).collect();
    if let Some(mut e) = std::fs::read_to_string(&file).ok().and_then(|t| serde_json::from_str::<Engine>(&t).ok()) {
        if e.build == build && needs.classes.keys().all(|c| e.classes.contains_key(c) || e_missing(&e, c)) {
            e.functions = functions;
            return Ok((e, false));
        }
    }
    let wanted: Vec<String> = needs.classes.keys().cloned().collect();
    let mut classes = if wanted.is_empty() { BTreeMap::new() } else { classes(pdb_path, &wanted)? };
    // (remember which wanted classes this build lacks, so the cache answers for them too)
    for c in &wanted {
        classes.entry(c.clone()).or_insert_with(|| Class { name: format!("{c} (missing)"), size: 0, bases: vec![], fields: vec![] });
    }
    let e = Engine { build, functions, classes };
    std::fs::create_dir_all(&dir).map_err(|err| format!("{}: {err}", dir.display()))?;
    std::fs::write(&file, serde_json::to_string_pretty(&e).unwrap_or_default()).map_err(|err| err.to_string())?;
    Ok((e, true))
}

fn e_missing(e: &Engine, c: &str) -> bool {
    e.classes.get(c).map(|k| k.name.ends_with("(missing)")).unwrap_or(false)
}

/// what `needs` asks for that this build lacks
pub fn check(e: &Engine, needs: &Needs) -> Vec<String> {
    let mut out = Vec::new();
    for f in &needs.functions {
        if !e.functions.contains_key(f) {
            out.push(format!("function {f}: missing"));
        }
    }
    for (c, fields) in &needs.classes {
        match e.classes.get(c) {
            Some(k) if !k.name.ends_with("(missing)") => {
                for f in fields {
                    if k.field(f).is_none() {
                        out.push(format!("{c}::{f}: missing"));
                    }
                }
            }
            _ => out.push(format!("class {c}: missing")),
        }
    }
    out
}

/// what moved between two builds, for what `needs` uses
pub fn diff(old: &Engine, new: &Engine, needs: &Needs) -> Vec<String> {
    let mut out = Vec::new();
    for (c, fields) in &needs.classes {
        let (Some(a), Some(b)) = (old.classes.get(c), new.classes.get(c)) else { continue };
        if a.size != b.size {
            out.push(format!("{c}: size {} -> {}", a.size, b.size));
        }
        for f in fields {
            match (a.field(f), b.field(f)) {
                (Some(x), Some(y)) if x.offset != y.offset || x.ty != y.ty => {
                    out.push(format!("{c}::{f}: offset {} ({}) -> {} ({})", x.offset, x.ty, y.offset, y.ty))
                }
                (Some(_), None) => out.push(format!("{c}::{f}: gone")),
                _ => {}
            }
        }
    }
    out
}

/// the newest other build in the cache (the one before an update)
pub fn previous(cache: &Path, current: &BuildId) -> Option<Engine> {
    let mut best: Option<(std::time::SystemTime, Engine)> = None;
    for entry in std::fs::read_dir(cache).ok()?.flatten() {
        let p = entry.path().join("engine.json");
        let Ok(meta) = std::fs::metadata(&p) else { continue };
        let Some(e) = std::fs::read_to_string(&p).ok().and_then(|t| serde_json::from_str::<Engine>(&t).ok()) else { continue };
        if &e.build == current {
            continue;
        }
        let t = meta.modified().unwrap_or(std::time::UNIX_EPOCH);
        if best.as_ref().map(|(bt, _)| t > *bt).unwrap_or(true) {
            best = Some((t, e));
        }
    }
    best.map(|(_, e)| e)
}

/// the first-time report for a build: what it lacks, what moved since the previous build
pub fn report(cache: &Path, e: &Engine, needs: &Needs, exe_version: &str) -> String {
    let mut s = format!("build {} ({exe_version})\n", e.build);
    let missing = check(e, needs);
    let _ = writeln!(s, "needs: {} functions, {} classes; missing: {}", needs.functions.len(), needs.classes.len(),
                     if missing.is_empty() { "nothing".to_string() } else { missing.len().to_string() });
    for m in &missing {
        let _ = writeln!(s, "  {m}");
    }
    match previous(cache, &e.build) {
        Some(old) => {
            let moved = diff(&old, e, needs);
            let _ = writeln!(s, "since build {}: {}", old.build, if moved.is_empty() { "nothing used moved".to_string() } else { format!("{} changes", moved.len()) });
            for m in &moved {
                let _ = writeln!(s, "  {m}");
            }
        }
        None => s.push_str("no earlier build cached\n"),
    }
    s
}
