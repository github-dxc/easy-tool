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

/// Controls copy-triggered text translation and model loading.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TextTranslationSettings {
    pub enabled: bool,
    #[serde(default)]
    pub direction: TranslationDirection,
    #[serde(default = "default_text_translation_debounce_seconds")]
    pub debounce_seconds: u64,
    #[serde(default)]
    pub zh_to_en_model_dir: Option<PathBuf>,
    #[serde(default)]
    pub en_to_zh_model_dir: Option<PathBuf>,
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
            direction: TranslationDirection::ZhToEn,
            debounce_seconds: default_text_translation_debounce_seconds(),
            zh_to_en_model_dir: None,
            en_to_zh_model_dir: None,
        }
    }
}

fn default_text_translation_debounce_seconds() -> u64 {
    1
}

impl Default for TranslationDirection {
    fn default() -> Self {
        Self::ZhToEn
    }
}

impl ImageRecognitionSettings {
    pub fn model_path(&self) -> PathBuf {
        self.model_dir
            .clone()
            .unwrap_or_else(default_image_recognition_model_path)
    }
}

impl TextTranslationSettings {
    pub fn model_path(&self) -> PathBuf {
        match self.direction {
            TranslationDirection::ZhToEn => self.zh_to_en_model_path(),
            TranslationDirection::EnToZh => self.en_to_zh_model_path(),
        }
    }

    pub fn zh_to_en_model_path(&self) -> PathBuf {
        self.zh_to_en_model_dir
            .clone()
            .unwrap_or_else(default_zh_to_en_translation_model_path)
    }

    pub fn en_to_zh_model_path(&self) -> PathBuf {
        self.en_to_zh_model_dir
            .clone()
            .unwrap_or_else(default_en_to_zh_translation_model_path)
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

fn default_zh_to_en_translation_model_path() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("resource")
        .join("Xenova")
        .join("opus-mt-zh-en")
}

fn default_en_to_zh_translation_model_path() -> PathBuf {
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
