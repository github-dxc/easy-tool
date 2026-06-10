use easy_tool::features::text_translation::translator::TranslationService;
use easy_tool::settings::{AiBackend, TencentCloudSettings, TextTranslationSettings};

fn main() -> Result<(), String> {
    let service = TranslationService::new(
        &TextTranslationSettings {
            enabled: true,
            zh_to_en_model_dir: None,
            en_to_zh_model_dir: None,
            debounce_seconds: 1,
        },
        AiBackend::Local,
        &TencentCloudSettings::default(),
    );

    println!("{}", service.translate("你好，世界")?);
    Ok(())
}
