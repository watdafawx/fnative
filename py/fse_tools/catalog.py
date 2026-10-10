"""FSE's mod catalog: mods made for fse, listed in one index (github.com/watdafawx/fse-mods, index.json), installed
into the mods folder from the hub's "Get mods" tab. Called through the fse py plugin:

    fse_tools.catalog:listing        -> {"mods": [entry + "installed", "missing"], "index", "error"?}
    fse_tools.catalog:install <name> -> {"ok", "name", "version", "folder", "pypath"?} or {"error"}

An index entry: name, title, author, description, repo, version, url (the mod's zip: one top folder holding
info.json), sha256 (of that zip), fse (oldest loader it runs on), plugins (fse plugins it needs), dependencies (its
info.json's), pypath (true: its folder goes on FSE_PYPATH, for mods with a Python half).

FSE mods can run native code (the py plugin), so a zip is only installed when its sha256 is the one the index lists:
a new version reaches players only through a change to the index. FSE_MOD_INDEX points at another index (a URL or
file:// path, for tests). The mod is unpacked as <mods>\\<name>_<version>\\ (a folder, so a Python half can be
imported), older copies are removed, and mod-list.json enables it; the game loads it at its next start.
"""
import hashlib
import io
import json
import os
import re
import shutil
import urllib.request
import zipfile
from pathlib import Path

from .mods import BUILT_IN, _home, _info, _mods_dir, _version_key

INDEX = os.environ.get("FSE_MOD_INDEX", "https://raw.githubusercontent.com/watdafawx/fse-mods/main/index.json")


def _fetch(url: str) -> bytes:
    with urllib.request.urlopen(url, timeout=30) as f:  # (file:// too)
        return f.read()


def _index():
    """(the index, an error or None): fetched, else the last one fetched"""
    cache = _home() / "catalog-cache.json"
    try:
        index = json.loads(_fetch(INDEX))
        try:
            cache.write_text(json.dumps(index))
        except OSError:
            pass
        return index, None
    except Exception as e:  # noqa: BLE001 - no network: say so, keep the cached one
        try:
            return json.loads(cache.read_text()), f"index: {e}"
        except (OSError, ValueError):
            return {"mods": []}, f"index: {e}"


def _copies(mods_dir: Path, name: str):
    """every copy of this mod in the mods folder: [(path, version)]"""
    out = []
    for p in mods_dir.glob(name + "*"):
        if p.name in (name, name + ".zip") or p.name.startswith(name + "_"):
            info = _info(p)
            if info and info.get("name") == name:
                out.append((p, info.get("version", "0")))
    return out


def _dev_copy(p: Path) -> bool:
    """a link to somewhere else or a git checkout: someone's working copy, never replaced"""
    return p.is_dir() and (os.path.realpath(p) != os.path.abspath(p) or (p / ".git").exists())


def listing(_: str = "") -> str:
    index, error = _index()
    mods_dir = _mods_dir()
    have = {}
    for p in mods_dir.iterdir() if mods_dir.exists() else ():
        info = _info(p) if p.suffix == ".zip" or p.is_dir() else None
        if info and info.get("name") and _version_key(info.get("version", "0")) > _version_key(have.get(info["name"], "-1")):
            have[info["name"]] = info.get("version", "0")
    rows = []
    for e in index.get("mods", []):
        need = [re.split(r"[<>=\s]", d.strip().lstrip("~").strip(), maxsplit=1)[0]
                for d in e.get("dependencies") or [] if not d.strip().startswith(("?", "!", "(?)"))]
        rows.append({**e, "installed": have.get(e.get("name")),
                     "missing": [n for n in need if n not in BUILT_IN and n not in have]})
    out = {"mods": rows, "index": INDEX}
    if error:
        out["error"] = error
    return json.dumps(out)


def _add_pypath(folder: Path):
    """put the folder on FSE_PYPATH in fse.env (beside the launcher); the game reads it at its next start"""
    env = _home() / "fse.env"
    lines = env.read_text(encoding="utf-8").splitlines() if env.exists() else []
    for i, line in enumerate(lines):
        if line.startswith("FSE_PYPATH="):
            parts = [p for p in line[len("FSE_PYPATH="):].split(";") if p]
            if str(folder) in parts:
                return
            # (an older version's folder goes)
            parts = [p for p in parts if not Path(p).name.startswith(folder.name.rsplit("_", 1)[0] + "_")]
            lines[i] = "FSE_PYPATH=" + ";".join(parts + [str(folder)])
            break
    else:
        lines.append("FSE_PYPATH=" + str(folder))
    env.write_text("\n".join(lines) + "\n", encoding="utf-8")


def _enable(mods_dir: Path, name: str):
    f = mods_dir / "mod-list.json"
    try:
        listed = json.loads(f.read_text(encoding="utf-8-sig"))
    except (OSError, ValueError):
        listed = {"mods": []}
    for m in listed["mods"]:
        if m.get("name") == name:
            m["enabled"] = True
            break
    else:
        listed["mods"].append({"name": name, "enabled": True})
    f.write_text(json.dumps(listed, indent=2), encoding="utf-8")


def install(name: str) -> str:
    name = name.strip()
    index, error = _index()
    if error:
        return json.dumps({"error": error + " (installs need the current index)"})
    e = next((m for m in index.get("mods", []) if m.get("name") == name), None)
    if not e:
        return json.dumps({"error": f"{name} is not in the index"})
    try:
        data = _fetch(e["url"])
    except Exception as x:  # noqa: BLE001
        return json.dumps({"error": f"download: {x}"})
    if hashlib.sha256(data).hexdigest() != e.get("sha256", "").lower():
        return json.dumps({"error": f"{name}: the download's sha256 is not the one the index lists (not installed)"})
    mods_dir = _mods_dir()
    old = _copies(mods_dir, name)
    dev = [p.name for p, _ in old if _dev_copy(p)]
    if dev:
        return json.dumps({"error": f"{name}: a working copy is installed ({', '.join(dev)}), left alone"})
    try:
        z = zipfile.ZipFile(io.BytesIO(data))
        tops = {n.split("/", 1)[0] for n in z.namelist()}
        if len(tops) != 1:
            raise ValueError("the zip must hold one folder")
        top = tops.pop()
        info = json.loads(z.read(top + "/info.json").decode("utf-8-sig"))
        if info.get("name") != name or info.get("version") != e.get("version"):
            raise ValueError(f"its info.json is {info.get('name')} {info.get('version')}, the index says {name} {e.get('version')}")
        dest = mods_dir / f"{name}_{info['version']}"
        tmp = mods_dir / f".{dest.name}.part"
        shutil.rmtree(tmp, ignore_errors=True)
        for m in z.infolist():
            rel = Path(m.filename).relative_to(top)
            if m.is_dir() or not rel.parts:
                continue
            if rel.is_absolute() or ".." in rel.parts:
                raise ValueError(f"bad path in the zip: {m.filename}")
            out = tmp / rel
            out.parent.mkdir(parents=True, exist_ok=True)
            out.write_bytes(z.read(m))
    except (ValueError, KeyError, OSError, zipfile.BadZipFile) as x:
        return json.dumps({"error": f"{name}: {x}"})
    for p, _ in old:
        shutil.rmtree(p) if p.is_dir() else p.unlink()
    tmp.rename(dest)
    _enable(mods_dir, name)
    out = {"ok": True, "name": name, "version": info["version"], "folder": str(dest),
           "replaced": [p.name for p, _ in old if p != dest]}
    if e.get("pypath"):
        _add_pypath(dest)
        out["pypath"] = True
    return json.dumps(out)
