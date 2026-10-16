//! 文本翻译窗口绑定，负责防抖输入和主线程界面更新。

use super::language::detect_translation_direction;
use super::requests::TranslationRequests;
use super::translator::TranslationService;
use crate::TextTranslationWindow;
use crate::platform::window::show_centered_window;
use crate::settings::AppSettings;
use slint::{CloseRequestResponse, ComponentHandle, Timer, TimerMode};
use std::rc::Rc;
use std::sync::{Arc, Mutex};
use std::time::Duration;

/// 创建翻译页面，清空或关闭时同时取消定时器与进行中的结果更新。
pub fn init_text_translation_window(
    requests: Arc<TranslationRequests>,
    settings: Arc<Mutex<AppSettings>>,
    translation_service: Arc<TranslationService>,
) -> TextTranslationWindow {
    let window = TextTranslationWindow::new().unwrap();
    // Slint 定时器仅在 UI 线程使用，无需 Arc 或跨线程互斥。
    let debounce_timer = Rc::new(Timer::default());
    let close_requests = Arc::clone(&requests);
    let close_timer = Rc::clone(&debounce_timer);
    window.window().on_close_requested(move || {
        close_timer.stop();
        close_requests.cancel();
        CloseRequestResponse::HideWindow
    });
    let weak_window = window.as_weak();
    window.on_source_text_edited(move |source_text| {
        debounce_timer.stop();
        let generation = requests.begin();
        let source_text = source_text.trim().to_string();
        if source_text.is_empty() {
            if let Some(window) = weak_window.upgrade() {
                show_translation_result(&window, "");
            }
            return;
        }
        let debounce_seconds = settings.lock().unwrap().text_translation.debounce_seconds;
        let weak_window = weak_window.clone();
        let requests = Arc::clone(&requests);
        let translation_service = Arc::clone(&translation_service);
        debounce_timer.start(
            TimerMode::SingleShot,
            Duration::from_secs(debounce_seconds),
            move || {
                // 手动打开编辑器或关闭功能也会使代次失效，过期任务不得发起 API 请求。
                if requests.is_current(generation) {
                    trigger_translation(
                        weak_window.clone(),
                        Arc::clone(&requests),
                        Arc::clone(&translation_service),
                        source_text.clone(),
                    );
                }
            },
        );
    });
    window
}

/// 从主页打开空白翻译编辑器，先居中再显示，避免加载状态提前显示窗口。
pub fn show_translation_editor(window: &TextTranslationWindow) {
    window.set_source_text("".into());
    window.set_translated_text("".into());
    window.set_translating(false);
    show_centered_window(
        window,
        slint::LogicalSize::new(window.get_initial_width(), window.get_initial_height()),
    );
}

/// 展示当前输入及加载状态；调用方必须位于 UI 线程。
pub fn show_translation_pending(window: &TextTranslationWindow, source_text: &str) {
    window.set_source_text(source_text.into());
    window.set_translated_text("".into());
    window.set_translating(true);
    let _ = window.show();
}

/// 更新译文并结束加载状态，不额外改变窗口可见性。
pub fn show_translation_result(window: &TextTranslationWindow, translated_text: &str) {
    window.set_translated_text(translated_text.into());
    window.set_translating(false);
}

/// 在 UI 线程登记请求，在后台调用 API，仅允许最新请求更新译文。
pub fn trigger_translation(
    weak_window: slint::Weak<TextTranslationWindow>,
    requests: Arc<TranslationRequests>,
    translation_service: Arc<TranslationService>,
    source_text: String,
) {
    // 代次必须在启动后台线程前分配，避免线程调度顺序颠倒新旧请求。
    let generation = requests.begin();
    let source_text = source_text.trim().to_string();
    let Some(window) = weak_window.upgrade() else {
        return;
    };
    if detect_translation_direction(&source_text).is_none() {
        show_translation_result(&window, "");
        return;
    }
    show_translation_pending(&window, &source_text);
    std::thread::spawn(move || {
        let translated_text = translation_service
            .translate_cancellable(&source_text, || !requests.is_current(generation))
            .unwrap_or_else(|err| format!("翻译失败：{err}"));
        if !requests.is_current(generation) {
            return;
        }
        if let Err(err) = weak_window.upgrade_in_event_loop(move |window| {
            // 排队等待 UI 执行时也可能出现新输入，因此落到 UI 线程后再检查一次。
            if requests.is_current(generation) {
                show_translation_result(&window, &translated_text);
            }
        }) {
            log::error!("show translation result failed: {err}");
        }
    });
}
