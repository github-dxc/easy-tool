//! Persistent application settings stored as a TOML file in the user config dir.

use std::fs;
use std::path::PathBuf;
use std::sync::{Arc, Mutex};

use serde::{Deserialize, Serialize};

/// 复制功能的三个互斥选项，顺序与设置下拉框、托盘子菜单保持一致。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
#[repr(i32)]
pub enum CopyMode {
    Disabled = 0,
    #[default]
    Timestamp = 1,
    Translation = 2,
}

impl CopyMode {
    pub const ALL: [Self; 3] = [Self::Disabled, Self::Timestamp, Self::Translation];

    /// 将下拉框索引转换为复制功能，未知索引安全回退到关闭。
    pub fn from_index(index: i32) -> Self {
        match index {
            1 => Self::Timestamp,
            2 => Self::Translation,
            _ => Self::Disabled,
        }
    }

    /// 返回设置下拉框使用的选项索引。
    pub fn index(self) -> i32 {
        self as i32
    }

    /// 返回托盘选项和切换提示共用的中文名称。
    pub fn label(self) -> &'static str {
        match self {
            Self::Disabled => "关闭",
            Self::Timestamp => "转换时间戳",
            Self::Translation => "翻译文本",
        }
    }
}

/// Top-level user preferences used by tray toggles and runtime features.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct AppSettings {
    pub tencent_cloud: TencentCloudSettings,
    /// 复制触发的唯一功能状态，保存为 disabled、timestamp 或 translation。
    pub copy_mode: CopyMode,
    pub clipboard_history: ClipboardHistorySettings,
    pub text_translation: TextTranslationSettings,
    /// 是否允许使用 Ctrl+Alt+X 切换复制功能，仅在设置页面提供开关。
    pub copy_mode_shortcut_enabled: bool,
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

/// 翻译参数；复制触发状态统一保存在 AppSettings.copy_mode 中。
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TextTranslationSettings {
    #[serde(default = "default_text_translation_debounce_seconds")]
    pub debounce_seconds: u64,
}

impl Default for AppSettings {
    /// 构造默认配置，复制功能选择时间戳转换，并开启历史记录。
    fn default() -> Self {
        Self {
            tencent_cloud: TencentCloudSettings::default(),
            copy_mode: CopyMode::default(),
            clipboard_history: default_enabled_clipboard_history(),
            text_translation: TextTranslationSettings::default(),
            copy_mode_shortcut_enabled: false,
        }
    }
}

impl AppSettings {
    /// 快捷切换复制功能；未启用快捷键时保持原值，当前关闭时先开启时间转换。
    pub fn switch_copy_mode(&mut self) -> bool {
        if !self.copy_mode_shortcut_enabled {
            return false;
        }
        self.copy_mode = match self.copy_mode {
            CopyMode::Timestamp => CopyMode::Translation,
            CopyMode::Disabled | CopyMode::Translation => CopyMode::Timestamp,
        };
        true
    }
}

/// 返回默认开启的剪贴板历史配置。
fn default_enabled_clipboard_history() -> ClipboardHistorySettings {
    ClipboardHistorySettings { enabled: true }
}

