"""How long the game took to start, and what took it: read from the game's own log (factorio-current.log, every
line stamped with seconds since start). Called through the fse py plugin by web/startup.html:

    POST /api/native/py/fse_tools.startup:report   -> {"total", "phases": [...], "mods": [...]}

- phases: Steam and init, mod settings, mod data (Lua), prototypes, sounds, graphics setup, sprites;
- per mod, measured: the time of each of its Lua stages (settings, data, data-updates, data-final-fixes and their
  -updates/-final-fixes in settings): from its "Loading mod" line to the next one;
- per mod, estimated: its share of the sprite and sound phases by the size of its images and sounds (the log
  doesn't break those down; they load in parallel, so bytes are the fair guess).
The log: FACTORIO_LOG, else factorio-current.log beside the mods folder (FACTORIO_MODS).
"""
import json
import os
import re
import zipfile
from pathlib import Path

LINE = re.compile(r"^\s*(\d+\.\d+) (.*)$")
LOADING = re.compile(r"Loading mod (?:settings )?(\S+) (\S+) \((\S+)\.lua\)")


def _log_path() -> Path:
    if os.environ.get("FACTORIO_LOG"):
        return Path(os.environ["FACTORIO_LOG"])
    return Path(os.environ.get("FACTORIO_MODS") or os.path.join(os.environ.get("APPDATA", ""), "Factorio", "mods")).parent / "factorio-current.log"


def _media_bytes(mods_dir: Path):
    """name -> (image bytes, sound bytes) of each mod's zip or folder (the highest version when there are several)"""
    out, versions = {}, {}
    for p in mods_dir.iterdir():
        m = re.match(r"(.+)_(\d+\.\d+\.\d+)(\.zip)?$", p.name)
        if not m:
            continue
        name, ver = m.group(1), tuple(int(x) for x in m.group(2).split("."))
        if versions.get(name, (-1,)) > ver:
            continue
        img = snd = 0
        try:
            if p.suffix == ".zip":
                with zipfile.ZipFile(p) as z:
                    for i in z.infolist():
                        n = i.filename.lower()
                        if n.endswith((".png", ".jpg")):
                            img += i.file_size
                        elif n.endswith((".ogg", ".wav", ".voc")):
                            snd += i.file_size
            elif p.is_dir():
                for f in p.rglob("*"):
                    n = f.name.lower()
                    if n.endswith((".png", ".jpg")):
                        img += f.stat().st_size
                    elif n.endswith((".ogg", ".wav", ".voc")):
                        snd += f.stat().st_size
        except (OSError, zipfile.BadZipFile):
            continue
        versions[name] = ver
        out[name] = (img, snd)
    return out


