//! Native Win32 GUI — no Electron, no web view, no extra runtime.
//!
//! The detection/IPC state machine runs on a worker thread; the UI thread only
//! paints a snapshot of it on a 250 ms `WM_TIMER`. That matters because talking
//! to Discord is a blocking pipe write and scanning windows can wait on other
//! processes — neither may ever freeze the window. Closing the window hides it
//! to the tray; Quit really exits.

use crate::config::{self, Config};
use crate::install;
use crate::runtime::{Runtime, Status};
use crate::sys;
use crate::util;
use std::mem::{size_of, zeroed};
use std::path::{Path, PathBuf};
use std::sync::mpsc::{self, Receiver, RecvTimeoutError, Sender};
use std::sync::{Arc, Mutex};
use std::thread::{self, JoinHandle};
use std::time::{Duration, Instant};

use windows_sys::Win32::Foundation::{COLORREF, HINSTANCE, HWND, LPARAM, LRESULT, POINT, RECT, WPARAM};
use windows_sys::Win32::Graphics::Gdi::{
    CreateFontW, DeleteObject, GetSysColor, GetSysColorBrush, SetBkColor, SetBkMode, SetTextColor,
    UpdateWindow, CLEARTYPE_QUALITY, COLOR_BTNFACE, COLOR_WINDOW, COLOR_WINDOWTEXT, DEFAULT_CHARSET,
    FW_BOLD, HBRUSH, HDC, HFONT, TRANSPARENT,
};
use windows_sys::Win32::System::LibraryLoader::GetModuleHandleW;
use windows_sys::Win32::UI::HiDpi::GetDpiForWindow;
use windows_sys::Win32::UI::Input::KeyboardAndMouse::SetFocus;
use windows_sys::Win32::UI::Shell::{
    Shell_NotifyIconW, NIF_ICON, NIF_MESSAGE, NIF_TIP, NIM_ADD, NIM_DELETE, NIM_MODIFY,
    NOTIFYICONDATAW,
};
use windows_sys::Win32::UI::WindowsAndMessaging::*;

const VERSION: &str = env!("CARGO_PKG_VERSION");

const CLASS_NAME: &str = "PulseRPC.MainWindow";
const RULES_CLASS: &str = "PulseRPC.RulesWindow";
const WINDOW_TITLE: &str = "PulseRPC";

const CLIENT_W: i32 = 860;
const CLIENT_H: i32 = 640;

const TIMER_UI: usize = 1;
const TRAY_ID: u32 = 1;
const WM_TRAY: u32 = WM_APP + 1;
const WM_RULES_CLOSED: u32 = WM_APP + 2;

// Colours are 0x00BBGGRR.
const CLR_GREEN: COLORREF = 0x003a8e1e;
const CLR_AMBER: COLORREF = 0x00006ab4;
const CLR_GRAY: COLORREF = 0x00808080;
const CLR_TEXT: COLORREF = 0x00202020;

// Control ids.
const ID_STATUS: i32 = 101;
const ID_DETAIL: i32 = 102;
const ID_DETAIL2: i32 = 103;
const ID_PAUSE: i32 = 104;
const ID_ID: i32 = 110;
const ID_SAVE: i32 = 111;
const ID_NOTE: i32 = 112;
const ID_HINT: i32 = 113;
const ID_AUTOSTART: i32 = 120;
const ID_ELAPSED: i32 = 121;
const ID_PRIVATE: i32 = 122;
const ID_IDLE: i32 = 130;
const ID_ROTATE: i32 = 131;
const ID_APPLY: i32 = 132;
const ID_OPENCFG: i32 = 140;
const ID_RULES: i32 = 141;
const ID_LOG: i32 = 142;
const ID_RELOAD: i32 = 143;
const ID_INFO: i32 = 144;
const ID_QUIT: i32 = 150;
const ID_FOOTER: i32 = 151;

const ID_TRAY_OPEN: i32 = 201;
const ID_TRAY_PAUSE: i32 = 202;
const ID_TRAY_CONFIG: i32 = 203;
const ID_TRAY_RULES: i32 = 204;
const ID_TRAY_QUIT: i32 = 205;

fn wide(s: &str) -> Vec<u16> {
    s.encode_utf16().chain(std::iter::once(0)).collect()
}

fn set_text(hwnd: HWND, text: &str) {
    if hwnd.is_null() {
        return;
    }
    let text = wide(text);
    unsafe { SetWindowTextW(hwnd, text.as_ptr()) };
}

fn get_text(hwnd: HWND) -> String {
    if hwnd.is_null() {
        return String::new();
    }
    unsafe {
        let len = GetWindowTextLengthW(hwnd);
        if len <= 0 {
            return String::new();
        }
        let mut buf = vec![0u16; len as usize + 1];
        let copied = GetWindowTextW(hwnd, buf.as_mut_ptr(), buf.len() as i32);
        String::from_utf16_lossy(&buf[..copied.max(0) as usize])
            .trim()
            .to_string()
    }
}

fn set_checked(hwnd: HWND, checked: bool) {
    let state = if checked { 1usize } else { 0usize }; // BST_CHECKED / BST_UNCHECKED
    unsafe { SendMessageW(hwnd, 241, state, 0) }; // BM_SETCHECK
}

fn is_checked(hwnd: HWND) -> bool {
    unsafe { SendMessageW(hwnd, 240, 0, 0) == 1 } // BM_GETCHECK / BST_CHECKED
}

#[derive(Default)]
struct Fonts {
    title: HFONT,
    status: HFONT,
    body: HFONT,
    small: HFONT,
}

impl Fonts {
    fn build(dpi: u32) -> Fonts {
        Fonts {
            title: make_font(dpi, 19.0, true),
            status: make_font(dpi, 13.0, true),
            body: make_font(dpi, 9.5, false),
            small: make_font(dpi, 8.5, false),
        }
    }

    fn delete(&mut self) {
        for font in [self.title, self.status, self.body, self.small] {
            if !font.is_null() {
                unsafe { DeleteObject(font) };
            }
        }
        *self = Fonts::default();
    }
}

fn make_font(dpi: u32, points: f32, bold: bool) -> HFONT {
    let height = -((points * dpi as f32 / 72.0).round() as i32);
    let face = wide("Segoe UI");
    unsafe {
        CreateFontW(
            height,
            0,
            0,
            0,
            if bold { FW_BOLD as i32 } else { 400 },
            0,
            0,
            0,
            DEFAULT_CHARSET as u32,
            0,
            0,
            CLEARTYPE_QUALITY as u32,
            0,
            face.as_ptr(),
        )
    }
}

