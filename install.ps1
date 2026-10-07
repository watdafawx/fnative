<#
Installs fnative from the unzipped release folder: writes fnative.env, copies the mods into Factorio's mods folder
and puts a "Factorio (fnative)" shortcut on the desktop. Safe to re-run (your fnative.env is kept).

    powershell -ExecutionPolicy Bypass -File install.ps1 [-Mods <folder>] [-NoMods] [-NoShortcut]
#>
param([string]$Mods = "$env:APPDATA\Factorio\mods", [switch]$NoMods, [switch]$NoShortcut)
$ErrorActionPreference = "Stop"
$here = $PSScriptRoot
if (-not (Test-Path "$here\factorio-native.exe")) { throw "run this from the unzipped fnative folder" }

if (-not (Get-Command python -ErrorAction SilentlyContinue)) {
    Write-Warning "Python (3.12) is not on PATH: the py plugin needs it. Install from python.org and tick 'Add to PATH'."
}

if (-not (Test-Path "$here\fnative.env")) {
    (Get-Content "$here\fnative.env.example" -Raw).Replace("{FNATIVE_PY}", "$here\py") | Set-Content "$here\fnative.env" -Encoding utf8
    Write-Host "wrote fnative.env"
}

if (-not $NoMods) {
    New-Item -ItemType Directory -Force $Mods | Out-Null
    Get-ChildItem "$here\mods" -Directory | ForEach-Object {
        Copy-Item $_.FullName $Mods -Recurse -Force
        Write-Host "mod: $($_.Name)"
    }
}

if (-not $NoShortcut) {
    $lnk = Join-Path ([Environment]::GetFolderPath("Desktop")) "Factorio (fnative).lnk"
    $s = (New-Object -ComObject WScript.Shell).CreateShortcut($lnk)
    $s.TargetPath = "$here\factorio-native.exe"; $s.WorkingDirectory = $here; $s.Save()
    Write-Host "shortcut: $lnk"
}

Write-Host "`nDone. Start the game with the shortcut, or set Steam launch options to:`n  `"$here\factorio-native.exe`" %COMMAND%"
