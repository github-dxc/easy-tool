//! Slint window setup and callbacks for the application home page.

use slint::{CloseRequestResponse, ComponentHandle};

use crate::HomeWindow;
use crate::platform::window::{activate_slint_window, show_centered_window};

/// 创建主页并绑定各工具及设置入口的回调。
pub fn init_home_window(
    open_time_trans: impl Fn() + 'static,
    open_clipboard_history: impl Fn() + 'static,
    open_text_translation: impl Fn() + 'static,
    open_settings: impl Fn() + 'static,
) -> HomeWindow {
    let window = HomeWindow::new().unwrap();
    window
        .window()
        .on_close_requested(|| CloseRequestResponse::HideWindow);

    window.on_open_time_trans(open_time_trans);
    window.on_open_clipboard_history(open_clipboard_history);
    window.on_open_text_translation(open_text_translation);
    window.on_open_settings(open_settings);

    window
}

/// 在隐藏状态完成居中后显示主页，已显示时只激活以保留当前位置。
pub fn show_home_window(window: &HomeWindow) {
    if window.window().is_visible() {
        activate_slint_window(window);
        return;
    }

    show_centered_window(
        window,
        slint::LogicalSize::new(window.get_initial_width(), window.get_initial_height()),
    );
}
