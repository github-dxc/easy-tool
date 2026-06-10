//! Application bootstrap and cross-feature event wiring.

use std::cell::RefCell;
use std::path::PathBuf;
use std::rc::Rc;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use arboard::Clipboard;
use log::info;
use rdev::{EventType, Key};
use single_instance::SingleInstance;
use slint::ComponentHandle;

use crate::config::APP_INSTANCE_ID;
use crate::features::clipboard_history::history::{ClipboardHistory, ClipboardHistoryItem};
use crate::features::clipboard_history::store::load_history;
use crate::features::clipboard_history::window::{
    init_clipboard_history_window, show_clipboard_history_window,
};
use crate::features::file_preview::ocr::OcrService;
use crate::features::file_preview::registry::register_file_context_menu;
use crate::features::file_preview::window::{
    init_file_preview_window, show_empty_file_preview_window, show_file_preview_window,
};
use crate::features::home::window::{init_home_window, show_home_window};
use crate::features::screenshot::window::{
    cancel_screenshot_window, init_screenshot_window, show_screenshot_window,
};
use crate::features::settings::window::{init_settings_window, show_settings_window};
use crate::features::text_translation::translator::TranslationService;
use crate::features::text_translation::window::{
    init_text_translation_window, show_translation_pending, trigger_translation,
};
use crate::features::time_trans::window::init_time_trans_window;
use crate::infrastructure::clipboard_listener::start_clipboard_history_listener;
use crate::infrastructure::global_input::start_global_input_listener;
use crate::infrastructure::logging::init_logging;
use crate::infrastructure::paths::clipboard_history_dir;
use crate::infrastructure::tray::{TrayMenuHandles, init_tray_icon, start_tray_event_pump};
use crate::platform::dialog::show_message_box;
use crate::platform::window::{
    activate_slint_window, display_size, set_position, show_without_taskbar_icon,
};
use crate::settings::{AppSettings, SettingsStore};

thread_local! {
    static UI_APP_WINDOWS: RefCell<Option<Rc<RefCell<AppWindows>>>> = const { RefCell::new(None) };
}

#[derive(Default)]
struct AppWindows {
    time_trans: Option<crate::TimeTrans>,
    history: Option<crate::ClipboardHistoryWindow>,
    translation: Option<crate::TextTranslationWindow>,
    file_preview: Option<crate::FilePreviewWindow>,
    screenshot: Option<crate::ScreenshotWindow>,
    settings: Option<crate::SettingsWindow>,
}

#[derive(Clone)]
struct ShortcutWindows {
    history: Arc<Mutex<Option<slint::Weak<crate::ClipboardHistoryWindow>>>>,
    screenshot: Arc<Mutex<Option<slint::Weak<crate::ScreenshotWindow>>>>,
    translation: Arc<Mutex<Option<slint::Weak<crate::TextTranslationWindow>>>>,
    time_trans: Arc<Mutex<Option<slint::Weak<crate::TimeTrans>>>>,
}

