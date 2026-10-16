//! 右下角状态提示：显示前定位，不抢焦点，一秒后自动隐藏。

use std::cell::Cell;
use std::rc::Rc;
use std::time::Duration;

use i_slint_backend_winit::WinitWindowAccessor;
use slint::{ComponentHandle, LogicalSize, Timer, TimerMode};
use winit::dpi::PhysicalPosition;

use crate::StatusNotificationWindow;
use crate::platform::window::hide_taskbar_icon;

const NOTIFICATION_SIZE: LogicalSize = LogicalSize::new(360.0, 68.0);
const NOTIFICATION_DURATION: Duration = Duration::from_secs(1);
const NOTIFICATION_MARGIN: f64 = 16.0;

pub struct StatusNotification {
    window: StatusNotificationWindow,
    close_timer: Rc<Timer>,
    generation: Rc<Cell<u64>>,
}

impl StatusNotification {
    /// 创建可复用的状态提示窗口和关闭定时器，原生窗口首次显示时才创建。
    pub fn new() -> Result<Self, slint::PlatformError> {
        Ok(Self {
            window: StatusNotificationWindow::new()?,
            close_timer: Rc::new(Timer::default()),
            generation: Rc::new(Cell::new(0)),
        })
    }

    /// 更新提示内容并重新计时，连续触发时仅允许最新一次显示任务生效。
    pub fn show(&self, message: &str) {
        self.close_timer.stop();
        let generation = self.generation.get().wrapping_add(1);
        self.generation.set(generation);
        self.window.set_message(message.into());
        let current = Rc::clone(&self.generation);
        let close_timer = Rc::clone(&self.close_timer);
        let weak = self.window.as_weak();
        if let Err(err) = slint::spawn_local(async move {
            let Some(window) = weak.upgrade() else { return };
            if current.get() != generation {
                return;
            }
            // 首次显示前等待隐藏的原生窗口就绪，避免先出现在左上角再跳动。
            let native = match window.window().winit_window().await {
                Ok(native) => native,
                Err(err) => {
                    log::error!("prepare status notification failed: {err}");
                    return;
                }
            };
            if current.get() != generation {
                return;
            }
            if !window.window().is_visible() {
                hide_taskbar_icon(&window);
                prepare_non_activating_window(&native);
            }
            window.window().set_size(NOTIFICATION_SIZE);
            if let Some((area, scale)) = notification_work_area(&native) {
                let size = NOTIFICATION_SIZE.to_physical(scale as f32);
                let (x, y) = bottom_right_position(
                    area,
                    (size.width as i32, size.height as i32),
                    (NOTIFICATION_MARGIN * scale).round() as i32,
                );
                native.set_outer_position(PhysicalPosition::new(x, y));
            }
            if let Err(err) = window.show() {
                log::error!("show status notification failed: {err}");
                return;
            }
            // 必须从真正显示后开始计时，旧定时器不能提前关闭更新后的提示。
            close_timer.start(TimerMode::SingleShot, NOTIFICATION_DURATION, move || {
                if current.get() == generation
                    && let Some(window) = weak.upgrade()
                {
                    let _ = window.hide();
                }
            });
        }) {
            log::error!("schedule status notification failed: {err}");
        }
    }
}

/// 将提示限制在工作区右下角，兼容负坐标屏幕及小于提示尺寸的工作区。
fn bottom_right_position(
    (left, top, right, bottom): (i32, i32, i32, i32),
    (width, height): (i32, i32),
    margin: i32,
) -> (i32, i32) {
    (
        (right - width - margin).max(left),
        (bottom - height - margin).max(top),
    )
}

#[cfg(target_os = "windows")]
/// 将提示设为不激活窗口，保证快捷切换后仍可继续在原应用中输入。
fn prepare_non_activating_window(window: &winit::window::Window) {
    use raw_window_handle::{HasWindowHandle, RawWindowHandle};
    use windows_sys::Win32::UI::Shell::SetWindowSubclass;
    use windows_sys::Win32::UI::WindowsAndMessaging::{
        GWL_EXSTYLE, GetWindowLongPtrW, SetWindowLongPtrW, WS_EX_APPWINDOW, WS_EX_NOACTIVATE,
        WS_EX_TOOLWINDOW,
    };
    if let Ok(handle) = window.window_handle()
        && let RawWindowHandle::Win32(handle) = handle.as_raw()
    {
        unsafe {
            let hwnd = handle.hwnd.get();
            // 后端在显示和隐藏时会重写样式，通过子类过程持续保留提示专用样式。
            if SetWindowSubclass(hwnd, Some(notification_window_proc), 1, 0) == 0 {
                log::error!("install status notification window procedure failed");
            }
            let style = GetWindowLongPtrW(hwnd, GWL_EXSTYLE);
            SetWindowLongPtrW(
                hwnd,
                GWL_EXSTYLE,
                (style | (WS_EX_NOACTIVATE | WS_EX_TOOLWINDOW) as isize)
                    & !(WS_EX_APPWINDOW as isize),
            );
        }
    }
}

