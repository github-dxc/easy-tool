use chrono::{DateTime, TimeZone, Utc};
use chrono_tz::Tz;
use core::time;
use std::os::windows;
use std::sync::{Arc, Mutex};
use std::thread::{self, sleep};
use std::time::Duration;
use rdev::{Event, listen};
use rdev::EventType;
use flexi_logger::{Logger, Duplicate, FileSpec, Criterion, Naming, Cleanup};
use log::{info, warn, error};

slint::include_modules!();

pub fn test_window() {
    let test_window = TestWindow::new().unwrap();

    let weak = test_window.as_weak();

    test_window.on_button_clicked(move || {
        let count = weak.unwrap().get_counter();
        println!("按钮被点击了 {} 次", count);
    });

    test_window.run().unwrap();
}

pub fn time_trans_window() {
    let tm = TestWindow::new().unwrap();

    let time_window = TimeTrans::new().unwrap();
    let tw = time_window.as_weak();

    time_window.on_close_window(move || {
        if let Some(window) = tw.upgrade() {
            let _ = window.hide();
        }
    });

    tm.show().unwrap();
    time_window.show().unwrap();

    slint::run_event_loop().unwrap();
}

// 键盘事件处理函数
pub fn keybord_event_handle(event: Event) -> Result<(), String> {
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