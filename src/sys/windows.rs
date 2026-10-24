//! Windows implementation using Toolhelp snapshots + USER32 window enumeration.

use super::{SysSnapshot, WindowInfo};
use std::mem::{size_of, zeroed};
use std::sync::atomic::{AtomicBool, Ordering};

use windows_sys::Win32::Foundation::{
    CloseHandle, BOOL, HANDLE, HWND, INVALID_HANDLE_VALUE, LPARAM, TRUE,
};
use windows_sys::Win32::Graphics::Gdi::{
    GetMonitorInfoW, MonitorFromWindow, MONITORINFO, MONITOR_DEFAULTTONEAREST,
};
use windows_sys::Win32::System::Console::{
    AttachConsole, GetStdHandle, SetConsoleCtrlHandler, SetStdHandle, ATTACH_PARENT_PROCESS,
    STD_ERROR_HANDLE, STD_OUTPUT_HANDLE,
};
use windows_sys::Win32::System::Diagnostics::ToolHelp::{
    CreateToolhelp32Snapshot, Process32FirstW, Process32NextW, PROCESSENTRY32W, TH32CS_SNAPPROCESS,
};
use windows_sys::Win32::System::IO::CancelIoEx;
use windows_sys::Win32::System::SystemInformation::GetTickCount64;
use windows_sys::Win32::System::Threading::{
    OpenProcess, QueryFullProcessImageNameW, PROCESS_QUERY_LIMITED_INFORMATION,
};
use windows_sys::Win32::UI::Input::KeyboardAndMouse::{GetLastInputInfo, LASTINPUTINFO};
use windows_sys::Win32::UI::WindowsAndMessaging::{
    EnumWindows, GetForegroundWindow, GetWindowLongW, GetWindowRect, GetWindowThreadProcessId,
    IsIconic, IsWindowVisible, SendMessageTimeoutW, GWL_EXSTYLE, GWL_STYLE, WS_CAPTION,
    WS_EX_TOOLWINDOW, WS_THICKFRAME,
};

static SHUTDOWN: AtomicBool = AtomicBool::new(false);

pub fn snapshot() -> SysSnapshot {
    let procs = list_processes();
    let mut windows: Vec<WindowInfo> = Vec::with_capacity(64);

    unsafe {
        EnumWindows(Some(enum_window), &mut windows as *mut Vec<WindowInfo> as LPARAM);
    }

    // Some apps own several windows; keep the most descriptive title per pid.
    windows.sort_by(|a, b| {
        a.pid
            .cmp(&b.pid)
            .then_with(|| b.title.len().cmp(&a.title.len()))
    });
    windows.dedup_by(|a, b| a.pid == b.pid);

    let fg = unsafe { GetForegroundWindow() };
    let mut foreground_pid = 0u32;
    if !fg.is_null() {
        unsafe { GetWindowThreadProcessId(fg, &mut foreground_pid) };
    }

    SysSnapshot {
        procs,
        windows,
        foreground_pid,
        idle_secs: idle_seconds(),
        has_foreground: foreground_pid != 0,
    }
}

