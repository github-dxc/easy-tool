//! Slint settings window setup and runtime integration.

use slint::{CloseRequestResponse, ComponentHandle};

use crate::SettingsWindow;
use crate::platform::dialog::show_message_box;
use crate::platform::window::show_centered_window;
use crate::settings::{AppSettings, CopyMode, TencentCloudSettings};

/// 创建设置页面；页面只收集输入，保存与服务更新由应用控制器负责。
pub fn init_settings_window(
    apply_settings: impl Fn(AppSettings) -> Result<(), String> + 'static,
) -> SettingsWindow {
    let window = SettingsWindow::new().unwrap();

    window
        .window()
        .on_close_requested(|| CloseRequestResponse::HideWindow);

    let weak_window = window.as_weak();
    window.on_apply_settings(move || {
        let Some(window) = weak_window.upgrade() else {
            return;
        };
        let mut updated_settings = AppSettings::default();
        updated_settings.copy_mode_shortcut_enabled = window.get_copy_mode_shortcut_enabled();
        updated_settings.copy_mode = CopyMode::from_index(window.get_copy_mode_index());
        updated_settings.clipboard_history.enabled = window.get_clipboard_history_enabled();
        updated_settings.text_translation.debounce_seconds =
            debounce_seconds_from_string(&window.get_text_translation_debounce_seconds());
        updated_settings.tencent_cloud = TencentCloudSettings {
            secret_id: window.get_tencent_secret_id().trim().to_string(),
            secret_key: window.get_tencent_secret_key().trim().to_string(),
        };

        if let Err(err) = apply_settings(updated_settings) {
            show_message_box("保存失败", &format!("无法保存设置：{err}"));
        }
    });

    window.on_close_requested({
        let weak_window = window.as_weak();
        move || {
            let _ = weak_window.upgrade_in_event_loop(|window| {
                window.hide().ok();
            });
        }
    });

    window
}

/// 填入当前配置，在显示前完成尺寸准备与居中定位。
pub fn show_settings_window(window: &SettingsWindow, settings: &AppSettings) {
    sync_settings_window(window, settings);
    show_centered_window(
        window,
        slint::LogicalSize::new(window.get_initial_width(), window.get_initial_height()),
    );
}

/// 同步页面数值而不改变窗口可见性，供托盘与设置入口共享使用。
pub fn sync_settings_window(window: &SettingsWindow, settings: &AppSettings) {
    window.set_copy_mode_shortcut_enabled(settings.copy_mode_shortcut_enabled);
    window.set_copy_mode_index(settings.copy_mode.index());
    window.set_clipboard_history_enabled(settings.clipboard_history.enabled);
    window.set_tencent_secret_id(settings.tencent_cloud.secret_id.clone().into());
    window.set_tencent_secret_key(settings.tencent_cloud.secret_key.clone().into());
    let debounce_seconds = settings.text_translation.debounce_seconds.to_string();
    window.set_text_translation_debounce_seconds(debounce_seconds.clone().into());
    window.set_last_valid_text_translation_debounce_seconds(debounce_seconds.into());
}

/// 将输入解析为正整数秒数，无效值回退到一秒。
fn debounce_seconds_from_string(value: &str) -> u64 {
    value
        .trim()
        .parse::<u64>()
        .ok()
        .filter(|v| *v > 0)
        .unwrap_or(1)
}
