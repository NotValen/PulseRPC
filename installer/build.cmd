@echo off
setlocal
cd /d "%~dp0.."

rem ── version ────────────────────────────────────────────────────────────────
set "APPVER="
for /f "tokens=2 delims== " %%v in ('findstr /b /r /c:"^version" Cargo.toml') do (
    if not defined APPVER set "APPVER=%%~v"
)
if not defined APPVER set "APPVER=1.1.0"

echo Building PulseRPC %APPVER% ...

rem ── release binary ─────────────────────────────────────────────────────────
cargo build --release
if errorlevel 1 (
    echo error: cargo build failed
    exit /b 1
)

rem ── locate Inno Setup ──────────────────────────────────────────────────────
set "ISCC="
if exist "%ProgramFiles(x86)%\Inno Setup 6\ISCC.exe" set "ISCC=%ProgramFiles(x86)%\Inno Setup 6\ISCC.exe"
if not defined ISCC if exist "%ProgramFiles%\Inno Setup 6\ISCC.exe" set "ISCC=%ProgramFiles%\Inno Setup 6\ISCC.exe"
if not defined ISCC if exist "%LocalAppData%\Programs\Inno Setup 6\ISCC.exe" set "ISCC=%LocalAppData%\Programs\Inno Setup 6\ISCC.exe"
if not defined ISCC for /f "delims=" %%p in ('where ISCC.exe 2^>nul') do (
    if not defined ISCC set "ISCC=%%p"
)
if not defined ISCC (
    echo error: Inno Setup 6 was not found.
    echo Install it with:  winget install -e --id JRSoftware.InnoSetup
    exit /b 1
)

rem ── installer ──────────────────────────────────────────────────────────────
if not exist dist mkdir dist
"%ISCC%" /Qp /DAppVersion=%APPVER% "installer\pulserpc.iss"
if errorlevel 1 (
    echo error: installer compilation failed
    exit /b 1
)

echo.
echo Installer: dist\PulseRPC-Setup-%APPVER%.exe
endlocal
