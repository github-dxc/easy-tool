//! 只在 UI 线程使用的窗口容器，按需创建并复用窗口。

use crate::features::clipboard_history::service::ClipboardHistoryService;
use crate::features::clipboard_history::window::init_clipboard_history_window;
use crate::features::text_translation::requests::TranslationRequests;
use crate::features::text_translation::translator::TranslationService;
use crate::features::text_translation::window::init_text_translation_window;
use crate::features::time_trans::window::init_time_trans_window;
use crate::platform::notification::StatusNotification;
use crate::settings::AppSettings;
use slint::ComponentHandle;
use std::sync::atomic::AtomicBool;
use std::sync::{Arc, Mutex};

#[derive(Default)]
pub(super) struct AppWindows {
    pub timestamp: Option<crate::TimeTrans>,
    pub history: Option<crate::ClipboardHistoryWindow>,
    pub translation: Option<crate::TextTranslationWindow>,
    pub settings: Option<crate::SettingsWindow>,
    notification: Option<StatusNotification>,
}

impl AppWindows {
    /// 复用同一个短暂提示窗口，连续切换时更新内容并重新开始一秒倒计时。
    pub fn notify(&mut self, message: &str) {
        if self.notification.is_none() {
            match StatusNotification::new() {
                Ok(notification) => self.notification = Some(notification),
                Err(err) => {
                    log::error!("create status notification failed: {err}");
                    return;
                }
            }
        }
        if let Some(notification) = &self.notification {
            notification.show(message);
        }
    }

    /// 按需创建时间转换窗口，返回句柄副本以便及时释放容器借用。
    pub fn timestamp(&mut self) -> crate::TimeTrans {
        self.timestamp
            .get_or_insert_with(init_time_trans_window)
            .clone_strong()
    }

    /// 打开历史记录时才创建窗口，后台记录服务无需依赖此窗口。
    pub fn history(
        &mut self,
        service: Arc<ClipboardHistoryService>,
        suppress_shortcuts: Arc<AtomicBool>,
    ) -> crate::ClipboardHistoryWindow {
        self.history
            .get_or_insert_with(|| init_clipboard_history_window(service, suppress_shortcuts))
            .clone_strong()
    }

    /// 按需创建翻译窗口，并共享请求代次以丢弃过期结果。
    pub fn translation(
        &mut self,
        requests: Arc<TranslationRequests>,
        settings: Arc<Mutex<AppSettings>>,
        service: Arc<TranslationService>,
    ) -> crate::TextTranslationWindow {
        self.translation
            .get_or_insert_with(|| init_text_translation_window(requests, settings, service))
            .clone_strong()
    }
}
