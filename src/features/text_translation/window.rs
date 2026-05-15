//! Slint window setup for displaying source and translated text.

use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};

use crate::TextTranslationWindow;
use slint::{CloseRequestResponse, ComponentHandle};

/// Builds the text translation result window.
pub fn init_text_translation_window(cancel_generation: Arc<AtomicU64>) -> TextTranslationWindow {
    let window = TextTranslationWindow::new().unwrap();

    window.window().on_close_requested(move || {
        cancel_generation.fetch_add(1, Ordering::SeqCst);
        CloseRequestResponse::HideWindow
    });

    window
}

/// Shows the translation window with a pending result state.
pub fn show_translation_pending(window: &TextTranslationWindow, source_text: &str) {
    window.set_source_text(source_text.into());
    window.set_translated_text("".into());
    window.set_translating(true);
    let _ = window.show();
}

/// Updates the translation window after inference finishes.
pub fn show_translation_result(window: &TextTranslationWindow, translated_text: &str) {
    window.set_translated_text(translated_text.into());
    window.set_translating(false);
}

/// Updates the translation text while inference is still generating tokens.
pub fn show_translation_partial(window: &TextTranslationWindow, translated_text: &str) {
    window.set_translated_text(translated_text.into());
    window.set_translating(true);
}
