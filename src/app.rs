//! Application bootstrap and cross-feature event wiring.

use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use arboard::Clipboard;
use log::info;
use rdev::{EventType, Key};
use single_instance::SingleInstance;
use slint::ComponentHandle;

use crate::config::APP_INSTANCE_ID;
use crate::features::clipboard_history::history::ClipboardHistory;
use crate::features::clipboard_history::window::{
    init_clipboard_history_window, show_clipboard_history_window,
};
use crate::features::file_preview::registry::register_file_context_menu;
use crate::features::file_preview::window::{init_file_preview_window, show_file_preview_window};
use crate::features::text_translation::translator::TranslationService;
use crate::features::text_translation::window::{
    init_text_translation_window, show_translation_partial, show_translation_pending,
    show_translation_result,
};
use crate::features::time_trans::window::init_time_trans_window;
use crate::infrastructure::clipboard_listener::start_clipboard_history_listener;
use crate::infrastructure::global_input::start_global_input_listener;
use crate::infrastructure::logging::init_logging;
use crate::infrastructure::tray::{init_tray_icon, start_tray_event_pump};
use crate::platform::dialog::show_message_box;
use crate::platform::window::{display_size, set_position};
use crate::settings::SettingsStore;

/// Starts the app, enforces a single instance, initializes features, and enters the Slint loop.
pub fn run() {
    if let Some(path) = std::env::args_os().nth(1) {
        init_logging().expect("failed to initialize logging");
        let file_preview_window = init_file_preview_window(true);
        show_file_preview_window(&file_preview_window, path.into());
        slint::run_event_loop_until_quit().unwrap();
        return;
    }

    let instance =
        SingleInstance::new(APP_INSTANCE_ID).expect("failed to create single instance lock");
    if !instance.is_single() {
        show_message_box("提示", "应用已经在运行中，程序即将退出。");
        return;
    }

    init_logging().expect("failed to initialize logging");
    if let Err(err) = register_file_context_menu() {
        log::error!("register file context menu failed: {err}");
    }

    let settings_store = SettingsStore::new();
    let settings = Arc::new(Mutex::new(
        settings_store
            .load_or_create()
            .expect("failed to load settings"),
    ));
    info!("settings path: {}", settings_store.path().display());

    let time_trans_window = init_time_trans_window();
    let weak_window = time_trans_window.as_weak();
    let translation_cancel_generation = Arc::new(AtomicU64::new(0));
    let text_translation_window =
        init_text_translation_window(Arc::clone(&translation_cancel_generation));
    let weak_translation_window = text_translation_window.as_weak();
    let clipboard_history = Arc::new(Mutex::new(ClipboardHistory::default()));
    let suppress_shortcuts = Arc::new(AtomicBool::new(false));
    let translation_service = Arc::new(TranslationService::new(
        &settings.lock().unwrap().text_translation,
    ));
    let clipboard_history_window = init_clipboard_history_window(
        Arc::clone(&clipboard_history),
        Arc::clone(&suppress_shortcuts),
    );
    let weak_history_window = clipboard_history_window.as_weak();
    let tray_state = init_tray_icon(&settings.lock().unwrap());
    start_clipboard_history_listener(
        Arc::clone(&clipboard_history),
        Arc::clone(&settings),
        weak_history_window.clone(),
    )
    .expect("failed to start clipboard history listener");

    let mouse_x = Arc::new(Mutex::new(0f64));
    let mouse_y = Arc::new(Mutex::new(0f64));
    let shortcut_state = Arc::new(Mutex::new(ShortcutState::default()));
    start_global_input_listener({
        let mouse_x = Arc::clone(&mouse_x);
        let mouse_y = Arc::clone(&mouse_y);
        let settings = Arc::clone(&settings);
        let clipboard_history = Arc::clone(&clipboard_history);
        let shortcut_state = Arc::clone(&shortcut_state);
        let suppress_shortcuts = Arc::clone(&suppress_shortcuts);
        let translation_service = Arc::clone(&translation_service);
        let translation_cancel_generation = Arc::clone(&translation_cancel_generation);
        let weak_translation_window = weak_translation_window.clone();

        move |event| {
            if let EventType::MouseMove { x, y } = event.event_type {
                *mouse_x.lock().unwrap() = x;
                *mouse_y.lock().unwrap() = y;
            }

            let should_show_history = update_shortcut_state(&shortcut_state, &event.event_type);
            if should_show_history && !suppress_shortcuts.load(Ordering::SeqCst) {
                shortcut_state.lock().unwrap().clear();
                if settings.lock().unwrap().clipboard_history.enabled {
                    let history = Arc::clone(&clipboard_history);
                    weak_history_window
                        .upgrade_in_event_loop(move |window| {
                            show_clipboard_history_window(&window, &history);
                        })
                        .expect("failed to show clipboard history window");
                }
            }

            if event.name.as_deref() == Some("\u{3}") {
                let settings_snapshot = settings.lock().unwrap().clone();
                if settings_snapshot.text_translation.enabled {
                    let translation_service = Arc::clone(&translation_service);
                    let translation_cancel_generation = Arc::clone(&translation_cancel_generation);
                    let weak_translation_window = weak_translation_window.clone();
                    std::thread::spawn(move || {
                        let translation_run_id =
                            translation_cancel_generation.fetch_add(1, Ordering::SeqCst) + 1;
                        std::thread::sleep(Duration::from_millis(200));

                        let source_text = Clipboard::new()
                            .ok()
                            .and_then(|mut clipboard| clipboard.get_text().ok())
                            .map(|text| text.trim().to_string())
                            .filter(|text| !text.is_empty());
                        let Some(source_text) = source_text else {
                            return;
                        };

                        let pending_source = source_text.clone();
                        if let Err(err) =
                            weak_translation_window.upgrade_in_event_loop(move |window| {
                                show_translation_pending(&window, &pending_source);
                            })
                        {
                            log::error!("show translation window failed: {err}");
                            return;
                        }

                        let partial_window = weak_translation_window.clone();
                        let partial_cancel_generation = Arc::clone(&translation_cancel_generation);
                        let translated_text = translation_service
                            .translate_streaming_cancellable(
                                &source_text,
                                move |partial_text| {
                                    if partial_cancel_generation.load(Ordering::SeqCst)
                                        != translation_run_id
                                    {
                                        return;
                                    }

                                    let partial_text = partial_text.to_string();
                                    if let Err(err) =
                                        partial_window.upgrade_in_event_loop(move |window| {
                                            show_translation_partial(&window, &partial_text);
                                        })
                                    {
                                        log::error!("show partial translation failed: {err}");
                                    }
                                },
                                || {
                                    translation_cancel_generation.load(Ordering::SeqCst)
                                        != translation_run_id
                                },
                            )
                            .unwrap_or_else(|err| format!("翻译失败: {err}"));

                        if translation_cancel_generation.load(Ordering::SeqCst)
                            != translation_run_id
                        {
                            return;
                        }

                        if let Err(err) =
                            weak_translation_window.upgrade_in_event_loop(move |window| {
                                show_translation_result(&window, &translated_text);
                            })
                        {
                            log::error!("show translation result failed: {err}");
                        }
                    });
                }

                if !settings_snapshot.copy_timestamp.enabled {
                    return Ok(());
                }

                let cur_x = *mouse_x.lock().unwrap();
                let cur_y = *mouse_y.lock().unwrap();
                weak_window
                    .upgrade_in_event_loop(move |window| {
                        std::thread::sleep(Duration::from_millis(200));

                        let mut clipboard = Clipboard::new().unwrap();
                        let text = clipboard.get_text().ok();

                        if let Some(text) = text {
                            window.set_input_value(text.trim().into());
                            window.set_close_time(3);

                            if !window.get_has_hover() {
                                let (move_x, move_y) = next_window_position(&window, cur_x, cur_y);
                                info!("set window pos to x:{move_x},y:{move_y},copy:{text}");
                                set_position(&window, move_x, move_y);
                            }
                        }
                    })
                    .expect("failed to send event to UI thread");
            }

            Ok(())
        }
    })
    .expect("failed to start global input listener");

    let _tray_timer = start_tray_event_pump(
        &tray_state,
        Arc::clone(&settings),
        settings_store,
        translation_service,
    );
    slint::run_event_loop_until_quit().unwrap();
}