/// Handles for every control, so DPI changes can reposition them.
struct Controls {
    title: HWND,
    subtitle: HWND,
    version: HWND,
    discord_group: HWND,
    activity_group: HWND,
    status: HWND,
    detail: HWND,
    detail2: HWND,
    pause: HWND,
    app_label: HWND,
    id: HWND,
    save: HWND,
    note: HWND,
    hint: HWND,
    autostart: HWND,
    elapsed: HWND,
    private: HWND,
    idle_label: HWND,
    idle: HWND,
    rotate_label: HWND,
    rotate: HWND,
    apply: HWND,
    info: HWND,
    open_config: HWND,
    rules: HWND,
    log: HWND,
    reload: HWND,
    footer: HWND,
    quit: HWND,
}

impl Default for Controls {
    fn default() -> Self {
        Controls {
            title: std::ptr::null_mut(),
            subtitle: std::ptr::null_mut(),
            version: std::ptr::null_mut(),
            discord_group: std::ptr::null_mut(),
            activity_group: std::ptr::null_mut(),
            status: std::ptr::null_mut(),
            detail: std::ptr::null_mut(),
            detail2: std::ptr::null_mut(),
            pause: std::ptr::null_mut(),
            app_label: std::ptr::null_mut(),
            id: std::ptr::null_mut(),
            save: std::ptr::null_mut(),
            note: std::ptr::null_mut(),
            hint: std::ptr::null_mut(),
            autostart: std::ptr::null_mut(),
            elapsed: std::ptr::null_mut(),
            private: std::ptr::null_mut(),
            idle_label: std::ptr::null_mut(),
            idle: std::ptr::null_mut(),
            rotate_label: std::ptr::null_mut(),
            rotate: std::ptr::null_mut(),
            apply: std::ptr::null_mut(),
            info: std::ptr::null_mut(),
            open_config: std::ptr::null_mut(),
            rules: std::ptr::null_mut(),
            log: std::ptr::null_mut(),
            reload: std::ptr::null_mut(),
            footer: std::ptr::null_mut(),
            quit: std::ptr::null_mut(),
        }
    }
}

/// Commands from the UI thread to the runtime worker.
enum Command {
    Pause(bool),
    Reload,
    Shutdown,
}

/// A cheap, cloneable picture of the runtime, published by the worker and read
/// by the UI. Nothing here needs a lock longer than a struct copy.
#[derive(Clone, Default)]
struct View {
    status: Status,
    client_id: String,
    idle_minutes: u64,
    rotate_secs: u64,
    show_elapsed: bool,
    hide_private: bool,
    generation: u64,
    rules: usize,
    categories: usize,
    idle_enabled: bool,
    /// Pre-rendered rule list; only rebuilt when the config changes.
    rules_text: String,
}

fn publish(view: &Arc<Mutex<View>>, runtime: &Runtime) {
    let cfg = runtime.config();
    let mut categories: Vec<&str> = cfg.rules.iter().map(|r| r.category.as_str()).collect();
    categories.sort_unstable();
    categories.dedup();
    let next = View {
        status: runtime.status(),
        client_id: cfg.presence.client_id.clone(),
        idle_minutes: (cfg.detection.idle_after_secs + 30) / 60,
        rotate_secs: cfg.presence.rotate_secs,
        show_elapsed: cfg.presence.show_elapsed,
        hide_private: cfg.privacy.hide_when_private,
        generation: runtime.generation(),
        rules: cfg.rules.len(),
        categories: categories.len(),
        idle_enabled: cfg.idle.enabled,
        rules_text: String::new(),
    };
    let generation = next.generation;
    if let Ok(mut slot) = view.lock() {
        let rebuild = slot.generation != generation;
        *slot = next;
        if rebuild {
            slot.rules_text = rules_text(cfg);
        }
    }
}

/// Owns the runtime for the whole session. Blocking work (Discord pipe,
/// process/window scans) happens here so the window always responds.
fn spawn_worker(
    mut runtime: Runtime,
    view: Arc<Mutex<View>>,
    rx: Receiver<Command>,
) -> Option<JoinHandle<()>> {
    thread::Builder::new()
        .name("pulserpc-core".to_string())
        .spawn(move || {
            publish(&view, &runtime);
            'outer: loop {
                runtime.tick();
                publish(&view, &runtime);
                loop {
                    match rx.recv_timeout(Duration::from_millis(250)) {
                        Ok(Command::Pause(paused)) => runtime.set_paused(paused),
                        Ok(Command::Reload) => runtime.reload_now(),
                        Ok(Command::Shutdown) | Err(RecvTimeoutError::Disconnected) => {
                            runtime.shutdown();
                            publish(&view, &runtime);
                            break 'outer;
                        }
                        Err(RecvTimeoutError::Timeout) => break,
                    }
                }
            }
        })
        .ok()
}

pub struct Gui {
    view: Arc<Mutex<View>>,
    commands: Sender<Command>,
    worker: Option<JoinHandle<()>>,
    config_dir: PathBuf,
    hwnd: HWND,
    hinst: HINSTANCE,
    hicon: HICON,
    dpi: u32,
    fonts: Fonts,
    controls: Controls,
    status_color: COLORREF,
    tray_added: bool,
    informed_tray: bool,
    last_head: String,
    last_detail: String,
    last_detail2: String,
    last_info: String,
    last_tip: String,
    note_until: Option<Instant>,
    rules_hwnd: HWND,
    rules_edit: HWND,
    rules_text: String,
    rules_gen: u64,
    pause_opt: Option<bool>,
    quitting: bool,
}

impl Gui {
    fn new(runtime: Runtime, config_dir: PathBuf, hinst: HINSTANCE, hicon: HICON) -> Gui {
        let view = Arc::new(Mutex::new(View {
            generation: u64::MAX,
            ..View::default()
        }));
        let (commands, rx) = mpsc::channel();
        let worker = spawn_worker(runtime, view.clone(), rx);
        Gui {
            view,
            commands,
            worker,
            config_dir,
            hwnd: std::ptr::null_mut(),
            hinst,
            hicon,
            dpi: 96,
            fonts: Fonts::default(),
            controls: Controls::default(),
            status_color: CLR_GRAY,
            tray_added: false,
            informed_tray: false,
            last_head: String::new(),
            last_detail: String::new(),
            last_detail2: String::new(),
            last_info: String::new(),
            last_tip: String::new(),
            note_until: None,
            rules_hwnd: std::ptr::null_mut(),
            rules_edit: std::ptr::null_mut(),
            rules_text: String::new(),
            rules_gen: u64::MAX,
            pause_opt: None,
            quitting: false,
        }
    }

    fn snapshot(&self) -> View {
        self.view
            .lock()
            .map(|v| v.clone())
            .unwrap_or_default()
    }

    fn send(&self, command: Command) {
        let _ = self.commands.send(command);
    }

    fn px(&self, value: i32) -> i32 {
        (value as f32 * self.dpi as f32 / 96.0).round() as i32
    }

