//! System tray icon, menu state, and menu event handling.

use std::sync::{Arc, Mutex};
use std::time::Duration;

use log::info;
use tray_icon::menu::{CheckMenuItem, Menu, MenuEvent, MenuItem, PredefinedMenuItem, Submenu};
use tray_icon::{MouseButton, MouseButtonState, TrayIcon, TrayIconBuilder, TrayIconEvent};

use crate::assets::load_tray_icon;
use crate::features::text_translation::translator::TranslationService;
use crate::settings::{AppSettings, SettingsStore};

const COPY_TIMESTAMP_MENU_ID: &str = "copy_timestamp_enabled";
const CLIPBOARD_HISTORY_MENU_ID: &str = "clipboard_history_enabled";
const SCREENSHOT_MENU_ID: &str = "screenshot_enabled";
const TEXT_TRANSLATION_ZH_EN_MENU_ID: &str = "text_translation_zh_en";
const TEXT_TRANSLATION_EN_ZH_MENU_ID: &str = "text_translation_en_zh";
const QUIT_MENU_ID: &str = "quit";

/// Keeps tray UI handles alive and exposes menu items needed by the event pump.
pub struct TrayState {
    pub icon: TrayIcon,
    pub menu: Menu,
    copy_timestamp_item: CheckMenuItem,
    clipboard_history_item: CheckMenuItem,
    screenshot_item: CheckMenuItem,
    text_translation_zh_en_item: CheckMenuItem,
    text_translation_en_zh_item: CheckMenuItem,
}

#[derive(Clone)]
pub struct TrayMenuHandles {
    copy_timestamp_item: CheckMenuItem,
    clipboard_history_item: CheckMenuItem,
    screenshot_item: CheckMenuItem,
    text_translation_zh_en_item: CheckMenuItem,
    text_translation_en_zh_item: CheckMenuItem,
}

impl TrayState {
    pub fn menu_handles(&self) -> TrayMenuHandles {
        TrayMenuHandles {
            copy_timestamp_item: self.copy_timestamp_item.clone(),
            clipboard_history_item: self.clipboard_history_item.clone(),
            screenshot_item: self.screenshot_item.clone(),
            text_translation_zh_en_item: self.text_translation_zh_en_item.clone(),
            text_translation_en_zh_item: self.text_translation_en_zh_item.clone(),
        }
    }
}

impl TrayMenuHandles {
    pub fn sync_from_settings(&self, settings: &AppSettings) {
        self.copy_timestamp_item
            .set_checked(settings.copy_timestamp.enabled);
        self.clipboard_history_item
            .set_checked(settings.clipboard_history.enabled);
        self.screenshot_item
            .set_checked(settings.screenshot.enabled);
        self.text_translation_zh_en_item
            .set_checked(settings.text_translation.enabled);
        self.text_translation_en_zh_item
            .set_checked(settings.text_translation.enabled);
    }
}

/// Creates the tray icon and initializes menu check states from settings.
pub fn init_tray_icon(settings: &AppSettings) -> TrayState {
    let tray_menu = Menu::new();

    let copy_timestamp_item = CheckMenuItem::with_id(
        COPY_TIMESTAMP_MENU_ID,
        "显示时间戳 - ctrl+c",
        true,
        settings.copy_timestamp.enabled,
        None,
    );
    tray_menu.append(&copy_timestamp_item).unwrap();

    let clipboard_history_item = CheckMenuItem::with_id(
        CLIPBOARD_HISTORY_MENU_ID,
        "复制历史 - ctrl+shift+c",
        true,
        settings.clipboard_history.enabled,
        None,
    );
    tray_menu.append(&clipboard_history_item).unwrap();

    let screenshot_item = CheckMenuItem::with_id(
        SCREENSHOT_MENU_ID,
        "截图快捷键 - alt+shift+z",
        true,
        settings.screenshot.enabled,
        None,
    );
    tray_menu.append(&screenshot_item).unwrap();

    let text_translation_menu = Submenu::new("翻译文本 - ctrl+c", true);
    let text_translation_zh_en_item = CheckMenuItem::with_id(
        TEXT_TRANSLATION_ZH_EN_MENU_ID,
        "中译英",
        true,
        settings.text_translation.enabled,
        None,
    );
    let text_translation_en_zh_item = CheckMenuItem::with_id(
        TEXT_TRANSLATION_EN_ZH_MENU_ID,
        "英译中",
        true,
        settings.text_translation.enabled,
        None,
    );
    text_translation_menu
        .append(&text_translation_zh_en_item)
        .unwrap();
    text_translation_menu
        .append(&text_translation_en_zh_item)
        .unwrap();
    tray_menu.append(&text_translation_menu).unwrap();
    tray_menu.append(&PredefinedMenuItem::separator()).unwrap();

    let quit_item = MenuItem::with_id(QUIT_MENU_ID, "退出", true, None);
    tray_menu.append(&quit_item).unwrap();

    const ICON_IMG: &[u8] = include_bytes!("../../assets/icons/icon.png");
    let tray_icon = TrayIconBuilder::new()
        .with_menu(Box::new(tray_menu.clone()))
        .with_menu_on_left_click(false)
        .with_tooltip("easy-tool")
        .with_icon(load_tray_icon(ICON_IMG))
        .build()
        .unwrap();

    TrayState {
        icon: tray_icon,
        menu: tray_menu,
        copy_timestamp_item,
        clipboard_history_item,
        screenshot_item,
        text_translation_zh_en_item,
        text_translation_en_zh_item,
    }
}

