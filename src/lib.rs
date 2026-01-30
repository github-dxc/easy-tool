mod utils;
use chrono::{DateTime, NaiveDateTime, TimeZone, Utc};
use chrono_tz::Tz;
use single_instance::SingleInstance;
use winit::dpi::{PhysicalPosition};
use slint::{ComponentHandle, SharedString, VecModel};
use i_slint_backend_winit::WinitWindowAccessor;
use raw_window_handle::{HasWindowHandle, RawWindowHandle};
use tray_icon::{Icon, TrayIcon};
use tray_icon::menu::MenuItem;
use std::rc::Rc;
use std::sync::{Arc, Mutex};
use std::thread::{self, sleep};
use std::time::Duration;
use rdev::{Event, listen};
use rdev::EventType;
use arboard::Clipboard;
use regex::Regex;
use tray_icon::{TrayIconBuilder, menu::{Menu,MenuEvent}};
use flexi_logger::{Logger, Duplicate, FileSpec, Criterion, Naming, Cleanup};
use log::{info, warn, error};
use utils::*;

slint::include_modules!();

// 程序主运行函数
pub fn run() {
    // 保证单实例
    let instance = SingleInstance::new("my_unique_easy_tool_app_id").unwrap();
    if !instance.is_single() {
        show_message_box("提示", "应用已经在运行中，程序即将退出。");
        return;
    }

    // 初始化日志
    init_log().unwrap();

    // 初始化窗口
    let time_trans_window = init_time_trans_window();
    let weak = time_trans_window.as_weak();

    // 初始化桌面托盘
    let (_tray_icon, _tray_menu) = init_tray_icon();

    // 初始化键盘事件监听
    let mouse_x = Arc::new(Mutex::new(0f64));
    let mouse_y = Arc::new(Mutex::new(0f64));
    init_rdev(move |event|{
        match event.event_type {
            EventType::MouseMove{ x, y } => {
                // 获取鼠标位置
                // info!("x:{},y:{}", x, y);
                
                *mouse_x.lock().unwrap() = x;
                *mouse_y.lock().unwrap() = y;
            }
            _ => {}
        }
        // 特殊处理按键事件
        if let Some(name) = event.name {
            info!("event name: {:?}", name);
            match name.as_str() {
                // 处理 Ctrl+C 组合键
                "\u{3}" => {
                    let cur_x = *mouse_x.lock().unwrap();
                    let cur_y = *mouse_y.lock().unwrap();
                    weak.upgrade_in_event_loop(move |window| {
                        // 读取文本
                        std::thread::sleep(Duration::from_millis(200));
                        let mut clipboard = Clipboard::new().unwrap();
                        window.set_input_value(clipboard.get_text().unwrap().into());
                        // 设置窗口位置到鼠标位置，如果已经触摸过则不移动
                        if !window.get_has_hover() {
                            info!("set window pos to x:{},y:{},copy:{}", cur_x, cur_y, clipboard.get_text().unwrap());
                            set_position(&window, cur_x + 20f64, cur_y + 10f64);
                        }
                    }).expect("Failed to send event to UI thread")
                }
                _ => {}
            }
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
    slint::run_event_loop_until_quit().unwrap();
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

    let label_model = Rc::new(VecModel::from(TIMEZONE_LABELS.iter().map(|&label| SharedString::from(label)).collect::<Vec<SharedString>>()));
    time_window.set_timezone_labels(label_model.clone().into());

    let zone_model = Rc::new(VecModel::from(TIMEZONES.iter().map(|&zone| SharedString::from(zone)).collect::<Vec<SharedString>>()));
    time_window.set_timezones(zone_model.clone().into());

    let tw = time_window.as_weak();
    time_window.on_close_window(move || {
        if let Some(ui) = tw.upgrade() {
            let _ = ui.hide();
        }
    });

    let tw1 = time_window.as_weak();
    time_window.on_show_window(move || {
        if let Some(ui) = tw1.upgrade() {
            let _ = ui.show();
            hide_taskbar_icon(&ui);
        }
    });

    time_window.on_copy_to_clipboard(|s| {
        let mut clipboard = Clipboard::new().unwrap();
        // 设置文本
        clipboard.set_text(s.into()).unwrap();
    });

    let tw2 = time_window.as_weak();
    time_window.on_update_result(move |input_value,unit,zone|{
        let (result,unit) = trans_string_timestamp(input_value.as_str(), unit, zone.to_string());
        
        if let Some(ui) = tw2.upgrade() {
            match result {
                Ok(result_value) => {
                    ui.set_result_value(result_value.into());
                    if let Some(u) = unit {
                        ui.set_timestamp_unit(u);
                    }
                },
                Err(str) => {
                    ui.set_result_value(str.into());
                }
            }
        }
    });

    let tw3 = time_window.as_weak();
    time_window.on_move_window(move || {
        if let Some(ui) = tw3.upgrade() {
            // 访问底层的 winit 窗口
            ui.window().with_winit_window(|winit_window| {
                // 调用系统原生的窗口拖拽功能
                let _ = winit_window.drag_window();
            });
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
        .with_icon(load_icon("./assets/icons/icon.png"))
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

// 设置窗口位置
fn set_position(time_window: &TimeTrans, x: f64, y: f64) {
    // 隐藏窗口的任务栏图标（改进：清除 WS_EX_APPWINDOW，设置 WS_EX_TOOLWINDOW，并刷新样式）
    #[cfg(target_os = "windows")]
    {
        // 访问底层的 winit 窗口
        time_window.window().with_winit_window(|winit_window| {
            // 设置位置
            winit_window.set_outer_position(PhysicalPosition::new(x, y));
        });
    } 
}

// 隐藏任务栏图标
fn hide_taskbar_icon(time_window: &TimeTrans) {
    // 隐藏窗口的任务栏图标（改进：清除 WS_EX_APPWINDOW，设置 WS_EX_TOOLWINDOW，并刷新样式）
    #[cfg(target_os = "windows")]
    {
        time_window.window().with_winit_window(|winit_window| {
            if let Ok(handle) = winit_window.window_handle() {
                if let RawWindowHandle::Win32(win32_handle) = handle.as_raw() {
                    let hwnd = win32_handle.hwnd.get() as isize;
                    unsafe {
                        use windows_sys::Win32::UI::WindowsAndMessaging::*;

                        // 获取当前样式并转换成 u32 处理更自然
                        let old_style = GetWindowLongPtrW(hwnd, GWL_EXSTYLE) as u32;
                        let new_style = (old_style | WS_EX_TOOLWINDOW) & !WS_EX_APPWINDOW;
                        
                        if old_style != new_style {
                            SetWindowLongPtrW(hwnd, GWL_EXSTYLE, new_style as isize);
                            
                            // 确保样式立即生效
                            SetWindowPos(
                                hwnd, 
                                0, 0, 0, 0, 0, 
                                SWP_FRAMECHANGED | SWP_NOMOVE | SWP_NOSIZE | SWP_NOZORDER
                            );
                        }
                    }
                }
            }
        });
    }
}

// 字符串时间戳相互转换
fn trans_string_timestamp(input: &str, unit: bool, zone: String) -> (Result<String, String>, Option<bool>) {
    let mut timestamp = 0i64;
    let tz: Tz = zone.parse().unwrap_or_default();
    
    // 10位时间戳
    let re = Regex::new(r"^[12]\d{9}$").unwrap();
    if re.is_match(input) {
        timestamp = input.parse().unwrap();
    }
    if timestamp != 0 {
        let dt = Utc.timestamp_opt(timestamp, 0).single().unwrap();
        let local_time = dt.with_timezone(&tz);
        return (Ok(local_time.format("%Y-%m-%d %H:%M:%S").to_string()), Some(false));
    }
    
    // 13位时间戳
    let re = Regex::new(r"^[12]\d{12}$").unwrap();
    if re.is_match(input) {
        timestamp = input.parse::<i64>().unwrap() / 1000;
    }
    if timestamp != 0 {
        let dt = Utc.timestamp_opt(timestamp, 0).single().unwrap();
        let local_time = dt.with_timezone(&tz);
        return (Ok(local_time.format("%Y-%m-%d %H:%M:%S").to_string()), Some(true));
    }

    // %Y-%m-%d %H:%M:%S 时间字符串
    let re = Regex::new(r"^\d{4}-\d{2}-\d{2} \d{2}:\d{2}:\d{2}$").unwrap();
    if re.is_match(input) {
        if let Ok(naive_dt) = NaiveDateTime::parse_from_str(input, "%Y-%m-%d %H:%M:%S") {
            let dt_in_tz = tz.from_local_datetime(&naive_dt).single();
            if let Some(dt) = dt_in_tz {
                let mut timestamp = dt.timestamp();
                if unit {
                    timestamp *= 1000;
                }
                return (Ok(timestamp.to_string()), None);
            }
        }
    }

    // %Y-%m-%dT%H:%M:%S+08:00 时间字符串
    let re = Regex::new(r"^\d{4}-\d{2}-\d{2}T\d{2}:\d{2}:\d{2}[+-]\d{2}:\d{2}$").unwrap();
    if re.is_match(input) {
        //根据input中后缀的时区偏移，转换为对应时区时间，然后再转成目标时区
        if let Ok(dt) = DateTime::parse_from_rfc3339(input) {
            let mut timestamp = dt.timestamp();
                if unit {
                    timestamp *= 1000;
                }
            return (Ok(timestamp.to_string()), None);
        }
    }

    // %Y-%m-%d 时间字符串
    let re = Regex::new(r"^\d{4}-\d{2}-\d{2}$").unwrap();
    if re.is_match(input) {
        if let Ok(naive_dt) = NaiveDateTime::parse_from_str(&(input.to_string() + " 00:00:00"), "%Y-%m-%d %H:%M:%S") {
            let dt_in_tz = tz.from_local_datetime(&naive_dt).single();
            if let Some(dt) = dt_in_tz {
                let mut timestamp = dt.timestamp();
                if unit {
                    timestamp *= 1000;
                }
                return (Ok(timestamp.to_string()), None);
            }
        }
    }

    // RFC 2822 时间字符串
    let re = Regex::new(r"^[A-Za-z]{3}, \d{1,2} [A-Za-z]{3} \d{4} \d{2}:\d{2}:\d{2} [+-]\d{4}$").unwrap();
    if re.is_match(input) {
        if let Ok(dt) = DateTime::parse_from_rfc2822(input) {
            let mut timestamp = dt.timestamp();
            if unit {
                timestamp *= 1000;
            }
            return (Ok(timestamp.to_string()), None);
        }
    }

    (Err("Error".to_string()), None)
}

const TIMEZONES: [&str; 12] = [
    "Asia/Shanghai",       // 中国标准时间 (CST)
    "Etc/UTC",             // 协调世界时
    "Asia/Tokyo",          // 日本标准时间 (JST)
    "Asia/Kolkata",        // 印度标准时间 (IST)
    "Asia/Singapore",      // 新加坡/马来西亚 (SGT)
    "Europe/London",       // 伦敦 (GMT/BST)
    "Europe/Paris",        // 巴黎 (CET/CEST)
    "America/New_York",    // 美国东部 (EST/EDT)
    "America/Chicago",     // 美国中部 (CST/CDT)
    "America/Denver",      // 美国山地 (MST/MDT)
    "America/Los_Angeles", // 美国太平洋 (PST/PDT)
    "Australia/Sydney"     // 悉尼 (AEST/AEDT)
];

const TIMEZONE_LABELS: [&str; 12] = [
    "CST/北京",
    "UTC",
    "JST/东京",
    "IST/新德里",
    "SGT/新加坡",
    "BST/伦敦",
    "CET/巴黎",
    "ET/纽约",
    "CT/芝加哥",
    "MT/丹佛",
    "PT/洛杉矶",
    "AET/悉尼",
];