    unsafe fn make(&self, class: &str, text: &str, style: u32, ex: u32, id: i32, font: HFONT) -> HWND {
        let class = wide(class);
        let text = wide(text);
        let hwnd = CreateWindowExW(
            ex,
            class.as_ptr(),
            text.as_ptr(),
            WS_CHILD | WS_VISIBLE | style,
            0,
            0,
            0,
            0,
            self.hwnd,
            id as usize as HMENU,
            self.hinst,
            std::ptr::null(),
        );
        if !hwnd.is_null() && !font.is_null() {
            SendMessageW(hwnd, WM_SETFONT, font as usize, 1);
        }
        hwnd
    }

    unsafe fn create_controls(&mut self) {
        self.dpi = GetDpiForWindow(self.hwnd).max(96);
        self.fonts = Fonts::build(self.dpi);

        let f = self.fonts.clone_refs();
        let btn = (WS_TABSTOP | 0) as u32; // BS_PUSHBUTTON
        let chk = (WS_TABSTOP | 3) as u32; // BS_AUTOCHECKBOX
        let group = BS_GROUPBOX as u32;
        let static_style = 0u32; // SS_LEFT | SS_NOPREFIX
        let static_right = 2u32 | 0x80; // SS_RIGHT | SS_NOPREFIX
        let edit_style = (WS_TABSTOP | ES_AUTOHSCROLL as u32) as u32;
        let num_style = (WS_TABSTOP | ES_AUTOHSCROLL as u32 | ES_NUMBER as u32) as u32;
        let info_style = (ES_MULTILINE as u32
            | ES_READONLY as u32
            | WS_VSCROLL
            | WS_TABSTOP) as u32;

        self.controls.title = self.make("STATIC", "PulseRPC", static_style, 0, 0, f.title);
        self.controls.subtitle = self.make(
            "STATIC",
            "Automatic Discord Rich Presence",
            static_style,
            0,
            0,
            f.small,
        );
        self.controls.version = self.make(
            "STATIC",
            &format!("version {VERSION}"),
            static_right,
            0,
            0,
            f.small,
        );

        self.controls.discord_group = self.make("BUTTON", "Discord", group, 0, 0, f.body);
        self.controls.activity_group = self.make("BUTTON", "Activity", group, 0, 0, f.body);

        self.controls.status = self.make("STATIC", "Starting…", static_style, 0, ID_STATUS, f.status);
        self.controls.detail = self.make("STATIC", "", static_style, 0, ID_DETAIL, f.body);
        self.controls.detail2 = self.make("STATIC", "", static_style, 0, ID_DETAIL2, f.small);
        self.controls.pause = self.make("BUTTON", "Pause", btn, 0, ID_PAUSE, f.body);

        self.controls.app_label =
            self.make("STATIC", "Discord Application ID", static_style, 0, 0, f.body);
        self.controls.id = self.make(
            "EDIT",
            "",
            edit_style,
            WS_EX_CLIENTEDGE,
            ID_ID,
            f.body,
        );
        self.controls.save = self.make("BUTTON", "Save", btn, 0, ID_SAVE, f.body);
        self.controls.note = self.make("STATIC", "", static_style, 0, ID_NOTE, f.small);
        self.controls.hint = self.make(
            "STATIC",
            "Create one at discord.com/developers/applications",
            static_style,
            0,
            ID_HINT,
            f.small,
        );

        self.controls.autostart =
            self.make("BUTTON", "Start with Windows", chk, 0, ID_AUTOSTART, f.body);
        self.controls.elapsed =
            self.make("BUTTON", "Show elapsed time", chk, 0, ID_ELAPSED, f.body);
        self.controls.private =
            self.make("BUTTON", "Hide private / incognito windows", chk, 0, ID_PRIVATE, f.body);

        self.controls.idle_label =
            self.make("STATIC", "Idle after (minutes, 0 = off)", static_style, 0, 0, f.body);
        self.controls.idle = self.make("EDIT", "", num_style, WS_EX_CLIENTEDGE, ID_IDLE, f.body);
        self.controls.rotate_label =
            self.make("STATIC", "Rotate status every (sec)", static_style, 0, 0, f.body);
        self.controls.rotate =
            self.make("EDIT", "", num_style, WS_EX_CLIENTEDGE, ID_ROTATE, f.body);
        self.controls.apply = self.make("BUTTON", "Apply", btn, 0, ID_APPLY, f.body);

        self.controls.info = self.make(
            "EDIT",
            "",
            info_style,
            WS_EX_CLIENTEDGE,
            ID_INFO,
            f.body,
        );
        self.controls.open_config =
            self.make("BUTTON", "Open config", btn, 0, ID_OPENCFG, f.body);
        self.controls.rules = self.make("BUTTON", "Rules", btn, 0, ID_RULES, f.body);
        self.controls.log = self.make("BUTTON", "Log", btn, 0, ID_LOG, f.body);
        self.controls.reload = self.make("BUTTON", "Reload", btn, 0, ID_RELOAD, f.body);

        self.controls.footer = self.make(
            "STATIC",
            &format!("Config: {}", self.config_dir.display()),
            static_style,
            0,
            ID_FOOTER,
            f.small,
        );
        self.controls.quit = self.make("BUTTON", "Quit", btn, 0, ID_QUIT, f.body);

        // Fixed-size window: scale the client area for the current DPI and
        // centre it on the primary screen.
        let style = WS_OVERLAPPED | WS_CAPTION | WS_SYSMENU | WS_MINIMIZEBOX;
        let mut rect = RECT {
            left: 0,
            top: 0,
            right: self.px(CLIENT_W),
            bottom: self.px(CLIENT_H),
        };
        AdjustWindowRectEx(&mut rect, style, 0, 0);
        let (w, h) = (rect.right - rect.left, rect.bottom - rect.top);
        let sw = GetSystemMetrics(SM_CXSCREEN);
        let sh = GetSystemMetrics(SM_CYSCREEN);
        let (x, y) = ((sw - w) / 2, (sh - h) / 2);
        SetWindowPos(self.hwnd, std::ptr::null_mut(), x, y, w, h, 4 | 16);

        self.layout();
        self.refresh_controls();
    }