fn next_window_position(window: &crate::TimeTrans, cur_x: f64, cur_y: f64) -> (f64, f64) {
    let mut move_x = cur_x;
    let mut move_y = cur_y;

    if let Some((disp_w, disp_h)) = display_size(window) {
        if move_x + 280f64 > disp_w {
            move_x = disp_w - 280f64;
        } else {
            move_x += 20f64;
        }

        if move_y + 135f64 > disp_h {
            move_y = disp_h - 135f64;
        } else {
            move_y += 10f64;
        }
    }

    (move_x, move_y)
}

// Tracks modifier state for the global Ctrl+Shift+C clipboard-history shortcut.
#[derive(Default)]
struct ShortcutState {
    ctrl: bool,
    shift: bool,
}

impl ShortcutState {
    fn clear(&mut self) {
        self.ctrl = false;
        self.shift = false;
    }
}

fn update_shortcut_state(
    shortcut_state: &Arc<Mutex<ShortcutState>>,
    event_type: &EventType,
) -> bool {
    let mut state = shortcut_state.lock().unwrap();

    match event_type {
        EventType::KeyPress(Key::ControlLeft | Key::ControlRight) => state.ctrl = true,
        EventType::KeyRelease(Key::ControlLeft | Key::ControlRight) => state.ctrl = false,
        EventType::KeyPress(Key::ShiftLeft | Key::ShiftRight) => state.shift = true,
        EventType::KeyRelease(Key::ShiftLeft | Key::ShiftRight) => state.shift = false,
        EventType::KeyPress(Key::KeyC) => return state.ctrl && state.shift,
        _ => {}
    }

    false
}
