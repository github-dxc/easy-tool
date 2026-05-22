//! Window helpers that bridge Slint windows with winit and Win32 APIs.

#[cfg(target_os = "windows")]
use std::cell::Cell;

use i_slint_backend_winit::WinitWindowAccessor;
use raw_window_handle::{HasWindowHandle, RawWindowHandle};
use slint::ComponentHandle;
use winit::dpi::PhysicalPosition;

use crate::TimeTrans;

#[cfg(target_os = "windows")]
thread_local! {
    static TASKBAR_OWNER_WINDOW: Cell<isize> = const { Cell::new(0) };
}

/// Moves the timestamp floating window to an absolute screen position.
pub fn set_position(time_window: &TimeTrans, x: f64, y: f64) {
    set_window_position(time_window, x, y);
}

/// Moves any Slint window to an absolute screen position.
pub fn set_window_position(window: &impl ComponentHandle, x: f64, y: f64) {
    window.window().with_winit_window(|winit_window| {
        winit_window.set_outer_position(PhysicalPosition::new(x, y));
    });
}

/// Returns the outer position of a Slint window in physical screen coordinates.
pub fn window_position(window: &impl ComponentHandle) -> Option<(f64, f64)> {
    let mut position = None;
    window.window().with_winit_window(|winit_window| {
        if let Ok(pos) = winit_window.outer_position() {
            position = Some((pos.x as f64, pos.y as f64));
        }
    });
    position
}

/// Returns the current cursor position in physical screen coordinates.
#[cfg(target_os = "windows")]
pub fn cursor_position() -> Option<(f64, f64)> {
    unsafe {
        use windows_sys::Win32::Foundation::POINT;
        use windows_sys::Win32::UI::WindowsAndMessaging::GetCursorPos;

        let mut point = POINT { x: 0, y: 0 };
        (GetCursorPos(&mut point) != 0).then_some((point.x as f64, point.y as f64))
    }
}

/// Non-Windows platforms do not expose cursor position here yet.
#[cfg(not(target_os = "windows"))]
pub fn cursor_position() -> Option<(f64, f64)> {
    None
}

/// Returns the current monitor size for the timestamp window.
pub fn display_size(time_window: &TimeTrans) -> Option<(f64, f64)> {
    let mut width = 0f64;
    let mut height = 0f64;
    time_window.window().with_winit_window(|winit_window| {
        if let Some(monitor) = winit_window.current_monitor() {
            let size = monitor.size();
            width = size.width as f64;
            height = size.height as f64;
        }
    });

    (width > 0f64 && height > 0f64).then_some((width, height))
}

/// Marks any Slint window as a tool window so it stays out of the taskbar.
pub fn hide_taskbar_icon(window: &impl ComponentHandle) {
    window.window().with_winit_window(|winit_window| {
        if let Ok(handle) = winit_window.window_handle()
            && let RawWindowHandle::Win32(win32_handle) = handle.as_raw()
        {
            let hwnd = win32_handle.hwnd.get() as isize;
            hide_hwnd_from_taskbar(hwnd);
        }
    });
}

#[cfg(target_os = "windows")]
fn hide_hwnd_from_taskbar(hwnd: isize) {
    unsafe {
        use windows_sys::Win32::UI::WindowsAndMessaging::*;

        let old_style = GetWindowLongPtrW(hwnd, GWL_EXSTYLE) as u32;
        let new_style = (old_style | WS_EX_TOOLWINDOW) & !WS_EX_APPWINDOW;
        let owner_hwnd = taskbar_owner_window();
        let was_visible = IsWindowVisible(hwnd) != 0;

        if was_visible {
            ShowWindow(hwnd, SW_HIDE);
        }

        if owner_hwnd != 0 {
            SetWindowLongPtrW(hwnd, GWLP_HWNDPARENT, owner_hwnd);
        }

        if old_style != new_style {
            SetWindowLongPtrW(hwnd, GWL_EXSTYLE, new_style as isize);
        }

        SetWindowPos(
            hwnd,
            0,
            0,
            0,
            0,
            0,
            SWP_FRAMECHANGED | SWP_NOMOVE | SWP_NOSIZE | SWP_NOZORDER | SWP_NOACTIVATE,
        );

        if was_visible {
            ShowWindow(hwnd, SW_SHOWNA);
        }
    }
}

#[cfg(not(target_os = "windows"))]
fn hide_hwnd_from_taskbar(_hwnd: isize) {}

#[cfg(target_os = "windows")]
fn taskbar_owner_window() -> isize {
    TASKBAR_OWNER_WINDOW.with(|owner| {
        let hwnd = owner.get();
        if hwnd != 0 {
            return hwnd;
        }

        let hwnd = create_taskbar_owner_window();
        owner.set(hwnd);
        hwnd
    })
}

#[cfg(target_os = "windows")]
fn create_taskbar_owner_window() -> isize {
    unsafe {
        use windows_sys::Win32::UI::WindowsAndMessaging::*;

        let class_name = wide_null("STATIC");
        let window_name = wide_null("easy-tool-taskbar-owner");
        CreateWindowExW(
            WS_EX_TOOLWINDOW,
            class_name.as_ptr(),
            window_name.as_ptr(),
            WS_POPUP,
            0,
            0,
            0,
            0,
            0,
            0,
            0,
            std::ptr::null(),
        ) as isize
    }
}

#[cfg(target_os = "windows")]
fn wide_null(value: &str) -> Vec<u16> {
    value.encode_utf16().chain(std::iter::once(0)).collect()
}

/// Brings an existing Slint window to the foreground when supported by the platform.
pub fn activate_slint_window(window: &impl ComponentHandle) {
    window.window().with_winit_window(|winit_window| {
        winit_window.set_minimized(false);
        winit_window.focus_window();

        #[cfg(target_os = "windows")]
        if let Ok(handle) = winit_window.window_handle()
            && let RawWindowHandle::Win32(win32_handle) = handle.as_raw()
        {
            let hwnd = win32_handle.hwnd.get() as isize;
            let _ = activate_window(hwnd);
        }
    });
}

#[cfg(target_os = "windows")]
/// Returns the foreground Win32 window handle for later paste activation.
pub fn foreground_window_handle() -> Option<isize> {
    let hwnd = unsafe { windows_sys::Win32::UI::WindowsAndMessaging::GetForegroundWindow() };
    (hwnd != 0).then_some(hwnd)
}

#[cfg(not(target_os = "windows"))]
/// Non-Windows platforms currently do not expose a foreground-window handle.
pub fn foreground_window_handle() -> Option<isize> {
    None
}

#[cfg(target_os = "windows")]
/// Brings a saved Win32 window handle back to the foreground.
pub fn activate_window(hwnd: isize) -> bool {
    unsafe { windows_sys::Win32::UI::WindowsAndMessaging::SetForegroundWindow(hwnd) != 0 }
}

#[cfg(not(target_os = "windows"))]
/// Non-Windows platforms currently do not support foreground activation here.
pub fn activate_window(_hwnd: isize) -> bool {
    false
}
