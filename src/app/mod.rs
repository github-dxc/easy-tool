//! 应用入口：组装服务、绑定页面和启动事件循环。

mod controller;
mod shortcuts;
mod windows;

use std::rc::Rc;
use std::sync::mpsc;
use std::time::Duration;

use crate::config::APP_INSTANCE_ID;
use crate::features::home::window::{init_home_window, show_home_window};
use crate::infrastructure::logging::init_logging;
use crate::infrastructure::tray::{init_tray_icon, start_tray_event_pump};
use crate::platform::dialog::show_message_box;
use crate::settings::{SettingsService, SettingsStore};
use controller::AppController;
use single_instance::SingleInstance;
use slint::ComponentHandle;

/// 区分后台事件，跨线程只传数据，不传递 Slint 窗口或应用控制器。
pub(super) enum AppEvent {
    ShowHistory,
    CopyRequested {
        position: (f64, f64),
    },
    CopiedText {
        generation: u64,
        text: String,
        position: (f64, f64),
    },
    HistoryChanged,
}

/// 检查单实例，创建服务与窗口入口，并保持托盘及消息定时器存活。
pub fn run() {
    let instance =
        SingleInstance::new(APP_INSTANCE_ID).expect("failed to create single instance lock");
    if !instance.is_single() {
        show_message_box("提示", "应用已经在运行中，程序即将退出。");
        return;
    }
    init_logging().expect("failed to initialize logging");
    let store = SettingsStore::new();
    log::info!("settings path: {}", store.path().display());
    let settings = SettingsService::load(store).expect("failed to load settings");
    let tray = init_tray_icon(&settings.snapshot());
    let (sender, receiver) = mpsc::channel();
    let controller = Rc::new(AppController::new(settings, tray.menu_handles(), sender));
    controller.start_services();

    let home = init_home_window(
        {
            let app = Rc::clone(&controller);
            move || app.show_timestamp()
        },
        {
            let app = Rc::clone(&controller);
            move || app.show_history_from_home()
        },
        {
            let app = Rc::clone(&controller);
            move || app.show_translation()
        },
        {
            let app = Rc::clone(&controller);
            move || app.show_settings()
        },
    );
    let _tray_timer = start_tray_event_pump(
        &tray,
        {
            let weak = home.as_weak();
            move || {
                if let Some(home) = weak.upgrade() {
                    show_home_window(&home);
                }
            }
        },
        {
            let app = Rc::clone(&controller);
            move |action| app.handle_tray_action(action)
        },
    );
    // 所有窗口操作只在 UI 线程执行，后台线程只向通道发送事件。
    let event_timer = slint::Timer::default();
    event_timer.start(
        slint::TimerMode::Repeated,
        Duration::from_millis(16),
        move || {
            // 限制单轮处理数量，持续复制时仍给绘制和用户输入留出执行机会。
            for event in receiver.try_iter().take(64) {
                controller.handle_event(event);
            }
        },
    );
    slint::run_event_loop_until_quit().unwrap();
}
