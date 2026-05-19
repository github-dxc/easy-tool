//! Slint settings window setup and runtime integration.

use std::path::PathBuf;
use std::sync::{Arc, Mutex};

use slint::{CloseRequestResponse, ComponentHandle};

use crate::features::text_translation::translator::TranslationService;
use crate::infrastructure::tray::TrayMenuHandles;
use crate::platform::dialog::{open_folder_dialog, show_message_box};
use crate::platform::window::activate_slint_window;
use crate::settings::{AppSettings, SettingsStore, TextTranslationSettings, TranslationDirection};
use crate::SettingsWindow;

pub fn init_settings_window(
    settings: Arc<Mutex<AppSettings>>,
    settings_store: SettingsStore,
    translation_service: Arc<TranslationService>,
    tray_menu_handles: TrayMenuHandles,
) -> SettingsWindow {
    let window = SettingsWindow::new().unwrap();

    window.window().on_close_requested(|| CloseRequestResponse::HideWindow);

    {
        let weak_window = window.as_weak();
        let settings = Arc::clone(&settings);
        let settings_store = settings_store.clone();
        let translation_service = Arc::clone(&translation_service);
        let tray_menu_handles = tray_menu_handles.clone();
        window.on_browse_zh_to_en_model_dir(move || {
            let Some(path) = open_folder_dialog("选择中译英模型目录") else {
                return;
            };

            let selected = path.to_string_lossy().to_string();
            if let Some(window) = weak_window.upgrade() {
                window.set_zh_to_en_model_dir(selected.into());
                apply_from_window(
                    &window,
                    &settings,
                    &settings_store,
                    &translation_service,
                    &tray_menu_handles,
                );
            }
        });
    }

    {
        let weak_window = window.as_weak();
        let settings = Arc::clone(&settings);
        let settings_store = settings_store.clone();
        let translation_service = Arc::clone(&translation_service);
        let tray_menu_handles = tray_menu_handles.clone();
        window.on_browse_en_to_zh_model_dir(move || {
            let Some(path) = open_folder_dialog("选择英译中模型目录") else {
                return;
            };

            let selected = path.to_string_lossy().to_string();
            if let Some(window) = weak_window.upgrade() {
                window.set_en_to_zh_model_dir(selected.into());
                apply_from_window(
                    &window,
                    &settings,
                    &settings_store,
                    &translation_service,
                    &tray_menu_handles,
                );
            }
        });
    }

    {
        let settings = Arc::clone(&settings);
        let translation_service = Arc::clone(&translation_service);
        let tray_menu_handles = tray_menu_handles.clone();
        let settings_store = settings_store.clone();
        window.on_apply_settings(
            move |copy_timestamp_enabled,
                  clipboard_history_enabled,
                  text_translation_enabled,
                  zh_to_en_enabled,
                  zh_to_en_model_dir,
                  en_to_zh_model_dir,
                  text_translation_debounce_seconds| {
                apply_settings_snapshot(
                    &settings,
                    &settings_store,
                    &translation_service,
                    &tray_menu_handles,
                    copy_timestamp_enabled,
                    clipboard_history_enabled,
                    text_translation_enabled,
                    zh_to_en_enabled,
                    &zh_to_en_model_dir,
                    &en_to_zh_model_dir,
                    &text_translation_debounce_seconds,
                );
            },
        );
    }

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

pub fn show_settings_window(window: &SettingsWindow, settings: &AppSettings) {
    window.set_copy_timestamp_enabled(settings.copy_timestamp.enabled);
    window.set_clipboard_history_enabled(settings.clipboard_history.enabled);
    window.set_text_translation_enabled(settings.text_translation.enabled);
    window.set_zh_to_en_enabled(
        settings.text_translation.direction == TranslationDirection::ZhToEn,
    );
    window.set_zh_to_en_model_dir(
        settings
            .text_translation
            .zh_to_en_model_path()
            .to_string_lossy()
            .to_string()
            .into(),
    );
    window.set_en_to_zh_model_dir(
        settings
            .text_translation
            .en_to_zh_model_path()
            .to_string_lossy()
            .to_string()
            .into(),
    );
    window.set_text_translation_debounce_seconds(
        settings.text_translation.debounce_seconds.to_string().into(),
    );

    let _ = window.show();
    activate_slint_window(window);
}

fn path_from_string(path: &str) -> Option<PathBuf> {
    let trimmed = path.trim();
    if trimmed.is_empty() {
        None
    } else {
        Some(PathBuf::from(trimmed))
    }
}

fn debounce_seconds_from_string(value: &str) -> u64 {
    value.trim().parse::<u64>().ok().filter(|v| *v > 0).unwrap_or(1)
}

fn apply_from_window(
    window: &SettingsWindow,
    settings: &Arc<Mutex<AppSettings>>,
    settings_store: &SettingsStore,
    translation_service: &Arc<TranslationService>,
    tray_menu_handles: &TrayMenuHandles,
) {
    apply_settings_snapshot(
        settings,
        settings_store,
        translation_service,
        tray_menu_handles,
        window.get_copy_timestamp_enabled(),
        window.get_clipboard_history_enabled(),
        window.get_text_translation_enabled(),
        window.get_zh_to_en_enabled(),
        &window.get_zh_to_en_model_dir(),
        &window.get_en_to_zh_model_dir(),
        &window.get_text_translation_debounce_seconds(),
    );
}

fn apply_settings_snapshot(
    settings: &Arc<Mutex<AppSettings>>,
    settings_store: &SettingsStore,
    translation_service: &Arc<TranslationService>,
    tray_menu_handles: &TrayMenuHandles,
    copy_timestamp_enabled: bool,
    clipboard_history_enabled: bool,
    text_translation_enabled: bool,
    zh_to_en_enabled: bool,
    zh_to_en_model_dir: &str,
    en_to_zh_model_dir: &str,
    text_translation_debounce_seconds: &str,
) {
    let new_text_translation = TextTranslationSettings {
        enabled: text_translation_enabled,
        direction: if zh_to_en_enabled {
            TranslationDirection::ZhToEn
        } else {
            TranslationDirection::EnToZh
        },
        debounce_seconds: debounce_seconds_from_string(text_translation_debounce_seconds),
        zh_to_en_model_dir: path_from_string(zh_to_en_model_dir),
        en_to_zh_model_dir: path_from_string(en_to_zh_model_dir),
    };

    let updated_settings = {
        let mut settings = settings.lock().unwrap();
        settings.copy_timestamp.enabled = copy_timestamp_enabled;
        settings.clipboard_history.enabled = clipboard_history_enabled;
        settings.text_translation = new_text_translation;
        settings.clone()
    };

    if let Err(err) = settings_store.save(&updated_settings) {
        show_message_box("保存失败", &format!("无法保存设置：{err}"));
        return;
    }

    tray_menu_handles.sync_from_settings(&updated_settings);
    translation_service.apply_settings(&updated_settings.text_translation);
}
