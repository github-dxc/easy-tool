//! Global keyboard and mouse event listener used by app-wide shortcuts.

use std::thread;

use log::error;
use rdev::{Event, listen};

/// 创建全局输入监听线程，将键盘和鼠标事件转交调用方。
pub fn start_global_input_listener<F>(event_handle: F) -> Result<(), String>
where
    F: Fn(Event) -> Result<(), String> + Send + 'static,
{
    thread::Builder::new()
        .name("rdev-listener".into())
        .spawn(move || {
            if let Err(err) = listen(move |event| {
                if let Err(err) = event_handle(event) {
                    error!("keyboard event handler error: {err}");
                }
            }) {
                error!("keyboard listener error: {err:?}");
            }
        })
        .map_err(|err| format!("spawn failed: {err}"))?;

    Ok(())
}
