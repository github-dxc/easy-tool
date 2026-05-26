//! Slint window setup for displaying source and translated text.

use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use slint::{CloseRequestResponse, ComponentHandle, Timer, TimerMode};

use crate::TextTranslationWindow;
use crate::features::text_translation::translator::TranslationService;
use crate::settings::AppSettings;

/// Builds the text translation result window.
pub fn init_text_translation_window(
    cancel_generation: Arc<AtomicU64>,
    settings: Arc<Mutex<AppSettings>>,
    translation_service: Arc<TranslationService>,
) -> TextTranslationWindow {
    let window = TextTranslationWindow::new().unwrap();
    let debounce_timer = Arc::new(Timer::default());

    let close_cancel_generation = Arc::clone(&cancel_generation);
    window.window().on_close_requested(move || {
        close_cancel_generation.fetch_add(1, Ordering::SeqCst);
        CloseRequestResponse::HideWindow
    });

    {
        let weak_window = window.as_weak();
        let debounce_timer = Arc::clone(&debounce_timer);
        let cancel_generation = Arc::clone(&cancel_generation);
        let settings = Arc::clone(&settings);
        let translation_service = Arc::clone(&translation_service);
        window.on_source_text_edited(move |source_text| {
            let source_text = source_text.trim().to_string();
            cancel_generation.fetch_add(1, Ordering::SeqCst);

            if source_text.is_empty() {
                if let Some(window) = weak_window.upgrade() {
                    window.set_translated_text("".into());
                    window.set_translating(false);
                }
                return;
            }

            let debounce_seconds = settings.lock().unwrap().text_translation.debounce_seconds;
            let weak_window = weak_window.clone();
            let cancel_generation = Arc::clone(&cancel_generation);
            let translation_service = Arc::clone(&translation_service);
            debounce_timer.start(
                TimerMode::SingleShot,
                Duration::from_secs(debounce_seconds),
                move || {
                    trigger_translation(
                        weak_window.clone(),
                        Arc::clone(&cancel_generation),
                        Arc::clone(&translation_service),
                        source_text.clone(),
                    );
                },
            );
        });
    }

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

pub fn trigger_translation(
    weak_window: slint::Weak<TextTranslationWindow>,
    cancel_generation: Arc<AtomicU64>,
    translation_service: Arc<TranslationService>,
    source_text: String,
) {
    let source_text = source_text.trim().to_string();
    if source_text.is_empty() {
        if let Some(window) = weak_window.upgrade() {
            window.set_translated_text("".into());
            window.set_translating(false);
        }
        return;
    }

    std::thread::spawn(move || {
        let translation_run_id = cancel_generation.fetch_add(1, Ordering::SeqCst) + 1;

        let pending_source = source_text.clone();
        if let Err(err) = weak_window.upgrade_in_event_loop(move |window| {
            show_translation_pending(&window, &pending_source);
        }) {
            log::error!("show translation window failed: {err}");
            return;
        }

        let partial_window = weak_window.clone();
        let partial_cancel_generation = Arc::clone(&cancel_generation);
        let translated_text = translation_service
            .translate_streaming_cancellable(
                &source_text,
                move |partial_text| {
                    if partial_cancel_generation.load(Ordering::SeqCst) != translation_run_id {
                        return;
                    }

                    let partial_text = partial_text.to_string();
                    if let Err(err) = partial_window.upgrade_in_event_loop(move |window| {
                        show_translation_partial(&window, &partial_text);
                    }) {
                        log::error!("show partial translation failed: {err}");
                    }
                },
                || cancel_generation.load(Ordering::SeqCst) != translation_run_id,
            )
            .unwrap_or_else(|err| format!("缈昏瘧澶辫触: {err}"));

        if cancel_generation.load(Ordering::SeqCst) != translation_run_id {
            return;
        }

        if let Err(err) = weak_window.upgrade_in_event_loop(move |window| {
            show_translation_result(&window, &translated_text);
        }) {
            log::error!("show translation result failed: {err}");
        }
    });
}
