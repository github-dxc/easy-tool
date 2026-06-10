//! Slint settings window setup and runtime integration.

use std::path::PathBuf;
use std::rc::Rc;
use std::sync::{Arc, Mutex};

use slint::{CloseRequestResponse, ComponentHandle};

use crate::SettingsWindow;
use crate::features::file_preview::ocr::OcrService;
use crate::features::text_translation::translator::TranslationService;
use crate::infrastructure::tray::TrayMenuHandles;
use crate::platform::dialog::{open_folder_dialog, show_message_box};
use crate::platform::window::activate_slint_window;
use crate::settings::{
    AiBackend, AppSettings, ImageRecognitionSettings, SettingsStore, TencentCloudSettings,
    TextTranslationSettings,
};

pub fn init_settings_window(
    settings: Arc<Mutex<AppSettings>>,
    settings_store: SettingsStore,
    translation_service: Arc<TranslationService>,
    ocr_service: Arc<OcrService>,
    tray_menu_handles: TrayMenuHandles,
    on_settings_applied: Rc<dyn Fn(&AppSettings)>,
) -> SettingsWindow {
    let window = SettingsWindow::new().unwrap();

    window
        .window()
        .on_close_requested(|| CloseRequestResponse::HideWindow);

    {
        let weak_window = window.as_weak();
        let settings = Arc::clone(&settings);
        let settings_store = settings_store.clone();
        let translation_service = Arc::clone(&translation_service);
        let ocr_service = Arc::clone(&ocr_service);
        let tray_menu_handles = tray_menu_handles.clone();
        let on_settings_applied = Rc::clone(&on_settings_applied);
        window.on_browse_image_recognition_model_dir(move || {
            let Some(path) = open_folder_dialog("选择图像识别模型目录") else {
                return;
            };

            let selected = path.to_string_lossy().to_string();
            if let Some(window) = weak_window.upgrade() {
                window.set_image_recognition_model_dir(selected.into());
                apply_from_window(
                    &window,
                    &settings,
                    &settings_store,
                    &translation_service,
                    &ocr_service,
                    &tray_menu_handles,
                    &on_settings_applied,
                );
            }
        });
    }

    {
        let weak_window = window.as_weak();
        let settings = Arc::clone(&settings);
        let settings_store = settings_store.clone();
        let translation_service = Arc::clone(&translation_service);
        let ocr_service = Arc::clone(&ocr_service);
        let tray_menu_handles = tray_menu_handles.clone();
        let on_settings_applied = Rc::clone(&on_settings_applied);
        window.on_browse_text_translation_zh_to_en_model_dir(move || {
            let Some(path) = open_folder_dialog("选择中译英模型目录") else {
                return;
            };

            let selected = path.to_string_lossy().to_string();
            if let Some(window) = weak_window.upgrade() {
                window.set_text_translation_zh_to_en_model_dir(selected.into());
                apply_from_window(
                    &window,
                    &settings,
                    &settings_store,
                    &translation_service,
                    &ocr_service,
                    &tray_menu_handles,
                    &on_settings_applied,
                );
            }
        });
    }

    {
        let weak_window = window.as_weak();
        let settings = Arc::clone(&settings);
        let settings_store = settings_store.clone();
        let translation_service = Arc::clone(&translation_service);
        let ocr_service = Arc::clone(&ocr_service);
        let tray_menu_handles = tray_menu_handles.clone();
        let on_settings_applied = Rc::clone(&on_settings_applied);
        window.on_browse_text_translation_en_to_zh_model_dir(move || {
            let Some(path) = open_folder_dialog("选择英译中模型目录") else {
                return;
            };

            let selected = path.to_string_lossy().to_string();
            if let Some(window) = weak_window.upgrade() {
                window.set_text_translation_en_to_zh_model_dir(selected.into());
                apply_from_window(
                    &window,
                    &settings,
                    &settings_store,
                    &translation_service,
                    &ocr_service,
                    &tray_menu_handles,
                    &on_settings_applied,
                );
            }
        });
    }

    {
        let weak_window = window.as_weak();
        let settings = Arc::clone(&settings);
        let translation_service = Arc::clone(&translation_service);
        let ocr_service = Arc::clone(&ocr_service);
        let tray_menu_handles = tray_menu_handles.clone();
        let settings_store = settings_store.clone();
        let on_settings_applied = Rc::clone(&on_settings_applied);
        window.on_apply_settings(
            move |copy_timestamp_enabled,
                  clipboard_history_enabled,
                  screenshot_enabled,
                  image_recognition_enabled,
                  text_translation_enabled,
                  tencent_backend_enabled,
                  tencent_secret_id,
                  tencent_secret_key,
                  image_recognition_model_dir,
                  text_translation_debounce_seconds| {
                let Some(window) = weak_window.upgrade() else {
                    return;
                };
                apply_settings_snapshot(
                    &settings,
                    &settings_store,
                    &translation_service,
                    &ocr_service,
                    &tray_menu_handles,
                    &on_settings_applied,
                    copy_timestamp_enabled,
                    clipboard_history_enabled,
                    screenshot_enabled,
                    image_recognition_enabled,
                    text_translation_enabled,
                    tencent_backend_enabled,
                    &tencent_secret_id,
                    &tencent_secret_key,
                    &image_recognition_model_dir,
                    &window.get_text_translation_zh_to_en_model_dir(),
                    &window.get_text_translation_en_to_zh_model_dir(),
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
    window.set_screenshot_enabled(settings.screenshot.enabled);
    window.set_image_recognition_enabled(settings.image_recognition.enabled);
    window.set_text_translation_enabled(settings.text_translation.enabled);
    window.set_tencent_backend_enabled(settings.ai_backend == AiBackend::Tencent);
    window.set_tencent_secret_id(settings.tencent_cloud.secret_id.clone().into());
    window.set_tencent_secret_key(settings.tencent_cloud.secret_key.clone().into());
    window.set_image_recognition_model_dir(
        settings
            .image_recognition
            .model_dir
            .as_ref()
            .map(|path| path.to_string_lossy().to_string())
            .unwrap_or_default()
            .into(),
    );
    window.set_text_translation_zh_to_en_model_dir(
        settings
            .text_translation
            .zh_to_en_model_dir
            .as_ref()
            .map(|path| path.to_string_lossy().to_string())
            .unwrap_or_default()
            .into(),
    );
    window.set_text_translation_en_to_zh_model_dir(
        settings
            .text_translation
            .en_to_zh_model_dir
            .as_ref()
            .map(|path| path.to_string_lossy().to_string())
            .unwrap_or_default()
            .into(),
    );
    window.set_text_translation_debounce_seconds(
        settings
            .text_translation
            .debounce_seconds
            .to_string()
            .into(),
    );
    window.set_last_valid_text_translation_debounce_seconds(
        settings
            .text_translation
            .debounce_seconds
            .to_string()
            .into(),
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
    value
        .trim()
        .parse::<u64>()
        .ok()
        .filter(|v| *v > 0)
        .unwrap_or(1)
}

fn apply_from_window(
    window: &SettingsWindow,
    settings: &Arc<Mutex<AppSettings>>,
    settings_store: &SettingsStore,
    translation_service: &Arc<TranslationService>,
    ocr_service: &Arc<OcrService>,
    tray_menu_handles: &TrayMenuHandles,
    on_settings_applied: &Rc<dyn Fn(&AppSettings)>,
) {
    apply_settings_snapshot(
        settings,
        settings_store,
        translation_service,
        ocr_service,
        tray_menu_handles,
        on_settings_applied,
        window.get_copy_timestamp_enabled(),
        window.get_clipboard_history_enabled(),
        window.get_screenshot_enabled(),
        window.get_image_recognition_enabled(),
        window.get_text_translation_enabled(),
        window.get_tencent_backend_enabled(),
        &window.get_tencent_secret_id(),
        &window.get_tencent_secret_key(),
        &window.get_image_recognition_model_dir(),
        &window.get_text_translation_zh_to_en_model_dir(),
        &window.get_text_translation_en_to_zh_model_dir(),
        &window.get_text_translation_debounce_seconds(),
    );
}

fn apply_settings_snapshot(
    settings: &Arc<Mutex<AppSettings>>,
    settings_store: &SettingsStore,
    translation_service: &Arc<TranslationService>,
    ocr_service: &Arc<OcrService>,
    tray_menu_handles: &TrayMenuHandles,
    on_settings_applied: &Rc<dyn Fn(&AppSettings)>,
    copy_timestamp_enabled: bool,
    clipboard_history_enabled: bool,
    screenshot_enabled: bool,
    image_recognition_enabled: bool,
    text_translation_enabled: bool,
    tencent_backend_enabled: bool,
    tencent_secret_id: &str,
    tencent_secret_key: &str,
    image_recognition_model_dir: &str,
    text_translation_zh_to_en_model_dir: &str,
    text_translation_en_to_zh_model_dir: &str,
    text_translation_debounce_seconds: &str,
) {
    let ai_backend = if tencent_backend_enabled {
        AiBackend::Tencent
    } else {
        AiBackend::Local
    };
    let tencent_cloud = TencentCloudSettings {
        secret_id: tencent_secret_id.trim().to_string(),
        secret_key: tencent_secret_key.trim().to_string(),
    };
    let new_text_translation = TextTranslationSettings {
        enabled: text_translation_enabled,
        zh_to_en_model_dir: path_from_string(text_translation_zh_to_en_model_dir),
        en_to_zh_model_dir: path_from_string(text_translation_en_to_zh_model_dir),
        debounce_seconds: debounce_seconds_from_string(text_translation_debounce_seconds),
    };
    let new_image_recognition = ImageRecognitionSettings {
        enabled: image_recognition_enabled,
        model_dir: path_from_string(image_recognition_model_dir),
    };

    let updated_settings = {
        let mut settings = settings.lock().unwrap();
        settings.ai_backend = ai_backend;
        settings.tencent_cloud = tencent_cloud;
        settings.copy_timestamp.enabled = copy_timestamp_enabled;
        settings.clipboard_history.enabled = clipboard_history_enabled;
        settings.screenshot.enabled = screenshot_enabled;
        settings.image_recognition = new_image_recognition;
        settings.text_translation = new_text_translation;
        settings.clone()
    };

    if let Err(err) = settings_store.save(&updated_settings) {
        show_message_box("保存失败", &format!("无法保存设置：{err}"));
        return;
    }

    tray_menu_handles.sync_from_settings(&updated_settings);
    translation_service.apply_settings(
        &updated_settings.text_translation,
        updated_settings.ai_backend,
        &updated_settings.tencent_cloud,
    );
    ocr_service.apply_settings(
        &updated_settings.image_recognition,
        updated_settings.ai_backend,
        &updated_settings.tencent_cloud,
    );
    on_settings_applied(&updated_settings);
}
