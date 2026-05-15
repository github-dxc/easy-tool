use easy_tool::features::text_translation::translator::TranslationService;
use easy_tool::settings::{TextTranslationSettings, TranslationDirection};

fn main() -> Result<(), String> {
    let service = TranslationService::new(&TextTranslationSettings {
        enabled: true,
        direction: TranslationDirection::ZhToEn,
    });

    println!("{}", service.translate("你好，世界")?);
    Ok(())
}
