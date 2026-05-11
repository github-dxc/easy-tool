use i_slint_backend_winit::WinitWindowAccessor;
use raw_window_handle::{HasWindowHandle, RawWindowHandle};
use slint::ComponentHandle;
use winit::dpi::PhysicalPosition;

use crate::TimeTrans;

pub fn set_position(time_window: &TimeTrans, x: f64, y: f64) {
    time_window.window().with_winit_window(|winit_window| {
        winit_window.set_outer_position(PhysicalPosition::new(x, y));
    });
}

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

#[cfg(target_os = "windows")]
pub fn foreground_window_handle() -> Option<isize> {
    let hwnd = unsafe { windows_sys::Win32::UI::WindowsAndMessaging::GetForegroundWindow() };
    (hwnd != 0).then_some(hwnd)
}

#[cfg(not(target_os = "windows"))]
pub fn foreground_window_handle() -> Option<isize> {
    None
}

#[cfg(target_os = "windows")]
pub fn activate_window(hwnd: isize) -> bool {
    unsafe { windows_sys::Win32::UI::WindowsAndMessaging::SetForegroundWindow(hwnd) != 0 }
}

#[cfg(not(target_os = "windows"))]
pub fn activate_window(_hwnd: isize) -> bool {
    false
}