impl ShortcutWindows {
    fn new() -> Self {
        Self {
            history: Arc::new(Mutex::new(None)),
            screenshot: Arc::new(Mutex::new(None)),
            translation: Arc::new(Mutex::new(None)),
            time_trans: Arc::new(Mutex::new(None)),
        }
    }
}

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
        let settings_snapshot = settings.lock().unwrap().clone();
        let ocr_service = Arc::new(OcrService::new(
            &settings_snapshot.image_recognition,
            settings_snapshot.ai_backend,
            &settings_snapshot.tencent_cloud,
        ));
        let file_preview_window = init_file_preview_window(true, Arc::clone(&ocr_service));
        show_file_preview_window(&file_preview_window, path.into());
        let model_cleanup_timer = slint::Timer::default();
        model_cleanup_timer.start(slint::TimerMode::Repeated, Duration::from_secs(5), {
            let ocr_service = Arc::clone(&ocr_service);
            move || {
                ocr_service.unload_if_idle();
            }
        });
        let _model_cleanup_timer = model_cleanup_timer;
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

    let translation_cancel_generation = Arc::new(AtomicU64::new(0));
    let settings_snapshot = settings.lock().unwrap().clone();
    let translation_service = Arc::new(TranslationService::new(
        &settings_snapshot.text_translation,
        settings_snapshot.ai_backend,
        &settings_snapshot.tencent_cloud,
    ));
    let ocr_service = Arc::new(OcrService::new(
        &settings_snapshot.image_recognition,
        settings_snapshot.ai_backend,
        &settings_snapshot.tencent_cloud,
    ));
    let clipboard_history_dir = clipboard_history_dir();
    let clipboard_history = Arc::new(Mutex::new(
        load_history(&clipboard_history_dir).unwrap_or_else(|err| {
            log::error!("load clipboard history failed: {err}");
            ClipboardHistory::default()
        }),
    ));
    let suppress_shortcuts = Arc::new(AtomicBool::new(false));
    let suppress_next_clipboard_history = Arc::new(Mutex::new(None));
    let clipboard_listener_started = Arc::new(AtomicBool::new(false));
    let app_windows = Rc::new(RefCell::new(AppWindows::default()));
    register_ui_app_windows(Rc::clone(&app_windows));
    let shortcut_windows = ShortcutWindows::new();
    let tray_state = init_tray_icon(&settings.lock().unwrap());

    let on_settings_applied: Rc<dyn Fn(&AppSettings)> = Rc::new({
        let app_windows = Rc::clone(&app_windows);
        let shortcut_windows = shortcut_windows.clone();
        let clipboard_history = Arc::clone(&clipboard_history);
        let settings = Arc::clone(&settings);
        let clipboard_history_dir = clipboard_history_dir.clone();
        let suppress_shortcuts = Arc::clone(&suppress_shortcuts);
        let suppress_next_clipboard_history = Arc::clone(&suppress_next_clipboard_history);
        let clipboard_listener_started = Arc::clone(&clipboard_listener_started);
        let translation_cancel_generation = Arc::clone(&translation_cancel_generation);
        let translation_service = Arc::clone(&translation_service);

        move |settings_snapshot| {
            if settings_snapshot.copy_timestamp.enabled {
                ensure_time_trans_window(&app_windows, &shortcut_windows);
            }
            if settings_snapshot.clipboard_history.enabled {
                ensure_clipboard_history_window(
                    &app_windows,
                    &shortcut_windows,
                    Arc::clone(&clipboard_history),
                    clipboard_history_dir.clone(),
                    Arc::clone(&suppress_shortcuts),
                    Arc::clone(&suppress_next_clipboard_history),
                );
                ensure_clipboard_listener_started(
                    &clipboard_listener_started,
                    ClipboardListenerDeps {
                        history: Arc::clone(&clipboard_history),
                        settings: Arc::clone(&settings),
                        shortcut_windows: shortcut_windows.clone(),
                        suppress_next_clipboard_history: Arc::clone(
                            &suppress_next_clipboard_history,
                        ),
                        history_dir: clipboard_history_dir.clone(),
                    },
                );
            }
            if settings_snapshot.screenshot.enabled {
                ensure_screenshot_window(&app_windows, &shortcut_windows);
            }
            if settings_snapshot.text_translation.enabled {
                ensure_text_translation_window(
                    &app_windows,
                    &shortcut_windows,
                    Arc::clone(&translation_cancel_generation),
                    Arc::clone(&settings),
                    Arc::clone(&translation_service),
                );
            }
        }
    });

    on_settings_applied(&settings_snapshot);

    let home_window = init_home_window(
        {
            let app_windows = Rc::clone(&app_windows);
            let shortcut_windows = shortcut_windows.clone();
            move || {
                let weak = ensure_time_trans_window(&app_windows, &shortcut_windows);
                if let Some(window) = weak.upgrade() {
                    let _ = show_without_taskbar_icon(&window);
                    activate_slint_window(&window);
                }
            }
        },
        {
            let app_windows = Rc::clone(&app_windows);
            let shortcut_windows = shortcut_windows.clone();
            let clipboard_history = Arc::clone(&clipboard_history);
            let clipboard_history_dir = clipboard_history_dir.clone();
            let suppress_shortcuts = Arc::clone(&suppress_shortcuts);
            let suppress_next_clipboard_history = Arc::clone(&suppress_next_clipboard_history);
            let settings = Arc::clone(&settings);
            let clipboard_listener_started = Arc::clone(&clipboard_listener_started);
            move || {
                let weak = ensure_clipboard_history_window(
                    &app_windows,
                    &shortcut_windows,
                    Arc::clone(&clipboard_history),
                    clipboard_history_dir.clone(),
                    Arc::clone(&suppress_shortcuts),
                    Arc::clone(&suppress_next_clipboard_history),
                );
                if settings.lock().unwrap().clipboard_history.enabled {
                    ensure_clipboard_listener_started(
                        &clipboard_listener_started,
                        ClipboardListenerDeps {
                            history: Arc::clone(&clipboard_history),
                            settings: Arc::clone(&settings),
                            shortcut_windows: shortcut_windows.clone(),
                            suppress_next_clipboard_history: Arc::clone(
                                &suppress_next_clipboard_history,
                            ),
                            history_dir: clipboard_history_dir.clone(),
                        },
                    );
                }
                if let Some(window) = weak.upgrade() {
                    show_clipboard_history_window(&window, &clipboard_history);
                }
            }
        },
        {
            let app_windows = Rc::clone(&app_windows);
            let shortcut_windows = shortcut_windows.clone();
            let translation_cancel_generation = Arc::clone(&translation_cancel_generation);
            let settings = Arc::clone(&settings);
            let translation_service = Arc::clone(&translation_service);
            move || {
                translation_cancel_generation.fetch_add(1, Ordering::SeqCst);
                let weak = ensure_text_translation_window(
                    &app_windows,
                    &shortcut_windows,
                    Arc::clone(&translation_cancel_generation),
                    Arc::clone(&settings),
                    Arc::clone(&translation_service),
                );
                if let Some(window) = weak.upgrade() {
                    show_translation_pending(&window, "");
                    window.set_translating(false);
                    activate_slint_window(&window);
                }
            }
        },
        {
            let app_windows = Rc::clone(&app_windows);
            let ocr_service = Arc::clone(&ocr_service);
            move || {
                let weak = ensure_file_preview_window(&app_windows, Arc::clone(&ocr_service));
                if let Some(window) = weak.upgrade() {
                    show_empty_file_preview_window(&window);
                    activate_slint_window(&window);
                }
            }
        },
        {
            let app_windows = Rc::clone(&app_windows);
            let shortcut_windows = shortcut_windows.clone();
            move || {
                let weak = ensure_screenshot_window(&app_windows, &shortcut_windows);
                if let Some(window) = weak.upgrade() {
                    show_screenshot_window(&window);
                }
            }
        },
        {
            let app_windows = Rc::clone(&app_windows);
            let settings = Arc::clone(&settings);
            let settings_store = settings_store.clone();
            let translation_service = Arc::clone(&translation_service);
            let ocr_service = Arc::clone(&ocr_service);
            let tray_menu_handles = tray_state.menu_handles();
            let on_settings_applied = Rc::clone(&on_settings_applied);
            move || {
                let settings_snapshot = settings.lock().unwrap().clone();
                let weak = ensure_settings_window(
                    &app_windows,
                    SettingsWindowDeps {
                        settings: Arc::clone(&settings),
                        settings_store: settings_store.clone(),
                        translation_service: Arc::clone(&translation_service),
                        ocr_service: Arc::clone(&ocr_service),
                        tray_menu_handles: tray_menu_handles.clone(),
                        on_settings_applied: Rc::clone(&on_settings_applied),
                    },
                );
                if let Some(window) = weak.upgrade() {
                    show_settings_window(&window, &settings_snapshot);
                }
            }
        },
    );

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
        let shortcut_windows = shortcut_windows.clone();
        let clipboard_history_dir = clipboard_history_dir.clone();
        let suppress_next_clipboard_history = Arc::clone(&suppress_next_clipboard_history);
        let clipboard_listener_started = Arc::clone(&clipboard_listener_started);

        move |event| {
            if let EventType::MouseMove { x, y } = event.event_type {
                *mouse_x.lock().unwrap() = x;
                *mouse_y.lock().unwrap() = y;
            }

            // update shortcut_state
            update_shortcut_state(&shortcut_state, &event.event_type);

            if matches!(event.event_type, EventType::KeyPress(Key::Escape)) {
                if let Some(weak) = shortcut_windows.screenshot.lock().unwrap().clone() {
                    let _ = weak.upgrade_in_event_loop(move |window| {
                        if window.window().is_visible() {
                            cancel_screenshot_window(&window);
                        }
                    });
                }
            }

            if should_show_history(&shortcut_state, &event.event_type)
                && !suppress_shortcuts.load(Ordering::SeqCst)
            {
                if settings.lock().unwrap().clipboard_history.enabled {
                    let history = Arc::clone(&clipboard_history);
                    if let Some(weak) = shortcut_windows.history.lock().unwrap().clone() {
                        weak.upgrade_in_event_loop(move |window| {
                            show_clipboard_history_window(&window, &history);
                        })
                        .expect("failed to show clipboard history window");
                    } else {
                        let shortcut_windows = shortcut_windows.clone();
                        let clipboard_history = Arc::clone(&clipboard_history);
                        let clipboard_history_dir = clipboard_history_dir.clone();
                        let suppress_shortcuts = Arc::clone(&suppress_shortcuts);
                        let suppress_next_clipboard_history =
                            Arc::clone(&suppress_next_clipboard_history);
                        let settings = Arc::clone(&settings);
                        let clipboard_listener_started = Arc::clone(&clipboard_listener_started);
                        slint::invoke_from_event_loop(move || {
                            with_ui_app_windows(|app_windows| {
                                let weak = ensure_clipboard_history_window(
                                    app_windows,
                                    &shortcut_windows,
                                    Arc::clone(&clipboard_history),
                                    clipboard_history_dir.clone(),
                                    Arc::clone(&suppress_shortcuts),
                                    Arc::clone(&suppress_next_clipboard_history),
                                );
                                ensure_clipboard_listener_started(
                                    &clipboard_listener_started,
                                    ClipboardListenerDeps {
                                        history: Arc::clone(&clipboard_history),
                                        settings: Arc::clone(&settings),
                                        shortcut_windows: shortcut_windows.clone(),
                                        suppress_next_clipboard_history: Arc::clone(
                                            &suppress_next_clipboard_history,
                                        ),
                                        history_dir: clipboard_history_dir.clone(),
                                    },
                                );
                                if let Some(window) = weak.upgrade() {
                                    show_clipboard_history_window(&window, &clipboard_history);
                                }
                            });
                        })
                        .expect("failed to create clipboard history window");
                    }
                }
            }

            if should_show_screenshot(&shortcut_state, &event.event_type)
                && !suppress_shortcuts.load(Ordering::SeqCst)
            {
                if settings.lock().unwrap().screenshot.enabled {
                    if let Some(weak) = shortcut_windows.screenshot.lock().unwrap().clone() {
                        weak.upgrade_in_event_loop(move |window| {
                            show_screenshot_window(&window);
                        })
                        .expect("failed to show screenshot window");
                    } else {
                        let shortcut_windows = shortcut_windows.clone();
                        slint::invoke_from_event_loop(move || {
                            with_ui_app_windows(|app_windows| {
                                let weak = ensure_screenshot_window(app_windows, &shortcut_windows);
                                if let Some(window) = weak.upgrade() {
                                    show_screenshot_window(&window);
                                }
                            });
                        })
                        .expect("failed to create screenshot window");
                    }
                }
            }

            if event.name.as_deref() == Some("\u{3}") {
                let settings_snapshot = settings.lock().unwrap().clone();
                if settings_snapshot.text_translation.enabled {
                    let translation_service = Arc::clone(&translation_service);
                    let translation_cancel_generation = Arc::clone(&translation_cancel_generation);
                    let shortcut_windows = shortcut_windows.clone();
                    let settings = Arc::clone(&settings);
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

                        slint::invoke_from_event_loop(move || {
                            let weak = shortcut_windows.translation.lock().unwrap().clone();
                            let weak = weak.unwrap_or_else(|| {
                                let mut created = None;
                                with_ui_app_windows(|app_windows| {
                                    created = Some(ensure_text_translation_window(
                                        app_windows,
                                        &shortcut_windows,
                                        Arc::clone(&translation_cancel_generation),
                                        Arc::clone(&settings),
                                        Arc::clone(&translation_service),
                                    ));
                                });
                                created.expect("translation window registry should be initialized")
                            });
                            trigger_translation(
                                weak,
                                translation_cancel_generation,
                                translation_service,
                                source_text,
                            );
                        })
                        .expect("failed to create translation window");
                    });
                }

                if !settings_snapshot.copy_timestamp.enabled {
                    return Ok(());
                }

                let cur_x = *mouse_x.lock().unwrap();
                let cur_y = *mouse_y.lock().unwrap();
                if let Some(weak) = shortcut_windows.time_trans.lock().unwrap().clone() {
                    weak.upgrade_in_event_loop(move |window| {
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
                } else {
                    let shortcut_windows = shortcut_windows.clone();
                    slint::invoke_from_event_loop(move || {
                        with_ui_app_windows(|app_windows| {
                            let weak = ensure_time_trans_window(app_windows, &shortcut_windows);
                            if let Some(window) = weak.upgrade() {
                                std::thread::sleep(Duration::from_millis(200));

                                let mut clipboard = Clipboard::new().unwrap();
                                let text = clipboard.get_text().ok();

                                if let Some(text) = text {
                                    window.set_input_value(text.trim().into());
                                    window.set_close_time(3);

                                    if !window.get_has_hover() {
                                        let (move_x, move_y) =
                                            next_window_position(&window, cur_x, cur_y);
                                        info!(
                                            "set window pos to x:{move_x},y:{move_y},copy:{text}"
                                        );
                                        set_position(&window, move_x, move_y);
                                    }
                                }
                            }
                        });
                    })
                    .expect("failed to create time translation window");
                }
            }

            Ok(())
        }
    })
    .expect("failed to start global input listener");

    let model_cleanup_timer = slint::Timer::default();
    model_cleanup_timer.start(slint::TimerMode::Repeated, Duration::from_secs(5), {
        let translation_service = Arc::clone(&translation_service);
        let ocr_service = Arc::clone(&ocr_service);
        move || {
            translation_service.unload_if_idle();
            ocr_service.unload_if_idle();
        }
    });

    let _tray_timer = start_tray_event_pump(
        &tray_state,
        Arc::clone(&settings),
        settings_store,
        translation_service,
        ocr_service,
        {
            let weak_home_window = home_window.as_weak();
            move || {
                if let Some(window) = weak_home_window.upgrade() {
                    show_home_window(&window);
                }
            }
        },
        {
            let on_settings_applied = Rc::clone(&on_settings_applied);
            move |settings| on_settings_applied(settings)
        },
    );
    let _model_cleanup_timer = model_cleanup_timer;
    slint::run_event_loop_until_quit().unwrap();
}