    fn layout(&self) {
        let c = &self.controls;
        let items: [(HWND, i32, i32, i32, i32); 29] = [
            (c.title, 24, 16, 500, 34),
            (c.subtitle, 24, 50, 500, 20),
            (c.version, 600, 22, 236, 18),
            (c.discord_group, 24, 216, 396, 330),
            (c.activity_group, 436, 216, 400, 330),
            (c.status, 44, 96, 640, 28),
            (c.pause, 708, 94, 108, 30),
            (c.detail, 44, 128, 768, 20),
            (c.detail2, 44, 150, 768, 18),
            (c.app_label, 44, 238, 340, 18),
            (c.id, 44, 258, 272, 25),
            (c.save, 324, 258, 76, 25),
            (c.note, 44, 288, 340, 18),
            (c.hint, 44, 306, 340, 18),
            (c.autostart, 44, 334, 340, 22),
            (c.elapsed, 44, 360, 340, 22),
            (c.private, 44, 386, 340, 22),
            (c.idle_label, 44, 422, 196, 18),
            (c.idle, 248, 418, 64, 24),
            (c.rotate_label, 44, 452, 196, 18),
            (c.rotate, 248, 448, 64, 24),
            (c.apply, 44, 484, 110, 30),
            (c.info, 456, 238, 360, 252),
            (c.open_config, 456, 502, 110, 28),
            (c.rules, 574, 502, 74, 28),
            (c.log, 656, 502, 64, 28),
            (c.reload, 728, 502, 88, 28),
            (c.footer, 24, 566, 690, 20),
            (c.quit, 736, 560, 100, 32),
        ];
        for (hwnd, x, y, w, h) in items {
            if hwnd.is_null() {
                continue;
            }
            unsafe {
                MoveWindow(
                    hwnd,
                    self.px(x),
                    self.px(y),
                    self.px(w),
                    self.px(h),
                    1,
                );
            }
        }
    }

    unsafe fn apply_fonts(&self) {
        let f = self.fonts.clone_refs();
        let c = &self.controls;
        for (hwnd, font) in [
            (c.title, f.title),
            (c.subtitle, f.small),
            (c.version, f.small),
            (c.discord_group, f.body),
            (c.activity_group, f.body),
            (c.status, f.status),
            (c.detail, f.body),
            (c.detail2, f.small),
            (c.pause, f.body),
            (c.app_label, f.body),
            (c.id, f.body),
            (c.save, f.body),
            (c.note, f.small),
            (c.hint, f.small),
            (c.autostart, f.body),
            (c.elapsed, f.body),
            (c.private, f.body),
            (c.idle_label, f.body),
            (c.idle, f.body),
            (c.rotate_label, f.body),
            (c.rotate, f.body),
            (c.apply, f.body),
            (c.info, f.body),
            (c.open_config, f.body),
            (c.rules, f.body),
            (c.log, f.body),
            (c.reload, f.body),
            (c.footer, f.small),
            (c.quit, f.body),
        ] {
            if !hwnd.is_null() {
                SendMessageW(hwnd, WM_SETFONT, font as usize, 1);
            }
        }
    }

    fn refresh_controls(&mut self) {
        let view = self.snapshot();
        set_text(self.controls.id, &view.client_id);
        set_text(self.controls.idle, &view.idle_minutes.to_string());
        set_text(self.controls.rotate, &view.rotate_secs.to_string());
        set_checked(self.controls.elapsed, view.show_elapsed);
        set_checked(self.controls.private, view.hide_private);
        set_checked(self.controls.autostart, install::autostart_enabled());
        set_text(
            self.controls.pause,
            if view.status.paused { "Resume" } else { "Pause" },
        );
        // Force the next tick to repaint status text.
        self.last_head.clear();
        self.last_detail.clear();
        self.last_detail2.clear();
        self.last_info.clear();
    }

    fn show_note(&mut self, text: &str) {
        set_text(self.controls.note, text);
        self.note_until = Some(Instant::now() + Duration::from_secs(6));
    }

    fn tick(&mut self) {
        if let Some(until) = self.note_until {
            if Instant::now() >= until {
                set_text(self.controls.note, "");
                self.note_until = None;
            }
        }

        let view = self.snapshot();
        if view.generation != self.rules_gen {
            self.rules_gen = view.generation;
            self.rules_text = view.rules_text.clone();
        }
        // A queued pause/resume wins until the worker confirms it.
        let paused = match self.pause_opt {
            Some(want) if want == view.status.paused => {
                self.pause_opt = None;
                want
            }
            Some(want) => want,
            None => view.status.paused,
        };
        set_text(
            self.controls.pause,
            if paused { "Resume" } else { "Pause" },
        );
        self.render_status(&view);
    }

    fn render_status(&mut self, view: &View) {
        let st = &view.status;
        let (head, color, line1, line2) = if st.paused {
            (
                "Presence paused".to_string(),
                CLR_GRAY,
                "PulseRPC is not sharing anything right now.".to_string(),
                "Press Resume to start sharing again.".to_string(),
            )
        } else if st.dry_run {
            (
                "Dry run — nothing is sent".to_string(),
                CLR_AMBER,
                presence_text(st),
                foreground_text(st),
            )
        } else if !st.has_id {
            (
                "Discord Application ID needed".to_string(),
                CLR_AMBER,
                "Paste your 17–20 digit Application ID below and press Save.".to_string(),
                "Create one at discord.com/developers/applications".to_string(),
            )
        } else if !st.connected {
            (
                "Waiting for Discord".to_string(),
                CLR_AMBER,
                "Start or restart Discord — PulseRPC reconnects automatically.".to_string(),
                presence_text(st),
            )
        } else {
            let head = if st.label.is_empty() {
                "Connected to Discord".to_string()
            } else {
                format!("Connected — {}", util::truncate(&st.label, 34))
            };
            (head, CLR_GREEN, presence_text(st), foreground_text(st))
        };

        self.status_color = color;
        if self.last_head != head {
            set_text(self.controls.status, &head);
            self.last_head = head;
        }
        if self.last_detail != line1 {
            set_text(self.controls.detail, &line1);
            self.last_detail = line1;
        }
        if self.last_detail2 != line2 {
            set_text(self.controls.detail2, &line2);
            self.last_detail2 = line2;
        }

        let info = self.info_text(view);
        if self.last_info != info {
            set_text(self.controls.info, &info);
            self.last_info = info;
        }

        self.update_tray_tip(st);
    }

    fn info_text(&self, view: &View) -> String {
        let st = &view.status;
        // Idle is only "on" when the global switch and the threshold agree; the
        // UI already knows both through the published view.
        let idle_on = view.idle_enabled && view.idle_minutes > 0;
        let discord = if st.paused {
            "paused".to_string()
        } else if st.dry_run {
            "dry run".to_string()
        } else if !st.has_id {
            "application id missing".to_string()
        } else if st.connected {
            "connected".to_string()
        } else {
            "waiting for Discord".to_string()
        };

        let app = if st.label.is_empty() {
            "—".to_string()
        } else {
            util::truncate(&st.label, 34)
        };
        let presence = util::truncate(&presence_text(st), 40);
        let foreground = if st.foreground_process.is_empty() {
            "—".to_string()
        } else {
            util::truncate(
                &format!("{}  {}", st.foreground_process, st.foreground_title),
                40,
            )
        };
        let idle = if idle_on {
            format!("{} idle", util::human_elapsed(st.idle_secs))
        } else {
            "disabled".to_string()
        };

        format!(
            "PRESENCE\r\n  {presence}\r\n  App        {app}\r\n\
             FOREGROUND\r\n  {foreground}\r\n\
             SYSTEM\r\n  Discord    {discord}\r\n  Updates    {}\r\n  Idle       {idle}\r\n  Rules      {} / {} categories\r\n\
             FILES\r\n  Config     {}\r\n  Log        {}",
            st.sends,
            view.rules,
            view.categories,
            short_path(&self.config_dir, 38),
            short_path(&config::log_path(), 38),
        )
    }

