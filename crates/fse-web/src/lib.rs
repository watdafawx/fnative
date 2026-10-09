//! fse plugin "web": a local HTTP API for reporting and control while the game runs.
//!
//! Listens on 127.0.0.1:FSE_WEB_PORT (8790) only. Every /api request needs the token, as
//! `Authorization: Bearer <token>` or `?token=<token>`; it is FSE_WEB_TOKEN, else made at start and written to
//! web-token.txt beside the launcher (a local agent reads it from there).
//!
//! web.open <page>: the page in a panel over the game (panel.rs), else the browser.
//!
//!   GET  /                              the dashboard (files from <home>/web/, also mods.html)
//!   GET  /api/status                    core build, uptime, whether the game thread is answering
//!   GET  /api/profile?consumer=web      the engine profiler's numbers since this consumer's last call
//!   POST /api/profiler/start | stop
//!   GET  /api/overlay | POST {"hub": bool, "startup": bool}   which corner buttons show over the main menu
//!   POST /api/native/<plugin>/<fn>      body -> a native function (threadsafe ones only), its output back
//!   POST /api/game                      {"interface": "...", "function": "...", "args": [...]}: run on the game
//!                                       thread by the fse-bridge mod (remote.call), its answer back (10 s max)
//!
//! Game-thread side (called by the fse-bridge mod every tick): web.take -> pending commands as a JSON array,
//! web.reply {"id": n, "ok": bool, "result": ..., "error": "..."}.

use std::collections::{HashMap, VecDeque};
use std::hash::{BuildHasher, Hasher};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::mpsc::{channel, Sender};
use std::sync::Mutex;
use std::time::{Duration, Instant};

mod overlay;
mod panel;

use fse_plugin as fp;
use serde_json::{json, Value};
use tiny_http::{Header, Method, Request, Response, Server};

static QUEUE: Mutex<VecDeque<Value>> = Mutex::new(VecDeque::new());
static WAITING: Mutex<Option<HashMap<u64, Sender<Value>>>> = Mutex::new(None);
static NEXT: AtomicU64 = AtomicU64::new(1);
static LAST_TAKE: Mutex<Option<Instant>> = Mutex::new(None);
static STARTED: Mutex<Option<Instant>> = Mutex::new(None);

fn token() -> String {
    if let Ok(t) = std::env::var("FSE_WEB_TOKEN") {
        return t;
    }
    // (RandomState is seeded randomly per process: two of its hashes make a 128-bit token)
    let mut s = String::new();
    for i in 0..2u64 {
        let mut h = std::collections::hash_map::RandomState::new().build_hasher();
        h.write_u64(i ^ std::process::id() as u64);
        s.push_str(&format!("{:016x}", h.finish()));
    }
    s
}

pub(crate) fn home() -> std::path::PathBuf {
    std::env::var("FSE_HOME").map(std::path::PathBuf::from).unwrap_or_default()
}

fn header(name: &str, value: &str) -> Header {
    Header::from_bytes(name.as_bytes(), value.as_bytes()).unwrap()
}

fn send_json(req: Request, code: u16, body: &Value) {
    let r = Response::from_string(body.to_string()).with_status_code(code)
        .with_header(header("Content-Type", "application/json"));
    let _ = req.respond(r);
}

fn query(url: &str, key: &str) -> Option<String> {
    let q = url.split_once('?')?.1;
    q.split('&').find_map(|kv| kv.split_once('=').filter(|(k, _)| *k == key).map(|(_, v)| v.to_string()))
}

/// a command for the game thread; waits for the bridge's answer
fn game(cmd: Value) -> Value {
    let id = NEXT.fetch_add(1, Ordering::Relaxed);
    let (tx, rx) = channel();
    WAITING.lock().unwrap().get_or_insert_with(HashMap::new).insert(id, tx);
    let mut cmd = cmd;
    cmd["id"] = json!(id);
    QUEUE.lock().unwrap().push_back(cmd);
    match rx.recv_timeout(Duration::from_secs(10)) {
        Ok(v) => v,
        Err(_) => {
            WAITING.lock().unwrap().get_or_insert_with(HashMap::new).remove(&id);
            json!({"ok": false, "error": "no answer from the game in 10 s (is the fse-bridge mod enabled? the game paused?)"})
        }
    }
}

