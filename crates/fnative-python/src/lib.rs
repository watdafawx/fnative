//! fnative plugin "py": any Python function `f(input: str) -> str` is callable from Lua as
//!     native.call("py", "package.module:function", input)      (or native.start, it is threadsafe)
//! The interpreter starts on the first call (not at game start) and stays: modules keep their state between calls,
//! so a module can load big data once. `sys.path` gets the entries of FNATIVE_PYPATH (';'-separated).
//! "py._reload" with input "package.module" reloads a module (development).
//! Python errors come back to Lua as `nil, traceback`. print() and stderr go to fnative.log (also `fnative.log(text)`).

use std::cell::RefCell;
use std::ffi::{c_char, c_void, CStr};
use std::sync::Once;

use pyo3::prelude::*;
use pyo3::types::PyModule;

type PluginFn = unsafe extern "C" fn(*mut c_void, *const c_char, *const c_char, usize, *mut *const c_char, *mut usize) -> i32;

#[repr(C)]
pub struct Host {
    abi: u32,
    version: *const c_char,
    log: unsafe extern "C" fn(*const c_char, *const c_char),
    engine_symbol: unsafe extern "C" fn(*const c_char) -> *mut c_void,
    register_fn: unsafe extern "C" fn(*const c_char, *const c_char, PluginFn, *mut c_void, u32) -> i32,
}

static mut HOST: *const Host = std::ptr::null();
static START: Once = Once::new();

thread_local! {
    static OUT: RefCell<Vec<u8>> = const { RefCell::new(Vec::new()) };
}

fn log(msg: &str) {
    unsafe {
        if !HOST.is_null() {
            let m = std::ffi::CString::new(msg.replace('\0', " ")).unwrap_or_default();
            ((*HOST).log)(c"py".as_ptr(), m.as_ptr());
        }
    }
}

/// fnative.log(text) from Python: a line in fnative.log
#[pyfunction]
fn py_log(text: &str) {
    for line in text.lines().filter(|l| !l.trim().is_empty()) {
        log(line);
    }
}

// sys.stdout / sys.stderr into fnative.log a line at a time (the game's own console is nowhere to be seen)
const STREAMS: &str = r#"
import sys, fnative
class _ToLog:
    def __init__(self, tag): self.tag, self.buf = tag, ""
    def write(self, s):
        self.buf += s
        while "\n" in self.buf:
            line, self.buf = self.buf.split("\n", 1)
            fnative.log(self.tag + line)
        return len(s)
    def flush(self):
        if self.buf: fnative.log(self.tag + self.buf); self.buf = ""
    def isatty(self): return False
sys.stdout, sys.stderr = _ToLog(""), _ToLog("stderr: ")
"#;

fn start_python() {
    START.call_once(|| {
        let t = std::time::Instant::now();
        pyo3::prepare_freethreaded_python();
        Python::with_gil(|py| {
            if let Ok(paths) = std::env::var("FNATIVE_PYPATH") {
                if let Ok(sys_path) = py.import_bound("sys").and_then(|s| s.getattr("path")) {
                    for p in paths.split(';').filter(|p| !p.is_empty()).rev() {
                        let _ = sys_path.call_method1("insert", (0, p));
                    }
                }
            }
            let streams = PyModule::new_bound(py, "fnative").and_then(|m| {
                m.add_function(wrap_pyfunction!(py_log, &m)?)?;
                m.setattr("log", m.getattr("py_log")?)?;
                py.import_bound("sys")?.getattr("modules")?.set_item("fnative", m)?;
                py.run_bound(STREAMS, None, None)
            });
            if let Err(e) = streams {
                log(&format!("stdout to the log: {}", error_text(py, e)));
            }
            let v: String = py.version().to_string();
            log(&format!("Python {} started in {:.2?}", v.split_whitespace().next().unwrap_or("?"), t.elapsed()));
        });
    });
}

fn error_text(py: Python<'_>, e: PyErr) -> String {
    let tb = e.traceback_bound(py).and_then(|t| t.format().ok()).unwrap_or_default();
    format!("{tb}{e}")
}

fn run(name: &str, input: &str) -> Result<String, String> {
    start_python();
    Python::with_gil(|py| {
        if name == "_reload" {
            let m = PyModule::import_bound(py, input).map_err(|e| error_text(py, e))?;
            let importlib = py.import_bound("importlib").map_err(|e| error_text(py, e))?;
            importlib.call_method1("reload", (m,)).map_err(|e| error_text(py, e))?;
            return Ok(format!("reloaded {input}"));
        }
        let (module, func) = name.split_once(':').ok_or_else(|| format!("py: \"{name}\" is not \"module:function\""))?;
        let m = PyModule::import_bound(py, module).map_err(|e| error_text(py, e))?;
        let f = m.getattr(func).map_err(|e| error_text(py, e))?;
        let r = f.call1((input,)).map_err(|e| error_text(py, e))?;
        r.extract::<String>().map_err(|_| format!("py: {name} must return a str"))
    })
}

unsafe extern "C" fn call(_ud: *mut c_void, name: *const c_char, input: *const c_char, len: usize,
                          out: *mut *const c_char, out_len: *mut usize) -> i32 {
    let name = CStr::from_ptr(name).to_string_lossy().into_owned();
    let input = String::from_utf8_lossy(std::slice::from_raw_parts(input as *const u8, len)).into_owned();
    // (a panic must not cross into the game: caught and reported as an error)
    let r = std::panic::catch_unwind(|| run(&name, &input)).unwrap_or_else(|_| Err("py: panic in the plugin".into()));
    let (rc, bytes) = match r {
        Ok(s) => (0, s.into_bytes()),
        Err(e) => (1, e.into_bytes()),
    };
    OUT.with(|o| {
        let mut o = o.borrow_mut();
        *o = bytes;
        *out = o.as_ptr() as *const c_char;
        *out_len = o.len();
    });
    rc
}

#[no_mangle]
pub unsafe extern "C" fn fnative_plugin_init(host: *const Host) -> i32 {
    if host.is_null() || (*host).abi != 1 {
        return 1;
    }
    HOST = host;
    ((*host).register_fn)(c"py".as_ptr(), c"*".as_ptr(), call, std::ptr::null_mut(), 1);
    log("ready (Python starts on the first call)");
    0
}