#[cfg(target_os = "windows")]
/// 在后端重设样式时保留非激活工具窗口属性，点击提示也不抢焦点，销毁时解除子类过程。
unsafe extern "system" fn notification_window_proc(
    hwnd: windows_sys::Win32::Foundation::HWND,
    message: u32,
    wparam: windows_sys::Win32::Foundation::WPARAM,
    lparam: windows_sys::Win32::Foundation::LPARAM,
    subclass_id: usize,
    _reference: usize,
) -> windows_sys::Win32::Foundation::LRESULT {
    use windows_sys::Win32::UI::Shell::{DefSubclassProc, RemoveWindowSubclass};
    use windows_sys::Win32::UI::WindowsAndMessaging::{
        GWL_EXSTYLE, MA_NOACTIVATE, STYLESTRUCT, WM_MOUSEACTIVATE, WM_NCDESTROY, WM_STYLECHANGING,
        WS_EX_APPWINDOW, WS_EX_NOACTIVATE, WS_EX_TOOLWINDOW,
    };
    unsafe {
        match message {
            WM_STYLECHANGING if wparam as i32 == GWL_EXSTYLE && lparam != 0 => {
                // STYLESTRUCT 由 Windows 在消息调用期间提供，修改即将应用的新样式。
                let style = &mut *(lparam as *mut STYLESTRUCT);
                style.styleNew =
                    (style.styleNew | WS_EX_NOACTIVATE | WS_EX_TOOLWINDOW) & !WS_EX_APPWINDOW;
            }
            WM_MOUSEACTIVATE => return MA_NOACTIVATE as isize,
            WM_NCDESTROY => {
                RemoveWindowSubclass(hwnd, Some(notification_window_proc), subclass_id);
            }
            _ => {}
        }
        DefSubclassProc(hwnd, message, wparam, lparam)
    }
}

#[cfg(not(target_os = "windows"))]
/// 其他平台保留窗口后端的默认非主动激活行为。
fn prepare_non_activating_window(_window: &winit::window::Window) {}

/// 优先读取前台应用所在屏幕的工作区及缩放比例，失败时回退到窗口所在屏幕。
fn notification_work_area(window: &winit::window::Window) -> Option<((i32, i32, i32, i32), f64)> {
    #[cfg(target_os = "windows")]
    {
        use windows_sys::Win32::Graphics::Gdi::{
            GetMonitorInfoW, MONITOR_DEFAULTTOPRIMARY, MONITORINFO, MonitorFromWindow,
        };
        use windows_sys::Win32::UI::WindowsAndMessaging::GetForegroundWindow;
        use winit::platform::windows::MonitorHandleExtWindows;
        unsafe {
            let monitor = MonitorFromWindow(GetForegroundWindow(), MONITOR_DEFAULTTOPRIMARY);
            let mut info: MONITORINFO = std::mem::zeroed();
            info.cbSize = std::mem::size_of::<MONITORINFO>() as u32;
            if GetMonitorInfoW(monitor, &mut info) != 0 {
                let scale = window
                    .available_monitors()
                    .find(|candidate| candidate.hmonitor() as isize == monitor)
                    .map(|monitor| monitor.scale_factor())
                    .unwrap_or(window.scale_factor());
                let area = info.rcWork;
                return Some(((area.left, area.top, area.right, area.bottom), scale));
            }
        }
    }
    let monitor = window
        .current_monitor()
        .or_else(|| window.primary_monitor())?;
    let origin = monitor.position();
    let size = monitor.size();
    Some((
        (
            origin.x,
            origin.y,
            origin.x + size.width as i32,
            origin.y + size.height as i32,
        ),
        monitor.scale_factor(),
    ))
}

#[cfg(test)]
mod tests {
    use super::*;

    /// 验证工作区边距、负坐标和小屏幕边界，防止提示被任务栏或屏幕边缘遮挡。
    #[test]
    fn notification_position_stays_inside_work_area() {
        assert_eq!(
            bottom_right_position((0, 0, 1920, 1040), (360, 68), 16),
            (1544, 956)
        );
        assert_eq!(
            bottom_right_position((-1920, -200, 0, 840), (450, 85), 20),
            (-470, 735)
        );
        assert_eq!(
            bottom_right_position((100, 50, 300, 120), (360, 68), 16),
            (100, 50)
        );
    }
}