unsafe extern "system" fn enum_window(hwnd: HWND, lparam: LPARAM) -> BOOL {
    let out = &mut *(lparam as *mut Vec<WindowInfo>);
    if IsWindowVisible(hwnd) == 0 || IsIconic(hwnd) != 0 {
        return TRUE;
    }

    // Skip tool windows (tray flyouts, invisible helper windows).
    let ex_style = GetWindowLongW(hwnd, GWL_EXSTYLE) as u32;
    if ex_style & WS_EX_TOOLWINDOW != 0 {
        return TRUE;
    }

    // SendMessageTimeout instead of GetWindowText: a hung application must
    // never be able to freeze our detection loop (and the GUI that drives it).
    let mut buf = vec![0u16; 1024];
    let mut copied = 0usize;
    let ok = SendMessageTimeoutW(
        hwnd,
        0x000D, // WM_GETTEXT
        buf.len(),
        buf.as_mut_ptr() as LPARAM,
        0x0002 | 0x0001, // SMTO_ABORTIFHUNG | SMTO_BLOCK
        120,
        &mut copied,
    );
    if ok == 0 || copied == 0 {
        return TRUE;
    }
    let title = String::from_utf16_lossy(&buf[..copied.min(buf.len())])
        .trim()
        .to_string();
    if title.is_empty() || title == "Default IME" || title == "MSCTFIME UI" {
        return TRUE;
    }

    let mut pid = 0u32;
    GetWindowThreadProcessId(hwnd, &mut pid);
    if pid == 0 {
        return TRUE;
    }

    out.push(WindowInfo {
        pid,
        title,
        fullscreen: is_fullscreen(hwnd),
    });
    TRUE
}

/// Fullscreen = window covers its monitor and has no caption/frame.
/// This catches exclusive- and borderless-fullscreen games.
unsafe fn is_fullscreen(hwnd: HWND) -> bool {
    let style = GetWindowLongW(hwnd, GWL_STYLE) as u32;
    if style & (WS_CAPTION | WS_THICKFRAME) != 0 {
        return false;
    }

    let mut rect: windows_sys::Win32::Foundation::RECT = zeroed();
    if GetWindowRect(hwnd, &mut rect) == 0 {
        return false;
    }
    let monitor = MonitorFromWindow(hwnd, MONITOR_DEFAULTTONEAREST);
    if monitor.is_null() {
        return false;
    }
    let mut info: MONITORINFO = zeroed();
    info.cbSize = size_of::<MONITORINFO>() as u32;
    if GetMonitorInfoW(monitor, &mut info) == 0 {
        return false;
    }

    let ww = rect.right - rect.left;
    let wh = rect.bottom - rect.top;
    let mw = info.rcMonitor.right - info.rcMonitor.left;
    let mh = info.rcMonitor.bottom - info.rcMonitor.top;
    ww >= mw - 4 && wh >= mh - 4
}

fn list_processes() -> Vec<(u32, String)> {
    let mut out = Vec::with_capacity(256);
    unsafe {
        let snap = CreateToolhelp32Snapshot(TH32CS_SNAPPROCESS, 0);
        if snap == INVALID_HANDLE_VALUE || snap.is_null() {
            return out;
        }
        let mut entry: PROCESSENTRY32W = zeroed();
        entry.dwSize = size_of::<PROCESSENTRY32W>() as u32;
        if Process32FirstW(snap, &mut entry) != 0 {
            loop {
                let end = entry
                    .szExeFile
                    .iter()
                    .position(|c| *c == 0)
                    .unwrap_or(entry.szExeFile.len());
                let name = String::from_utf16_lossy(&entry.szExeFile[..end]);
                if !name.is_empty() {
                    out.push((entry.th32ProcessID, name));
                }
                if Process32NextW(snap, &mut entry) == 0 {
                    break;
                }
            }
        }
        CloseHandle(snap);
    }
    out
}

fn idle_seconds() -> u64 {
    unsafe {
        let mut info: LASTINPUTINFO = zeroed();
        info.cbSize = size_of::<LASTINPUTINFO>() as u32;
        if GetLastInputInfo(&mut info) == 0 {
            return 0;
        }
        let now = GetTickCount64();
        now.saturating_sub(u64::from(info.dwTime)) / 1000
    }
}

pub fn process_is_alive(pid: u32) -> bool {
    if pid == 0 {
        return false;
    }
    unsafe {
        let handle = OpenProcess(PROCESS_QUERY_LIMITED_INFORMATION, 0, pid);
        if handle.is_null() {
            return false;
        }
        let mut exit_code = 0u32;
        let ok = windows_sys::Win32::System::Threading::GetExitCodeProcess(handle, &mut exit_code);
        CloseHandle(handle);
        ok != 0 && exit_code == 259 // STILL_ACTIVE
    }
}