/// Polls tray menu events from the Slint event loop and persists setting changes.
pub fn start_tray_event_pump(
    tray_state: &TrayState,
    settings: Arc<Mutex<AppSettings>>,
    settings_store: SettingsStore,
    translation_service: Arc<TranslationService>,
    show_home_window: impl Fn() + 'static,
) -> slint::Timer {
    let tray_timer = slint::Timer::default();
    let copy_timestamp_item = tray_state.copy_timestamp_item.clone();
    let clipboard_history_item = tray_state.clipboard_history_item.clone();
    let screenshot_item = tray_state.screenshot_item.clone();
    let text_translation_zh_en_item = tray_state.text_translation_zh_en_item.clone();
    let text_translation_en_zh_item = tray_state.text_translation_en_zh_item.clone();

    tray_timer.start(
        slint::TimerMode::Repeated,
        Duration::from_millis(16),
        move || {
            while let Ok(event) = TrayIconEvent::receiver().try_recv() {
                if let TrayIconEvent::Click {
                    button: MouseButton::Left,
                    button_state: MouseButtonState::Up,
                    ..
                } = event
                {
                    show_home_window();
                }
            }

            while let Ok(event) = MenuEvent::receiver().try_recv() {
                info!("menu event: {event:?}");

                if event.id.as_ref() == COPY_TIMESTAMP_MENU_ID {
                    let checked = copy_timestamp_item.is_checked();
                    if let Ok(mut settings) = settings.lock() {
                        settings.copy_timestamp.enabled = checked;
                        save_settings(&settings_store, &settings);
                    }
                } else if event.id.as_ref() == CLIPBOARD_HISTORY_MENU_ID {
                    let checked = clipboard_history_item.is_checked();
                    if let Ok(mut settings) = settings.lock() {
                        settings.clipboard_history.enabled = checked;
                        save_settings(&settings_store, &settings);
                    }
                } else if event.id.as_ref() == SCREENSHOT_MENU_ID {
                    let checked = screenshot_item.is_checked();
                    if let Ok(mut settings) = settings.lock() {
                        settings.screenshot.enabled = checked;
                        save_settings(&settings_store, &settings);
                    }
                } else if event.id.as_ref() == TEXT_TRANSLATION_ZH_EN_MENU_ID {
                    let checked = text_translation_zh_en_item.is_checked();
                    if let Ok(mut settings) = settings.lock() {
                        settings.text_translation.enabled = checked;
                        text_translation_zh_en_item.set_checked(checked);
                        text_translation_en_zh_item.set_checked(checked);
                        translation_service.apply_settings(
                            &settings.text_translation,
                            settings.ai_backend,
                            &settings.tencent_cloud,
                        );
                        save_settings(&settings_store, &settings);
                    }
                } else if event.id.as_ref() == TEXT_TRANSLATION_EN_ZH_MENU_ID {
                    let checked = text_translation_en_zh_item.is_checked();
                    if let Ok(mut settings) = settings.lock() {
                        settings.text_translation.enabled = checked;
                        text_translation_zh_en_item.set_checked(checked);
                        text_translation_en_zh_item.set_checked(checked);
                        translation_service.apply_settings(
                            &settings.text_translation,
                            settings.ai_backend,
                            &settings.tencent_cloud,
                        );
                        save_settings(&settings_store, &settings);
                    }
                } else if event.id.as_ref() == QUIT_MENU_ID {
                    info!("quit application");
                    request_application_quit();
                    break;
                }
            }
        },
    );
    tray_timer
}

fn save_settings(settings_store: &SettingsStore, settings: &AppSettings) {
    if let Err(err) = settings_store.save(settings) {
        log::error!("save settings failed: {err}");
    }
}

fn request_application_quit() {
    std::thread::Builder::new()
        .name("quit-fallback".into())
        .spawn(|| {
            std::thread::sleep(Duration::from_millis(500));
            log::warn!("force exit after quit request");
            log::logger().flush();
            std::process::exit(0);
        })
        .unwrap_or_else(|err| {
            log::error!("spawn quit fallback failed: {err}");
            std::process::exit(0);
        });

    if let Err(err) = slint::quit_event_loop() {
        log::error!("quit event loop failed: {err}");
        log::logger().flush();
        std::process::exit(0);
    }
}
