//! fse-pdb: what a Factorio build offers fse, from its factorio.pdb.
//!
//!   fse-pdb info                     exe and pdb build ids (they must match), function count
//!   fse-pdb functions <part> [n]     functions whose readable name contains <part>, with their pdb names
//!   fse-pdb classes <part> [n]       class names containing <part>, with sizes
//!   fse-pdb class <Name>...          layouts: size, bases, fields with offsets and types
//!   fse-pdb report                   check the needs (dist/needs.json, dist/plugins/*.needs.json) against this
//!                                        build, cache it in dist/cache/<build>, compare with the previous build
//! The game is FACTORIO_EXE, else the Steam install; --game <exe> overrides.

use std::path::{Path, PathBuf};

use fse_engine as fe;

/// the game: FACTORIO_EXE, else the first Steam library that has it (the usual places), else Steam's default
fn default_game() -> PathBuf {
    let rel = r"steamapps\common\Factorio\bin\x64\factorio.exe";
    let roots = [r"C:\Program Files (x86)\Steam", r"C:\Program Files\Steam", r"D:\SteamLibrary", r"E:\SteamLibrary",
                 r"F:\SteamLibrary", r"G:\SteamLibrary", r"D:\Steam", r"E:\Steam"];
    roots.iter().map(|r| PathBuf::from(r).join(rel)).find(|p| p.exists())
        .unwrap_or_else(|| PathBuf::from(roots[0]).join(rel))
}

fn main() {
    let mut args: Vec<String> = std::env::args().skip(1).collect();
    let mut game = std::env::var("FACTORIO_EXE").map(PathBuf::from).unwrap_or_else(|_| default_game());
    if let Some(i) = args.iter().position(|a| a == "--game") {
        if i + 1 < args.len() {
            game = PathBuf::from(args.remove(i + 1));
            args.remove(i);
        }
    }
    let pdb = game.with_extension("pdb");
    let r = match args.first().map(String::as_str) {
        Some("info") => info(&game, &pdb),
        Some("functions") => functions(&pdb, args.get(1).map(String::as_str).unwrap_or(""), limit(&args)),
        Some("classes") => classes(&pdb, args.get(1).map(String::as_str).unwrap_or(""), limit(&args)),
        Some("class") => class(&pdb, &args[1..]),
        Some("report") => report(&game, &pdb),
        _ => Err("usage: fse-pdb info | functions <part> [n] | classes <part> [n] | class <Name>... | report".into()),
    };
    if let Err(e) = r {
        eprintln!("fse-pdb: {e}");
        std::process::exit(1);
    }
}

fn limit(args: &[String]) -> usize {
    args.get(2).and_then(|s| s.parse().ok()).unwrap_or(40)
}

fn info(game: &Path, pdb: &Path) -> Result<(), String> {
    let exe = std::fs::read(game).map_err(|e| format!("{}: {e}", game.display()))?;
    let a = fe::exe_build(&exe, false)?;
    let b = fe::pdb_build(pdb)?;
    println!("exe {a}\npdb {b}\n{}", if a == b { "match" } else { "MISMATCH: the pdb is not this exe's (a half-applied update?)" });
    let t = std::time::Instant::now();
    let f = fe::functions(pdb)?;
    println!("{} functions read in {:.0?}", f.len(), t.elapsed());
    Ok(())
}

fn functions(pdb: &Path, part: &str, n: usize) -> Result<(), String> {
    let f = fe::functions(pdb)?;
    let mut rows: Vec<(String, &String, u32)> = f.iter().filter_map(|(k, v)| {
        let plain = fe::undecorate(k);
        plain.contains(part).then_some((plain, k, *v))
    }).collect();
    rows.sort();
    for (plain, raw, rva) in rows.iter().take(n) {
        println!("{rva:08x}  {plain}  {}", if plain != *raw { raw.as_str() } else { "" });
    }
    if rows.len() > n {
        println!("... {} more", rows.len() - n);
    }
    Ok(())
}

fn classes(pdb: &Path, part: &str, n: usize) -> Result<(), String> {
    for (name, size) in fe::class_names(pdb, part, n)? {
        println!("{size:6}  {name}");
    }
    Ok(())
}

fn class(pdb: &Path, names: &[String]) -> Result<(), String> {
    let t = std::time::Instant::now();
    let found = fe::classes(pdb, names)?;
    for n in names {
        match found.get(n) {
            None => println!("{n}: not found"),
            Some(c) => {
                println!("{} ({} bytes)", c.name, c.size);
                for (b, off) in &c.bases {
                    println!("  base {off:5}  {b}");
                }
                for f in &c.fields {
                    println!("  {:6}  {}  {}", f.offset, f.name, f.ty);
                }
            }
        }
    }
    eprintln!("({:.1?})", t.elapsed());
    Ok(())
}

fn report(game: &Path, pdb: &Path) -> Result<(), String> {
    let dist = Path::new(env!("CARGO_MANIFEST_DIR")).join("..").join("..").join("dist");
    let mut needs = fe::Needs::read_folder(&dist);
    needs.merge(fe::Needs::read_folder(&dist.join("plugins")));
    let fns = fe::functions(pdb)?;
    let (e, fresh) = fe::load(&dist.join("cache"), pdb, &fns, &needs)?;
    let version = std::fs::metadata(game).map(|m| format!("{} bytes", m.len())).unwrap_or_default();
    let text = fe::report(&dist.join("cache"), &e, &needs, &version);
    println!("{text}{}", if fresh { "(read from the pdb and cached)" } else { "(from the cache)" });
    Ok(())
}
