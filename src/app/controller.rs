//! 应用控制器：协调功能入口、配置更新与后台事件。

use slint::ComponentHandle;
use std::cell::RefCell;
use std::rc::Rc;
use std::sync::atomic::AtomicBool;
use std::sync::{Arc, mpsc::Sender};

use super::shortcuts::{CopyDispatcher, start_shortcuts};
use super::{AppEvent, windows::AppWindows};
use crate::features::clipboard_history::service::ClipboardHistoryService;
use crate::features::clipboard_history::window::{
    refresh_clipboard_history_window, show_clipboard_history_window,
};
use crate::features::settings::window::{
    init_settings_window, show_settings_window, sync_settings_window,
};
use crate::features::text_translation::requests::TranslationRequests;
use crate::features::text_translation::translator::TranslationService;
use crate::features::text_translation::window::{show_translation_editor, trigger_translation};
use crate::features::time_trans::window::{show_copied_timestamp, show_current_timestamp};
use crate::infrastructure::paths::clipboard_history_dir;
use crate::infrastructure::tray::{TrayAction, TrayMenuHandles};
use crate::platform::dialog::show_message_box;
use crate::settings::{AppSettings, CopyMode, SettingsService};

pub(super) struct AppController {
    windows: RefCell<AppWindows>,
    settings: SettingsService,
    history: Arc<ClipboardHistoryService>,
    translation: Arc<TranslationService>,
    requests: Arc<TranslationRequests>,
    suppress_shortcuts: Arc<AtomicBool>,
    tray: TrayMenuHandles,
    events: Sender<AppEvent>,
    copies: CopyDispatcher,
}

impl AppController {
    /// 组装共享服务；窗口保留为空，首次使用时再创建。
    pub fn new(settings: SettingsService, tray: TrayMenuHandles, events: Sender<AppEvent>) -> Self {
        let snapshot = settings.snapshot();
        Self {
            windows: RefCell::new(AppWindows::default()),
            history: Arc::new(ClipboardHistoryService::load(clipboard_history_dir())),
            translation: Arc::new(TranslationService::new(&snapshot.tencent_cloud)),
            requests: Arc::new(TranslationRequests::default()),
            suppress_shortcuts: Arc::new(AtomicBool::new(false)),
            copies: CopyDispatcher::new(events.clone()),
            settings,
            tray,
            events,
        }
    }

    /// 启动全局快捷键，并按配置启用独立于窗口的历史记录监听。
    pub fn start_services(&self) {
        start_shortcuts(self.events.clone(), Arc::clone(&self.suppress_shortcuts))
            .expect("failed to start global input listener");
        self.ensure_history_listener();
    }

    /// 在历史记录开启时启动监听；服务内部保证不会重复创建线程。
    fn ensure_history_listener(&self) {
        if !self.settings.snapshot().clipboard_history.enabled {
            return;
        }
        let events = self.events.clone();
        if let Err(err) = self
            .history
            .start_listener(self.settings.shared(), move || {
                let _ = events.send(AppEvent::HistoryChanged);
            })
        {
            log::error!("start clipboard listener failed: {err}");
        }
    }

    /// 在 UI 线程统一接收后台事件，配置开关在执行时再次检查。
    pub fn handle_event(&self, event: AppEvent) {
        match event {
            AppEvent::SwitchCopyMode => {
                let mut settings = self.settings.snapshot();
                if settings.switch_copy_mode() {
                    let feature = settings.copy_mode.label();
                    // 复用配置入口，统一保存、同步托盘与页面，并取消被关闭的翻译请求。
                    match self.apply_settings(settings) {
                        // 仅在配置保存成功后提示，避免显示尚未生效的功能状态。
                        Ok(()) => {
                            self.windows
                                .borrow_mut()
                                .notify(&format!("切换ctrl+c快捷功能为{feature}！"));
                        }
                        Err(err) => {
                            show_message_box("切换失败", &format!("无法保存设置：{err}"));
                        }
                    }
                }
            }
            AppEvent::ShowHistory => {
                if self.settings.snapshot().clipboard_history.enabled {
                    self.show_history();
                }
            }
            AppEvent::CopyRequested { position } => {
                let settings = self.settings.snapshot();
                if settings.copy_mode != CopyMode::Disabled {
                    self.copies.request(position);
                }
            }
            AppEvent::CopiedText {
                generation,
                text,
                position,
            } => {
                if self.copies.is_current(generation) {
                    self.handle_copied_text(text, position);
                }
            }
            AppEvent::HistoryChanged => {
                // 未打开过历史窗口时跳过刷新，数据仍由后台服务正常保存。
                let windows = self.windows.borrow();
                if let Some(window) = windows.history.as_ref() {
                    if window.window().is_visible() {
                        refresh_clipboard_history_window(window, &self.history);
                    }
                }
            }
        }
    }

