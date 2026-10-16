//! 系统剪贴板监听适配，只通知变化，不依赖具体功能或窗口。

use clipboard_master::{CallbackResult, ClipboardHandler, Master};
use std::io;
use std::thread;
use std::time::Duration;

/// 创建系统监听线程，线程停止时通知调用方释放运行状态。
pub fn start_clipboard_listener(
    on_change: impl FnMut() + Send + 'static,
    on_stopped: impl FnOnce() + Send + 'static,
) -> Result<(), String> {
    thread::Builder::new()
        .name("clipboard-master-listener".into())
        .spawn(move || {
            let result =
                Master::new(ChangeHandler { on_change }).and_then(|mut master| master.run());
            if let Err(err) = result {
                log::error!("clipboard listener failed: {err}");
            }
            on_stopped();
        })
        .map_err(|err| format!("spawn clipboard listener failed: {err}"))?;
    Ok(())
}

struct ChangeHandler<F> {
    on_change: F,
}

impl<F: FnMut()> ClipboardHandler for ChangeHandler<F> {
    /// 稍等源应用完成格式发布后通知业务层；等待仅发生在监听线程。
    fn on_clipboard_change(&mut self) -> CallbackResult {
        thread::sleep(Duration::from_millis(80));
        (self.on_change)();
        CallbackResult::Next
    }

    /// 记录系统剪贴板错误，并允许继续处理后续变化。
    fn on_clipboard_error(&mut self, error: io::Error) -> CallbackResult {
        log::error!("clipboard listener callback error: {error}");
        CallbackResult::Next
    }
}
