use log::info;
use tray_icon::menu::{Menu, MenuEvent, MenuItem};
use tray_icon::{TrayIcon, TrayIconBuilder};

use crate::assets::load_tray_icon;

pub fn init_tray_icon() -> (TrayIcon, Menu) {
    let tray_menu = Menu::new();
    let quit_item = MenuItem::with_id("quit", "退出", true, None);
    tray_menu.append(&quit_item).unwrap();

    const ICON_IMG: &[u8] = include_bytes!("../../assets/icons/icon.png");
    let tray_icon = TrayIconBuilder::new()
        .with_menu(Box::new(tray_menu.clone()))
        .with_menu_on_left_click(false)
        .with_tooltip("easy-tool")
        .with_icon(load_tray_icon(ICON_IMG))
        .build()
        .unwrap();

    (tray_icon, tray_menu)
}

pub fn start_tray_event_pump() -> slint::Timer {
    let tray_timer = slint::Timer::default();
    tray_timer.start(
        slint::TimerMode::Repeated,
        std::time::Duration::from_millis(16),
        move || {
            if let Ok(event) = MenuEvent::receiver().try_recv() {
                info!("menu event: {event:?}");
                if event.id.as_ref() == "quit" {
                    info!("quit application");
                    slint::quit_event_loop().unwrap();
                }
            }
        },
    );
    tray_timer
}