    /// 根据当前选项执行复制功能，关闭时跳过处理。
    fn handle_copied_text(&self, text: String, position: (f64, f64)) {
        let settings = self.settings.snapshot();
        match settings.copy_mode {
            CopyMode::Timestamp => {
                let window = self.windows.borrow_mut().timestamp();
                show_copied_timestamp(&window, &text, position);
            }
            CopyMode::Translation => {
                let window = self.translation_window();
                trigger_translation(
                    window.as_weak(),
                    Arc::clone(&self.requests),
                    Arc::clone(&self.translation),
                    text,
                );
            }
            CopyMode::Disabled => {}
        }
    }

    /// 从主页显示每秒刷新的当前时间，不受复制触发开关限制。
    pub fn show_timestamp(&self) {
        let window = self.windows.borrow_mut().timestamp();
        show_current_timestamp(&window);
    }

    /// 菜单和快捷键统一读取最新历史记录，并居中显示窗口。
    pub fn show_history(&self) {
        let window = self.history_window();
        show_clipboard_history_window(&window, &self.history);
    }

    /// 获取历史窗口句柄，统一组装主页与快捷键入口所需的依赖。
    fn history_window(&self) -> crate::ClipboardHistoryWindow {
        self.windows.borrow_mut().history(
            Arc::clone(&self.history),
            Arc::clone(&self.suppress_shortcuts),
        )
    }

    /// 获取翻译窗口句柄，避免各调用点重复组装相同依赖。
    fn translation_window(&self) -> crate::TextTranslationWindow {
        self.windows.borrow_mut().translation(
            Arc::clone(&self.requests),
            self.settings.shared(),
            Arc::clone(&self.translation),
        )
    }

    /// 打开空白翻译编辑器，同时取消之前的请求和防抖任务。
    pub fn show_translation(&self) {
        self.requests.cancel();
        let window = self.translation_window();
        show_translation_editor(&window);
    }

    /// 按需创建设置窗口，使用弱引用回调避免控制器和窗口相互持有。
    pub fn show_settings(self: &Rc<Self>) {
        let window = {
            let mut windows = self.windows.borrow_mut();
            windows
                .settings
                .get_or_insert_with(|| {
                    let app = Rc::downgrade(self);
                    init_settings_window(move |settings| {
                        app.upgrade()
                            .ok_or_else(|| "应用已经退出".to_string())?
                            .apply_settings(settings)
                    })
                })
                .clone_strong()
        };
        show_settings_window(&window, &self.settings.snapshot());
    }

    /// 接收托盘动作，仅修改配置快照，再交给统一入口保存和生效。
    pub fn handle_tray_action(&self, action: TrayAction) {
        let mut settings = self.settings.snapshot();
        match action {
            TrayAction::CopyMode(mode) => settings.copy_mode = mode,
            TrayAction::ClipboardHistory(enabled) => settings.clipboard_history.enabled = enabled,
        }
        if let Err(err) = self.apply_settings(settings) {
            // 菜单组件先切换了勾选状态，保存失败时恢复到实际生效值。
            self.tray.sync_from_settings(&self.settings.snapshot());
            show_message_box("保存失败", &format!("无法保存设置：{err}"));
        }
    }

    /// 配置保存成功后，统一同步翻译服务、托盘、设置页面和后台监听。
    fn apply_settings(&self, settings: AppSettings) -> Result<(), String> {
        let previous = self.settings.snapshot();
        let settings = match self.settings.apply(settings) {
            Ok(settings) => settings,
            Err(err) => {
                // 保存失败时设置页也恢复实际配置，避免界面显示了未生效的开关。
                if let Some(window) = self.windows.borrow().settings.as_ref() {
                    sync_settings_window(window, &previous);
                }
                return Err(err);
            }
        };
        self.translation.apply_settings(&settings.tencent_cloud);
        self.tray.sync_from_settings(&settings);
        if previous.copy_mode == CopyMode::Translation
            && settings.copy_mode != CopyMode::Translation
        {
            self.requests.cancel();
        }
        {
            let windows = self.windows.borrow();
            if let Some(window) = windows.settings.as_ref() {
                sync_settings_window(window, &settings);
            }
        }
        self.ensure_history_listener();
        Ok(())
    }
}