fn serve_file(req: Request, path: &str) {
    let rel = if path == "/" { "index.html" } else { path.trim_start_matches('/') };
    if rel.contains("..") {
        return send_json(req, 400, &json!({"error": "bad path"}));
    }
    let file = home().join("web").join(rel);
    match std::fs::read(&file) {
        Ok(bytes) => {
            let ctype = match file.extension().and_then(|e| e.to_str()) {
                Some("html") => "text/html; charset=utf-8",
                Some("js") => "text/javascript",
                Some("css") => "text/css",
                Some("json") => "application/json",
                Some("png") => "image/png",
                _ => "application/octet-stream",
            };
            let _ = req.respond(Response::from_data(bytes).with_header(header("Content-Type", ctype)));
        }
        Err(_) => send_json(req, 404, &json!({"error": format!("no {rel}")})),
    }
}

fn handle(mut req: Request, token: &str) {
    let url = req.url().to_string();
    let path = url.split('?').next().unwrap_or("/").to_string();
    if !path.starts_with("/api/") {
        return serve_file(req, &path);
    }
    let auth = req.headers().iter().find(|h| h.field.equiv("Authorization")).map(|h| h.value.as_str().to_string());
    let ok = auth.as_deref() == Some(&format!("Bearer {token}")) || query(&url, "token").as_deref() == Some(token);
    if !ok {
        return send_json(req, 401, &json!({"error": "token needed (web-token.txt beside fse.exe)"}));
    }
    let mut body = String::new();
    let _ = req.as_reader().read_to_string(&mut body);
    let method = req.method().clone();
    match (method, path.as_str()) {
        (Method::Get, "/api/status") => {
            let answering = LAST_TAKE.lock().unwrap().map(|t| t.elapsed() < Duration::from_secs(2)).unwrap_or(false);
            let up = STARTED.lock().unwrap().map(|t| t.elapsed().as_secs()).unwrap_or(0);
            send_json(req, 200, &json!({"core": fp::core_version(), "build": fp::build(), "uptime_s": up,
                                         "game_thread_answering": answering}))
        }
        (Method::Get, "/api/profile") => {
            let who = query(&url, "consumer").unwrap_or_else(|| "web".into());
            match fp::call("profiler", "profile", who.as_bytes()) {
                Ok(b) => {
                    let v: Value = serde_json::from_slice(&b).unwrap_or(Value::Null);
                    send_json(req, 200, &v)
                }
                Err(e) => send_json(req, 502, &json!({"error": e})),
            }
        }
        (Method::Post, p) if p == "/api/profiler/start" || p == "/api/profiler/stop" => {
            let f = p.rsplit('/').next().unwrap_or("stop");
            match fp::call("profiler", f, b"") {
                Ok(b) => send_json(req, 200, &json!({"ok": true, "result": String::from_utf8_lossy(&b)})),
                Err(e) => send_json(req, 502, &json!({"error": e})),
            }
        }
        (Method::Get, "/api/overlay") => send_json(req, 200, &overlay::settings()),
        (Method::Post, "/api/overlay") => match serde_json::from_str::<Value>(&body) {
            Ok(v) => match overlay::set(&v) {
                Ok(()) => send_json(req, 200, &overlay::settings()),
                Err(e) => send_json(req, 500, &json!({"error": e.to_string()})),
            },
            Err(e) => send_json(req, 400, &json!({"error": e.to_string()})),
        },
        (Method::Post, p) if p.starts_with("/api/native/") => {
            let rest = &p["/api/native/".len()..];
            let Some((plugin, f)) = rest.split_once('/') else {
                return send_json(req, 400, &json!({"error": "/api/native/<plugin>/<function>"}));
            };
            match fp::call(plugin, f, body.as_bytes()) {
                Ok(b) => {
                    let text = String::from_utf8_lossy(&b).into_owned();
                    let v = serde_json::from_str::<Value>(&text).unwrap_or(Value::String(text));
                    send_json(req, 200, &json!({"ok": true, "result": v}))
                }
                Err(e) => send_json(req, 502, &json!({"ok": false, "error": e})),
            }
        }
        (Method::Post, "/api/game") => match serde_json::from_str::<Value>(&body) {
            Ok(cmd) if cmd.get("interface").is_some() && cmd.get("function").is_some() => {
                let answer = game(cmd);
                let code = if answer.get("ok").and_then(Value::as_bool).unwrap_or(false) { 200 } else { 502 };
                send_json(req, code, &answer)
            }
            _ => send_json(req, 400, &json!({"error": "body: {\"interface\": ..., \"function\": ..., \"args\": [...]}"})),
        },
        _ => send_json(req, 404, &json!({"error": format!("no {path}")})),
    }
}

