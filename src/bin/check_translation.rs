use easy_tool::features::text_translation::translator::TranslationService;
use easy_tool::settings::{TextTranslationSettings, TranslationDirection};

fn main() -> Result<(), String> {
    let service = TranslationService::new(&TextTranslationSettings {
        enabled: true,
        direction: TranslationDirection::ZhToEn,
        debounce_seconds: 1,
        zh_to_en_model_dir: None,
        en_to_zh_model_dir: None,
    });

    println!("{}", service.translate("你好，世界")?);
    Ok(())
}
