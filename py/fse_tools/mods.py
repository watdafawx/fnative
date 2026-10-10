"""The mod list as the in-game manager doesn't show it: when each mod was added and updated, the real load order, and
which ones have a newer version on the mod portal. Called through the fse py plugin by web/mods.html:

    POST /api/native/py/fse_tools.mods:listing      -> {"mods": [...], ...}
    POST /api/native/py/fse_tools.mods:portal       -> {"<name>": {"version", "released_at"}} (latest releases)

- added: the first time fse saw the mod (kept in mods-seen.json beside the launcher: a file's own creation date
  resets on every update, because the updater replaces the zip). Mods already there the first time get their file's
  creation date, the best guess there is.
- updated: the date of the mod's file (when this version arrived).
- load order: the order the game loaded the mods' data.lua in, read from factorio-current.log (the game's own answer).
Folders: FSE_MODS, else FACTORIO_MODS (the mods folder; default %APPDATA%\\Factorio\\mods) and the log in the folder above it.
"""
import json
import os
import re
import time
import urllib.parse
import urllib.request
import zipfile
from pathlib import Path

BUILT_IN = {"base", "core", "space-age", "quality", "elevated-rails"}
GAME_VERSION = os.environ.get("FACTORIO_VERSION", "2.0")  # (releases for other game versions don't count)


def _mods_dir() -> Path:
    # (FSE_MODS: what the core found for this game's arguments, --mod-directory included)
    return Path(os.environ.get("FSE_MODS") or os.environ.get("FACTORIO_MODS") or os.path.join(os.environ.get("APPDATA", ""), "Factorio", "mods"))


def _home() -> Path:
    return Path(os.environ.get("FSE_HOME", "."))


def _version_key(v: str):
    return tuple(int(x) if x.isdigit() else 0 for x in v.split("."))


def _info(path: Path):
    """info.json of a mod zip or folder, or None"""
    try:
        if path.suffix == ".zip":
            with zipfile.ZipFile(path) as z:
                name = next((n for n in z.namelist() if n.count("/") == 1 and n.endswith("/info.json")), None)
                return json.loads(z.read(name).decode("utf-8-sig")) if name else None
        f = path / "info.json"
        return json.loads(f.read_text(encoding="utf-8-sig")) if f.exists() else None
    except (OSError, ValueError, KeyError, zipfile.BadZipFile):
        return None


def _load_order(log: Path):
    """every active mod in the game's own load order: its "Checksum of <mod>" lines after the data stage (unlike
    "Loading mod" lines, which only mods with a data.lua get)"""
    order = []
    try:
        for line in log.read_text(encoding="utf-8", errors="replace").splitlines():
            m = re.search(r"Checksum of (\S+): ", line)
            if m:
                if m.group(1) not in order:
                    order.append(m.group(1))
            elif order:
                break  # (the block ended)
    except OSError:
        pass
    return order


def listing(_: str = "") -> str:
    mods_dir = _mods_dir()
    try:
        enabled = {m["name"]: m["enabled"] for m in
                   json.loads((mods_dir / "mod-list.json").read_text(encoding="utf-8-sig"))["mods"]}
    except (OSError, ValueError, KeyError):
        enabled = {}
    seen_file = _home() / "mods-seen.json"
    try:
        seen = json.loads(seen_file.read_text())
    except (OSError, ValueError):
        seen = {}
    found = {}
    for p in mods_dir.iterdir():
        if not (p.suffix == ".zip" or (p.is_dir() and (p / "info.json").exists())):
            continue
        info = _info(p)
        if not info or not info.get("name"):
            continue
        st = p.stat()
        size = st.st_size if p.is_file() else sum(f.stat().st_size for f in p.rglob("*") if f.is_file())
        row = {
            "name": info["name"], "version": info.get("version", "?"), "title": info.get("title") or info["name"],
            "author": info.get("author", ""), "file": p.name, "folder": p.is_dir(), "size": size,
            "updated": st.st_mtime, "created": st.st_ctime,
            "requires": [re.split(r"[<>=\s]", d.strip().lstrip("~").strip(), maxsplit=1)[0]
                         for d in info.get("dependencies") or [] if not d.strip().startswith(("?", "!", "(?)"))],
            "description": (info.get("description") or "")[:300],
        }
        old = found.get(row["name"])
        if old is None or _version_key(row["version"]) > _version_key(old["version"]):
            if old:
                row["older_copies"] = old.get("older_copies", []) + [old["file"]]
            found[row["name"]] = row
        else:
            old.setdefault("older_copies", []).append(row["file"])
    now = time.time()
    changed = False
    first_run = not seen  # (mods there before fse's first look get their file's date: the best guess there is)
    for name, row in found.items():
        if name not in seen:
            seen[name] = min(row["created"], row["updated"]) if first_run else now
            changed = True
        row["added"] = seen[name]
        row["enabled"] = enabled.get(name, True)
    if changed:
        try:
            seen_file.write_text(json.dumps(seen, indent=0, sort_keys=True))
        except OSError:
            pass
    log = mods_dir.parent / "factorio-current.log"
    order = _load_order(log)
    pos = {n: i + 1 for i, n in enumerate(order)}
    rows = sorted(found.values(), key=lambda r: (pos.get(r["name"], 10 ** 6), r["name"].lower()))
    for r in rows:
        r["load"] = pos.get(r["name"])
    built_in = [{"name": n, "load": pos[n], "built_in": True} for n in order if n in BUILT_IN]
    return json.dumps({"mods": rows, "built_in": built_in, "mods_dir": str(mods_dir),
                       "load_order_from": str(log), "load_order_time": log.stat().st_mtime if log.exists() else None,
                       "enabled": sum(1 for r in rows if r["enabled"]), "total": len(rows)})


def portal(names_json: str = "") -> str:
    """latest releases on the mod portal for these mods (a JSON list; default: every installed one), cached 6 h"""
    cache_file = _home() / "portal-cache.json"
    try:
        cache = json.loads(cache_file.read_text())
    except (OSError, ValueError):
        cache = {}
    names = json.loads(names_json) if names_json.strip() else [
        r["name"] for r in json.loads(listing())["mods"]]
    stale = [n for n in names if n not in BUILT_IN and time.time() - cache.get(n, {}).get("checked", 0) > 6 * 3600]
    for i in range(0, len(stale), 100):  # (the portal takes a name list: 100 per request)
        chunk = stale[i:i + 100]
        # (names may hold spaces: "Flare Stack")
        url = "https://mods.factorio.com/api/mods?page_size=max&namelist=" + ",".join(urllib.parse.quote(n, safe="") for n in chunk)
        try:
            with urllib.request.urlopen(url, timeout=20) as f:
                data = json.loads(f.read())
        except Exception as e:  # noqa: BLE001 - no network: say so, keep what is cached
            return json.dumps({"error": f"mod portal: {e}", "cached": {n: cache[n] for n in names if n in cache}})
        got = {r["name"]: r for r in data.get("results", [])}
        for n in chunk:
            # (the newest release for this game version: the portal already lists releases for Factorio 2.1)
            rels = [x for x in (got.get(n) or {}).get("releases") or []
                    if (x.get("info_json") or {}).get("factorio_version") == GAME_VERSION]
            rel = max(rels, key=lambda x: _version_key(x.get("version", "0")), default={})
            cache[n] = {"version": rel.get("version"), "released_at": rel.get("released_at"), "checked": time.time(),
                        "on_portal": n in got}
    try:
        cache_file.write_text(json.dumps(cache))
    except OSError:
        pass
    return json.dumps({n: cache[n] for n in names if n in cache})