    fn update_tray_tip(&mut self, st: &Status) {
        if !self.tray_added {
            return;
        }
        let state = if st.paused {
            "paused".to_string()
        } else if !st.has_id {
            "application id missing".to_string()
        } else if st.connected {
            if st.label.is_empty() {
                "connected".to_string()
            } else {
                util::truncate(&st.label, 40)
            }
        } else {
            "waiting for Discord".to_string()
        };
        let tip = format!("PulseRPC — {state}");
        if tip == self.last_tip {
            return;
        }
        self.last_tip = tip.clone();
        unsafe {
            let mut nid: NOTIFYICONDATAW = zeroed();
            nid.cbSize = size_of::<NOTIFYICONDATAW>() as u32;
            nid.hWnd = self.hwnd;
            nid.uID = TRAY_ID;
            nid.uFlags = NIF_TIP;
            copy_wide(&tip, &mut nid.szTip);
            Shell_NotifyIconW(NIM_MODIFY, &nid);
        }
    }

    unsafe fn add_tray_icon(&mut self) {
        let mut nid: NOTIFYICONDATAW = zeroed();
        nid.cbSize = size_of::<NOTIFYICONDATAW>() as u32;
        nid.hWnd = self.hwnd;
        nid.uID = TRAY_ID;
        nid.uFlags = NIF_MESSAGE | NIF_ICON | NIF_TIP;
        nid.uCallbackMessage = WM_TRAY;
        nid.hIcon = self.hicon;
        copy_wide("PulseRPC", &mut nid.szTip);
        self.tray_added = Shell_NotifyIconW(NIM_ADD, &nid) != 0;
    }

    unsafe fn remove_tray_icon(&mut self) {
        if !self.tray_added {
            return;
        }
        let mut nid: NOTIFYICONDATAW = zeroed();
        nid.cbSize = size_of::<NOTIFYICONDATAW>() as u32;
        nid.hWnd = self.hwnd;
        nid.uID = TRAY_ID;
        Shell_NotifyIconW(NIM_DELETE, &nid);
        self.tray_added = false;
    }

    unsafe fn balloon(&mut self, title: &str, text: &str) {
        if !self.tray_added {
            return;
        }
        let mut nid: NOTIFYICONDATAW = zeroed();
        nid.cbSize = size_of::<NOTIFYICONDATAW>() as u32;
        nid.hWnd = self.hwnd;
        nid.uID = TRAY_ID;
        nid.uFlags = 0x10; // NIF_INFO
        nid.dwInfoFlags = 1; // NIIF_INFO
        copy_wide(title, &mut nid.szInfoTitle);
        copy_wide(text, &mut nid.szInfo);
        Shell_NotifyIconW(NIM_MODIFY, &nid);
    }

    fn show_window(&self) {
        unsafe {
            ShowWindow(self.hwnd, SW_SHOW);
            if IsIconic(self.hwnd) != 0 {
                ShowWindow(self.hwnd, SW_RESTORE);
            }
            SetForegroundWindow(self.hwnd);
        }
    }

    unsafe fn hide_window(&mut self) {
        ShowWindow(self.hwnd, SW_HIDE);
        if !self.informed_tray {
            self.informed_tray = true;
            self.balloon(
                "PulseRPC is still running",
                "Right-click the tray icon to open it again or quit.",
            );
        }
    }

    unsafe fn tray_menu(&mut self) {
        let menu = CreatePopupMenu();
        if menu.is_null() {
            return;
        }
        let open = wide("Open PulseRPC");
        AppendMenuW(menu, MF_STRING, ID_TRAY_OPEN as usize, open.as_ptr());
        let paused = self.snapshot().status.paused;
        let pause_label = if paused {
            wide("Resume presence")
        } else {
            wide("Pause presence")
        };
        AppendMenuW(menu, MF_STRING, ID_TRAY_PAUSE as usize, pause_label.as_ptr());
        AppendMenuW(menu, MF_SEPARATOR, 0, std::ptr::null());
        let config = wide("Open config folder");
        AppendMenuW(menu, MF_STRING, ID_TRAY_CONFIG as usize, config.as_ptr());
        let rules = wide("View rules");
        AppendMenuW(menu, MF_STRING, ID_TRAY_RULES as usize, rules.as_ptr());
        AppendMenuW(menu, MF_SEPARATOR, 0, std::ptr::null());
        let quit = wide("Quit PulseRPC");
        AppendMenuW(menu, MF_STRING, ID_TRAY_QUIT as usize, quit.as_ptr());

        let mut point = POINT { x: 0, y: 0 };
        GetCursorPos(&mut point);
        SetForegroundWindow(self.hwnd);
        let command = TrackPopupMenu(
            menu,
            TPM_RETURNCMD | TPM_NONOTIFY | TPM_RIGHTBUTTON,
            point.x,
            point.y,
            0,
            self.hwnd,
            std::ptr::null(),
        );
        DestroyMenu(menu);
        PostMessageW(self.hwnd, WM_NULL, 0, 0);

        match command as i32 {
            ID_TRAY_OPEN => self.show_window(),
            ID_TRAY_PAUSE => self.toggle_pause(),
            ID_TRAY_CONFIG => {
                open_path_async(self.config_dir.clone());
            }
            ID_TRAY_RULES => {
                open_path_async(self.config_dir.join("apps"));
            }
            ID_TRAY_QUIT => self.quit(),
            _ => {}
        }
    }

    fn toggle_pause(&mut self) {
        let paused = self.snapshot().status.paused;
        self.send(Command::Pause(!paused));
        // Optimistic label until the worker's next snapshot confirms it.
        self.pause_opt = Some(!paused);
        set_text(
            self.controls.pause,
            if paused { "Pause" } else { "Resume" },
        );
        self.last_head.clear();
        self.last_detail.clear();
        self.last_detail2.clear();
        self.last_info.clear();
        self.show_note(if paused {
            "Presence resumed"
        } else {
            "Presence paused"
        });
    }

    fn save_id(&mut self) {
        let id = get_text(self.controls.id);
        if !config::client_id_looks_valid(&id) {
            sys::message_box(
                "PulseRPC",
                "A Discord Application ID has 17 to 20 digits.\n\n\
                 Open discord.com/developers/applications, choose your\n\
                 application, and copy the Application ID from\n\
                 General Information.\n\n\
                 This is not a bot token or your Discord user id.",
                true,
            );
            unsafe { SetFocus(self.controls.id) };
            return;
        }
        match config::set_client_id(&self.config_dir, &id) {
            Ok(_) => {
                set_text(self.controls.id, &id);
                self.send(Command::Reload);
                self.show_note("Application ID saved — connecting…");
                self.last_info.clear();
                self.last_head.clear();
            }
            Err(e) => sys::message_box("PulseRPC", &e, true),
        }
    }

