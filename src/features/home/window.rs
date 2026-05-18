//! Slint window setup and callbacks for the application home page.

use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex};

use slint::{CloseRequestResponse, ComponentHandle};

use crate::assets::load_slint_image;
use crate::features::clipboard_history::history::ClipboardHistory;
use crate::features::clipboard_history::window::show_clipboard_history_window;
use crate::features::file_preview::window::show_empty_file_preview_window;
use crate::features::text_translation::window::show_translation_pending;
use crate::platform::window::{activate_slint_window, hide_taskbar_icon};
use crate::{
    ClipboardHistoryWindow, FilePreviewWindow, HomeWindow, TextTranslationWindow, TimeTrans,
};

/// Builds the home window and binds each tool tile to the matching feature window.
pub fn init_home_window(
    time_trans_window: &TimeTrans,
    clipboard_history_window: &ClipboardHistoryWindow,
    clipboard_history: Arc<Mutex<ClipboardHistory>>,
    text_translation_window: &TextTranslationWindow,
    translation_cancel_generation: Arc<AtomicU64>,
    file_preview_window: &FilePreviewWindow,
) -> HomeWindow {
    let window = HomeWindow::new().unwrap();
    window.set_tool_icon(load_app_icon());

    window
        .window()
        .on_close_requested(|| CloseRequestResponse::HideWindow);

    let weak_time = time_trans_window.as_weak();
    window.on_open_time_trans(move || {
        if let Some(ui) = weak_time.upgrade() {
            let _ = ui.show();
            hide_taskbar_icon(&ui);
            activate_slint_window(&ui);
        }
    });

    let weak_history = clipboard_history_window.as_weak();
    window.on_open_clipboard_history(move || {
        if let Some(ui) = weak_history.upgrade() {
            show_clipboard_history_window(&ui, &clipboard_history);
        }
    });

    let weak_translation = text_translation_window.as_weak();
    window.on_open_text_translation(move || {
        translation_cancel_generation.fetch_add(1, Ordering::SeqCst);
        if let Some(ui) = weak_translation.upgrade() {
            show_translation_pending(&ui, "");
            ui.set_translating(false);
            activate_slint_window(&ui);
        }
    });

    let weak_preview = file_preview_window.as_weak();
    window.on_open_file_preview(move || {
        if let Some(ui) = weak_preview.upgrade() {
            show_empty_file_preview_window(&ui);
            activate_slint_window(&ui);
        }
    });

    window
}

/// Shows the home page without creating a new application instance.
pub fn show_home_window(window: &HomeWindow) {
    if window.window().is_visible() {
        activate_slint_window(window);
        return;
    }

    let _ = window.show();
    activate_slint_window(window);
}

fn load_app_icon() -> slint::Image {
    const ICON_IMG: &[u8] = include_bytes!("../../../assets/icons/icon.png");
    load_slint_image(ICON_IMG)
}
