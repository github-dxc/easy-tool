//! Persistent application settings stored as a TOML file in the user config dir.

use std::fs;
use std::path::PathBuf;
use std::sync::{Arc, Mutex};

use serde::{Deserialize, Serialize};

/// Top-level user preferences used by tray toggles and runtime features.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AppSettings {
    #[serde(default)]
    pub tencent_cloud: TencentCloudSettings,
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

/// Tencent Cloud credentials used by text translation.
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
    /// 构造默认配置，开启时间转换与历史记录，关闭复制触发翻译。
    fn default() -> Self {
        Self {
            tencent_cloud: TencentCloudSettings::default(),
            copy_timestamp: default_enabled_copy_timestamp(),
            clipboard_history: default_enabled_clipboard_history(),
            text_translation: TextTranslationSettings::default(),
        }
    }
}

/// 返回默认开启的复制时间转换配置。
fn default_enabled_copy_timestamp() -> CopyTimestampSettings {
    CopyTimestampSettings { enabled: true }
}

/// 返回默认开启的剪贴板历史配置。
fn default_enabled_clipboard_history() -> ClipboardHistorySettings {
    ClipboardHistorySettings { enabled: true }
}

impl Default for TextTranslationSettings {
    /// 默认关闭复制触发翻译，并使用一秒输入防抖时间。
    fn default() -> Self {
        Self {
            enabled: false,
            debounce_seconds: default_text_translation_debounce_seconds(),
        }
    }
}

/// 返回默认翻译防抖时间，单位为秒。
fn default_text_translation_debounce_seconds() -> u64 {
    1
}

/// Loads and saves settings from the configured TOML file.
#[derive(Debug, Clone)]
pub struct SettingsStore {
    path: PathBuf,
}

impl SettingsStore {
    /// 创建配置存储并使用当前用户的默认配置路径。
    pub fn new() -> Self {
        Self {
            path: default_config_path(),
        }
    }

    /// 使用指定路径创建配置存储，方便隔离测试和自定义配置位置。
    pub fn at_path(path: PathBuf) -> Self {
        Self { path }
    }

    /// 返回当前配置文件路径，供日志和诊断使用。
    pub fn path(&self) -> &PathBuf {
        &self.path
    }

    /// 读取配置文件；首次运行时保存并返回默认配置。
    pub fn load_or_create(&self) -> Result<AppSettings, String> {
        if !self.path.exists() {
            let settings = AppSettings::default();
            self.save(&settings)?;
            return Ok(settings);
        }

        let content =
            fs::read_to_string(&self.path).map_err(|err| format!("read settings failed: {err}"))?;
        // Serde ignores removed settings so older config files remain usable.
        toml::from_str(&content).map_err(|err| format!("parse settings failed: {err}"))
    }

    /// 将配置序列化为 TOML 并写入配置目录。
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

/// 统一管理配置快照，并保证保存成功后才更新运行时状态。
pub struct SettingsService {
    store: SettingsStore,
    current: Arc<Mutex<AppSettings>>,
}

impl SettingsService {
    /// 从配置存储加载初始状态，缺少配置时创建默认文件。
    pub fn load(store: SettingsStore) -> Result<Self, String> {
        let settings = store.load_or_create()?;
        Ok(Self {
            store,
            current: Arc::new(Mutex::new(settings)),
        })
    }

    /// 返回独立配置快照，调用方无需长时间持有配置锁。
    pub fn snapshot(&self) -> AppSettings {
        self.current.lock().unwrap().clone()
    }

    /// 向后台服务提供共享只读约定的配置状态，写入统一经过 apply。
    pub fn shared(&self) -> Arc<Mutex<AppSettings>> {
        Arc::clone(&self.current)
    }