    fn apply_settings(&mut self) {
        let idle_text = get_text(self.controls.idle);
        let rotate_text = get_text(self.controls.rotate);
        let idle_minutes: u64 = match idle_text.parse() {
            Ok(v) if v <= 1_440 => v,
            _ => {
                sys::message_box(
                    "PulseRPC",
                    "Idle time must be a whole number of minutes from 0 to 1440.\n\
                     0 turns idle detection off.",
                    true,
                );
                return;
            }
        };
        let rotate: u64 = match rotate_text.parse() {
            Ok(v) if v == 0 || (15..=3_600).contains(&v) => v,
            _ => {
                sys::message_box(
                    "PulseRPC",
                    "Rotation must be 0 (off) or between 15 and 3600 seconds.",
                    true,
                );
                return;
            }
        };

        let saved = (|| -> Result<(), String> {
            config::set_config_value(
                &self.config_dir,
                "detection",
                "idle_after_secs",
                &(idle_minutes * 60).to_string(),
            )?;
            config::set_config_value(
                &self.config_dir,
                "presence",
                "rotate_secs",
                &rotate.to_string(),
            )?;
            Ok(())
        })();

        match saved {
            Ok(()) => {
                self.send(Command::Reload);
                self.show_note("Settings saved");
                self.last_info.clear();
                self.last_head.clear();
            }
            Err(e) => sys::message_box("PulseRPC", &e, true),
        }
    }

    fn toggle_setting(&mut self, section: &str, key: &str, enabled: bool) {
        let value = if enabled { "true" } else { "false" };
        match config::set_config_value(&self.config_dir, section, key, value) {
            Ok(_) => {
                self.send(Command::Reload);
                self.show_note("Setting saved");
                self.last_info.clear();
            }
            Err(e) => {
                sys::message_box("PulseRPC", &e, true);
                self.refresh_controls();
            }
        }
    }

    fn toggle_autostart(&mut self) {
        let want = is_checked(self.controls.autostart);
        match install::set_autostart(want) {
            Ok(()) => self.show_note(if want {
                "PulseRPC will start with Windows"
            } else {
                "Autostart disabled"
            }),
            Err(e) => {
                sys::message_box("PulseRPC", &e, true);
                set_checked(self.controls.autostart, !want);
            }
        }
    }

    unsafe fn open_log(&mut self) {
        let path = config::log_path();
        if let Some(parent) = path.parent() {
            let _ = std::fs::create_dir_all(parent);
        }
        if !path.exists() {
            let _ = std::fs::write(&path, b"");
        }
        open_log_async(path);
    }

    unsafe fn show_rules(&mut self) {
        if !self.rules_hwnd.is_null() {
            ShowWindow(self.rules_hwnd, SW_SHOW);
            SetForegroundWindow(self.rules_hwnd);
            return;
        }

        let class = wide(RULES_CLASS);
        let mut wc: WNDCLASSW = zeroed();
        wc.style = 3; // CS_HREDRAW | CS_VREDRAW
        wc.lpfnWndProc = Some(rules_proc);
        wc.hInstance = self.hinst;
        wc.hIcon = self.hicon;
        wc.hCursor = LoadCursorW(std::ptr::null_mut(), IDC_ARROW);
        wc.hbrBackground = (COLOR_BTNFACE as usize + 1) as HBRUSH;
        wc.lpszClassName = class.as_ptr();
        RegisterClassW(&wc);

        let text = self.rules_text.clone();
        let text = if text.is_empty() {
            "Rules are loading…".to_string()
        } else {
            text
        };
        let text = wide(&text.replace('\n', "\r\n"));

        let mut rect: RECT = zeroed();
        GetWindowRect(self.hwnd, &mut rect);
        let w = self.px(720);
        let h = self.px(620);
        let x = rect.left + ((rect.right - rect.left) - w) / 2;
        let y = rect.top + ((rect.bottom - rect.top) - h) / 2;

        let title = wide(&format!("PulseRPC — Rules ({})", VERSION));
        let hwnd = CreateWindowExW(
            0,
            class.as_ptr(),
            title.as_ptr(),
            WS_OVERLAPPED | WS_CAPTION | WS_SYSMENU | WS_MINIMIZEBOX | WS_VISIBLE,
            x,
            y,
            w,
            h,
            self.hwnd,
            std::ptr::null_mut(),
            self.hinst,
            std::ptr::null(),
        );
        if hwnd.is_null() {
            return;
        }
        self.rules_hwnd = hwnd;

        let mut client: RECT = zeroed();
        GetClientRect(hwnd, &mut client);
        let pad = self.px(12);
        let edit_class = wide("EDIT");
        let info_style = (ES_MULTILINE as u32
            | ES_READONLY as u32
            | WS_CHILD
            | WS_VISIBLE
            | WS_TABSTOP
            | WS_VSCROLL) as u32;
        let edit = CreateWindowExW(
            WS_EX_CLIENTEDGE,
            edit_class.as_ptr(),
            text.as_ptr(),
            info_style,
            pad,
            pad,
            (client.right - pad * 2).max(40),
            (client.bottom - pad * 2).max(40),
            hwnd,
            1 as usize as HMENU,
            self.hinst,
            std::ptr::null(),
        );
        self.rules_edit = edit;
        if !edit.is_null() {
            SendMessageW(edit, WM_SETFONT, self.fonts.body as usize, 1);
            SetWindowLongPtrW(hwnd, GWLP_USERDATA, edit as isize);
        }
        ShowWindow(hwnd, SW_SHOW);
        UpdateWindow(hwnd);
    }

    unsafe fn quit(&mut self) {
        if self.quitting {
            return;
        }
        self.quitting = true;
        if !self.rules_hwnd.is_null() {
            DestroyWindow(self.rules_hwnd);
            self.rules_hwnd = std::ptr::null_mut();
            self.rules_edit = std::ptr::null_mut();
        }
        DestroyWindow(self.hwnd);
    }

    /// Ask the worker to clear the presence and stop, without hanging the
    /// window when Discord is slow to answer.
    fn stop_worker(&mut self) {
        let Some(worker) = self.worker.take() else {
            return;
        };
        let _ = self.commands.send(Command::Shutdown);
        let (done, wait) = mpsc::channel();
        thread::spawn(move || {
            let _ = worker.join();
            let _ = done.send(());
        });
        let _ = wait.recv_timeout(Duration::from_millis(1_500));
    }

    unsafe fn resize_for_dpi(&mut self, dpi: u32, suggested: *const RECT) {
        self.dpi = dpi.max(96);
        self.fonts.delete();
        self.fonts = Fonts::build(self.dpi);
        self.apply_fonts();
        self.layout();
        if !suggested.is_null() {
            let r = *suggested;
            SetWindowPos(
                self.hwnd,
                std::ptr::null_mut(),
                r.left,
                r.top,
                r.right - r.left,
                r.bottom - r.top,
                4 | 16, // NOZORDER | NOACTIVATE
            );
        }
    }
}

