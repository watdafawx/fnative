//! Engine objects as values: every type record of factorio.pdb, indexed once (a few seconds, then kept), and a reader
//! that walks a live object by field names and decodes what it finds. Pointers to polymorphic classes resolve to the
//! object's real class through MSVC's RTTI. Memory is read with ReadProcessMemory: a bad pointer is an error, never a
//! crash.
//!
//! Paths: field names separated by dots ("entityTarget.target.heldStack"). Fields of base classes are found too;
//! pointers are followed; a number indexes an array or a std::vector ("items.3"). A segment "^Class" views the current
//! object as that base or derived class.

use std::collections::HashMap;
use std::path::Path;

use pdb::{FallibleIterator, PrimitiveKind, TypeData, TypeFinder, TypeIndex, TypeInformation};
use serde_json::{json, Map, Value};

pub struct Types {
    finder: TypeFinder<'static>,
    /// complete class, struct and union definitions by name
    by_name: HashMap<String, TypeIndex>,
    /// complete enumerations by name
    enums: HashMap<String, TypeIndex>,
    image: (usize, usize),
}

/// a type with modifiers stripped and forward references resolved
#[derive(Clone, Debug)]
enum Ty {
    Prim(PrimitiveKind),
    /// a pointer to a primitive (void*, char*, int*)
    PrimPtr(PrimitiveKind),
    Ptr(TypeIndex),
    Class(TypeIndex, String, u64),
    /// name, underlying type, the enumeration's own record (for its value names)
    Enum(String, TypeIndex, TypeIndex),
    Array(TypeIndex, u64),
    Bits(TypeIndex, u8, u8),
    Unknown(String),
}

fn prim_size(k: PrimitiveKind) -> u64 {
    use PrimitiveKind::*;
    match k {
        Bool8 | Char | RChar | UChar | I8 | U8 => 1,
        WChar | RChar16 | I16 | Short | U16 | UShort => 2,
        I32 | Long | U32 | ULong | F32 | HRESULT | Bool32 | RChar32 => 4,
        I64 | Quad | U64 | UQuad | F64 | Bool64 => 8,
        _ => 0,
    }
}

/// bytes of this process at `addr`, or None if it isn't readable
pub fn read_mem(addr: usize, len: usize) -> Option<Vec<u8>> {
    use windows_sys::Win32::System::Diagnostics::Debug::ReadProcessMemory;
    use windows_sys::Win32::System::Threading::GetCurrentProcess;
    if addr < 0x10000 || len > 1 << 20 {
        return None;
    }
    let mut buf = vec![0u8; len];
    let mut got = 0usize;
    let ok = unsafe {
        ReadProcessMemory(GetCurrentProcess(), addr as *const _, buf.as_mut_ptr() as *mut _, len, &mut got)
    };
    (ok != 0 && got == len).then_some(buf)
}

/// writes bytes of this process at `addr`; false if it isn't writable
pub fn write_mem(addr: usize, bytes: &[u8]) -> bool {
    use windows_sys::Win32::System::Diagnostics::Debug::WriteProcessMemory;
    use windows_sys::Win32::System::Threading::GetCurrentProcess;
    if addr < 0x10000 || read_mem(addr, bytes.len()).is_none() {
        return false;
    }
    let mut done = 0usize;
    let ok = unsafe {
        WriteProcessMemory(GetCurrentProcess(), addr as *const _, bytes.as_ptr() as *const _, bytes.len(), &mut done)
    };
    ok != 0 && done == bytes.len()
}

fn read_u64(addr: usize) -> Option<u64> {
    read_mem(addr, 8).map(|b| u64::from_le_bytes(b.try_into().unwrap()))
}

fn read_u32(addr: usize) -> Option<u32> {
    read_mem(addr, 4).map(|b| u32::from_le_bytes(b.try_into().unwrap()))
}

/// ".?AVInserter@@" -> "Inserter", ".?AVItem@Foo@@" -> "Foo::Item" (templates are left as they are)
fn rtti_name(raw: &str) -> String {
    let s = raw.trim_start_matches(".?AV").trim_start_matches(".?AU").trim_end_matches("@@");
    if s.contains('?') || s.contains('$') {
        return s.to_string();
    }
    s.split('@').rev().collect::<Vec<_>>().join("::")
}