fn register_ui_app_windows(app_windows: Rc<RefCell<AppWindows>>) {
    UI_APP_WINDOWS.with(|store| {
        *store.borrow_mut() = Some(app_windows);
    });
}

fn with_ui_app_windows(action: impl FnOnce(&Rc<RefCell<AppWindows>>)) {
    UI_APP_WINDOWS.with(|store| {
        if let Some(app_windows) = store.borrow().as_ref() {
            action(app_windows);
        } else {
            log::error!("UI window registry is not initialized");
        }
    });
}

fn ensure_time_trans_window(
    app_windows: &Rc<RefCell<AppWindows>>,
    shortcut_windows: &ShortcutWindows,
) -> slint::Weak<crate::TimeTrans> {
    let mut windows = app_windows.borrow_mut();
    let window = windows
        .time_trans
        .get_or_insert_with(init_time_trans_window);
    let weak = window.as_weak();
    *shortcut_windows.time_trans.lock().unwrap() = Some(weak.clone());
    weak
}

fn ensure_clipboard_history_window(
    app_windows: &Rc<RefCell<AppWindows>>,
    shortcut_windows: &ShortcutWindows,
    clipboard_history: Arc<Mutex<ClipboardHistory>>,
    clipboard_history_dir: PathBuf,
    suppress_shortcuts: Arc<AtomicBool>,
    suppress_next_clipboard_history: Arc<Mutex<Option<ClipboardHistoryItem>>>,
) -> slint::Weak<crate::ClipboardHistoryWindow> {
    let mut windows = app_windows.borrow_mut();
    let window = windows.history.get_or_insert_with(|| {
        init_clipboard_history_window(
            clipboard_history,
            clipboard_history_dir,
            suppress_shortcuts,
            suppress_next_clipboard_history,
        )
    });
    let weak = window.as_weak();
    *shortcut_windows.history.lock().unwrap() = Some(weak.clone());
    weak
}