impl Fonts {
    fn clone_refs(&self) -> Fonts {
        Fonts {
            title: self.title,
            status: self.status,
            body: self.body,
            small: self.small,
        }
    }
}

impl Drop for Gui {
    fn drop(&mut self) {
        self.stop_worker();
        self.fonts.delete();
    }
}

fn copy_wide(text: &str, target: &mut [u16]) {
    let mut i = 0;
    for unit in text.encode_utf16() {
        if i + 1 >= target.len() {
            break;
        }
        target[i] = unit;
        i += 1;
    }
    target[i] = 0;
}

fn presence_text(st: &Status) -> String {
    let mut text = st.details.clone();
    if !st.state.is_empty() {
        if !text.is_empty() {
            text.push_str(" · ");
        }
        text.push_str(&st.state);
    }
    if text.trim().is_empty() {
        return "Waiting for a matching app…".to_string();
    }
    util::truncate(text.trim(), 92)
}

fn foreground_text(st: &Status) -> String {
    if st.foreground_process.is_empty() {
        return "No foreground window detected.".to_string();
    }
    let mut text = st.foreground_process.clone();
    if !st.foreground_title.is_empty() {
        text.push_str(" — ");
        text.push_str(&st.foreground_title);
    }
    util::truncate(&text, 92)
}

/// Open a folder without ever blocking the UI thread. `ShellExecute` can wait
/// on the shell (or on an "open with" prompt), which would freeze the window.
fn open_path_async(path: PathBuf) {
    std::thread::spawn(move || {
        let _ = install::open_in_shell(&path);
    });
}

/// Log files have no default handler on a clean Windows install, and asking
/// the shell would pop a blocking "open with" dialog. Notepad always works.
fn open_log_async(path: PathBuf) {
    std::thread::spawn(move || {
        let opened = std::process::Command::new("notepad.exe")
            .arg(&path)
            .spawn()
            .is_ok();
        if !opened {
            if let Some(parent) = path.parent() {
                let _ = install::open_in_shell(parent);
            }
        }
    });
}

fn short_path(path: &Path, max: usize) -> String {
    let full = path.display().to_string();
    let len = full.chars().count();
    if len <= max {
        return full;
    }
    let skip = len.saturating_sub(max.saturating_sub(2));
    let tail: String = full.chars().skip(skip).collect();
    format!("…{}", tail.trim_start_matches(['\\', '/']))
}

fn rules_text(cfg: &Config) -> String {
    use std::collections::BTreeMap;
    let mut groups: BTreeMap<&str, Vec<&config::Rule>> = BTreeMap::new();
    for rule in &cfg.rules {
        groups.entry(rule.category.as_str()).or_default().push(rule);
    }

    let mut out = String::new();
    out.push_str(&format!(
        "PulseRPC {VERSION} — {} rules in {} categories\n",
        cfg.rules.len(),
        groups.len()
    ));
    out.push_str("Edit config/apps/*.toml while PulseRPC runs — changes apply automatically.\n\n");
    for (category, rules) in groups {
        out.push_str(&format!("[{category}]\n"));
        for rule in rules {
            let mut flags = Vec::new();
            if rule.requires_foreground {
                flags.push("foreground-only");
            }
            if rule.media {
                flags.push("media");
            }
            if rule.fullscreen_only {
                flags.push("fullscreen-only");
            }
            let flags = if flags.is_empty() {
                String::new()
            } else {
                format!("  ({})", flags.join(", "))
            };
            out.push_str(&format!(
                "  {:<22} p{:<4} {}{}\n",
                rule.id,
                rule.priority,
                rule.process_display.join(", "),
                flags
            ));
        }
        out.push('\n');
    }
    out.push_str(&format!(
        "Config folder: {}\n",
        cfg.path
            .parent()
            .map(|p| p.display().to_string())
            .unwrap_or_default()
    ));
    out
}

unsafe fn gui_ptr(hwnd: HWND) -> Option<&'static mut Gui> {
    let ptr = GetWindowLongPtrW(hwnd, GWLP_USERDATA) as *mut Gui;
    if ptr.is_null() {
        None
    } else {
        Some(&mut *ptr)
    }
}

unsafe extern "system" fn wndproc(hwnd: HWND, msg: u32, wparam: WPARAM, lparam: LPARAM) -> LRESULT {
    match msg {
        WM_CREATE => {
            let create = &*(lparam as *const CREATESTRUCTW);
            let gui = create.lpCreateParams as *mut Gui;
            if gui.is_null() {
                return -1;
            }
            SetWindowLongPtrW(hwnd, GWLP_USERDATA, gui as isize);
            (*gui).hwnd = hwnd;
            (*gui).create_controls();
            (*gui).add_tray_icon();
            0
        }
        WM_TIMER => {
            if wparam == TIMER_UI {
                if let Some(gui) = gui_ptr(hwnd) {
                    gui.tick();
                }
            }
            0
        }
        WM_COMMAND => {
            let id = (wparam & 0xffff) as i32;
            let code = ((wparam >> 16) & 0xffff) as u32;
            if let Some(gui) = gui_ptr(hwnd) {
                if id == ID_ID && code == 0x100 {
                    // EN_SETFOCUS: select the whole id so typing replaces it.
                    SendMessageW(lparam as HWND, 0x00B1, 0, -1); // EM_SETSEL
                    return 0;
                }
                if code == 0 {
                    match id {
                        ID_PAUSE => gui.toggle_pause(),
                        ID_SAVE => gui.save_id(),
                        ID_APPLY => gui.apply_settings(),
                        ID_AUTOSTART => gui.toggle_autostart(),
                        ID_ELAPSED => {
                            let on = is_checked(gui.controls.elapsed);
                            gui.toggle_setting("presence", "show_elapsed", on);
                        }
                        ID_PRIVATE => {
                            let on = is_checked(gui.controls.private);
                            gui.toggle_setting("privacy", "hide_when_private", on);
                        }
                        ID_OPENCFG => {
                            open_path_async(gui.config_dir.clone());
                        }
                        ID_RULES => gui.show_rules(),
                        ID_LOG => gui.open_log(),
                        ID_RELOAD => {
                            gui.send(Command::Reload);
                            gui.show_note("Reloading config…");
                            gui.last_info.clear();
                            gui.last_head.clear();
                        }
                        ID_QUIT => gui.quit(),
                        _ => {}
                    }
                }
            }
            0
        }
        WM_CTLCOLORSTATIC => {
            let hdc = wparam as HDC;
            let ctl = lparam as HWND;
            let id = GetDlgCtrlID(ctl);
            if let Some(gui) = gui_ptr(hwnd) {
                if id == ID_INFO {
                    SetBkMode(hdc, 2); // OPAQUE
                    SetBkColor(hdc, GetSysColor(COLOR_WINDOW));
                    SetTextColor(hdc, CLR_TEXT);
                    return GetSysColorBrush(COLOR_WINDOW) as LRESULT;
                }
                SetBkMode(hdc, TRANSPARENT as i32);
                SetTextColor(
                    hdc,
                    match id {
                        ID_STATUS => gui.status_color,
                        ID_DETAIL2 | ID_FOOTER | ID_NOTE | ID_HINT => CLR_GRAY,
                        _ => GetSysColor(COLOR_WINDOWTEXT),
                    },
                );
                return GetSysColorBrush(COLOR_BTNFACE) as LRESULT;
            }
            DefWindowProcW(hwnd, msg, wparam, lparam)
        }
        WM_CLOSE => {
            if let Some(gui) = gui_ptr(hwnd) {
                gui.hide_window();
            }
            0
        }
        WM_TRAY => {
            let event = lparam as u32;
            if let Some(gui) = gui_ptr(hwnd) {
                match event {
                    WM_LBUTTONUP | WM_LBUTTONDBLCLK => gui.show_window(),
                    WM_RBUTTONUP | WM_CONTEXTMENU => gui.tray_menu(),
                    _ => {}
                }
            }
            0
        }
        WM_RULES_CLOSED => {
            if let Some(gui) = gui_ptr(hwnd) {
                gui.rules_hwnd = std::ptr::null_mut();
                gui.rules_edit = std::ptr::null_mut();
            }
            0
        }
        WM_DPICHANGED => {
            let dpi = (wparam & 0xffff) as u32;
            if let Some(gui) = gui_ptr(hwnd) {
                gui.resize_for_dpi(dpi, lparam as *const RECT);
            }
            0
        }
        WM_DESTROY => {
            if let Some(gui) = gui_ptr(hwnd) {
                gui.remove_tray_icon();
                gui.stop_worker();
            }
            SetWindowLongPtrW(hwnd, GWLP_USERDATA, 0);
            PostQuitMessage(0);
            0
        }
        _ => DefWindowProcW(hwnd, msg, wparam, lparam),
    }
}

