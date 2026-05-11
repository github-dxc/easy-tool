use std::sync::{Arc, Mutex};

use log::info;
use tray_icon::menu::{CheckMenuItem, Menu, MenuEvent, MenuItem, PredefinedMenuItem};
use tray_icon::{TrayIcon, TrayIconBuilder};

use crate::assets::load_tray_icon;
use crate::settings::{AppSettings, SettingsStore};

const COPY_TIMESTAMP_MENU_ID: &str = "copy_timestamp_enabled";
const QUIT_MENU_ID: &str = "quit";

pub struct TrayState {
    pub icon: TrayIcon,
    pub menu: Menu,
    copy_timestamp_item: CheckMenuItem,
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
    }
}

pub fn start_tray_event_pump(
    tray_state: &TrayState,
    settings: Arc<Mutex<AppSettings>>,
    settings_store: SettingsStore,
) -> slint::Timer {
    let tray_timer = slint::Timer::default();
    let copy_timestamp_item = tray_state.copy_timestamp_item.clone();

    tray_timer.start(
        slint::TimerMode::Repeated,
        std::time::Duration::from_millis(16),
        move || {
            if let Ok(event) = MenuEvent::receiver().try_recv() {
                info!("menu event: {event:?}");

                if event.id.as_ref() == COPY_TIMESTAMP_MENU_ID {
                    let checked = copy_timestamp_item.is_checked();
                    if let Ok(mut settings) = settings.lock() {
                        settings.copy_timestamp.enabled = checked;
                        if let Err(err) = settings_store.save(&settings) {
                            log::error!("save settings failed: {err}");
                        }
                    }
                } else if event.id.as_ref() == QUIT_MENU_ID {
                    info!("quit application");
                    slint::quit_event_loop().unwrap();
                }
            }
        },
    );
    tray_timer
}
