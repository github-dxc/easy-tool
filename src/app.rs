use std::sync::atomic::{AtomicBool, Ordering};
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
use crate::features::time_trans::window::init_time_trans_window;
use crate::infrastructure::clipboard_listener::start_clipboard_history_listener;
use crate::infrastructure::global_input::start_global_input_listener;
use crate::infrastructure::logging::init_logging;
use crate::infrastructure::tray::{init_tray_icon, start_tray_event_pump};
use crate::platform::dialog::show_message_box;
use crate::platform::window::{display_size, foreground_window_handle, set_position};
use crate::settings::SettingsStore;

pub fn run() {
    let instance =
        SingleInstance::new(APP_INSTANCE_ID).expect("failed to create single instance lock");
    if !instance.is_single() {
        show_message_box("提示", "应用已经在运行中，程序即将退出。");
        return;
    }

    init_logging().expect("failed to initialize logging");

    let settings_store = SettingsStore::new();
    let settings = Arc::new(Mutex::new(
        settings_store
            .load_or_create()
            .expect("failed to load settings"),
    ));
    info!("settings path: {}", settings_store.path().display());

    let time_trans_window = init_time_trans_window();
    let weak_window = time_trans_window.as_weak();
    let clipboard_history = Arc::new(Mutex::new(ClipboardHistory::default()));
    let paste_target_window = Arc::new(Mutex::new(None));
    let suppress_shortcuts = Arc::new(AtomicBool::new(false));
    let clipboard_history_window = init_clipboard_history_window(
        Arc::clone(&clipboard_history),
        Arc::clone(&paste_target_window),
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
        let paste_target_window = Arc::clone(&paste_target_window);
        let suppress_shortcuts = Arc::clone(&suppress_shortcuts);

        move |event| {
            if let EventType::MouseMove { x, y } = event.event_type {
                *mouse_x.lock().unwrap() = x;
                *mouse_y.lock().unwrap() = y;
            }

            let should_show_history = update_shortcut_state(&shortcut_state, &event.event_type);
            if should_show_history && !suppress_shortcuts.load(Ordering::SeqCst) {
                shortcut_state.lock().unwrap().clear();
                if settings.lock().unwrap().clipboard_history.enabled {
                    *paste_target_window.lock().unwrap() = foreground_window_handle();
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

    let _tray_timer = start_tray_event_pump(&tray_state, Arc::clone(&settings), settings_store);
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