#[allow(dead_code)]
pub fn process_path(pid: u32) -> Option<String> {
    unsafe {
        let handle = OpenProcess(PROCESS_QUERY_LIMITED_INFORMATION, 0, pid);
        if handle.is_null() {
            return None;
        }
        let mut buf = vec![0u16; 1024];
        let mut size = buf.len() as u32;
        let ok = QueryFullProcessImageNameW(handle, 0, buf.as_mut_ptr(), &mut size);
        CloseHandle(handle);
        if ok == 0 {
            return None;
        }
        Some(String::from_utf16_lossy(&buf[..size as usize]))
    }
}

pub fn cancel_io(file: &std::fs::File) {
    use std::os::windows::io::AsRawHandle;
    unsafe {
        CancelIoEx(file.as_raw_handle() as HANDLE, std::ptr::null());
    }
}

/// Re-attach to the launching terminal so a GUI-subsystem build still prints
/// when run from cmd/PowerShell. Returns false when there is no usable console.
///
/// Handles inherited from the parent (pipes, `>` redirects, captured output)
/// are left untouched: reopening `CONOUT$` would bypass them.
pub fn attach_parent_console() -> bool {
    unsafe {
        let out = GetStdHandle(STD_OUTPUT_HANDLE);
        let err = GetStdHandle(STD_ERROR_HANDLE);
        let out_ok = handle_valid(out);
        let err_ok = handle_valid(err);

        if out_ok && err_ok {
            return true;
        }

        if AttachConsole(ATTACH_PARENT_PROCESS) == 0 {
            return false;
        }
        // Console code page matters for the arrows/em dashes in our messages.
        windows_sys::Win32::System::Console::SetConsoleOutputCP(65001);

        let name: Vec<u16> = "CONOUT$\0".encode_utf16().collect();
        let handle = windows_sys::Win32::Storage::FileSystem::CreateFileW(
            name.as_ptr(),
            0x4000_0000 | 0x8000_0000, // GENERIC_WRITE | GENERIC_READ
            3,                          // FILE_SHARE_READ | FILE_SHARE_WRITE
            std::ptr::null(),
            3, // OPEN_EXISTING
            0,
            std::ptr::null_mut(),
        );
        if handle_valid(handle) {
            if !out_ok {
                SetStdHandle(STD_OUTPUT_HANDLE, handle);
            }
            if !err_ok {
                SetStdHandle(STD_ERROR_HANDLE, handle);
            }
        }
        true // attached, even if CONOUT$ could not be reopened
    }
}

fn handle_valid(handle: HANDLE) -> bool {
    !handle.is_null() && handle != INVALID_HANDLE_VALUE
}

unsafe extern "system" fn ctrl_handler(_ctrl_type: u32) -> BOOL {
    SHUTDOWN.store(true, Ordering::SeqCst);
    TRUE
}

pub fn install_shutdown_handler() {
    unsafe {
        SetConsoleCtrlHandler(Some(ctrl_handler), TRUE);
    }
}

pub fn shutdown_requested() -> bool {
    SHUTDOWN.load(Ordering::SeqCst)
}

/// Simple native message box for startup errors and confirmations.
pub fn message_box(title: &str, text: &str, error: bool) {
    use windows_sys::Win32::UI::WindowsAndMessaging::{
        MessageBoxW, MB_ICONERROR, MB_ICONINFORMATION, MB_OK,
    };
    let title: Vec<u16> = title.encode_utf16().chain(std::iter::once(0)).collect();
    let text: Vec<u16> = text.encode_utf16().chain(std::iter::once(0)).collect();
    let icon = if error { MB_ICONERROR } else { MB_ICONINFORMATION };
    unsafe {
        MessageBoxW(
            std::ptr::null_mut(),
            text.as_ptr(),
            title.as_ptr(),
            MB_OK | icon,
        );
    }
}
