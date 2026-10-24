# [PulseRPC](https://raw.githubusercontent.com/NotValen/PulseRPC/99c0245b955b423b688770075eec2fa4c0cf85b8/assets/preview.png)
![alt text](https://raw.githubusercontent.com/NotValen/PulseRPC/99c0245b955b423b688770075eec2fa4c0cf85b8/assets/preview.png "Logo Title Text 1")


Automatic Discord Rich Presence for 100+ desktop applications — a single small
Rust executable with a native Windows window. It watches what you are doing and
keeps your Discord profile in sync. No Electron, no background services, no
telemetry.

```
Editing config.rs
PulseRPC · 12m
```

## Features

- **Easy native GUI**: paste your Discord Application ID, toggle the options you
  care about, pause/resume, open the config or the log — no text editor needed.
  Closing the window keeps PulseRPC running in the notification area.
- **103 built-in app rules** across development tools, browsers, games, media
  players, productivity apps, chat apps and utilities — and every rule is a
  plain TOML file you can edit or extend.
- **Smart detection**: the focused window wins, fullscreen games win over
  launchers, title patterns separate "Visual Studio Code" from "Chrome with a
  VS Code tab", and background music players can be configured to always show.
- **Dynamic messages**: rotating status pools, template variables
  (`{filename}`, `{project}`, `{artist}`, `{track}`, `{elapsed}`, `{time_of_day}`…)
  and per-app buttons.
- **Hot reload**: save `config.toml` or any file in `config/apps/` and the
  presence updates within a second. A broken edit is rejected and the previous
  working configuration keeps running.
- **Light on resources**: a single process, well under 1% CPU and roughly
  10–15 MB resident memory.
- **Idle and privacy aware**: goes "Away" after inactivity and never publishes
  incognito/InPrivate window titles.
- **Native Windows installer** (Inno Setup `Setup.exe`) plus a portable mode.
  No PowerShell anywhere in the install path.
- Works with the **Discord desktop app** (stable, PTB and Canary) over the
  local IPC pipe. Discord does not need to be running first — PulseRPC
  reconnects on its own.

## Install

### Option A — `PulseRPC-Setup-x.y.z.exe` (recommended)

1. Run the installer. It installs per-user to
   `%LOCALAPPDATA%\Programs\PulseRPC` (no UAC prompt).
2. Paste your Discord **Application ID** into the wizard page (you can skip it
   and fill it in the window later).
3. Leave "Start PulseRPC when I sign in" enabled.
4. The window opens when setup finishes. Paste the ID if you skipped it and
   press **Save** — the status turns green and your presence goes live.

Uninstall through **Settings → Apps → PulseRPC**. Your configuration is
kept unless you say otherwise; the uninstaller asks.

### Option B — portable / built-in installer

```cmd
pulserpc.exe --install            :: copy to %LOCALAPPDATA%\Programs\PulseRPC
pulserpc.exe --install --start    :: …and launch it
pulserpc.exe --uninstall          :: remove startup entry, registry entry and files
```

`--install` writes the default config next to the executable, adds a
`HKCU\...\Run` entry (`pulserpc.exe --tray`) and registers an entry in
*Add or remove programs*. Both install methods are native code; no PowerShell
scripts are shipped or executed.

## The window

```
┌ PulseRPC ──────────────────────────────────────────── version 1.1.0 ┐
│  Connected — Visual Studio Code                    [ Pause ]        │
│  Editing config.rs · main.rs                                        │
│  Code.exe — config.rs - PulseRPC - Visual Studio Code               │
│                                                                     │
│  ┌ Discord ─────────────────┐  ┌ Activity ──────────────────────┐   │
│  │ Discord Application ID   │  │ PRESENCE …                     │   │
│  │ [ 123456789012345678 ]   │  │ FOREGROUND …                   │   │
│  │ [Save]                   │  │ SYSTEM …                       │   │
│  │ [x] Start with Windows   │  │ FILES …                        │   │
│  │ [x] Show elapsed time    │  │ [Open config] [Rules] [Log]    │   │
│  │ [x] Hide private windows │  │ [Reload]                       │   │
│  │ Idle after (min)  [10]   │  └────────────────────────────────┘   │
│  │ Rotate every (s)  [45]   │                                       │
│  │ [Apply]                  │                                       │
│  └──────────────────────────┘                                       │
│  Config: C:\Users\you\AppData\Local\Programs\PulseRPC\config       │
│                                                   [ Quit ]          │
└─────────────────────────────────────────────────────────────────────┘
```

- **Status card** shows whether Discord is connected, what is currently being
  shared, and the foreground app. Green means everything is working, amber
  means it is waiting for Discord or for an Application ID.
- **Pause** stops updating and clears your presence; **Resume** starts again.
- **Start with Windows** writes/removes the `HKCU\...\Run` value (uses
  `--tray`, so sign-in does not pop the window).
- **Show elapsed time**, **Hide private windows**, **Idle after** and
  **Rotate every** are saved into `config.toml`. Only those lines change, so
  your comments and hand-written settings survive.
- **Open config / Rules / Log** open the config folder, a searchable rule list
  window, and the log file.
- **Reload** re-reads the config immediately (it is also watched while running).
- **Closing the window hides it to the notification area.** Right-click the
  tray icon for Open, Pause/Resume, config and Quit. Only **Quit** actually
  exits.

Starting PulseRPC again while it is running (including from a shortcut) opens
the existing window instead of starting a second copy.

## Discord application setup

Rich Presence needs a Discord *application* (it is just an id — you are not
publishing a bot):

1. Open <https://discord.com/developers/applications> → **New Application**.
2. Copy the **Application ID** from *General Information*.
3. Paste it into the PulseRPC window and press **Save** — or from a terminal:

   ```cmd
   pulserpc.exe --set-id 123456789012345678
   ```

   …or edit `presence.client_id` in `config.toml`. The id is hot-reloaded, so a
   running instance reconnects immediately.
4. *(Optional, for artwork)* Under **Rich Presence → Art Assets** upload images
   named after the `large_image` / `small_image` keys used by the rules, e.g.
   `vscode`, `spotify`, `cs2`, `idle`. Missing assets do not break anything —
   Discord simply shows no image.

## Quick start

```cmd
pulserpc.exe                :: open the GUI (the normal way to run it)
pulserpc.exe --tray         :: start hidden in the notification area
pulserpc.exe --validate     :: parse config, print warnings
pulserpc.exe --rules        :: list the 103 configured apps
pulserpc.exe --probe        :: show what is running and which rule wins
pulserpc.exe --once         :: render one activity as JSON and exit
pulserpc.exe --dry-run      :: run normally, print activities, never connect
pulserpc.exe --headless     :: run without a window (console/service style)
```

### Command line

| Option | Meaning |
| --- | --- |
| `-c, --config <DIR>` | Config directory (default: `config\` next to the exe, else `%LOCALAPPDATA%\PulseRPC\config`) |
| `--id <ID>` | Override the Discord application id for this run only |
| `--set-id <ID>` | Store an application id in `config.toml` and exit |
| `--tray` | Start with the window hidden (used by the autostart entry) |
| `--headless` | Run the detection loop without the GUI |
| `--once` | Detect once, print the activity JSON and exit |
| `--dry-run` | Run the normal loop but print activities instead of sending them |
| `--probe` | Print every process/window and the winning rule |
| `--rules` | Print all configured applications |
| `--validate` | Validate the configuration |
| `--no-watch` | Disable hot reload |
| `-q, --quiet` / `-v, --verbose` | Less / more logging |
| `--install`, `--uninstall`, `--dir`, `--no-autostart`, `--start` | Native installer (see above) |
| `-V, --version`, `-h, --help` | Version / help |

Exit codes: `0` success, `1` runtime/config error, `2` bad arguments.

## Configuration

```
%LOCALAPPDATA%\Programs\PulseRPC\
├─ pulserpc.exe
└─ config\
   ├─ config.toml            global settings, idle + fallback presence
   └─ apps\
      ├─ development.toml    [[app]] rules, grouped by category
      ├─ browsers.toml
      ├─ media.toml
      ├─ games.toml
      ├─ productivity.toml
      ├─ communication.toml
      └─ utilities.toml
```

Everything is hot-reloadable. Files are watched by modification time and size
(no extra threads or `ReadDirectoryChangesW` handles); when a reload fails the
error goes to the log and the old config stays active.

> The default config is also embedded in the executable, so a fresh install
> with no `config\` folder provisions itself on first run.

### Global options (`config.toml`)

```toml
[presence]
client_id = "000000000000000000"  # your Discord Application ID
rotate_secs = 45                  # rotating message cadence (0 = off)
refresh_secs = 60                 # re-assert the presence periodically
min_update_secs = 4               # minimum gap between updates
switch_delay_ms = 1200            # ignore focus flicker shorter than this
resume_window_secs = 300          # keep the session timer when returning
keep_last_secs = 45               # grace period before the fallback state
show_elapsed = true               # show the "elapsed" timer by default
activity_type = 0                 # 0 Playing, 2 Listening, 3 Watching, 5 Competing
buttons = true                    # set false to hide all rule buttons
# large_image = "logo"            # optional global image fallbacks
# small_image = "dot"

[detection]
poll_ms = 2500                    # process/window scan interval
foreground_bonus = 1000           # score for the focused window
fullscreen_bonus = 150            # extra score for fullscreen windows
idle_after_secs = 600             # 0 disables idle detection
process_blacklist = ["explorer.exe", "searchhost.exe", ...]
title_blacklist = ["incognito", "inprivate", "private browsing"]  # hide titles

[privacy]
hide_when_private = true          # hide titles of private browsing windows
hide_paths = true                 # reserved for path-based rules
hide_hostname = true              # blank the {host} variable

[logging]
level = "info"                    # error | warn | info | debug | trace
file = true
max_kb = 2048                     # single-generation rotation

[idle]                            # after detection.idle_after_secs
enabled = true
details = ["Away", "Idle", "Stepped away"]
state = ["{idle_for} idle", "Back soon"]
large_image = "idle"

[fallback]                        # nothing configured in focus
enabled = true
details = ["On the desktop", "Between apps"]
state = ["{time_of_day}", "{day} · {time}"]
```

### Application rules (`config/apps/*.toml`)

```toml
[[app]]
id = "vscode"                     # unique id (required)
name = "Visual Studio Code"       # display name, used by {app}
category = "development"          # grouping in the Rules window
priority = 65                     # base score
process = ["Code.exe", "Code - Insiders.exe"]  # case-insensitive globs

# Title matching (all optional, case-insensitive)
title_contains = ["chrome"]       # at least one must match
title_all = ["code", "github"]    # every entry must match
title_excludes = ["incognito"]
title_regex = ' - Unity [0-9]'    # regex on the raw title

requires_foreground = false       # only report when focused
fullscreen_only = false           # only report fullscreen windows
min_title_len = 0
path_contains = []                # foreground exe path fragments (optional)

# Title cleanup before templates see it
strip_prefixes = ["● "]
strip_suffixes = [" - Visual Studio Code"]
strip_regex = [' - Obsidian.*$']

# Media parsing: "Artist - Title" → {artist}, {track}, {media}
media = false
media_order = "artist-first"      # or "title-first"
project_index = 1                 # which " - " segment is {project}
file_index = 0                    # which " - " segment is {file}

# What gets sent to Discord — every field can be a list (rotating pool)
details = ["Editing {filename}", "Working on {project}"]
state = ["{project} · {elapsed}", "{time_of_day} coding"]
large_image = "vscode"
large_text = "Visual Studio Code"
small_image = ""
small_text = ""
activity_type = 0
rotate_secs = 45                  # override the global cadence
show_elapsed = true
no_elapsed = false
enabled = true
buttons = [{ label = "Website", url = "https://code.visualstudio.com" }]
```

Unknown fields are ignored, so upgrading never breaks an existing config.
Duplicate `id`s, invalid regexes and missing `process` patterns are reported by
`--validate` and block the reload (the previous config keeps running).

### Template variables

| Variable | Example |
| --- | --- |
| `{app}`, `{game}` | `Visual Studio Code` |
| `{process}`, `{exe}` | `Code.exe` |
| `{pid}` | `18392` |
| `{title}` | `config.rs - PulseRPC - Visual Studio Code` |
| `{title_short}` | `config.rs` (trimmed to 48 chars) |
| `{project}`, `{workspace}` | `PulseRPC` |
| `{file}` | `config.rs` |
| `{filename}` | `config.rs` (basename, no directories) |
| `{artist}` | `Daft Punk` |
| `{track}`, `{song}` | `One More Time` |
| `{media}`, `{now_playing}` | `Daft Punk — One More Time` |
| `{elapsed}` | `1h 30m` |
| `{elapsed_short}` | `1h` |
| `{elapsed_mmss}` | `1:30:25` |
| `{started}` | `13:05` |
| `{time}` / `{date}` / `{day}` | `13:07` / `2026-10-24` / `Saturday` |
| `{time_of_day}`, `{tod}` | `Morning`, `Afternoon`, `Evening`, `Night` |
| `{idle_for}` | `12m` |
| `{user}` | your Windows user name |
| `{host}` | computer name (blank when `privacy.hide_hostname`) |
| `{category}` | `development` |
| `{private}` / `{fullscreen}` | `yes` / `no` |
| `{instances}` | number of running copies of the process |

Unknown placeholders are left as-is so typos are obvious. If a template
collapses to punctuation only (for example because `{artist}` is empty), the
next entry of the pool is used; if every entry collapses, the app name is used.

## How detection works

Each poll (2.5 s by default) PulseRPC takes a snapshot of processes, their
visible windows, the foreground window and the idle time, then scores every
rule against every process:

```
score = priority
      + 1000 if the process owns the foreground window
      +  150 if a window is fullscreen (borderless or exclusive)
      +    2 if the process pattern is an exact match (not a glob)
      +    5 if the rule matches on title text
```

The highest score wins. A rule switch has to survive a 1.2 s dwell time, so
alt-tabbing through windows does not spam updates. Leaving an app keeps the
last presence for 45 s before the fallback state, and returning within 5
minutes resumes the original session timer. Updates are rate-limited to 5 per
20 s with a 4 s minimum gap, matching Discord's limits, and the presence is
re-asserted every 60 s so a Discord restart cannot leave a stale profile.

Only the foreground process's executable path is resolved, which keeps the scan
cheap and avoids touching other users' processes.

### Performance

Measured on Windows 11, release build (GUI hidden in the tray), 200+ running
processes:

| Metric | Value |
| --- | --- |
| Binary size | ~2.1 MB |
| Resident memory | ~11–15 MB |
| CPU over an 8 s sample | ~150 ms (≈2%) |
| Idle wakeups | 4 / s (state machine), 1 scan / 2.5 s |

The GUI is plain Win32 (no web view): the window costs a few MB when open and
the hidden/tray state costs about the same as the old headless build.

## Privacy

- Only process names, window titles and (for the focused app) its executable
  path are read. Nothing is sent anywhere except the activity fields you
  configure, and only to the local Discord client over the IPC pipe.
- No network code, no analytics, no auto-update.
- Window titles of incognito/InPrivate/private windows are dropped before any
  template sees them (`detection.title_blacklist` + `privacy.hide_when_private`).
- Chat apps ship with generic messages; add `{title_short}` yourself if you
  want to share the active conversation.
- `privacy.hide_hostname = true` keeps `{host}` blank.
- The log (`%LOCALAPPDATA%\PulseRPC\logs\pulserpc.log`) contains app
  names and error details, never full page titles, and rotates at 2 MB.

## Troubleshooting

| Symptom | Fix |
| --- | --- |
| Nothing shows on Discord | Discord must be running on the same machine. The status card turns green when connected; the log says `Connected to Discord`. |
| Status says "Discord Application ID needed" | Paste the 17–20 digit Application ID and press Save, or run `pulserpc --set-id <ID>`. |
| Closing the window did not stop it | By design: it is in the notification area. Right-click the tray icon → **Quit PulseRPC**. |
| Tray icon is missing | Windows hides new icons under the `^` chevron; drag it onto the taskbar or enable it in *Settings → Personalization → Taskbar*. |
| Presence is stuck on the desktop state | Run `pulserpc --probe` and check which rule wins; the app may need a new `process` entry. |
| Images are missing | Upload images with matching keys under *Rich Presence → Art Assets*; text still works without them. |
| "PulseRPC is already running" | A second copy opens the existing window by design. Quit the first one to start fresh, or use `--dry-run` for testing. |
| A config edit did nothing | `pulserpc --validate` shows parse errors; a failed reload keeps the old config. The log explains what was rejected. |
| Want to see exactly what would be sent | `pulserpc --dry-run --verbose` and `pulserpc --once`. |

Log file: `%LOCALAPPDATA%\PulseRPC\logs\pulserpc.log`
(rotated to `pulserpc.old.log`).
Single-instance lock: `%LOCALAPPDATA%\PulseRPC\pulserpc.lock`.

## Build from source

Requirements: Rust 1.80+ with the MSVC toolchain, and Inno Setup 6 for the
installer.

```cmd
cargo test
cargo build --release          :: target\release\pulserpc.exe
installer\build.cmd            :: dist\PulseRPC-Setup-1.1.0.exe
```

`installer\build.cmd` reads the version from `Cargo.toml`, builds the release
binary and calls `ISCC.exe` (from `%ProgramFiles(x86)%`, `%ProgramFiles%` or
`%LOCALAPPDATA%\Programs\Inno Setup 6`). Install Inno Setup with:

```cmd
winget install -e --id JRSoftware.InnoSetup
```

The application icon is generated by a dependency-free Rust tool:

```cmd
rustc -O tools\make_icon.rs -o tools\make_icon.exe
tools\make_icon.exe            :: rewrites assets\pulserpc.ico + assets\preview.png
```

### Project layout

```
src\
  main.rs        CLI dispatch, one-shot modes, single-instance lock
  cli.rs         argument parser and help text
  winui.rs       native Win32 window, tray icon, settings UI
  config.rs      TOML schema, validation, embedded defaults, hot-reload stamp
  detect.rs      snapshot probes + scoring
  runtime.rs     state machine (switch delay, idle, rate limiting, reload)
  discord.rs     hand-rolled Discord IPC client (pipe, frames, ping/pong)
  render.rs      templates, rotating pools, activity JSON
  text.rs        title cleanup and media heuristics
  install.rs     native install/uninstall, autostart helpers
  log.rs         file + console logger
  sys\           Windows (Toolhelp/EnumWindows) and Unix backends
config\          shipped defaults (also embedded in the binary)
installer\       Inno Setup script and build script
```

## License

MIT — see [LICENSE](LICENSE).
