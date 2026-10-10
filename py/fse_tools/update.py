"""fse updates: the newest release on GitHub (github.com/watdafawx/fse), and installing it over this one. Called
through the fse py plugin by the fse-hub mod and the dashboard:

    fse_tools.update:check <this version>   -> {"current", "latest", "newer", "notes", "url", "installable", "why"?}
    fse_tools.update:install                -> {"ok", "version", "files"} or {"error"}

The release zip mirrors the game's folder (bin/x64/version.dll, fse/...) and comes with <zip>.sha256; nothing is
written unless the download matches it. The zip is unpacked into <fse>/update first, then each file is swapped in:
a file the running game holds (fse.dll, the plugins, version.dll) can't be overwritten but can be renamed, so it
becomes <name>.fse-old and the new one takes its place; the game uses them from its next start, and the next check
deletes the .fse-old files. fse.env and plugins/profiler.json (settings) are never replaced. Only an install in a game folder (<game>/fse beside
<game>/bin/x64/factorio.exe) updates itself: a dev build (fse/dist) doesn't.
FSE_UPDATE_API: another release JSON (the GitHub API's answer, or a file:// one for tests).
"""
import hashlib
import io
import json
import os
import shutil
import time
import urllib.request
import zipfile
from pathlib import Path

from .mods import _home, _version_key

API = os.environ.get("FSE_UPDATE_API", "https://api.github.com/repos/watdafawx/fse/releases/latest")
KEEP = {"fse/fse.env", "fse/plugins/profiler.json"}  # (the player's settings: kept once they exist)


def _fetch(url: str) -> bytes:
    req = urllib.request.Request(url, headers={"User-Agent": "fse-update", "Accept": "application/vnd.github+json"})
    with urllib.request.urlopen(req, timeout=60) as f:
        return f.read()


def _game() -> Path:
    return _home().resolve().parent


def _cleanup():
    """the files an update renamed away (free once the game that held them has closed)"""
    for root in (_home(), _game() / "bin" / "x64"):
        for f in root.rglob("*.fse-old") if root.exists() else ():
            try:
                f.unlink()
            except OSError:
                pass


def _release():
    """the newest release: (version, zip url, sha256 url, notes)"""
    r = json.loads(_fetch(API))
    assets = {a["name"]: a["browser_download_url"] for a in r.get("assets", [])}
    zips = [n for n in assets if n.startswith("fse-") and n.endswith("-windows.zip")]
    if not zips:
        raise ValueError("the release has no fse zip")
    return (r["tag_name"].lstrip("v"), assets[zips[0]], assets.get(zips[0] + ".sha256"), r.get("body") or "")


def check(current: str = "") -> str:
    _cleanup()
    cache_file = _home() / "update-cache.json"
    try:
        cache = json.loads(cache_file.read_text())
    except (OSError, ValueError):
        cache = {}
    if time.time() - cache.get("checked", 0) > 6 * 3600 or cache.get("api") != API:
        try:
            latest, url, sha_url, notes = _release()
        except Exception as e:  # noqa: BLE001 - no network: say so
            return json.dumps({"current": current, "error": f"release check: {e}"})
        cache = {"checked": time.time(), "api": API, "latest": latest, "url": url, "sha_url": sha_url, "notes": notes}
        try:
            cache_file.write_text(json.dumps(cache))
        except OSError:
            pass
    out = {"current": current, "latest": cache["latest"], "url": cache["url"], "notes": cache["notes"][:2000],
           "newer": bool(current) and _version_key(cache["latest"]) > _version_key(current)}
    out["installable"] = (_game() / "bin" / "x64" / "factorio.exe").exists() and bool(cache.get("sha_url"))
    if not out["installable"]:
        out["why"] = ("the release has no sha256" if not cache.get("sha_url")
                      else "not installed in a game folder (a dev build): unzip the release yourself")
    return json.dumps(out)


def _swap(new: Path, dest: Path):
    dest.parent.mkdir(parents=True, exist_ok=True)
    try:
        os.replace(new, dest)
    except PermissionError:  # (held by the running game: renamed away, the new one goes in its place)
        old = dest.with_name(dest.name + ".fse-old")
        n = 1
        while old.exists():
            try:
                old.unlink()
            except OSError:
                old = dest.with_name(f"{dest.name}.{n}.fse-old")
                n += 1
        os.replace(dest, old)
        os.replace(new, dest)


def install(_: str = "") -> str:
    try:
        latest, url, sha_url, _notes = _release()
        if not sha_url:
            return json.dumps({"error": "the release has no sha256: not installed"})
        if not (_game() / "bin" / "x64" / "factorio.exe").exists():
            return json.dumps({"error": "not installed in a game folder (a dev build): not installed"})
        data = _fetch(url)
        want = _fetch(sha_url).decode().split()[0].lower()
        if hashlib.sha256(data).hexdigest() != want:
            return json.dumps({"error": "the download's sha256 doesn't match the release's: not installed"})
        staging = _home() / "update"
        shutil.rmtree(staging, ignore_errors=True)
        files = []
        with zipfile.ZipFile(io.BytesIO(data)) as z:
            for m in z.infolist():
                rel = Path(m.filename)
                if m.is_dir() or rel.is_absolute() or ".." in rel.parts or m.filename in KEEP and (_game() / rel).exists():
                    continue
                if rel.parts[0] not in ("fse", "bin"):
                    continue
                out = staging / rel
                out.parent.mkdir(parents=True, exist_ok=True)
                out.write_bytes(z.read(m))
                files.append(rel)
        for rel in files:
            _swap(staging / rel, _game() / rel)
        shutil.rmtree(staging, ignore_errors=True)
        (_home() / "update-cache.json").unlink(missing_ok=True)
    except Exception as e:  # noqa: BLE001 - say what went wrong, in the hub
        return json.dumps({"error": f"update: {e}"})
    return json.dumps({"ok": True, "version": latest, "files": len(files)})
