//! Events from native code to Lua. Plugins (hooks in the engine, worker threads) emit them from any thread; they
//! wait in a ring of the last CAP events. Lua takes them on its own thread, at its own pace: native.events(since)
//! hands out every event newer than `since` and the newest number, so each mod keeps its own place and none takes
//! events away from another. Lua never runs inside engine code this way.

use std::collections::VecDeque;
use std::sync::Mutex;

const CAP: usize = 8192;

pub struct Event {
    pub seq: u64,
    pub plugin: String,
    pub name: String,
    pub data: String,
}

static RING: Mutex<(u64, VecDeque<Event>)> = Mutex::new((0, VecDeque::new()));

/// an event; emitted while the simulation updates, it is a simulation event too (see mp.rs) unless `local`
pub fn emit(plugin: &str, name: &str, data: &str, local: bool) {
    if !local && crate::mp::updating() {
        crate::mp::sim_event(plugin, name, data);
    }
    let mut r = RING.lock().unwrap();
    r.0 += 1;
    let seq = r.0;
    if r.1.len() == CAP {
        r.1.pop_front();
    }
    r.1.push_back(Event { seq, plugin: plugin.into(), name: name.into(), data: data.into() });
}

/// the events after `since` (all that are still there) and the newest number; f sees each one
pub fn since(since: u64, mut f: impl FnMut(&Event)) -> u64 {
    let r = RING.lock().unwrap();
    for e in r.1.iter().filter(|e| e.seq > since) {
        f(e);
    }
    r.0
}
