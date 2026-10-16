//! 全局快捷键识别，以及复制文本的非阻塞延迟读取。

use super::AppEvent;
use crate::infrastructure::global_input::start_global_input_listener;
use arboard::Clipboard;
use rdev::{EventType, Key};
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{Arc, Mutex, mpsc::Sender};
use std::time::Duration;

#[derive(Clone, Copy, Default)]
struct ShortcutState {
    ctrl: bool,
    shift: bool,
    alt: bool,
    x_pressed: bool,
    position: (f64, f64),
}

#[derive(Debug, PartialEq)]
enum ShortcutAction {
    Copy,
    History,
    SwitchCopyMode,
}

impl ShortcutState {
    /// 跟踪修饰键和鼠标位置，为快捷键识别及浮窗定位提供状态。
    fn update(&mut self, event: &EventType) {
        match event {
            EventType::KeyPress(Key::ControlLeft | Key::ControlRight) => self.ctrl = true,
            EventType::KeyRelease(Key::ControlLeft | Key::ControlRight) => self.ctrl = false,
            EventType::KeyPress(Key::ShiftLeft | Key::ShiftRight) => self.shift = true,
            EventType::KeyRelease(Key::ShiftLeft | Key::ShiftRight) => self.shift = false,
            EventType::KeyPress(Key::Alt | Key::AltGr) => self.alt = true,
            EventType::KeyRelease(Key::Alt | Key::AltGr) => self.alt = false,
            EventType::KeyRelease(Key::KeyX) => self.x_pressed = false,
            EventType::MouseMove { x, y } => self.position = (*x, *y),
            _ => {}
        }
    }

    /// 区分复制、历史和功能切换快捷键，长按 X 时仅切换一次。
    fn action(&mut self, event: &EventType) -> Option<ShortcutAction> {
        if matches!(event, EventType::KeyPress(Key::KeyX)) {
            // 无论修饰键是否匹配都记录按下状态，只有松开 X 后才允许再次切换。
            let repeated = std::mem::replace(&mut self.x_pressed, true);
            return (self.ctrl && self.alt && !self.shift && !repeated)
                .then_some(ShortcutAction::SwitchCopyMode);
        }
        if self.ctrl && !self.alt && matches!(event, EventType::KeyPress(Key::KeyC)) {
            return Some(if self.shift {
                ShortcutAction::History
            } else {
                ShortcutAction::Copy
            });
        }
        None
    }
}

/// 启动输入监听；回调只识别动作并发送数据，不访问窗口和剪贴板。
pub(super) fn start_shortcuts(
    events: Sender<AppEvent>,
    suppressed: Arc<AtomicBool>,
) -> Result<(), String> {
    let state = Mutex::new(ShortcutState::default());
    start_global_input_listener(move |event| {
        let mut state = state.lock().unwrap();
        state.update(&event.event_type);
        // Windows 上以系统状态校准修饰键，兼容启动前已按住按键的情况。
        if let Some((ctrl, shift, alt)) = current_system_modifiers() {
            state.ctrl = ctrl;
            state.shift = shift;
            state.alt = alt;
        }
        let action = state.action(&event.event_type);
        if suppressed.load(Ordering::SeqCst) {
            return Ok(());
        }
        let message = match action {
            Some(ShortcutAction::SwitchCopyMode) => AppEvent::SwitchCopyMode,
            Some(ShortcutAction::History) => AppEvent::ShowHistory,
            Some(ShortcutAction::Copy) => AppEvent::CopyRequested {
                // 首次复制前可能没有鼠标移动事件，直接读取按键时的系统坐标。
                position: current_system_cursor_position().unwrap_or(state.position),
            },
            None => return Ok(()),
        };
        let _ = events.send(message);
        Ok(())
    })
}

#[cfg(target_os = "windows")]
/// 在复制快捷键触发时读取鼠标物理坐标，避免沿用启动时的默认零坐标。
fn current_system_cursor_position() -> Option<(f64, f64)> {
    let mut point = windows_sys::Win32::Foundation::POINT { x: 0, y: 0 };
    let found =
        unsafe { windows_sys::Win32::UI::WindowsAndMessaging::GetCursorPos(&mut point) != 0 };
    found.then_some((point.x as f64, point.y as f64))
}

#[cfg(not(target_os = "windows"))]
/// 非 Windows 平台继续使用输入监听记录的鼠标坐标。
fn current_system_cursor_position() -> Option<(f64, f64)> {
    None
}

#[cfg(target_os = "windows")]
/// 读取 Windows 当前修饰键状态，修复输入监听状态可能发生的偏差。
fn current_system_modifiers() -> Option<(bool, bool, bool)> {
    use windows_sys::Win32::UI::Input::KeyboardAndMouse::{
        GetAsyncKeyState, VK_CONTROL, VK_MENU, VK_SHIFT,
    };
    Some(unsafe {
        (
            GetAsyncKeyState(VK_CONTROL as i32) < 0,
            GetAsyncKeyState(VK_SHIFT as i32) < 0,
            GetAsyncKeyState(VK_MENU as i32) < 0,
        )
    })
}

