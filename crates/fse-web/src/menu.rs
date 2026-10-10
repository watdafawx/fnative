//! An "FSE hub" button in the game's own menus, under Settings: the main menu and the pause menu (Esc in a game).
//! The engine's menu buttons come from MenuGui<Result>::addResultButton(label, result), which maps the button to a
//! result number that a click writes into the menu's `result`; the menu state's process (InMainMenu, InGameMenu)
//! acts on the numbers it knows and ignores any other. So: while the menu is being built, after the button labelled
//! gui-menu.settings one more is added with a result of our own, and before each process that result is taken back
//! (0, "none") and the hub opens in the panel.
//! Everything comes from factorio.pdb (plugins/web.needs.json); a build without it keeps the overlay buttons.

use std::cell::Cell;
use std::ffi::c_void;
use std::sync::OnceLock;

use fse_plugin as fp;
use retour::GenericDetour;

type Ctor = unsafe extern "C" fn(usize, usize) -> usize;
type AddResultButton = unsafe extern "C" fn(usize, *const c_void, i32, usize) -> usize;
type Process = unsafe extern "C" fn(usize, usize);
type Literal = unsafe extern "C" fn(*mut c_void, *const MsvcString) -> *mut c_void;
type Dtor = unsafe extern "C" fn(*mut c_void);

struct Menu {
    name: &'static str,
    ctor: &'static str,
    add: &'static str,
    process: &'static str,
    state: &'static str, // the menu state class: its `gui`
    gui: &'static str,   // MenuGui<...>: its `result`
}

const MENUS: [Menu; 2] = [
    Menu {
        name: "main menu",
        ctor: "??0MainMenuGui@@QEAA@AEBVPackageFilesystemInfo@@@Z",
        add: "?addResultButton@?$MenuGui@W4MainMenuResult@@@@QEAAPEAVTextButton@agui@@AEBVLocalisedString@@W4MainMenuResult@@PEBVButtonStyle@3@@Z",
        process: "?process@InMainMenu@@UEAAXPEAVAppManager@@@Z",
        state: "AppManagerStateWithGuiManualConstruction<MainMenuGui>",
        gui: "MenuGui<enum MainMenuResult>",
    },
    Menu {
        name: "pause menu",
        ctor: "??0GameMenuGui@@QEAA@PEBVPlayer@@@Z",
        add: "?addResultButton@?$MenuGui@W4GameMenuResult@@@@QEAAPEAVTextButton@agui@@AEBVLocalisedString@@W4GameMenuResult@@PEBVButtonStyle@3@@Z",
        process: "?process@InGameMenu@@UEAAXPEAVAppManager@@@Z",
        state: "AppManagerStateWithGuiManualConstruction<GameMenuGui>",
        gui: "MenuGui<enum GameMenuResult>",
    },
];
const LITERAL: &str = "?literal@LocalisedString@@SA?AV1@AEBV?$basic_string@DU?$char_traits@D@std@@V?$allocator@D@2@@std@@@Z";
const DTOR: &str = "??1LocalisedString@@QEAA@XZ";

const AFTER: &[u8] = b"gui-menu.settings";
const OURS: i32 = 70; // (no menu knows it: process ignores it)
const LABEL: &str = "FSE hub";

static CTOR_HOOK: [OnceLock<GenericDetour<Ctor>>; 2] = [OnceLock::new(), OnceLock::new()];
static ADD_HOOK: [OnceLock<GenericDetour<AddResultButton>>; 2] = [OnceLock::new(), OnceLock::new()];
static PROCESS_HOOK: [OnceLock<GenericDetour<Process>>; 2] = [OnceLock::new(), OnceLock::new()];
static OFFSETS: [OnceLock<(usize, usize)>; 2] = [OnceLock::new(), OnceLock::new()]; // (state.gui, gui.result)
static ENGINE: OnceLock<Engine> = OnceLock::new();

struct Engine {
    literal: Literal,
    dtor: Dtor,
    key: usize, // LocalisedString.key
}

thread_local! {
    static BUILDING: Cell<bool> = const { Cell::new(false) };
}

/// MSVC's std::string: a 16-byte buffer (or a pointer to the text), its size, its capacity
#[repr(C)]
struct MsvcString {
    buf: [u8; 16],
    size: usize,
    cap: usize,
}

