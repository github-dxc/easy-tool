//! System tray icon, menu state, and menu event handling.

use std::time::Duration;

use log::info;
use tray_icon::menu::{CheckMenuItem, Menu, MenuEvent, MenuItem, PredefinedMenuItem, Submenu};
use tray_icon::{MouseButton, MouseButtonState, TrayIcon, TrayIconBuilder, TrayIconEvent};

use crate::assets::load_tray_icon;
use crate::settings::{AppSettings, CopyMode};

const CLIPBOARD_HISTORY_MENU_ID: &str = "clipboard_history_enabled";
const QUIT_MENU_ID: &str = "quit";

/// 托盘只发出用户动作，配置保存和服务更新交给应用控制器。
pub enum TrayAction {
    CopyMode(CopyMode),
    ClipboardHistory(bool),
}

/// Keeps tray UI handles alive and exposes menu items needed by the event pump.
pub struct TrayState {
    pub icon: TrayIcon,
    pub menu: Menu,
    handles: TrayMenuHandles,
}

#[derive(Clone)]
pub struct TrayMenuHandles {
    copy_mode_items: [(CopyMode, CheckMenuItem); 3],
    clipboard_history_item: CheckMenuItem,
}

impl TrayState {
    /// 复制托盘菜单句柄，供应用控制器同步勾选状态。
    pub fn menu_handles(&self) -> TrayMenuHandles {
        self.handles.clone()
    }
}

impl TrayMenuHandles {
    /// 根据实际生效的配置更新托盘菜单勾选状态。
    pub fn sync_from_settings(&self, settings: &AppSettings) {
        let selected = settings.copy_mode;
        for (mode, item) in &self.copy_mode_items {
            item.set_checked(*mode == selected);
        }
        self.clipboard_history_item
            .set_checked(settings.clipboard_history.enabled);
    }
}

/// 创建托盘图标与菜单，并设置功能开关的初始状态。
pub fn init_tray_icon(settings: &AppSettings) -> TrayState {
    let tray_menu = Menu::new();

    let copy_menu = Submenu::new("复制功能", true);
    let selected = settings.copy_mode;
    let copy_mode_items = CopyMode::ALL.map(|mode| {
        let item = CheckMenuItem::with_id(
            format!("copy_mode_{}", mode.index()),
            mode.label(),
            true,
            mode == selected,
            None,
        );
        copy_menu.append(&item).unwrap();
        (mode, item)
    });
    tray_menu.append(&copy_menu).unwrap();

    let clipboard_history_item = CheckMenuItem::with_id(
        CLIPBOARD_HISTORY_MENU_ID,
        "复制历史 - Ctrl+Shift+C",
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
        handles: TrayMenuHandles {
            copy_mode_items,
            clipboard_history_item,
        },
    }
}

/// 轮询托盘事件，将显示主页和功能开关动作交给调用方。
pub fn start_tray_event_pump(
    tray_state: &TrayState,
    show_home_window: impl Fn() + 'static,
    on_action: impl Fn(TrayAction) + 'static,
) -> slint::Timer {
    let tray_timer = slint::Timer::default();
    let handles = tray_state.menu_handles();

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

                let action = if let Some((mode, _)) = handles
                    .copy_mode_items
                    .iter()
                    .find(|(_, item)| item.id() == &event.id)
                {
                    // 子菜单是单选；再次点击当前项仍选中，不能按复选框状态反向关闭。
                    Some(TrayAction::CopyMode(*mode))
                } else if event.id.as_ref() == CLIPBOARD_HISTORY_MENU_ID {
                    Some(TrayAction::ClipboardHistory(
                        handles.clipboard_history_item.is_checked(),
                    ))
                } else if event.id.as_ref() == QUIT_MENU_ID {
                    info!("quit application");
                    request_application_quit();
                    break;
                } else {
                    None
                };
                if let Some(action) = action {
                    on_action(action);
                }
            }
        },
    );
    tray_timer
}

/// 请求退出事件循环，并启动后台兜底退出以防监听线程阻碍结束。
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