    /// 校验并保存新配置；保存失败时保留原来的内存状态。
    pub fn apply(&self, mut settings: AppSettings) -> Result<AppSettings, String> {
        settings.tencent_cloud.secret_id = settings.tencent_cloud.secret_id.trim().to_string();
        settings.tencent_cloud.secret_key = settings.tencent_cloud.secret_key.trim().to_string();
        settings.text_translation.debounce_seconds =
            settings.text_translation.debounce_seconds.max(1);
        self.store.save(&settings)?;
        *self.current.lock().unwrap() = settings.clone();
        Ok(settings)
    }
}

/// 生成当前用户配置目录中的应用配置文件路径。
fn default_config_path() -> PathBuf {
    dirs::config_dir()
        .unwrap_or_else(|| std::env::current_dir().unwrap_or_else(|_| PathBuf::from(".")))
        .join("easy-tool")
        .join("config.toml")
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::tempdir;

    #[test]
    /// 验证旧配置缺少腾讯云密钥时使用空值并保留已有设置。
    fn missing_credentials_default_to_empty() {
        let settings: AppSettings = toml::from_str(
            r#"
[text_translation]
enabled = true
debounce_seconds = 3
"#,
        )
        .unwrap();

        assert!(settings.tencent_cloud.secret_id.is_empty());
        assert!(settings.tencent_cloud.secret_key.is_empty());
        assert!(settings.text_translation.enabled);
        assert_eq!(settings.text_translation.debounce_seconds, 3);
    }

    #[test]
    /// 验证旧配置可读取，重新保存时移除废弃字段并保留 API 设置。
    fn saving_legacy_settings_removes_retired_options_and_preserves_api_preferences() {
        let dir = tempdir().unwrap();
        let store = SettingsStore {
            path: dir.path().join("config.toml"),
        };
        fs::write(
            store.path(),
            r#"
ai_backend = "local"
image_recognition_model_dir = "old-ocr"

[tencent_cloud]
secret_id = "test-id"
secret_key = "test-key"

[copy_timestamp]
enabled = false

[text_translation]
enabled = true
debounce_seconds = 5
direction = "en_to_zh"
zh_to_en_model_dir = "old-zh-en"
en_to_zh_model_dir = "old-en-zh"

[image_recognition]
enabled = true
model_dir = "legacy-ocr"

[screenshot]
enabled = true
"#,
        )
        .unwrap();

        let settings = store.load_or_create().unwrap();
        assert!(!settings.copy_timestamp.enabled);
        assert!(settings.text_translation.enabled);
        assert_eq!(settings.text_translation.debounce_seconds, 5);
        assert_eq!(settings.tencent_cloud.secret_id, "test-id");
        assert_eq!(settings.tencent_cloud.secret_key, "test-key");
        store.save(&settings).unwrap();

        let saved = fs::read_to_string(store.path()).unwrap();
        assert!(!saved.contains("ai_backend"));
        assert!(!saved.contains("model_dir"));
        assert!(!saved.contains("direction"));
        assert!(!saved.contains("[image_recognition]"));
        assert!(!saved.contains("[screenshot]"));
        let reloaded = store.load_or_create().unwrap();
        assert_eq!(reloaded.tencent_cloud.secret_key, "test-key");
        assert_eq!(reloaded.text_translation.debounce_seconds, 5);
    }
    /// 验证配置保存成功后才生效，并统一清理密钥空白和无效防抖值。
    #[test]
    fn settings_service_applies_validated_preferences() {
        let dir = tempdir().unwrap();
        let store = SettingsStore::at_path(dir.path().join("config.toml"));
        let service = SettingsService::load(store.clone()).unwrap();
        let mut changed = service.snapshot();
        changed.text_translation.enabled = true;
        changed.text_translation.debounce_seconds = 0;
        changed.tencent_cloud.secret_id = " test-id ".into();
        service.apply(changed).unwrap();
        assert!(service.snapshot().text_translation.enabled);
        assert_eq!(service.snapshot().text_translation.debounce_seconds, 1);
        assert_eq!(
            store.load_or_create().unwrap().tencent_cloud.secret_id,
            "test-id"
        );
    }

    /// 验证写入失败后内存仍保留原配置，避免托盘和运行状态出现分歧。
    #[test]
    fn settings_service_preserves_state_when_save_fails() {
        let dir = tempdir().unwrap();
        let path = dir.path().join("config.toml");
        let service = SettingsService::load(SettingsStore::at_path(path.clone())).unwrap();
        std::fs::remove_file(&path).unwrap();
        std::fs::create_dir(&path).unwrap();
        let mut changed = service.snapshot();
        changed.copy_timestamp.enabled = false;
        assert!(service.apply(changed).is_err());
        assert!(service.snapshot().copy_timestamp.enabled);
    }
}
