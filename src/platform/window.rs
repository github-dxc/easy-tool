//! Window helpers that bridge Slint windows with winit and Win32 APIs.

#[cfg(target_os = "windows")]
use std::cell::Cell;
use std::time::Duration;

use i_slint_backend_winit::WinitWindowAccessor;
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

/// 设置时间转换浮窗的位置，坐标使用屏幕物理像素。
pub fn set_position(time_window: &TimeTrans, x: f64, y: f64) {
    set_window_position(time_window, x, y);
}

/// 通过原生窗口句柄设置 Slint 窗口的位置。
pub fn set_window_position(window: &impl ComponentHandle, x: f64, y: f64) {
    window.window().with_winit_window(|winit_window| {
        winit_window.set_outer_position(PhysicalPosition::new(x, y));
    });
}

/// 将自绘标题栏的拖动请求转交给原生窗口，保持系统拖动行为。
pub fn drag_window(window: &impl ComponentHandle) {
    window.window().with_winit_window(|native| {
        let _ = native.drag_window();
    });
}

/// 获取时间转换浮窗所在显示器的尺寸。
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

/// 等待隐藏的原生窗口就绪，先设置尺寸和居中位置，再显示并激活。
pub fn show_centered_window<T>(window: &T, size: slint::LogicalSize)
where
    T: ComponentHandle + 'static,
{
    show_centered_window_with_style(window, size, false, |_| true);
}

/// 仅在显示请求仍有效时居中打开窗口，避免异步创建完成后重新弹出已关闭的页面。
pub fn show_centered_window_if<T>(
    window: &T,
    size: slint::LogicalSize,
    should_show: impl Fn(&T) -> bool + 'static,
) where
    T: ComponentHandle + 'static,
{
    show_centered_window_with_style(window, size, false, should_show);
}

/// 先应用隐藏任务栏图标的工具窗口样式，再居中显示浮窗。
pub fn show_centered_tool_window<T>(window: &T, size: slint::LogicalSize)
where
    T: ComponentHandle + 'static,
{
    show_centered_window_with_style(window, size, true, |_| true);
}

/// 在原生窗口就绪后统一完成样式、尺寸、位置和显示，避免首次显示跳动。
fn show_centered_window_with_style<T>(
    window: &T,
    size: slint::LogicalSize,
    tool_window: bool,
    should_show: impl Fn(&T) -> bool + 'static,
) where
    T: ComponentHandle + 'static,
{
    let weak = window.as_weak();
    // 原生窗口可能尚未创建，异步等待期间保持隐藏，避免默认位置被绘制出来。
    if let Err(err) = slint::spawn_local(async move {
        let Some(window) = weak.upgrade() else {
            return;
        };
        if !should_show(&window) {
            return;
        }
        let native = match window.window().winit_window().await {
            Ok(native) => native,
            Err(err) => {
                log::error!("prepare centered window failed: {err}");
                return;
            }
        };
        // 等待原生窗口期间用户可能已关闭页面，显示前必须再次确认。
        if !should_show(&window) {
            return;
        }
        if tool_window {
            hide_taskbar_icon(&window);
        }
        // 首次显示前原生窗口可能仍是默认大小，必须先应用页面给出的逻辑尺寸。
        window.window().set_size(size);
        if let Some(monitor) = native
            .current_monitor()
            .or_else(|| native.primary_monitor())
        {
            let origin = monitor.position();
            let monitor_size = monitor.size();
            // 使用包含边框的实际尺寸，并计入显示器原点，兼容多屏与负坐标。
            let outer = native.outer_size();
            let x = origin.x + ((monitor_size.width as i64 - outer.width as i64) / 2) as i32;
            let y = origin.y + ((monitor_size.height as i64 - outer.height as i64) / 2) as i32;
            native.set_outer_position(PhysicalPosition::new(x, y));
        }
        if let Err(err) = window.show() {
            log::error!("show centered window failed: {err}");
            return;
        }
        activate_slint_window(&window);
    }) {
        log::error!("schedule centered window failed: {err}");
    }
}

/// 以工具窗口形式显示组件，并在创建完成后重试隐藏任务栏图标。
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

/// 等待原生窗口句柄就绪后隐藏任务栏图标，限制重试次数。
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

/// 尝试取得原生句柄并隐藏任务栏图标，返回是否已经应用。
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

#[cfg(target_os = "windows")]
/// 关闭 Windows 窗口过渡动画；其他平台保持空操作。
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
/// 关闭 Windows 窗口过渡动画；其他平台保持空操作。
fn disable_window_transitions(_hwnd: isize) {}

#[cfg(target_os = "windows")]
/// 设置原生窗口的工具窗口样式及所有者，使其不出现在任务栏。
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
/// 设置原生窗口的工具窗口样式及所有者，使其不出现在任务栏。
fn hide_hwnd_from_taskbar(_hwnd: isize) {}

#[cfg(target_os = "windows")]
/// 取得当前 UI 线程共享的隐藏所有者窗口，首次调用时创建。
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
/// 创建隐藏原生窗口，作为工具窗口的所有者。
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
/// 将字符串转换为以空字符结束的 UTF-16，供 Windows API 使用。
fn wide_null(value: &str) -> Vec<u16> {
    value.encode_utf16().chain(std::iter::once(0)).collect()
}

/// 获取 Slint 原生窗口句柄并请求将其激活到前台。
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
/// 尝试激活指定原生窗口；非 Windows 平台返回失败。
pub fn activate_window(hwnd: isize) -> bool {
    unsafe { windows_sys::Win32::UI::WindowsAndMessaging::SetForegroundWindow(hwnd) != 0 }
}

#[cfg(not(target_os = "windows"))]
/// 尝试激活指定原生窗口；非 Windows 平台返回失败。
pub fn activate_window(_hwnd: isize) -> bool {
    false
}
