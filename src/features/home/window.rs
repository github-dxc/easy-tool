//! Slint window setup and callbacks for the application home page.

use slint::{CloseRequestResponse, ComponentHandle};

use crate::HomeWindow;
use crate::platform::window::{activate_slint_window, center_window};

/// Builds the home window and binds each tool tile to the matching feature window.
pub fn init_home_window(
    open_time_trans: impl Fn() + 'static,
    open_clipboard_history: impl Fn() + 'static,
    open_text_translation: impl Fn() + 'static,
    open_file_preview: impl Fn() + 'static,
    open_screenshot: impl Fn() + 'static,
    open_settings: impl Fn() + 'static,
) -> HomeWindow {
    let window = HomeWindow::new().unwrap();
    window
        .window()
        .on_close_requested(|| CloseRequestResponse::HideWindow);

    window.on_open_time_trans(open_time_trans);
    window.on_open_clipboard_history(open_clipboard_history);
    window.on_open_text_translation(open_text_translation);
    window.on_open_file_preview(open_file_preview);
    window.on_open_screenshot(open_screenshot);
    window.on_open_settings(open_settings);

    window
}

/// Shows the home page without creating a new application instance.
pub fn show_home_window(window: &HomeWindow) {
    if window.window().is_visible() {
        activate_slint_window(window);
        return;
    }

    let _ = window.show();
    center_window(window);
    activate_slint_window(window);
}
