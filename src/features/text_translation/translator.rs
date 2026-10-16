//! Text translation through the Tencent Cloud TMT API.

use std::sync::Mutex;

use crate::features::text_translation::language::{
    DetectedTranslationDirection, detect_translation_direction,
};
use crate::infrastructure::tencent_cloud::client::{TencentTranslationDirection, translate_text};
use crate::settings::TencentCloudSettings;

pub struct TranslationService {
    credentials: Mutex<TencentCloudSettings>,
}

impl TranslationService {
    /// 使用当前腾讯云凭据创建文本翻译服务。
    pub fn new(credentials: &TencentCloudSettings) -> Self {
        Self {
            credentials: Mutex::new(credentials.clone()),
        }
    }

    /// 更新翻译服务保存的 API 凭据快照。
    pub fn apply_settings(&self, credentials: &TencentCloudSettings) {
        *self.credentials.lock().unwrap() = credentials.clone();
    }

    /// 按照检测到的语言方向调用 API，不额外进行取消检查。
    pub fn translate(&self, text: &str) -> Result<String, String> {
        self.translate_cancellable(text, || false)
    }

    /// 请求前及返回后检查取消状态，使已失效结果不再生效。
    pub fn translate_cancellable(
        &self,
        text: &str,
        should_cancel: impl Fn() -> bool,
    ) -> Result<String, String> {
        let Some(direction) = detect_translation_direction(text) else {
            return Ok(String::new());
        };
        if should_cancel() {
            return Ok(String::new());
        }

        let credentials = self.credentials.lock().unwrap().clone();
        let translated = translate_text(&credentials, api_direction_for(direction), text)?;
        if should_cancel() {
            return Ok(String::new());
        }
        Ok(translated)
    }
}

/// 将语言检测结果映射为腾讯云 API 使用的源目标语言。
fn api_direction_for(direction: DetectedTranslationDirection) -> TencentTranslationDirection {
    match direction {
        DetectedTranslationDirection::ZhToEn => TencentTranslationDirection::ZhToEn,
        DetectedTranslationDirection::EnToZh => TencentTranslationDirection::EnToZh,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    /// 验证语言检测方向与 API 翻译方向一致。
    fn maps_detected_direction_to_tencent_direction() {
        assert!(matches!(
            api_direction_for(DetectedTranslationDirection::ZhToEn),
            TencentTranslationDirection::ZhToEn
        ));
        assert!(matches!(
            api_direction_for(DetectedTranslationDirection::EnToZh),
            TencentTranslationDirection::EnToZh
        ));
    }

    #[test]
    /// 验证不含可翻译文字的输入不会请求 API。
    fn skips_api_for_text_without_detected_direction() {
        let service = TranslationService::new(&TencentCloudSettings::default());
        assert_eq!(service.translate("123, !?"), Ok(String::new()));
    }

    #[test]
    /// 验证已取消任务在访问 API 前结束。
    fn skips_cancelled_api_request() {
        let service = TranslationService::new(&TencentCloudSettings::default());
        assert_eq!(
            service.translate_cancellable("hello", || true),
            Ok(String::new())
        );
    }

    #[test]
    /// 验证未配置腾讯云访问密钥时返回明确错误。
    fn translation_requires_api_credentials() {
        let service = TranslationService::new(&TencentCloudSettings::default());
        assert_eq!(
            service.translate("你好"),
            Err("Tencent Cloud secret_id is empty".to_string())
        );
    }
}
