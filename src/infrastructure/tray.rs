use std::sync::{Arc, Mutex};
use std::time::Duration;

use log::info;
use tray_icon::menu::{CheckMenuItem, Menu, MenuEvent, MenuItem, PredefinedMenuItem};
use tray_icon::{TrayIcon, TrayIconBuilder};

use crate::assets::load_tray_icon;
use crate::settings::{AppSettings, SettingsStore};

const COPY_TIMESTAMP_MENU_ID: &str = "copy_timestamp_enabled";
const CLIPBOARD_HISTORY_MENU_ID: &str = "clipboard_history_enabled";
const QUIT_MENU_ID: &str = "quit";

pub struct TrayState {
    pub icon: TrayIcon,
    pub menu: Menu,
    copy_timestamp_item: CheckMenuItem,
    clipboard_history_item: CheckMenuItem,
}

pub fn init_tray_icon(settings: &AppSettings) -> TrayState {
    let tray_menu = Menu::new();

    let copy_timestamp_item = CheckMenuItem::with_id(
        COPY_TIMESTAMP_MENU_ID,
        "复制后显示时间戳",
        true,
        settings.copy_timestamp.enabled,
        None,
    );
    tray_menu.append(&copy_timestamp_item).unwrap();

    let clipboard_history_item = CheckMenuItem::with_id(
        CLIPBOARD_HISTORY_MENU_ID,
        "启用复制历史",
        true,
        settings.clipboard_history.enabled,
        None,
    );
    tray_menu.append(&clipboard_history_item).unwrap();
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
    }
}

pub fn start_tray_event_pump(
    tray_state: &TrayState,
    settings: Arc<Mutex<AppSettings>>,
    settings_store: SettingsStore,
) -> slint::Timer {
    let tray_timer = slint::Timer::default();
    let copy_timestamp_item = tray_state.copy_timestamp_item.clone();
    let clipboard_history_item = tray_state.clipboard_history_item.clone();

    tray_timer.start(
        slint::TimerMode::Repeated,
        Duration::from_millis(16),
        move || {
            while let Ok(event) = MenuEvent::receiver().try_recv() {
                info!("menu event: {event:?}");

                if event.id.as_ref() == COPY_TIMESTAMP_MENU_ID {
                    let checked = copy_timestamp_item.is_checked();
                    if let Ok(mut settings) = settings.lock() {
                        settings.copy_timestamp.enabled = checked;
                        if let Err(err) = settings_store.save(&settings) {
                            log::error!("save settings failed: {err}");
                        }
                    }
                } else if event.id.as_ref() == CLIPBOARD_HISTORY_MENU_ID {
                    let checked = clipboard_history_item.is_checked();
                    if let Ok(mut settings) = settings.lock() {
                        settings.clipboard_history.enabled = checked;
                        if let Err(err) = settings_store.save(&settings) {
                            log::error!("save settings failed: {err}");
                        }
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
