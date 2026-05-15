//! Persistent application settings stored as a TOML file in the user config dir.

use std::fs;
use std::path::PathBuf;

use serde::{Deserialize, Serialize};

/// Top-level user preferences used by tray toggles and runtime features.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AppSettings {
    #[serde(default = "default_enabled_copy_timestamp")]
    pub copy_timestamp: CopyTimestampSettings,
    #[serde(default = "default_enabled_clipboard_history")]
    pub clipboard_history: ClipboardHistorySettings,
    #[serde(default)]
    pub text_translation: TextTranslationSettings,
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

/// Controls copy-triggered text translation and model loading.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TextTranslationSettings {
    pub enabled: bool,
    #[serde(default)]
    pub direction: TranslationDirection,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum TranslationDirection {
    ZhToEn,
    EnToZh,
}

impl Default for AppSettings {
    fn default() -> Self {
        Self {
            copy_timestamp: CopyTimestampSettings { enabled: true },
            clipboard_history: ClipboardHistorySettings { enabled: true },
            text_translation: TextTranslationSettings::default(),
        }
    }
}

fn default_enabled_copy_timestamp() -> CopyTimestampSettings {
    CopyTimestampSettings { enabled: true }
}

fn default_enabled_clipboard_history() -> ClipboardHistorySettings {
    ClipboardHistorySettings { enabled: true }
}

impl Default for TextTranslationSettings {
    fn default() -> Self {
        Self {
            enabled: false,
            direction: TranslationDirection::ZhToEn,
        }
    }
}

impl Default for TranslationDirection {
    fn default() -> Self {
        Self::ZhToEn
    }
}

impl TextTranslationSettings {
    pub fn model_path(&self) -> PathBuf {
        match self.direction {
            TranslationDirection::ZhToEn => zh_to_en_translation_model_path(),
            TranslationDirection::EnToZh => en_to_zh_translation_model_path(),
        }
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

fn zh_to_en_translation_model_path() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("resource")
        .join("Xenova")
        .join("opus-mt-zh-en")
}

fn en_to_zh_translation_model_path() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("resource")
        .join("Xenova")
        .join("opus-mt-en-zh")
}