def report(_: str = "") -> str:
    log = _log_path()
    lines = []
    for raw in log.read_text(encoding="utf-8", errors="replace").splitlines():
        m = LINE.match(raw)
        if m:
            lines.append((float(m.group(1)), m.group(2)))
    def first(pred):
        return next((t for t, s in lines if pred(s)), None)

    t_settings = first(lambda s: "Loading mod settings" in s)
    t_data = first(lambda s: s.startswith("Loading mod core") or ("(data.lua)" in s and "Loading mod" in s))
    t_proto = first(lambda s: s.startswith("Checksum for core"))
    t_proto_end = first(lambda s: s.startswith("Prototype list checksum"))
    t_sounds = first(lambda s: s.startswith("Loading sounds"))
    t_sounds_end = next((t for t, s in lines if t_sounds is not None and t > t_sounds), None)
    t_sprites = first(lambda s: "Parallel sprite loader initialized" in s or "Initial atlas bitmap size" in s)
    t_sprites_end = first(lambda s: "Sprite loader stats" in s) or first(lambda s: s.startswith("Video memory usage"))
    t_done = first(lambda s: s.startswith("Factorio initialised"))
    loaded_map = first(lambda s: s.startswith("Loading map "))

    # every mod's Lua stages: its "Loading mod" line to the next line that starts another stage
    stage_lines = [(t, s) for t, s in lines if (t_settings or 0) <= t <= (t_proto or 1e9)]
    marks = []
    for t, s in stage_lines:
        m = LOADING.search(s)
        if m:
            marks.append((t, m.group(1), m.group(2), m.group(3)))
    ends = [m[0] for m in marks[1:]] + [t_proto or (marks[-1][0] if marks else 0)]
    mods = {}
    # (the data stage's last file runs into the engine closing the stage, with no log line between: that time goes
    # to its own row, not to whichever mod loads last)
    if marks and t_proto is not None and not marks[-1][3].startswith("settings"):
        t_last = marks[-1][0]
        mods["(last file + engine closing the data stage)"] = {
            "name": "(last file + engine closing the data stage)", "version": "", "lua": t_proto - t_last,
            "stages": {"data-final-fixes": round(t_proto - t_last, 3)}}
        ends[-1] = t_last
    for (t, name, ver, stage), end in zip(marks, ends):
        if stage.startswith("settings") and t_data is not None and end > t_data:
            end = t_data  # (the settings stage's last file ends where the data stage begins)
        row = mods.setdefault(name, {"name": name, "version": ver, "stages": {}, "lua": 0.0})
        row["stages"][stage] = round(row["stages"].get(stage, 0) + (end - t), 3)
        row["lua"] += end - t

    phases = []
    def phase(name, a, b, what):
        if a is not None and b is not None and b >= a:
            phases.append({"name": name, "start": a, "end": b, "seconds": round(b - a, 3), "what": what})
    phase("Start, Steam, finding mods", 0.0, t_settings, "before any mod runs")
    phase("Mod settings (Lua)", t_settings, t_data, "settings.lua files")
    phase("Mod data (Lua)", t_data, t_proto, "data.lua, data-updates.lua, data-final-fixes.lua: per mod below")
    phase("Prototypes", t_proto, t_proto_end, "the engine checks and builds every prototype mods defined")
    phase("Sounds", t_sounds, t_sounds_end, "every sound file")
    phase("Graphics setup", t_sounds_end or t_proto_end, t_sprites, "options, atlases")
    phase("Sprites", t_sprites, t_sprites_end, "every image, decoded on all cores and packed into atlases")
    phase("Finishing", t_sprites_end, t_done, "")

    # (mods without Lua files have no "Loading mod" line; the checksum list has every active mod)
    for t, s in lines:
        m = re.match(r"Checksum of (\S+): ", s)
        if m and m.group(1) not in mods and m.group(1) not in ("core",):
            mods[m.group(1)] = {"name": m.group(1), "version": "", "stages": {}, "lua": 0.0}

    sprite_s = next((p["seconds"] for p in phases if p["name"] == "Sprites"), 0.0)
    sound_s = next((p["seconds"] for p in phases if p["name"] == "Sounds"), 0.0)
    media = _media_bytes(Path(os.environ.get("FACTORIO_MODS") or os.path.join(os.environ.get("APPDATA", ""), "Factorio", "mods")))
    active = set(mods)
    img_total = sum(media[n][0] for n in active if n in media) or 1
    snd_total = sum(media[n][1] for n in active if n in media) or 1
    rows = []
    for name, r in mods.items():
        img, snd = media.get(name, (0, 0))
        r["lua"] = round(r["lua"], 3)
        r["image_bytes"], r["sound_bytes"] = img, snd
        r["sprites_est"] = round(sprite_s * img / img_total, 3)
        r["sounds_est"] = round(sound_s * snd / snd_total, 3)
        r["total_est"] = round(r["lua"] + r["sprites_est"] + r["sounds_est"], 3)
        rows.append(r)
    rows.sort(key=lambda r: -r["total_est"])
    return json.dumps({
        "log": str(log), "log_time": log.stat().st_mtime, "total": t_done, "map_loaded_at": loaded_map,
        "time_to_load_mods": next((float(s.split(":")[-1]) for t, s in lines if "Time to load mods" in s), None),
        "phases": phases, "mods": rows,
        "note": "lua is measured from the log; sprites_est and sounds_est split those phases by each mod's image and "
                "sound bytes (the log doesn't break them down)",
    })
