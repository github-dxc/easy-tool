//! Persistent application settings stored as a TOML file in the user config dir.

use std::fs;
use std::path::PathBuf;

use serde::{Deserialize, Serialize};

/// Top-level user preferences used by tray toggles and runtime features.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AppSettings {
    #[serde(default)]
    pub ai_backend: AiBackend,
    #[serde(default)]
    pub tencent_cloud: TencentCloudSettings,
    #[serde(default = "default_enabled_copy_timestamp")]
    pub copy_timestamp: CopyTimestampSettings,
    #[serde(default = "default_enabled_clipboard_history")]
    pub clipboard_history: ClipboardHistorySettings,
    #[serde(default = "default_enabled_screenshot")]
    pub screenshot: ScreenshotSettings,
    #[serde(default)]
    pub text_translation: TextTranslationSettings,
    #[serde(default = "default_enabled_image_recognition")]
    pub image_recognition: ImageRecognitionSettings,
}

/// Controls whether copied timestamps show the floating conversion window.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CopyTimestampSettings {
    pub enabled: bool,
}

/// Controls whether clipboard changes are captured into history.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ClipboardHistorySettings {
    pub enabled: bool,
}

/// Controls whether the global screenshot shortcut is enabled.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ScreenshotSettings {
    pub enabled: bool,
}

/// Controls image text recognition and model loading.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ImageRecognitionSettings {
    pub enabled: bool,
    #[serde(default)]
    pub model_dir: Option<PathBuf>,
}

/// Selects the backend used by AI-powered features.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "snake_case")]
pub enum AiBackend {
    #[default]
    Local,
    Tencent,
}

/// Tencent Cloud credentials used when the Tencent backend is selected.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct TencentCloudSettings {
    #[serde(default)]
    pub secret_id: String,
    #[serde(default)]
    pub secret_key: String,
}

/// Controls copy-triggered text translation.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TextTranslationSettings {
    pub enabled: bool,
    #[serde(default = "default_text_translation_debounce_seconds")]
    pub debounce_seconds: u64,
}

impl Default for AppSettings {
    fn default() -> Self {
        Self {
            ai_backend: AiBackend::default(),
            tencent_cloud: TencentCloudSettings::default(),
            copy_timestamp: CopyTimestampSettings { enabled: true },
            clipboard_history: ClipboardHistorySettings { enabled: true },
            screenshot: ScreenshotSettings { enabled: true },
            text_translation: TextTranslationSettings::default(),
            image_recognition: ImageRecognitionSettings::default(),
        }
    }
}

fn default_enabled_copy_timestamp() -> CopyTimestampSettings {
    CopyTimestampSettings { enabled: true }
}

fn default_enabled_clipboard_history() -> ClipboardHistorySettings {
    ClipboardHistorySettings { enabled: true }
}

fn default_enabled_screenshot() -> ScreenshotSettings {
    ScreenshotSettings { enabled: true }
}

fn default_enabled_image_recognition() -> ImageRecognitionSettings {
    ImageRecognitionSettings {
        enabled: true,
        model_dir: None,
    }
}

impl Default for ImageRecognitionSettings {
    fn default() -> Self {
        Self {
            enabled: true,
            model_dir: None,
        }
    }
}

impl Default for TextTranslationSettings {
    fn default() -> Self {
        Self {
            enabled: false,
            debounce_seconds: default_text_translation_debounce_seconds(),
        }
    }
}

fn default_text_translation_debounce_seconds() -> u64 {
    1
}

impl ImageRecognitionSettings {
    pub fn model_path(&self) -> PathBuf {
        self.model_dir
            .clone()
            .unwrap_or_else(default_image_recognition_model_path)
    }
}

/// Loads and saves settings from the configured TOML file.
#[derive(Debug, Clone)]
pub struct SettingsStore {
    path: PathBuf,
}

impl SettingsStore {
    pub fn new() -> Self {
        Self {
            path: default_config_path(),
        }
    }

    pub fn path(&self) -> &PathBuf {
        &self.path
    }

    pub fn load_or_create(&self) -> Result<AppSettings, String> {
        if !self.path.exists() {
            let settings = AppSettings::default();
            self.save(&settings)?;
            return Ok(settings);
        }

        let content =
            fs::read_to_string(&self.path).map_err(|err| format!("read settings failed: {err}"))?;
        let settings: AppSettings =
            toml::from_str(&content).map_err(|err| format!("parse settings failed: {err}"))?;

        Ok(settings)
    }

    pub fn save(&self, settings: &AppSettings) -> Result<(), String> {
        if let Some(parent) = self.path.parent() {
            fs::create_dir_all(parent)
                .map_err(|err| format!("create settings dir failed: {err}"))?;
        }

        let content = toml::to_string_pretty(settings)
            .map_err(|err| format!("serialize settings failed: {err}"))?;
        fs::write(&self.path, content).map_err(|err| format!("write settings failed: {err}"))
    }
}

fn default_config_path() -> PathBuf {
    dirs::config_dir()
        .unwrap_or_else(|| std::env::current_dir().unwrap_or_else(|_| PathBuf::from(".")))
        .join("easy-tool")
        .join("config.toml")
}

pub(crate) fn default_zh_to_en_translation_model_path() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("resource")
        .join("Xenova")
        .join("opus-mt-zh-en")
}

pub(crate) fn default_en_to_zh_translation_model_path() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("resource")
        .join("Xenova")
        .join("opus-mt-en-zh")
}

fn default_image_recognition_model_path() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("resource")
        .join("image-recognition")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn missing_backend_fields_default_to_local_with_empty_tencent_credentials() {
        let settings: AppSettings = toml::from_str(
            r#"
[text_translation]
enabled = true
debounce_seconds = 3
"#,
        )
        .expect("settings should deserialize without backend fields");

        assert_eq!(settings.ai_backend, AiBackend::Local);
        assert_eq!(settings.tencent_cloud.secret_id, "");
        assert_eq!(settings.tencent_cloud.secret_key, "");
    }

    #[test]
    fn legacy_text_translation_fields_do_not_break_deserialization() {
        let settings: AppSettings = toml::from_str(
            r#"
[text_translation]
enabled = true
debounce_seconds = 5
direction = "en_to_zh"
zh_to_en_model_dir = "resource/Xenova/opus-mt-zh-en"
en_to_zh_model_dir = "resource/Xenova/opus-mt-en-zh"
"#,
        )
        .expect("legacy text translation settings should deserialize");

        assert!(settings.text_translation.enabled);
        assert_eq!(settings.text_translation.debounce_seconds, 5);
    }
}
