//! Slint window setup for Base64 encoding and decoding.

use base64::Engine;
use slint::{CloseRequestResponse, ComponentHandle};

use crate::Base64ConvertWindow;

/// Builds the Base64 conversion window.
pub fn init_base64_convert_window() -> Base64ConvertWindow {
    let window = Base64ConvertWindow::new().unwrap();

    window.window().on_close_requested(|| CloseRequestResponse::HideWindow);

    window.on_source_text_edited({
        let weak = window.as_weak();
        move |source_text| {
            if let Some(window) = weak.upgrade() {
                convert_and_update(&window, source_text.as_str());
            }
        }
    });

    window.on_toggle_mode({
        let weak = window.as_weak();
        move || {
            if let Some(window) = weak.upgrade() {
                let new_mode = !window.get_encode_mode();
                window.set_encode_mode(new_mode);
                let source = window.get_source_text().to_string();
                convert_and_update(&window, &source);
            }
        }
    });

    window
}

/// Shows the Base64 conversion window and resets state.
pub fn show_base64_convert_window(window: &Base64ConvertWindow) {
    window.set_source_text("".into());
    window.set_result_text("".into());
    window.set_encode_mode(true);
    let _ = window.show();
}

fn convert_and_update(window: &Base64ConvertWindow, source: &str) {
    let trimmed = source.trim();
    if trimmed.is_empty() {
        window.set_result_text("".into());
        return;
    }

    let result = if window.get_encode_mode() {
        base64::engine::general_purpose::STANDARD.encode(trimmed.as_bytes())
    } else {
        match base64::engine::general_purpose::STANDARD.decode(trimmed) {
            Ok(bytes) => String::from_utf8_lossy(&bytes).to_string(),
            Err(err) => format!("[解码失败: {err}]"),
        }
    };

    window.set_result_text(result.into());
}
