//! Window helpers that bridge Slint windows with winit and Win32 APIs.

#[cfg(target_os = "windows")]
use std::cell::Cell;
use std::time::Duration;

use i_slint_backend_winit::WinitWindowAccessor;
use log::info;
use raw_window_handle::{HasWindowHandle, RawWindowHandle};
use slint::ComponentHandle;
use winit::dpi::PhysicalPosition;

use crate::TimeTrans;

const TASKBAR_HIDE_RETRY_DELAY: Duration = Duration::from_millis(16);
const TASKBAR_HIDE_RETRY_ATTEMPTS: u8 = 10;

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

/// Allows the native window frame to be maximized and resized.
pub fn make_window_resizable(window: &impl ComponentHandle) {
    window.window().with_winit_window(|winit_window| {
        winit_window.set_resizable(true);

        #[cfg(target_os = "windows")]
        if let Ok(handle) = winit_window.window_handle()
            && let RawWindowHandle::Win32(win32_handle) = handle.as_raw()
        {
            let hwnd = win32_handle.hwnd.get() as isize;
            enable_hwnd_resize_and_maximize(hwnd);
        }
    });
}

/// Re-applies resizable/maximize styles after the native window has been created.
pub fn make_window_resizable_when_ready<T>(window: slint::Weak<T>, attempts_left: u8)
where
    T: ComponentHandle + 'static,
{
    slint::Timer::single_shot(TASKBAR_HIDE_RETRY_DELAY, move || {
        let Some(window) = window.upgrade() else {
            return;
        };

        make_window_resizable(&window);
        if attempts_left > 0 {
            make_window_resizable_when_ready(window.as_weak(), attempts_left - 1);
        }
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

#[cfg(target_os = "windows")]
fn enable_hwnd_resize_and_maximize(hwnd: isize) {
    unsafe {
        use windows_sys::Win32::UI::WindowsAndMessaging::*;

        let old_style = GetWindowLongPtrW(hwnd, GWL_STYLE) as u32;
        let new_style = old_style | WS_MAXIMIZEBOX | WS_THICKFRAME;

        if old_style != new_style {
            SetWindowLongPtrW(hwnd, GWL_STYLE, new_style as isize);
            SetWindowPos(
                hwnd,
                0,
                0,
                0,
                0,
                0,
                SWP_FRAMECHANGED | SWP_NOMOVE | SWP_NOSIZE | SWP_NOZORDER | SWP_NOACTIVATE,
            );
        }
    }
}

/// Shows a Slint window after marking it as a tool window when possible.
pub fn show_without_taskbar_icon<T>(window: &T) -> Result<(), slint::PlatformError>
where
    T: ComponentHandle + 'static,
{
    let was_visible = window.window().is_visible();
    let prepared = if was_visible {
        false
    } else {
        hide_taskbar_icon(window)
    };

    let result = window.show();
    if result.is_ok() {
        let attempts = if prepared {
            1
        } else {
            TASKBAR_HIDE_RETRY_ATTEMPTS
        };
        hide_taskbar_icon_when_ready(window.as_weak(), attempts);
    }

    result
}

/// Shows a tool-style Slint window that stays out of the taskbar and cannot be activated.
pub fn show_without_taskbar_icon_or_activation<T>(window: &T) -> Result<(), slint::PlatformError>
where
    T: ComponentHandle + 'static,
{
    apply_taskbar_hidden_noactivate(window);

    let result = window.show();
    if result.is_ok() {
        apply_taskbar_hidden_noactivate_when_ready(window.as_weak(), TASKBAR_HIDE_RETRY_ATTEMPTS);
    }

    result
}

fn apply_taskbar_hidden_noactivate_when_ready<T>(window: slint::Weak<T>, attempts_left: u8)
where
    T: ComponentHandle + 'static,
{
    slint::Timer::single_shot(TASKBAR_HIDE_RETRY_DELAY, move || {
        let Some(window) = window.upgrade() else {
            return;
        };

        if apply_taskbar_hidden_noactivate(&window) || attempts_left == 0 {
            return;
        }

        apply_taskbar_hidden_noactivate_when_ready(window.as_weak(), attempts_left - 1);
    });
}

fn apply_taskbar_hidden_noactivate(window: &impl ComponentHandle) -> bool {
    let mut applied = false;
    window.window().with_winit_window(|winit_window| {
        if let Ok(handle) = winit_window.window_handle()
            && let RawWindowHandle::Win32(win32_handle) = handle.as_raw()
        {
            let hwnd = win32_handle.hwnd.get() as isize;
            apply_hwnd_taskbar_hidden_noactivate(hwnd);
            applied = true;
        }
    });
    applied
}

fn hide_taskbar_icon_when_ready<T>(window: slint::Weak<T>, attempts_left: u8)
where
    T: ComponentHandle + 'static,
{
    slint::Timer::single_shot(TASKBAR_HIDE_RETRY_DELAY, move || {
        let Some(window) = window.upgrade() else {
            return;
        };

        if hide_taskbar_icon(&window) || attempts_left == 0 {
            return;
        }

        hide_taskbar_icon_when_ready(window.as_weak(), attempts_left - 1);
    });
}

/// Marks any Slint window as a tool window so it stays out of the taskbar.
pub fn hide_taskbar_icon(window: &impl ComponentHandle) -> bool {
    let mut applied = false;
    window.window().with_winit_window(|winit_window| {
        if let Ok(handle) = winit_window.window_handle()
            && let RawWindowHandle::Win32(win32_handle) = handle.as_raw()
        {
            let hwnd = win32_handle.hwnd.get() as isize;
            disable_window_transitions(hwnd);
            hide_hwnd_from_taskbar(hwnd);
            applied = true;
        }
    });
    applied
}

/// Prevents a Slint window from becoming the active foreground window on click.
pub fn prevent_window_activation(window: &impl ComponentHandle) -> bool {
    let mut applied = false;
    window.window().with_winit_window(|winit_window| {
        if let Ok(handle) = winit_window.window_handle()
            && let RawWindowHandle::Win32(win32_handle) = handle.as_raw()
        {
            let hwnd = win32_handle.hwnd.get() as isize;
            info!("prevent window activation requested: hwnd={hwnd:#x}");
            prevent_hwnd_activation(hwnd);
            applied = true;
        }
    });
    applied
}

pub fn prevent_window_activation_when_ready<T>(window: slint::Weak<T>, attempts_left: u8)
where
    T: ComponentHandle + 'static,
{
    slint::Timer::single_shot(TASKBAR_HIDE_RETRY_DELAY, move || {
        let Some(window) = window.upgrade() else {
            return;
        };

        if prevent_window_activation(&window) || attempts_left == 0 {
            return;
        }

        prevent_window_activation_when_ready(window.as_weak(), attempts_left - 1);
    });
}

#[cfg(target_os = "windows")]
fn disable_window_transitions(hwnd: isize) {
    unsafe {
        use windows_sys::Win32::Graphics::Dwm::{
            DWMWA_TRANSITIONS_FORCEDISABLED, DwmSetWindowAttribute,
        };

        let disabled: i32 = 1;
        let _ = DwmSetWindowAttribute(
            hwnd,
            DWMWA_TRANSITIONS_FORCEDISABLED as u32,
            std::ptr::addr_of!(disabled).cast(),
            std::mem::size_of_val(&disabled) as u32,
        );
    }
}

#[cfg(not(target_os = "windows"))]
fn disable_window_transitions(_hwnd: isize) {}

#[cfg(target_os = "windows")]
fn prevent_hwnd_activation(hwnd: isize) {
    unsafe {
        use windows_sys::Win32::UI::WindowsAndMessaging::*;

        let old_style = GetWindowLongPtrW(hwnd, GWL_EXSTYLE) as u32;
        let new_style = old_style | WS_EX_NOACTIVATE;
        let was_visible = IsWindowVisible(hwnd) != 0;

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

        let applied_style = GetWindowLongPtrW(hwnd, GWL_EXSTYLE) as u32;
        info!(
            "prevent window activation applied: hwnd={hwnd:#x}, visible={}, old_style={old_style:#010x}, new_style={new_style:#010x}, applied_style={applied_style:#010x}, has_noactivate={}",
            was_visible,
            applied_style & WS_EX_NOACTIVATE != 0
        );
    }
}

#[cfg(not(target_os = "windows"))]
fn prevent_hwnd_activation(_hwnd: isize) {}

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
fn apply_hwnd_taskbar_hidden_noactivate(hwnd: isize) {
    unsafe {
        use windows_sys::Win32::UI::WindowsAndMessaging::*;

        disable_window_transitions(hwnd);

        let old_style = GetWindowLongPtrW(hwnd, GWL_EXSTYLE) as u32;
        let new_style = (old_style | WS_EX_TOOLWINDOW | WS_EX_NOACTIVATE) & !WS_EX_APPWINDOW;
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

        let applied_style = GetWindowLongPtrW(hwnd, GWL_EXSTYLE) as u32;
        info!(
            "apply noactivate taskbar-hidden style: hwnd={hwnd:#x}, owner={owner_hwnd:#x}, visible={}, old_style={old_style:#010x}, new_style={new_style:#010x}, applied_style={applied_style:#010x}, has_noactivate={}, has_toolwindow={}, has_appwindow={}",
            was_visible,
            applied_style & WS_EX_NOACTIVATE != 0,
            applied_style & WS_EX_TOOLWINDOW != 0,
            applied_style & WS_EX_APPWINDOW != 0
        );
    }
}

#[cfg(not(target_os = "windows"))]
fn apply_hwnd_taskbar_hidden_noactivate(_hwnd: isize) {}

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