fn ensure_text_translation_window(
    app_windows: &Rc<RefCell<AppWindows>>,
    shortcut_windows: &ShortcutWindows,
    translation_cancel_generation: Arc<AtomicU64>,
    settings: Arc<Mutex<AppSettings>>,
    translation_service: Arc<TranslationService>,
) -> slint::Weak<crate::TextTranslationWindow> {
    let mut windows = app_windows.borrow_mut();
    let window = windows.translation.get_or_insert_with(|| {
        init_text_translation_window(translation_cancel_generation, settings, translation_service)
    });
    let weak = window.as_weak();
    *shortcut_windows.translation.lock().unwrap() = Some(weak.clone());
    weak
}

fn ensure_file_preview_window(
    app_windows: &Rc<RefCell<AppWindows>>,
    ocr_service: Arc<OcrService>,
) -> slint::Weak<crate::FilePreviewWindow> {
    let mut windows = app_windows.borrow_mut();
    let window = windows
        .file_preview
        .get_or_insert_with(|| init_file_preview_window(false, ocr_service));
    window.as_weak()
}

fn ensure_screenshot_window(
    app_windows: &Rc<RefCell<AppWindows>>,
    shortcut_windows: &ShortcutWindows,
) -> slint::Weak<crate::ScreenshotWindow> {
    let mut windows = app_windows.borrow_mut();
    let window = windows
        .screenshot
        .get_or_insert_with(init_screenshot_window);
    let weak = window.as_weak();
    *shortcut_windows.screenshot.lock().unwrap() = Some(weak.clone());
    weak
}