impl MsvcString {
    unsafe fn bytes(&self) -> &[u8] {
        let p = if self.cap < 16 { self.buf.as_ptr() } else { *(self.buf.as_ptr() as *const *const u8) };
        std::slice::from_raw_parts(p, self.size)
    }
}

unsafe extern "C" fn ctor<const N: usize>(this: usize, arg: usize) -> usize {
    BUILDING.with(|b| b.set(true));
    let r = CTOR_HOOK[N].get().unwrap().call(this, arg);
    BUILDING.with(|b| b.set(false));
    r
}

unsafe extern "C" fn add<const N: usize>(this: usize, label: *const c_void, result: i32, style: usize) -> usize {
    let hook = ADD_HOOK[N].get().unwrap();
    let r = hook.call(this, label, result, style);
    let e = ENGINE.get().unwrap();
    if BUILDING.with(|b| b.get()) && (*((label as usize + e.key) as *const MsvcString)).bytes() == AFTER {
        let mut text = MsvcString { buf: [0; 16], size: LABEL.len(), cap: 15 };
        text.buf[..LABEL.len()].copy_from_slice(LABEL.as_bytes());
        let mut ls = [0u64; 64]; // (a LocalisedString: 104 bytes in 2.0)
        let ls = ls.as_mut_ptr() as *mut c_void;
        (e.literal)(ls, &text);
        hook.call(this, ls, OURS, style);
        (e.dtor)(ls);
    }
    r
}

unsafe extern "C" fn process<const N: usize>(this: usize, app: usize) {
    let (gui, result) = *OFFSETS[N].get().unwrap();
    let gui = *((this + gui) as *const usize);
    if gui != 0 {
        let result = (gui + result) as *mut i32;
        if *result == OURS {
            *result = 0;
            crate::panel::open("");
        }
    }
    PROCESS_HOOK[N].get().unwrap().call(this, app)
}

fn hook<T>(r: retour::Result<T>) -> Result<T, String> {
    r.map_err(|e| format!("hook: {e}"))
}

fn sym(n: &str) -> Result<usize, String> {
    fp::engine_symbol(n).ok_or(format!("{n} not in this build"))
}

fn off(c: &str, f: &str) -> Result<usize, String> {
    fp::field_offset(c, f).map(|v| v as usize).ok_or(format!("{c}.{f} not in this build"))
}

unsafe fn install_menu<const N: usize>() -> Result<(), String> {
    let m = &MENUS[N];
    let _ = OFFSETS[N].set((off(m.state, "gui")?, off(m.gui, "result")?));
    let (c, a, p) = (sym(m.ctor)?, sym(m.add)?, sym(m.process)?);
    let ch = hook(GenericDetour::<Ctor>::new(std::mem::transmute::<usize, Ctor>(c), ctor::<N>))?;
    let ah = hook(GenericDetour::<AddResultButton>::new(std::mem::transmute::<usize, AddResultButton>(a), add::<N>))?;
    let ph = hook(GenericDetour::<Process>::new(std::mem::transmute::<usize, Process>(p), process::<N>))?;
    let (ch, ah, ph) = (CTOR_HOOK[N].get_or_init(|| ch), ADD_HOOK[N].get_or_init(|| ah), PROCESS_HOOK[N].get_or_init(|| ph));
    hook(ph.enable())?;
    hook(ah.enable())?;
    hook(ch.enable())
}

/// the hooks: the menus that got the button, or why none did (then the overlay buttons stay)
pub unsafe fn install() -> Result<Vec<&'static str>, String> {
    let _ = ENGINE.set(Engine {
        literal: std::mem::transmute::<usize, Literal>(sym(LITERAL)?),
        dtor: std::mem::transmute::<usize, Dtor>(sym(DTOR)?),
        key: off("LocalisedString", "key")?,
    });
    let mut done = Vec::new();
    for (i, r) in [install_menu::<0>(), install_menu::<1>()].into_iter().enumerate() {
        match r {
            Ok(()) => done.push(MENUS[i].name),
            Err(e) => fp::log(&format!("{} button: {e}", MENUS[i].name)),
        }
    }
    if done.contains(&MENUS[0].name) { Ok(done) } else { Err("not in the main menu".into()) }
}