unsafe extern "system" fn rules_proc(hwnd: HWND, msg: u32, wparam: WPARAM, lparam: LPARAM) -> LRESULT {
    match msg {
        WM_SIZE => {
            // The edit handle lives in GWLP_USERDATA: this window never
            // touches the Gui, so it is safe to create/destroy while the main
            // window procedure holds a mutable borrow.
            let edit = GetWindowLongPtrW(hwnd, GWLP_USERDATA) as HWND;
            if !edit.is_null() {
                let w = (lparam & 0xffff) as i32;
                let h = ((lparam >> 16) & 0xffff) as i32;
                let pad = 12;
                MoveWindow(
                    edit,
                    pad,
                    pad,
                    (w - pad * 2).max(40),
                    (h - pad * 2).max(40),
                    1,
                );
            }
            0
        }
        WM_CLOSE => {
            DestroyWindow(hwnd);
            0
        }
        WM_DESTROY => {
            PostMessageW(GetParent(hwnd), WM_RULES_CLOSED, 0, 0);
            SetWindowLongPtrW(hwnd, GWLP_USERDATA, 0);
            0
        }
        _ => DefWindowProcW(hwnd, msg, wparam, lparam),
    }
}

unsafe fn register_main_class(hinst: HINSTANCE, hicon: HICON) -> Result<(), String> {
    let class = wide(CLASS_NAME);
    let mut wc: WNDCLASSW = zeroed();
    wc.style = 3; // CS_HREDRAW | CS_VREDRAW
    wc.lpfnWndProc = Some(wndproc);
    wc.hInstance = hinst;
    wc.hIcon = hicon;
    wc.hCursor = LoadCursorW(std::ptr::null_mut(), IDC_ARROW);
    wc.hbrBackground = (COLOR_BTNFACE as usize + 1) as HBRUSH;
    wc.lpszClassName = class.as_ptr();
    if RegisterClassW(&wc) == 0 {
        let err = std::io::Error::last_os_error();
        // 1410 = class already registered (second instance in the same
        // process is impossible here, but handle it anyway).
        if err.raw_os_error() != Some(1410) {
            return Err(format!("cannot register window class: {err}"));
        }
    }
    Ok(())
}

/// Run the GUI until the user quits. `start_hidden` is used by `--tray`.
pub fn run(runtime: Runtime, config_dir: PathBuf, start_hidden: bool) -> Result<i32, String> {
    unsafe {
        let hinst = GetModuleHandleW(std::ptr::null());
        if hinst.is_null() {
            return Err("cannot get module handle".to_string());
        }
        let hicon = LoadIconW(hinst, 1 as *const u16);
        let hicon = if hicon.is_null() {
            LoadIconW(std::ptr::null_mut(), IDI_APPLICATION)
        } else {
            hicon
        };
        register_main_class(hinst, hicon)?;

        let gui = Box::new(Gui::new(runtime, config_dir, hinst, hicon));
        let ptr = Box::into_raw(gui);

        let class = wide(CLASS_NAME);
        let title = wide(WINDOW_TITLE);
        let style = WS_OVERLAPPED | WS_CAPTION | WS_SYSMENU | WS_MINIMIZEBOX;
        let hwnd = CreateWindowExW(
            0,
            class.as_ptr(),
            title.as_ptr(),
            style,
            CW_USEDEFAULT,
            CW_USEDEFAULT,
            400,
            300,
            std::ptr::null_mut(),
            std::ptr::null_mut(),
            hinst,
            ptr as *const std::ffi::c_void,
        );
        if hwnd.is_null() {
            drop(Box::from_raw(ptr));
            return Err(format!(
                "cannot create window: {}",
                std::io::Error::last_os_error()
            ));
        }

        SetTimer(hwnd, TIMER_UI, 250, None);

        if !start_hidden {
            ShowWindow(hwnd, SW_SHOW);
            UpdateWindow(hwnd);
            SetForegroundWindow(hwnd);
        }

        let mut msg: MSG = zeroed();
        while GetMessageW(&mut msg, std::ptr::null_mut(), 0, 0) > 0 {
            if IsDialogMessageW(hwnd, &msg) == 0 {
                TranslateMessage(&msg);
                DispatchMessageW(&msg);
            }
        }

        drop(Box::from_raw(ptr));
        Ok(msg.wParam as i32)
    }
}

/// Bring an already-running instance to the foreground.
pub fn activate_existing() {
    unsafe {
        let class = wide(CLASS_NAME);
        let hwnd = FindWindowW(class.as_ptr(), std::ptr::null());
        if hwnd.is_null() {
            return;
        }
        ShowWindow(hwnd, SW_SHOW);
        if IsIconic(hwnd) != 0 {
            ShowWindow(hwnd, SW_RESTORE);
        }
        SetForegroundWindow(hwnd);
    }
}