impl Types {
    /// reads every type record of the pdb (seconds; keep the result)
    pub fn open(pdb_path: &Path, image: (usize, usize)) -> Result<Types, String> {
        let file = std::fs::File::open(pdb_path).map_err(|e| format!("{}: {e}", pdb_path.display()))?;
        let mut pdb = pdb::PDB::open(file).map_err(|e| e.to_string())?;
        let info: &'static TypeInformation<'static> = Box::leak(Box::new(pdb.type_information().map_err(|e| e.to_string())?));
        let mut finder = info.finder();
        let mut by_name = HashMap::new();
        let mut enums = HashMap::new();
        let mut it = info.iter();
        while let Some(t) = it.next().map_err(|e| e.to_string())? {
            finder.update(&it);
            let (name, fwd) = match t.parse() {
                Ok(TypeData::Class(c)) => (c.name.to_string().into_owned(), c.properties.forward_reference()),
                Ok(TypeData::Union(u)) => (u.name.to_string().into_owned(), u.properties.forward_reference()),
                Ok(TypeData::Enumeration(e)) => {
                    if !e.properties.forward_reference() {
                        enums.entry(e.name.to_string().into_owned()).or_insert(t.index());
                    }
                    continue;
                }
                _ => continue,
            };
            if !fwd {
                by_name.entry(name).or_insert(t.index());
            }
        }
        Ok(Types { finder, by_name, enums, image })
    }

    pub fn has_class(&self, name: &str) -> bool {
        self.by_name.contains_key(name)
    }