#[cfg(not(target_os = "windows"))]
/// 非 Windows 平台使用输入事件累计的修饰键状态。
fn current_system_modifiers() -> Option<(bool, bool, bool)> {
    None
}

pub(super) struct CopyDispatcher {
    timer: slint::Timer,
    generation: Arc<AtomicU64>,
    events: Sender<AppEvent>,
}

impl CopyDispatcher {
    /// 创建仅在 UI 线程调度的复制定时器和跨线程请求代次。
    pub fn new(events: Sender<AppEvent>) -> Self {
        Self {
            timer: slint::Timer::default(),
            generation: Arc::new(AtomicU64::new(0)),
            events,
        }
    }

    /// 合并连续复制动作，等待源应用写入后在后台读取一次文本。
    pub fn request(&self, position: (f64, f64)) {
        let generation = self.generation.fetch_add(1, Ordering::SeqCst) + 1;
        let current = Arc::clone(&self.generation);
        let events = self.events.clone();
        self.timer.start(
            slint::TimerMode::SingleShot,
            Duration::from_millis(200),
            move || {
                let current = Arc::clone(&current);
                let events = events.clone();
                std::thread::spawn(move || {
                    if current.load(Ordering::SeqCst) != generation {
                        return;
                    }
                    let text = Clipboard::new()
                        .ok()
                        .and_then(|mut clipboard| clipboard.get_text().ok())
                        .map(|text| text.trim().to_string())
                        .filter(|text| !text.is_empty());
                    if let Some(text) = text {
                        let _ = events.send(AppEvent::CopiedText {
                            generation,
                            text,
                            position,
                        });
                    }
                });
            },
        );
    }

    /// 在 UI 消费结果时再次检查代次，防止旧读取覆盖后一次复制。
    pub fn is_current(&self, generation: u64) -> bool {
        self.generation.load(Ordering::SeqCst) == generation
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// 验证复制与历史快捷键互斥，普通输入和松键不会触发动作。
    #[test]
    fn distinguishes_copy_history_and_plain_input() {
        let mut state = ShortcutState::default();
        assert_eq!(state.action(&EventType::KeyPress(Key::KeyC)), None);
        state.update(&EventType::KeyPress(Key::ControlLeft));
        assert_eq!(
            state.action(&EventType::KeyPress(Key::KeyC)),
            Some(ShortcutAction::Copy)
        );
        state.update(&EventType::KeyPress(Key::ShiftRight));
        assert_eq!(
            state.action(&EventType::KeyPress(Key::KeyC)),
            Some(ShortcutAction::History)
        );
        assert_eq!(state.action(&EventType::KeyRelease(Key::KeyC)), None);
        state.update(&EventType::KeyRelease(Key::ControlLeft));
        assert_eq!(state.action(&EventType::KeyPress(Key::KeyC)), None);
    }

    /// 验证左右修饰键均可切换，长按不会连续触发，松开 X 后可以再次切换。
    #[test]
    fn switches_copy_mode_once_per_key_press() {
        for ctrl in [Key::ControlLeft, Key::ControlRight] {
            for alt in [Key::Alt, Key::AltGr] {
                let mut state = ShortcutState::default();
                state.update(&EventType::KeyPress(ctrl));
                state.update(&EventType::KeyPress(alt));
                assert_eq!(
                    state.action(&EventType::KeyPress(Key::KeyX)),
                    Some(ShortcutAction::SwitchCopyMode)
                );
                assert_eq!(state.action(&EventType::KeyPress(Key::KeyX)), None);
                assert_eq!(state.action(&EventType::KeyPress(Key::KeyC)), None);
                state.update(&EventType::KeyRelease(Key::KeyX));
                assert_eq!(state.action(&EventType::KeyRelease(Key::KeyX)), None);
                assert_eq!(
                    state.action(&EventType::KeyPress(Key::KeyX)),
                    Some(ShortcutAction::SwitchCopyMode)
                );
            }
        }
    }

    /// 验证切换仅匹配 Ctrl+Alt+X，缺少修饰键或额外按住 Shift 均不触发。
    #[test]
    fn copy_mode_switch_requires_exact_modifiers() {
        for ctrl in [false, true] {
            for alt in [false, true] {
                for shift in [false, true] {
                    let mut state = ShortcutState {
                        ctrl,
                        alt,
                        shift,
                        ..Default::default()
                    };
                    let expected =
                        (ctrl && alt && !shift).then_some(ShortcutAction::SwitchCopyMode);
                    assert_eq!(state.action(&EventType::KeyPress(Key::KeyX)), expected);
                }
            }
        }
    }
}
