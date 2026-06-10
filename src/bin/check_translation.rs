use easy_tool::features::text_translation::translator::TranslationService;
use easy_tool::settings::TextTranslationSettings;

fn main() -> Result<(), String> {
    let service = TranslationService::new(&TextTranslationSettings {
        enabled: true,
        debounce_seconds: 1,
    });

    println!("{}", service.translate("你好，世界")?);
    Ok(())
}
