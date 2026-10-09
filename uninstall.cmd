@echo off
rem Removes fse from the game: the loader (bin\x64\version.dll) and this folder. The fse-* mods stay in your mods
rem folder: without fse they do nothing, and mods that use them keep loading.
setlocal
for %%I in ("%~dp0.") do set "HERE=%%~fI"
if not exist "%HERE%\..\bin\x64\factorio.exe" (echo This is not inside a Factorio folder. & pause & exit /b 1)
tasklist /fi "imagename eq factorio.exe" | "%SystemRoot%\System32\find.exe" /i "factorio.exe" >nul && (echo Close Factorio first. & pause & exit /b 1)
del "%HERE%\..\bin\x64\version.dll"
cd /d "%HERE%\.."
echo fse removed.
pause
(goto) 2>nul & rd /s /q "%HERE%"
