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
use crate::features::clipboard_history::store::load_history;
use crate::features::clipboard_history::window::{
    init_clipboard_history_window, show_clipboard_history_window,
};
use crate::features::file_preview::ocr::OcrService;
use crate::features::file_preview::registry::register_file_context_menu;
use crate::features::file_preview::window::{init_file_preview_window, show_file_preview_window};
use crate::features::home::window::{init_home_window, show_home_window};
use crate::features::screenshot::window::{
    cancel_screenshot_window, init_screenshot_window, show_screenshot_window,
};
use crate::features::settings::window::init_settings_window;
use crate::features::text_translation::translator::TranslationService;
use crate::features::text_translation::window::{
    init_text_translation_window, trigger_translation,
};
use crate::features::time_trans::window::init_time_trans_window;
use crate::infrastructure::clipboard_listener::start_clipboard_history_listener;
use crate::infrastructure::global_input::start_global_input_listener;
use crate::infrastructure::logging::init_logging;
use crate::infrastructure::paths::clipboard_history_dir;
use crate::infrastructure::tray::{init_tray_icon, start_tray_event_pump};
use crate::platform::dialog::show_message_box;
use crate::platform::window::{display_size, set_position};
use crate::settings::SettingsStore;

/// Starts the app, enforces a single instance, initializes features, and enters the Slint loop.
pub fn run() {
    if let Some(path) = std::env::args_os().nth(1) {
        init_logging().expect("failed to initialize logging");
        let settings_store = SettingsStore::new();
        let settings = Arc::new(Mutex::new(
            settings_store
                .load_or_create()
                .expect("failed to load settings"),
        ));
        let ocr_service = Arc::new(OcrService::new(&settings.lock().unwrap().image_recognition));
        let file_preview_window = init_file_preview_window(true, Arc::clone(&ocr_service));
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
    let translation_service = Arc::new(TranslationService::new(
        &settings.lock().unwrap().text_translation,
    ));
    let ocr_service = Arc::new(OcrService::new(&settings.lock().unwrap().image_recognition));
    let text_translation_window = init_text_translation_window(
        Arc::clone(&translation_cancel_generation),
        Arc::clone(&settings),
        Arc::clone(&translation_service),
    );
    let weak_translation_window = text_translation_window.as_weak();
    let clipboard_history_dir = clipboard_history_dir();
    let clipboard_history = Arc::new(Mutex::new(
        load_history(&clipboard_history_dir).unwrap_or_else(|err| {
            log::error!("load clipboard history failed: {err}");
            ClipboardHistory::default()
        }),
    ));
    let suppress_shortcuts = Arc::new(AtomicBool::new(false));
    let suppress_next_clipboard_history = Arc::new(Mutex::new(None));
    let clipboard_history_window = init_clipboard_history_window(
        Arc::clone(&clipboard_history),
        clipboard_history_dir.clone(),
        Arc::clone(&suppress_shortcuts),
        Arc::clone(&suppress_next_clipboard_history),
    );
    let file_preview_window = init_file_preview_window(false, Arc::clone(&ocr_service));
    let screenshot_window = init_screenshot_window();
    let tray_state = init_tray_icon(&settings.lock().unwrap());
    let settings_window = init_settings_window(
        Arc::clone(&settings),
        settings_store.clone(),
        Arc::clone(&translation_service),
        Arc::clone(&ocr_service),
        tray_state.menu_handles(),
    );
    let home_window = init_home_window(
        &time_trans_window,
        &clipboard_history_window,
        Arc::clone(&clipboard_history),
        &text_translation_window,
        Arc::clone(&translation_cancel_generation),
        &file_preview_window,
        &screenshot_window,
        &settings_window,
        Arc::clone(&settings),
    );
    let weak_history_window = clipboard_history_window.as_weak();
    start_clipboard_history_listener(
        Arc::clone(&clipboard_history),
        Arc::clone(&settings),
        weak_history_window.clone(),
        Arc::clone(&suppress_next_clipboard_history),
        clipboard_history_dir,
    )
    .expect("failed to start clipboard history listener");

    let mouse_x = Arc::new(Mutex::new(0f64));
    let mouse_y = Arc::new(Mutex::new(0f64));
    let shortcut_state = Arc::new(Mutex::new(ShortcutState::default()));
    let weak_screenshot_window = screenshot_window.as_weak();
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
        let weak_screenshot_window = weak_screenshot_window.clone();

        move |event| {
            if let EventType::MouseMove { x, y } = event.event_type {
                *mouse_x.lock().unwrap() = x;
                *mouse_y.lock().unwrap() = y;
            }

            // update shortcut_state
            update_shortcut_state(&shortcut_state, &event.event_type);

            if matches!(event.event_type, EventType::KeyPress(Key::Escape)) {
                let _ = weak_screenshot_window.upgrade_in_event_loop(move |window| {
                    if window.window().is_visible() {
                        cancel_screenshot_window(&window);
                    }
                });
            }

            if should_show_history(&shortcut_state, &event.event_type)
                && !suppress_shortcuts.load(Ordering::SeqCst)
            {
                if settings.lock().unwrap().clipboard_history.enabled {
                    let history = Arc::clone(&clipboard_history);
                    weak_history_window
                        .upgrade_in_event_loop(move |window| {
                            show_clipboard_history_window(&window, &history);
                        })
                        .expect("failed to show clipboard history window");
                }
            }

            if should_show_screenshot(&shortcut_state, &event.event_type)
                && !suppress_shortcuts.load(Ordering::SeqCst)
            {
                if settings.lock().unwrap().screenshot.enabled {
                    weak_screenshot_window
                        .upgrade_in_event_loop(move |window| {
                            show_screenshot_window(&window);
                        })
                        .expect("failed to show screenshot window");
                }
            }

            if event.name.as_deref() == Some("\u{3}") {
                let settings_snapshot = settings.lock().unwrap().clone();
                if settings_snapshot.text_translation.enabled {
                    let translation_service = Arc::clone(&translation_service);
                    let translation_cancel_generation = Arc::clone(&translation_cancel_generation);
                    let weak_translation_window = weak_translation_window.clone();
                    std::thread::spawn(move || {
                        std::thread::sleep(Duration::from_millis(200));

                        let source_text = Clipboard::new()
                            .ok()
                            .and_then(|mut clipboard| clipboard.get_text().ok())
                            .map(|text| text.trim().to_string())
                            .filter(|text| !text.is_empty());
                        let Some(source_text) = source_text else {
                            return;
                        };

                        trigger_translation(
                            weak_translation_window,
                            translation_cancel_generation,
                            translation_service,
                            source_text,
                        );
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
        {
            let weak_home_window = home_window.as_weak();
            move || {
                if let Some(window) = weak_home_window.upgrade() {
                    show_home_window(&window);
                }
            }
        },
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

// Tracks modifier state for global shortcuts.
#[derive(Default)]
struct ShortcutState {
    ctrl: bool,
    alt: bool,
    shift: bool,
    c: bool,
    z: bool,
}

fn update_shortcut_state(shortcut_state: &Arc<Mutex<ShortcutState>>, event_type: &EventType) {
    let mut state = shortcut_state.lock().unwrap();

    match event_type {
        EventType::KeyPress(Key::ControlLeft | Key::ControlRight) => state.ctrl = true,
        EventType::KeyRelease(Key::ControlLeft | Key::ControlRight) => state.ctrl = false,
        EventType::KeyPress(Key::Alt | Key::AltGr) => state.alt = true,
        EventType::KeyRelease(Key::Alt | Key::AltGr) => state.alt = false,
        EventType::KeyPress(Key::ShiftLeft | Key::ShiftRight) => state.shift = true,
        EventType::KeyRelease(Key::ShiftLeft | Key::ShiftRight) => state.shift = false,
        EventType::KeyPress(Key::KeyC) => state.c = true,
        EventType::KeyRelease(Key::KeyC) => state.c = false,
        EventType::KeyPress(Key::KeyZ) => state.z = true,
        EventType::KeyRelease(Key::KeyZ) => state.z = false,
        _ => {}
    }
}

fn should_show_history(shortcut_state: &Arc<Mutex<ShortcutState>>, event_type: &EventType) -> bool {
    if !matches!(event_type, EventType::KeyPress(Key::KeyC)) {
        return false;
    }

    if let Some(modifiers) = current_system_modifiers() {
        return modifiers.ctrl && modifiers.shift;
    }

    let state = shortcut_state.lock().unwrap();
    state.ctrl && state.shift
}

fn should_show_screenshot(
    shortcut_state: &Arc<Mutex<ShortcutState>>,
    event_type: &EventType,
) -> bool {
    if !matches!(event_type, EventType::KeyPress(Key::KeyZ)) {
        return false;
    }

    if let Some(modifiers) = current_system_modifiers() {
        return modifiers.alt && modifiers.shift;
    }

    let state = shortcut_state.lock().unwrap();
    state.alt && state.shift
}

#[derive(Clone, Copy)]
struct ShortcutModifiers {
    ctrl: bool,
    alt: bool,
    shift: bool,
}

#[cfg(target_os = "windows")]
fn current_system_modifiers() -> Option<ShortcutModifiers> {
    use windows_sys::Win32::UI::Input::KeyboardAndMouse::{
        GetAsyncKeyState, VK_CONTROL, VK_MENU, VK_SHIFT,
    };

    const KEY_IS_DOWN: i16 = i16::MIN;

    Some(ShortcutModifiers {
        ctrl: unsafe { GetAsyncKeyState(VK_CONTROL as i32) & KEY_IS_DOWN != 0 },
        alt: unsafe { GetAsyncKeyState(VK_MENU as i32) & KEY_IS_DOWN != 0 },
        shift: unsafe { GetAsyncKeyState(VK_SHIFT as i32) & KEY_IS_DOWN != 0 },
    })
}

#[cfg(not(target_os = "windows"))]
fn current_system_modifiers() -> Option<ShortcutModifiers> {
    None
}