impl Default for TextTranslationSettings {
    /// 翻译默认使用一秒输入防抖时间。
    fn default() -> Self {
        Self {
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

    /// 验证默认复制模式与缺省字段，并检查新建配置只保存一个枚举选项。
    #[test]
    fn defaults_save_a_single_copy_mode() {
        let settings: AppSettings = toml::from_str("").unwrap();
        assert_eq!(settings.copy_mode, CopyMode::Timestamp);
        assert!(settings.clipboard_history.enabled);
        assert!(!settings.copy_mode_shortcut_enabled);
        assert!(settings.tencent_cloud.secret_id.is_empty());
        assert!(settings.tencent_cloud.secret_key.is_empty());
        assert_eq!(settings.text_translation.debounce_seconds, 1);

        let dir = tempdir().unwrap();
        let store = SettingsStore::at_path(dir.path().join("config.toml"));
        assert_eq!(
            store.load_or_create().unwrap().copy_mode,
            CopyMode::Timestamp
        );
        let saved: toml::Value =
            toml::from_str(&fs::read_to_string(store.path()).unwrap()).unwrap();
        assert_eq!(saved["copy_mode"].as_str(), Some("timestamp"));
        assert!(saved.get("copy_timestamp").is_none());
        assert!(saved["text_translation"].get("enabled").is_none());
    }

    /// 验证配置保存成功后才生效，并统一清理密钥空白和无效防抖值。
    #[test]
    fn settings_service_applies_validated_preferences() {
        let dir = tempdir().unwrap();
        let store = SettingsStore::at_path(dir.path().join("config.toml"));
        let service = SettingsService::load(store.clone()).unwrap();
        let mut changed = service.snapshot();
        changed.copy_mode = CopyMode::Translation;
        changed.text_translation.debounce_seconds = 0;
        changed.tencent_cloud.secret_id = " test-id ".into();
        changed.tencent_cloud.secret_key = " test-key ".into();
        service.apply(changed).unwrap();
        assert_eq!(service.snapshot().copy_mode, CopyMode::Translation);
        assert_eq!(service.snapshot().text_translation.debounce_seconds, 1);
        let saved = store.load_or_create().unwrap();
        assert_eq!(saved.tencent_cloud.secret_id, "test-id");
        assert_eq!(saved.tencent_cloud.secret_key, "test-key");
    }

    /// 验证写入失败后内存仍保留原枚举，避免托盘和运行状态出现分歧。
    #[test]
    fn settings_service_preserves_state_when_save_fails() {
        let dir = tempdir().unwrap();
        let path = dir.path().join("config.toml");
        let service = SettingsService::load(SettingsStore::at_path(path.clone())).unwrap();
        fs::remove_file(&path).unwrap();
        fs::create_dir(&path).unwrap();
        let mut changed = service.snapshot();
        changed.copy_mode = CopyMode::Translation;
        assert!(service.apply(changed).is_err());
        assert_eq!(service.snapshot().copy_mode, CopyMode::Timestamp);
    }

    /// 验证三种复制模式可从任意状态选择、重复选择，并以对应枚举字符串保存和重载。
    #[test]
    fn copy_mode_selection_persists_from_every_state() {
        for initial in CopyMode::ALL {
            for (selected, serialized) in [
                (CopyMode::Disabled, "disabled"),
                (CopyMode::Timestamp, "timestamp"),
                (CopyMode::Translation, "translation"),
            ] {
                let dir = tempdir().unwrap();
                let store = SettingsStore::at_path(dir.path().join("config.toml"));
                let service = SettingsService::load(store.clone()).unwrap();
                let mut settings = service.snapshot();
                settings.copy_mode = initial;
                settings.text_translation.debounce_seconds = 3;
                settings.tencent_cloud.secret_id = "test-id".into();
                service.apply(settings).unwrap();

                for _ in 0..2 {
                    let mut changed = service.snapshot();
                    changed.copy_mode = selected;
                    let applied = service.apply(changed).unwrap();
                    for settings in [applied, service.snapshot(), store.load_or_create().unwrap()] {
                        assert_eq!(settings.copy_mode, selected);
                        assert_eq!(settings.text_translation.debounce_seconds, 3);
                        assert_eq!(settings.tencent_cloud.secret_id, "test-id");
                    }
                    let saved: toml::Value =
                        toml::from_str(&fs::read_to_string(store.path()).unwrap()).unwrap();
                    assert_eq!(saved["copy_mode"].as_str(), Some(serialized));
                    assert!(saved.get("copy_timestamp").is_none());
                    assert!(saved["text_translation"].get("enabled").is_none());
                }
            }
        }
    }

    /// 验证快捷切换关闭时不会改变三种复制模式中的任何一种。
    #[test]
    fn disabled_copy_mode_shortcut_preserves_selection() {
        for mode in CopyMode::ALL {
            let mut settings = AppSettings {
                copy_mode: mode,
                ..Default::default()
            };
            assert!(!settings.switch_copy_mode());
            assert_eq!(settings.copy_mode, mode);
        }
    }

    /// 验证关闭时先开启时间转换，之后只在两个功能间切换，并持久化枚举与快捷键状态。
    #[test]
    fn copy_mode_shortcut_cycles_and_persists_from_every_state() {
        use CopyMode::{Disabled, Timestamp, Translation};
        for (initial, sequence) in [
            (Disabled, [Timestamp, Translation, Timestamp]),
            (Timestamp, [Translation, Timestamp, Translation]),
            (Translation, [Timestamp, Translation, Timestamp]),
        ] {
            let dir = tempdir().unwrap();
            let store = SettingsStore::at_path(dir.path().join("config.toml"));
            let service = SettingsService::load(store.clone()).unwrap();
            let mut settings = service.snapshot();
            settings.copy_mode_shortcut_enabled = true;
            settings.copy_mode = initial;
            service.apply(settings).unwrap();
            for expected in sequence {
                let mut settings = service.snapshot();
                assert!(settings.switch_copy_mode());
                service.apply(settings).unwrap();
                let reloaded = SettingsService::load(store.clone()).unwrap().snapshot();
                assert!(reloaded.copy_mode_shortcut_enabled);
                assert_eq!(reloaded.copy_mode, expected);
            }
            let mut settings = service.snapshot();
            settings.copy_mode_shortcut_enabled = false;
            service.apply(settings).unwrap();
            assert!(!store.load_or_create().unwrap().copy_mode_shortcut_enabled);
        }
    }

    /// 验证未知枚举值或错误类型报告读取失败，不覆盖原配置文件。
    #[test]
    fn invalid_copy_modes_are_rejected_without_rewriting_config() {
        for content in [
            "copy_mode = \"unknown\"",
            "copy_mode = 1",
            "copy_mode = true",
        ] {
            let dir = tempdir().unwrap();
            let store = SettingsStore::at_path(dir.path().join("config.toml"));
            fs::write(store.path(), content).unwrap();
            assert!(store.load_or_create().is_err());
            assert_eq!(fs::read_to_string(store.path()).unwrap(), content);
        }
    }

    /// 验证读取合法的新格式时保留文件内容，不进行隐式迁移或重写。
    #[test]
    fn reading_settings_does_not_rewrite_config() {
        let dir = tempdir().unwrap();
        let store = SettingsStore::at_path(dir.path().join("config.toml"));
        let content = "# 用户备注\ncopy_mode = \"disabled\"\n";
        fs::write(store.path(), content).unwrap();
        assert_eq!(
            store.load_or_create().unwrap().copy_mode,
            CopyMode::Disabled
        );
        assert_eq!(fs::read_to_string(store.path()).unwrap(), content);
    }
}