// ---- game-thread side ------------------------------------------------------------------------------------------

fp::export!(f_take, |_, _| {
    *LAST_TAKE.lock().unwrap() = Some(Instant::now());
    let mut q = QUEUE.lock().unwrap();
    if q.is_empty() {
        return Ok("[]".into());
    }
    let all: Vec<Value> = q.drain(..).collect();
    Ok(Value::Array(all).to_string())
});

/// web.url -> "http://127.0.0.1:8790/?token=..." (for in-game links: the hub mod's buttons)
static URL: Mutex<String> = Mutex::new(String::new());
fp::export!(f_url, |_, _| Ok(URL.lock().unwrap().clone()));

// web.open "mods.html" -> the page in the panel over the game ("" the dashboard), or the browser without WebView2
fp::export!(f_open, |_, input| {
    if input.contains("..") || input.contains(':') {
        return Err("a page of the dashboard, like mods.html".into());
    }
    panel::open(input);
    Ok("ok".into())
});

/// a game is ticking: the bridge mod asks for commands every tick
fn in_game() -> bool {
    LAST_TAKE.lock().unwrap().map(|t| t.elapsed() < Duration::from_millis(1500)).unwrap_or(false)
}

fp::export!(f_reply, |_, input| {
    let v: Value = serde_json::from_str(input).map_err(|e| format!("reply: {e}"))?;
    let id = v.get("id").and_then(Value::as_u64).ok_or("reply: no id")?;
    if let Some(tx) = WAITING.lock().unwrap().get_or_insert_with(HashMap::new).remove(&id) {
        let _ = tx.send(v);
    }
    Ok("ok".into())
});

#[no_mangle]
pub unsafe extern "C" fn fse_plugin_init(host: *const fp::Host) -> i32 {
    if !fp::init(host, "web") {
        return 1;
    }
    let first: u16 = std::env::var("FSE_WEB_PORT").ok().and_then(|p| p.parse().ok()).unwrap_or(8790);
    // (a port just let go by a game that closed can stay taken for a moment: the next free one of ten)
    let Some((port, server)) = (first..first.saturating_add(10)).find_map(|p| Server::http(("127.0.0.1", p)).ok().map(|s| (p, s)))
    else {
        fp::log(&format!("can't listen on 127.0.0.1:{first}-{}", first.saturating_add(9)));
        return 0;
    };
    let tok = token();
    let _ = std::fs::write(home().join("web-token.txt"), &tok);
    *STARTED.lock().unwrap() = Some(Instant::now());
    // (the game-thread functions: take is cheap when nothing waits, so the bridge can call it every tick)
    let url = format!("http://127.0.0.1:{port}/?token={tok}");
    *URL.lock().unwrap() = url.clone();
    fp::register("web", "url", f_url, fp::THREADSAFE);
    panel::set_base(&url);
    fp::register("web", "open", f_open, fp::THREADSAFE);
    let buttons = std::env::var("FSE_OVERLAY").map(|v| v != "0").unwrap_or(true);
    if buttons || panel::enabled() {
        overlay::start(buttons, in_game);
    }
    fp::register("web", "take", f_take, 0);
    fp::register("web", "reply", f_reply, 0);
    std::thread::Builder::new().name("fse-web".into()).spawn(move || {
        for req in server.incoming_requests() {
            let t = tok.clone();
            // (a request waiting on the game thread must not hold up the others)
            std::thread::spawn(move || handle(req, &t));
        }
    }).ok();
    fp::log(&format!("listening on http://127.0.0.1:{port}/ (token in web-token.txt)"));
    0
}