struct SettingsWindowDeps {
    settings: Arc<Mutex<AppSettings>>,
    settings_store: SettingsStore,
    translation_service: Arc<TranslationService>,
    ocr_service: Arc<OcrService>,
    tray_menu_handles: TrayMenuHandles,
    on_settings_applied: Rc<dyn Fn(&AppSettings)>,
}

fn ensure_settings_window(
    app_windows: &Rc<RefCell<AppWindows>>,
    deps: SettingsWindowDeps,
) -> slint::Weak<crate::SettingsWindow> {
    let mut windows = app_windows.borrow_mut();
    let window = windows.settings.get_or_insert_with(|| {
        init_settings_window(
            deps.settings,
            deps.settings_store,
            deps.translation_service,
            deps.ocr_service,
            deps.tray_menu_handles,
            deps.on_settings_applied,
        )
    });
    window.as_weak()
}

struct ClipboardListenerDeps {
    history: Arc<Mutex<ClipboardHistory>>,
    settings: Arc<Mutex<AppSettings>>,
    shortcut_windows: ShortcutWindows,
    suppress_next_clipboard_history: Arc<Mutex<Option<ClipboardHistoryItem>>>,
    history_dir: PathBuf,
}

fn ensure_clipboard_listener_started(started: &Arc<AtomicBool>, deps: ClipboardListenerDeps) {
    if started.swap(true, Ordering::SeqCst) {
        return;
    }

    let Some(history_window) = deps.shortcut_windows.history.lock().unwrap().clone() else {
        started.store(false, Ordering::SeqCst);
        log::error!("clipboard history listener requires a clipboard history window");
        return;
    };

    if let Err(err) = start_clipboard_history_listener(
        deps.history,
        deps.settings,
        history_window,
        deps.suppress_next_clipboard_history,
        deps.history_dir,
    ) {
        started.store(false, Ordering::SeqCst);
        log::error!("failed to start clipboard history listener: {err}");
    }
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn shortcut_windows_start_empty() {
        let windows = ShortcutWindows::new();

        assert!(windows.history.lock().unwrap().is_none());
        assert!(windows.screenshot.lock().unwrap().is_none());
        assert!(windows.translation.lock().unwrap().is_none());
        assert!(windows.time_trans.lock().unwrap().is_none());
    }
}
