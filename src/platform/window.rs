//! Window helpers that bridge Slint windows with winit and Win32 APIs.

use i_slint_backend_winit::WinitWindowAccessor;
use raw_window_handle::{HasWindowHandle, RawWindowHandle};
use slint::ComponentHandle;
use winit::dpi::PhysicalPosition;

use crate::TimeTrans;

/// Moves the timestamp floating window to an absolute screen position.
pub fn set_position(time_window: &TimeTrans, x: f64, y: f64) {
    time_window.window().with_winit_window(|winit_window| {
        winit_window.set_outer_position(PhysicalPosition::new(x, y));
    });
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

/// Marks the floating window as a tool window so it stays out of the taskbar.
pub fn hide_taskbar_icon(time_window: &TimeTrans) {
    time_window.window().with_winit_window(|winit_window| {
        if let Ok(handle) = winit_window.window_handle()
            && let RawWindowHandle::Win32(win32_handle) = handle.as_raw()
        {
            let hwnd = win32_handle.hwnd.get() as isize;
            unsafe {
                use windows_sys::Win32::UI::WindowsAndMessaging::*;

                let old_style = GetWindowLongPtrW(hwnd, GWL_EXSTYLE) as u32;
                let new_style = (old_style | WS_EX_TOOLWINDOW) & !WS_EX_APPWINDOW;

                if old_style != new_style {
                    SetWindowLongPtrW(hwnd, GWL_EXSTYLE, new_style as isize);
                    SetWindowPos(
                        hwnd,
                        0,
                        0,
                        0,
                        0,
                        0,
                        SWP_FRAMECHANGED | SWP_NOMOVE | SWP_NOSIZE | SWP_NOZORDER,
                    );
                }
            }
        }
    });
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