    fn data(&self, idx: TypeIndex) -> Option<TypeData<'static>> {
        self.finder.find(idx).ok()?.parse().ok()
    }

    fn ty(&self, idx: TypeIndex) -> Ty {
        let mut idx = idx;
        for _ in 0..16 {
            match self.data(idx) {
                Some(TypeData::Modifier(m)) => idx = m.underlying_type,
                Some(TypeData::Primitive(p)) if p.indirection.is_some() => return Ty::PrimPtr(p.kind),
                Some(TypeData::Primitive(p)) => return Ty::Prim(p.kind),
                Some(TypeData::Pointer(p)) => return Ty::Ptr(p.underlying_type),
                Some(TypeData::Class(c)) => {
                    let name = c.name.to_string().into_owned();
                    if c.properties.forward_reference() {
                        match self.by_name.get(&name) {
                            Some(&i) => idx = i,
                            None => return Ty::Unknown(name),
                        }
                    } else {
                        return Ty::Class(idx, name, c.size);
                    }
                }
                Some(TypeData::Union(u)) => {
                    let name = u.name.to_string().into_owned();
                    if u.properties.forward_reference() {
                        match self.by_name.get(&name) {
                            Some(&i) => idx = i,
                            None => return Ty::Unknown(name),
                        }
                    } else {
                        return Ty::Class(idx, name, u.size);
                    }
                }
                Some(TypeData::Enumeration(e)) => return Ty::Enum(e.name.to_string().into_owned(), e.underlying_type, idx),
                Some(TypeData::Array(a)) => return Ty::Array(a.element_type, a.dimensions.last().copied().unwrap_or(0) as u64),
                Some(TypeData::Bitfield(b)) => return Ty::Bits(b.underlying_type, b.length, b.position),
                Some(_) => return Ty::Unknown("?".into()),
                None => return Ty::Unknown(format!("#{}", idx.0)),
            }
        }
        Ty::Unknown("?".into())
    }

    fn size(&self, t: &Ty) -> u64 {
        match t {
            Ty::Prim(k) => prim_size(*k),
            Ty::Ptr(_) | Ty::PrimPtr(_) => 8,
            Ty::Class(_, _, s) | Ty::Array(_, s) => *s,
            Ty::Enum(_, u, _) | Ty::Bits(u, _, _) => self.size(&self.ty(*u)),
            Ty::Unknown(_) => 0,
        }
    }

    fn type_name(&self, t: &Ty) -> String {
        self.type_name_at(t, 0)
    }

    fn type_name_at(&self, t: &Ty, depth: u32) -> String {
        if depth > 8 {
            return "...".into();
        }
        match t {
            Ty::Prim(k) => format!("{k:?}"),
            Ty::PrimPtr(k) => format!("{k:?}*"),
            Ty::Ptr(p) => format!("{}*", self.type_name_at(&self.ty(*p), depth + 1)),
            Ty::Class(_, n, _) | Ty::Enum(n, _, _) | Ty::Unknown(n) => n.clone(),
            Ty::Array(e, s) => format!("{}[{s} bytes]", self.type_name_at(&self.ty(*e), depth + 1)),
            Ty::Bits(u, l, _) => format!("{}:{l}", self.type_name_at(&self.ty(*u), depth + 1)),
        }
    }

    fn enum_name(&self, e: TypeIndex, value: i64) -> Option<String> {
        let Some(TypeData::Enumeration(en)) = self.data(e) else { return None };
        let mut next = Some(en.fields);
        while let Some(idx) = next.take() {
            if let Some(TypeData::FieldList(list)) = self.data(idx) {
                for f in list.fields {
                    if let TypeData::Enumerate(x) = f {
                        let v = match x.value {
                            pdb::Variant::U8(v) => v as i64, pdb::Variant::U16(v) => v as i64,
                            pdb::Variant::U32(v) => v as i64, pdb::Variant::U64(v) => v as i64,
                            pdb::Variant::I8(v) => v as i64, pdb::Variant::I16(v) => v as i64,
                            pdb::Variant::I32(v) => v as i64, pdb::Variant::I64(v) => v,
                        };
                        if v == value {
                            return Some(x.name.to_string().into_owned());
                        }
                    }
                }
                next = list.continuation;
            }
        }
        None
    }

    /// the direct members and bases of a class: (name, offset, type); bases as ("^Base", offset, type)
    fn members(&self, class: TypeIndex) -> Vec<(String, u64, TypeIndex)> {
        let mut out = Vec::new();
        let mut next = match self.data(class) {
            Some(TypeData::Class(c)) => c.fields,
            Some(TypeData::Union(u)) => Some(u.fields),
            _ => None,
        };
        while let Some(idx) = next.take() {
            if let Some(TypeData::FieldList(list)) = self.data(idx) {
                for f in list.fields {
                    match f {
                        TypeData::Member(m) => out.push((m.name.to_string().into_owned(), m.offset, m.field_type)),
                        TypeData::BaseClass(b) => {
                            let n = self.type_name(&self.ty(b.base_class));
                            out.push((format!("^{n}"), b.offset as u64, b.base_class))
                        }
                        _ => {}
                    }
                }
                next = list.continuation;
            }
        }
        out
    }

    /// a field of a class or of any of its bases: (offset from the class's start, type)
    fn find_field(&self, class: TypeIndex, name: &str, depth: u32) -> Option<(u64, TypeIndex)> {
        let members = self.members(class);
        if let Some((_, off, t)) = members.iter().find(|(n, _, _)| n == name) {
            return Some((*off, *t));
        }
        if depth > 12 {
            return None;
        }
        for (n, off, t) in &members {
            if n.starts_with('^') {
                if let Ty::Class(b, _, _) = self.ty(*t) {
                    if let Some((o, ft)) = self.find_field(b, name, depth + 1) {
                        return Some((off + o, ft));
                    }
                }
            }
        }
        None
    }

    /// where `base` sits inside `class` (through any depth of bases)
    fn base_offset(&self, class: TypeIndex, base: &str, depth: u32) -> Option<u64> {
        if depth > 12 {
            return None;
        }
        for (n, off, t) in self.members(class) {
            if let Some(b) = n.strip_prefix('^') {
                if b == base {
                    return Some(off);
                }
                if let Ty::Class(bi, _, _) = self.ty(t) {
                    if let Some(o) = self.base_offset(bi, base, depth + 1) {
                        return Some(off + o);
                    }
                }
            }
        }
        None
    }

    /// the real class of the object at `addr` (one with a vtable from the game): (complete object address, class name)
    pub fn dynamic(&self, addr: usize) -> Option<(usize, String)> {
        let (base, end) = self.image;
        let vt = read_u64(addr)? as usize;
        if vt < base || vt >= end {
            return None;
        }
        let col = read_u64(vt - 8)? as usize;
        if col < base || col >= end || read_u32(col)? != 1 {
            return None;
        }
        let offset = read_u32(col + 4)? as usize;
        let td = base + read_u32(col + 12)? as usize;
        let raw = read_mem(td + 16, 256)?;
        let name = String::from_utf8_lossy(&raw[..raw.iter().position(|&c| c == 0)?]).into_owned();
        Some((addr.wrapping_sub(offset), rtti_name(&name)))
    }

    /// a pointer's target as (address, type): resolved to its real class when it has one
    fn follow(&self, addr: usize, pointee: TypeIndex) -> Option<(usize, Ty)> {
        let p = read_u64(addr)? as usize;
        if p == 0 {
            return None;
        }
        let t = self.ty(pointee);
        if let Ty::Class(..) = t {
            if let Some((full, name)) = self.dynamic(p) {
                if let Some(&i) = self.by_name.get(&name) {
                    return Some((full, self.ty(i)));
                }
            }
        }
        Some((p, t))
    }

    /// walks `path` from the object at `addr` of class `class` and decodes the value there (`depth`: how many levels
    /// of nested objects are expanded)
    pub fn read(&self, addr: usize, class: &str, path: &str, depth: u32) -> Result<Value, String> {
        let start = *self.by_name.get(class).ok_or(format!("no class {class} in this build"))?;
        let (addr, t) = self.walk(addr, self.ty(start), path)?;
        if read_mem(addr, self.size(&t).clamp(1, 4096) as usize).is_none() {
            return Err(format!("can't read {} at {addr:#x}", self.type_name(&t)));
        }
        Ok(self.decode(addr, &t, depth))
    }

    /// the object at `addr` (its real class from RTTI) walked along `path`
    pub fn read_dynamic(&self, addr: usize, path: &str, depth: u32) -> Result<Value, String> {
        let (full, name) = self.dynamic(addr).ok_or("not an engine object with a vtable")?;
        self.read(full, &name, path, depth)
    }

    fn walk(&self, mut addr: usize, mut t: Ty, path: &str) -> Result<(usize, Ty), String> {
        for seg in path.split('.').filter(|s| !s.is_empty()) {
            // (a pointer is followed before its fields are looked up)
            if let Ty::Ptr(p) = t {
                (addr, t) = self.follow(addr, p).ok_or(format!("null pointer before {seg}"))?;
            }
            if let Some(target) = seg.strip_prefix('^') {
                let Ty::Class(ci, cname, _) = &t else { return Err(format!("^{target}: not a class")) };
                let &ti = self.by_name.get(target).ok_or(format!("no class {target}"))?;
                if let Some(off) = self.base_offset(*ci, target, 0) {
                    addr += off as usize; // (up to a base)
                } else if let Some(off) = self.base_offset(ti, cname, 0) {
                    addr -= off as usize; // (down to a derived class)
                } else {
                    return Err(format!("{cname} and {target} are unrelated"));
                }
                t = self.ty(ti);
                continue;
            }
            if let Ok(i) = seg.parse::<u64>() {
                (addr, t) = self.index(addr, &t, i).ok_or(format!("{seg}: out of range"))?;
                continue;
            }
            let Ty::Class(ci, cname, _) = &t else { return Err(format!("{seg}: {} has no fields", self.type_name(&t))) };
            let (off, ft) = self.find_field(*ci, seg, 0).ok_or(format!("{cname} has no field {seg}"))?;
            addr += off as usize;
            t = self.ty(ft);
        }
        Ok((addr, t))
    }

    /// element i of an array or std::vector
    fn index(&self, addr: usize, t: &Ty, i: u64) -> Option<(usize, Ty)> {
        match t {
            Ty::Array(e, size) => {
                let et = self.ty(*e);
                let es = self.size(&et).max(1);
                (i < size / es).then(|| (addr + (i * es) as usize, et))
            }
            Ty::Class(..) if self.type_name(t).starts_with("std::vector<") => {
                let (first, elem) = self.vector(addr, t)?;
                let es = self.size(&elem).max(1);
                let last = read_u64(addr + 8)? as usize;
                let n = (last.saturating_sub(first)) as u64 / es;
                (i < n).then(|| (first + (i * es) as usize, elem))
            }
            _ => None,
        }
    }

    /// a std::vector's first element address and element type (MSVC: _Mypair._Myval2._Myfirst, _Mylast, _Myend)
    fn vector(&self, addr: usize, t: &Ty) -> Option<(usize, Ty)> {
        let (a, ft) = self.walk(addr, t.clone(), "_Mypair._Myval2._Myfirst").ok()?;
        let Ty::Ptr(e) = ft else { return None };
        Some((read_u64(a)? as usize, self.ty(e)))
    }

    fn decode(&self, addr: usize, t: &Ty, depth: u32) -> Value {
        let name = self.type_name(t);
        match t {
            Ty::Prim(k) => self.prim(addr, *k).unwrap_or(Value::Null),
            // (an enum's value by its name when it has one)
            Ty::Enum(_, u, e) => {
                let v = self.decode(addr, &self.ty(*u), depth);
                v.as_i64().and_then(|n| self.enum_name(*e, n)).map(Value::String).unwrap_or(v)
            }
            Ty::Bits(u, len, pos) => {
                let size = self.size(&self.ty(*u)) as usize;
                let Some(b) = read_mem(addr, size) else { return Value::Null };
                let mut v = 0u64;
                for (i, x) in b.iter().enumerate() {
                    v |= (*x as u64) << (8 * i);
                }
                json!((v >> pos) & ((1u64 << len) - 1))
            }
            Ty::PrimPtr(_) => match read_u64(addr) {
                Some(0) | None => Value::Null,
                Some(v) => json!({"ptr": v, "type": name}),
            },
            Ty::Ptr(p) => match read_u64(addr) {
                Some(0) => Value::Null,
                Some(v) => {
                    let pt = self.ty(*p);
                    // (a class pointer as its real class, at the complete object's address)
                    let (ptr, ty) = match self.dynamic(v as usize) {
                        Some((full, n)) if matches!(pt, Ty::Class(..)) => (full as u64, n),
                        _ => (v, self.type_name(&pt)),
                    };
                    json!({"ptr": ptr, "type": ty})
                }
                None => Value::Null,
            },
            Ty::Array(e, size) => {
                let et = self.ty(*e);
                let es = self.size(&et).max(1);
                if matches!(et, Ty::Prim(PrimitiveKind::Char | PrimitiveKind::RChar | PrimitiveKind::UChar)) {
                    return read_mem(addr, *size as usize)
                        .map(|b| json!(String::from_utf8_lossy(&b[..b.iter().position(|&c| c == 0).unwrap_or(b.len())])))
                        .unwrap_or(Value::Null);
                }
                Value::Array((0..(size / es).min(64)).map(|i| self.decode(addr + (i * es) as usize, &et, depth)).collect())
            }
            Ty::Class(ci, _, _) => {
                if name.starts_with("std::basic_string<char,") {
                    return self.string(addr).map(Value::String).unwrap_or(Value::Null);
                }
                if name.starts_with("std::vector<") {
                    let first = self.vector(addr, t);
                    let n = match (&first, read_u64(addr + 8)) {
                        (Some((f, e)), Some(l)) => (l as usize).saturating_sub(*f) as u64 / self.size(e).max(1),
                        _ => 0,
                    };
                    if depth == 0 {
                        return json!({"type": name, "size": n});
                    }
                    let Some((f, e)) = first else { return json!([]) };
                    let es = self.size(&e).max(1);
                    return Value::Array((0..n.min(256)).map(|i| self.decode(f + (i * es) as usize, &e, depth - 1)).collect());
                }
                // (FixedPointNumberTemplate<int,8,...>: an integer of 1/256ths)
                if let Some(rest) = name.strip_prefix("FixedPointNumberTemplate<") {
                    let bits: u32 = rest.split(',').nth(1).and_then(|b| b.trim().parse().ok()).unwrap_or(0);
                    let inner = self.members(*ci).into_iter().find(|(n, _, _)| !n.starts_with('^'));
                    if let Some((_, off, ft)) = inner {
                        if let Some(v) = self.decode(addr + off as usize, &self.ty(ft), 0).as_f64() {
                            return json!(v / (1u64 << bits) as f64);
                        }
                    }
                }
                // (a class around one number, an ID or a tick: that number)
                let members = self.members(*ci);
                if let [(n, off, ft)] = members.as_slice() {
                    let inner = self.ty(*ft);
                    if !n.starts_with('^') && matches!(inner, Ty::Prim(_) | Ty::Enum(..) | Ty::Class(..)) {
                        let v = self.decode(addr + *off as usize, &inner, depth);
                        if v.is_number() || v.is_boolean() || (v.is_string() && matches!(inner, Ty::Enum(..))) {
                            return v;
                        }
                    }
                }
                if depth == 0 {
                    return json!({"type": name, "addr": addr});
                }
                let mut m = Map::new();
                m.insert("type".into(), json!(name));
                self.fields_into(&mut m, addr, *ci, depth, 0);
                Value::Object(m)
            }
            Ty::Unknown(n) => json!({"type": n, "addr": addr}),
        }
    }

    /// every field of a class and its bases, flattened (a base's fields first)
    fn fields_into(&self, m: &mut Map<String, Value>, addr: usize, class: TypeIndex, depth: u32, level: u32) {
        for (n, off, t) in self.members(class) {
            let ft = self.ty(t);
            if n.starts_with('^') {
                if let (Ty::Class(b, _, _), true) = (&ft, level < 12) {
                    self.fields_into(m, addr + off as usize, *b, depth, level + 1);
                }
            } else if !n.starts_with("__vfptr") {
                m.insert(n, self.decode(addr + off as usize, &ft, depth - 1));
            }
        }
    }

    /// writes a number or boolean into a primitive (or enum, or one-number wrapper) field along `path`: for this
    /// peer's own settings (camera, UI). Writing simulation state desyncs multiplayer and can break the game.
    pub fn write(&self, addr: usize, class: &str, path: &str, value: &Value) -> Result<(), String> {
        let start = *self.by_name.get(class).ok_or(format!("no class {class} in this build"))?;
        let (mut addr, mut t) = self.walk(addr, self.ty(start), path)?;
        // (a class around one number: that number; an enum: its underlying integer)
        for _ in 0..4 {
            match &t {
                Ty::Class(ci, _, _) => {
                    let m = self.members(*ci);
                    let [(n, off, ft)] = m.as_slice() else { return Err(format!("{} is not a number", self.type_name(&t))) };
                    if n.starts_with('^') {
                        return Err(format!("{} is not a number", self.type_name(&t)));
                    }
                    addr += *off as usize;
                    t = self.ty(*ft);
                }
                Ty::Enum(_, u, _) => t = self.ty(*u),
                _ => break,
            }
        }
        let Ty::Prim(k) = t else { return Err(format!("{} is not a number", self.type_name(&t))) };
        use PrimitiveKind::*;
        let n = value.as_f64().or_else(|| value.as_bool().map(|b| b as u8 as f64)).ok_or("give a number or boolean")?;
        let bytes: Vec<u8> = match k {
            Bool8 | Bool32 | Bool64 => { let mut v = vec![0u8; prim_size(k) as usize]; v[0] = (n != 0.0) as u8; v }
            Char | RChar | I8 | UChar | U8 => vec![n as i64 as u8],
            I16 | Short | WChar | RChar16 | U16 | UShort => (n as i64 as u16).to_le_bytes().to_vec(),
            I32 | Long | HRESULT | U32 | ULong | RChar32 => (n as i64 as u32).to_le_bytes().to_vec(),
            I64 | Quad | U64 | UQuad => (n as i64).to_le_bytes().to_vec(),
            F32 => (n as f32).to_le_bytes().to_vec(),
            F64 => n.to_le_bytes().to_vec(),
            _ => return Err(format!("can't write a {k:?}")),
        };
        write_mem(addr, &bytes).then_some(()).ok_or(format!("can't write at {addr:#x}"))
    }

    fn prim(&self, addr: usize, k: PrimitiveKind) -> Option<Value> {
        use PrimitiveKind::*;
        let b = read_mem(addr, prim_size(k) as usize)?;
        Some(match k {
            Bool8 | Bool32 | Bool64 => json!(b.iter().any(|&x| x != 0)),
            Char | RChar | I8 => json!(b[0] as i8),
            UChar | U8 => json!(b[0]),
            I16 | Short => json!(i16::from_le_bytes([b[0], b[1]])),
            WChar | RChar16 | U16 | UShort => json!(u16::from_le_bytes([b[0], b[1]])),
            I32 | Long | HRESULT => json!(i32::from_le_bytes(b[..4].try_into().ok()?)),
            U32 | ULong | RChar32 => json!(u32::from_le_bytes(b[..4].try_into().ok()?)),
            I64 | Quad => json!(i64::from_le_bytes(b[..8].try_into().ok()?)),
            U64 | UQuad => json!(u64::from_le_bytes(b[..8].try_into().ok()?)),
            F32 => json!(f32::from_le_bytes(b[..4].try_into().ok()?)),
            F64 => json!(f64::from_le_bytes(b[..8].try_into().ok()?)),
            _ => return None,
        })
    }

    /// an MSVC std::string: 16-byte inline buffer or heap pointer, then size, then capacity
    fn string(&self, addr: usize) -> Option<String> {
        let size = read_u64(addr + 16)? as usize;
        let cap = read_u64(addr + 24)? as usize;
        if cap < size || size > 1 << 20 {
            return None;
        }
        let data = if cap >= 16 { read_u64(addr)? as usize } else { addr };
        read_mem(data, size).map(|b| String::from_utf8_lossy(&b).into_owned())
    }

    /// an enum value by its name
    pub fn enum_value(&self, en: &str, name: &str) -> Option<i64> {
        let v = self.layout(en).ok()?;
        v["values"].as_array()?.iter().find(|x| x["name"] == name).and_then(|x| x["value"].as_i64())
    }

    /// a field's offset in a class (through its bases)
    pub fn field_offset(&self, class: &str, field: &str) -> Option<u64> {
        let &ci = self.by_name.get(class)?;
        self.find_field(ci, field, 0).map(|(o, _)| o)
    }

    /// a class's size in bytes
    pub fn size_of(&self, class: &str) -> Option<u64> {
        let &ci = self.by_name.get(class)?;
        Some(self.size(&self.ty(ci)))
    }

    /// a class's fields (with bases) as {name, offset, type}, or an enum's values as {name, value}: for exploring
    pub fn layout(&self, class: &str) -> Result<Value, String> {
        if let Some(&e) = self.enums.get(class) {
            let mut values = Vec::new();
            if let Some(TypeData::Enumeration(en)) = self.data(e) {
                let mut next = Some(en.fields);
                while let Some(idx) = next.take() {
                    if let Some(TypeData::FieldList(list)) = self.data(idx) {
                        for f in list.fields {
                            if let TypeData::Enumerate(x) = f {
                                let v = format!("{:?}", x.value);
                                let n: i64 = v.split(['(', ')']).nth(1).and_then(|t| t.parse().ok()).unwrap_or(0);
                                values.push(json!({"name": x.name.to_string(), "value": n}));
                            }
                        }
                        next = list.continuation;
                    }
                }
            }
            return Ok(json!({"name": class, "values": values}));
        }
        let &ci = self.by_name.get(class).ok_or(format!("no class {class} in this build"))?;
        let Ty::Class(_, _, size) = self.ty(ci) else { return Err(format!("{class} is not a class")) };
        let members: Vec<Value> = self.members(ci).into_iter()
            .map(|(n, o, t)| json!({"name": n, "offset": o, "type": self.type_name(&self.ty(t))})).collect();
        Ok(json!({"name": class, "size": size, "fields": members}))
    }
}
