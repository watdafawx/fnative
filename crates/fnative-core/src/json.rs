//! native.to_json: a Lua value to JSON text, walked with the Lua C API (lua.rs).

use std::collections::HashSet;
use std::ffi::c_int;
use std::fmt::Write;

use crate::api;
use crate::lua::{lua_string, LuaState, LUA_TBOOLEAN, LUA_TNUMBER, LUA_TSTRING, LUA_TTABLE};

const MAX_DEPTH: usize = 100;

fn escape(s: &str, out: &mut String) {
    out.push('"');
    for c in s.chars() {
        match c {
            '"' => out.push_str("\\\""),
            '\\' => out.push_str("\\\\"),
            '\n' => out.push_str("\\n"),
            '\r' => out.push_str("\\r"),
            '\t' => out.push_str("\\t"),
            c if (c as u32) < 0x20 => {
                let _ = write!(out, "\\u{:04x}", c as u32);
            }
            c => out.push(c),
        }
    }
    out.push('"');
}

fn number(v: f64, out: &mut String) {
    if !v.is_finite() {
        out.push_str("null");
    } else if v.fract() == 0.0 && v.abs() < 1e15 {
        let _ = write!(out, "{}", v as i64);
    } else {
        let _ = write!(out, "{v}");
    }
}

enum Key {
    Int(i64),
    Str(String),
}

/// writes the value at stack index `idx` to `out`
pub unsafe fn write(l: *mut LuaState, idx: c_int, out: &mut String, skip: &HashSet<String>, depth: usize)
                    -> Result<(), String> {
    let a = api();
    let idx = (a.absindex)(l, idx);
    match (a.type_of)(l, idx) {
        LUA_TBOOLEAN => out.push_str(if (a.toboolean)(l, idx) != 0 { "true" } else { "false" }),
        LUA_TNUMBER => number((a.tonumberx)(l, idx, std::ptr::null_mut()), out),
        LUA_TSTRING => escape(&lua_string(l, idx).unwrap_or_default(), out),
        LUA_TTABLE => {
            if depth >= MAX_DEPTH {
                return Err("nested too deep (a cycle?)".into());
            }
            if (a.checkstack)(l, 4) == 0 {
                return Err("Lua stack full".into());
            }
            let mut entries: Vec<(Key, String)> = Vec::new();
            (a.pushnil)(l);
            while (a.next)(l, idx) != 0 {
                let key = match (a.type_of)(l, -2) {
                    LUA_TNUMBER => {
                        let v = (a.tonumberx)(l, -2, std::ptr::null_mut());
                        if v.fract() == 0.0 { Key::Int(v as i64) } else { Key::Str(v.to_string()) }
                    }
                    LUA_TSTRING => Key::Str(lua_string(l, -2).unwrap_or_default()),
                    _ => Key::Str(String::new()),
                };
                let skipped = matches!(&key, Key::Str(k) if skip.contains(k));
                if !skipped {
                    let mut v = String::new();
                    let r = write(l, -1, &mut v, skip, depth + 1);
                    if let Err(e) = r {
                        (a.settop)(l, -3);
                        return Err(e);
                    }
                    entries.push((key, v));
                }
                (a.settop)(l, -2); // (the value off; the key stays for lua_next)
            }
            let n = entries.len() as i64;
            let array = n > 0 && {
                let mut seen = vec![false; n as usize];
                entries.iter().all(|(k, _)| match k {
                    Key::Int(i) if *i >= 1 && *i <= n && !seen[(*i - 1) as usize] => {
                        seen[(*i - 1) as usize] = true;
                        true
                    }
                    _ => false,
                })
            };
            if array {
                entries.sort_by_key(|(k, _)| if let Key::Int(i) = k { *i } else { 0 });
                out.push('[');
                for (i, (_, v)) in entries.iter().enumerate() {
                    if i > 0 {
                        out.push(',');
                    }
                    out.push_str(v);
                }
                out.push(']');
            } else {
                // (an empty table is {}: Lua can't tell an empty array from an empty object)
                out.push('{');
                for (i, (k, v)) in entries.iter().enumerate() {
                    if i > 0 {
                        out.push(',');
                    }
                    match k {
                        Key::Int(n) => escape(&n.to_string(), out),
                        Key::Str(s) => escape(s, out),
                    }
                    out.push(':');
                    out.push_str(v);
                }
                out.push('}');
            }
        }
        _ => out.push_str("null"),
    }
    Ok(())
}
