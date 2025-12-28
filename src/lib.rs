use chrono::{DateTime, TimeZone, Utc};
use chrono_tz::Tz;
use tray_icon::{Icon, TrayIcon};
use tray_icon::menu::MenuItem;
use core::time;
use std::os::windows;
use std::sync::{Arc, Mutex};
use std::thread::{self, sleep};
use std::time::Duration;
use rdev::{Event, listen};
use rdev::EventType;
use tray_icon::{TrayIconBuilder,TrayIconEvent, menu::{Menu,MenuEvent}};
use flexi_logger::{Logger, Duplicate, FileSpec, Criterion, Naming, Cleanup};
use log::{info, warn, error};

slint::include_modules!();

// 程序主运行函数
pub fn run() {

    // 初始化日志
    init_log().unwrap();

    // 初始化窗口
    let _time_trans_window = init_time_trans_window();

    // 初始化桌面托盘
    let (_tray_icon, _tray_menu) = init_tray_icon();

    // 初始化键盘事件监听
    init_rdev(|event|{
        if let Some(name) = event.name {
            info!("event name: {:?}", name);
        }
        match event.event_type {
            EventType::MouseMove{ x, y } => {
                // 获取鼠标位置
                info!("x:{},y:{}", x, y);
            }
            _ => {}
        }
        Ok(())
    }).unwrap();

    // 运行事件循环
    let tray_timer = slint::Timer::default();
    tray_timer.start(slint::TimerMode::Repeated, std::time::Duration::from_millis(16), move || {
        // 监听托盘事件
        // if let Ok(event) = TrayIconEvent::receiver().try_recv() {
        //     // 如果点击了某个菜单想打开 Slint，就在这里初始化 Slint 窗口
        //     log::info!("tray event: {:?}", event);
        // }

        // 监听菜单事件
        if let Ok(event) = MenuEvent::receiver().try_recv() {
            log::info!("menu event: {:?}", event);
            match event.id.as_ref() {
                "quit" => {
                    log::info!("退出程序");
                    slint::quit_event_loop().unwrap(); // 发出退出信号
                }
                _ => {}
            }
        }
    });
    slint::run_event_loop().unwrap();
}

pub fn test_window() {
    let test_window = TestWindow::new().unwrap();

    let weak = test_window.as_weak();

    test_window.on_button_clicked(move || {
        let count = weak.unwrap().get_counter();
        println!("按钮被点击了 {} 次", count);
    });

    test_window.run().unwrap();
}

// 时间转换窗口
pub fn init_time_trans_window() -> TimeTrans {
    let time_window = TimeTrans::new().unwrap();
    let tw = time_window.as_weak();

    time_window.on_close_window(move || {
        if let Some(window) = tw.upgrade() {
            let _ = window.hide();
        }
    });

    time_window
}

// 初始化键盘事件
pub fn init_rdev<F>(event_handle: F) -> Result<(), String> 
where F: Fn(Event) -> Result<(), String> + Send + 'static
{
    thread::Builder::new()
        .name("rdev-listener".into())
        .spawn(move || {
            // rdev::listen 是阻塞的，放在独立线程
            if let Err(err) = listen(move |event| {
                if let Err(e) = event_handle(event) {
                    error!("Keyboard event handle error: {:?}", e);
                }
            }) {
                error!("Keyboard listener error: {:?}", err);
            }
        })
        .map_err(|e| format!("spawn failed: {}", e))?;
    Ok(())
}

// 初始化日志实现库
pub fn init_log() -> Result<(), String> {
    Logger::try_with_str("info").map_err(|e|{println!("log err:{}",e);e})
        .unwrap()
        .log_to_file(FileSpec::default().directory("logs").basename("easy-tool"))
        .duplicate_to_stdout(Duplicate::Info) // 同时在stdout打印info及以上
        .rotate(
            Criterion::Size(10_000_000), // 10 MB
            Naming::Numbers,
            Cleanup::KeepLogFiles(7),
        )
        .start().map_err(|e|{println!("init_log start err:{}",e);e})
        .map_err(|e|format!("init_log err: {}",e))?;
    Ok(())
}

// 初始化托盘菜单
pub fn init_tray_icon() -> (TrayIcon, Menu) {
    let tray_menu = Menu::new();
    let quit_item = MenuItem::with_id("quit", "退出", true, None);
    tray_menu.append(&quit_item).unwrap();
    let tray_icon = TrayIconBuilder::new()
        .with_menu(Box::new(tray_menu.clone()))
        .with_menu_on_left_click(false)
        .with_tooltip("system-tray - tray icon library!")
        .with_icon(load_icon("ui/icons/icon.png"))
        .build()
        .unwrap();
    (tray_icon, tray_menu)
}

// 加载图标文件
fn load_icon(path: &str) -> Icon {
    // 打开图片文件 转换为RGBA8格式
    let img = image::open(path)
        .expect("无法打开图标文件")
        .into_rgba8();
    // 获取图片宽高
    let (width, height) = img.dimensions();
    // 获取原始像素字节流
    let rgba = img.into_raw();
    Icon::from_rgba(rgba, width, height).expect("创建图标失败")
}