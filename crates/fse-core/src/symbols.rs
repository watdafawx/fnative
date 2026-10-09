//! Engine symbols: the public function symbols of factorio.pdb (shipped with every build), as absolute addresses
//! in this process. Names are as the PDB has them: plain C names (`lua_pushstring`) or MSVC-mangled C++ names
//! (`?update@Inserter@@QEAAXXZ`). Read fresh at every start (60 ms), so a game update needs nothing done.

use std::collections::HashMap;

use fse_engine::undecorate;
use windows_sys::Win32::System::LibraryLoader::GetModuleHandleW;

pub struct Symbols {
    base: usize,
    rva: HashMap<String, u32>,
}

impl Symbols {
    pub fn new(rva: HashMap<String, u32>) -> Self {
        let base = unsafe { GetModuleHandleW(std::ptr::null()) } as usize;
        Symbols { base, rva }
    }

    pub fn len(&self) -> usize {
        self.rva.len()
    }

    pub fn map(&self) -> &HashMap<String, u32> {
        &self.rva
    }

    /// the address of a function in this process, by its PDB name
    pub fn addr(&self, name: &str) -> Option<usize> {
        self.rva.get(name).map(|r| self.base + *r as usize)
    }

    /// readable names of the functions matching `part` ("Inserter::update", "lua_push"): C++ names are matched
    /// undecorated ("?update@Inserter@@QEAAXXZ" is "Inserter::update")
    pub fn find(&self, part: &str, limit: usize) -> Vec<String> {
        // (cheap filter first: every identifier of the query must appear in the decorated name)
        let words: Vec<&str> = part.split(|c: char| !(c.is_alphanumeric() || c == '_')).filter(|w| !w.is_empty()).collect();
        let mut out: Vec<String> = Vec::new();
        for name in self.rva.keys() {
            if !words.iter().all(|w| name.contains(w)) {
                continue;
            }
            let plain = undecorate(name);
            if plain.contains(part) {
                out.push(plain);
            }
        }
        out.sort_unstable();
        out.dedup();
        out.truncate(limit);
        out
    }
}
