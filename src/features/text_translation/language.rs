#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DetectedTranslationDirection {
    ZhToEn,
    EnToZh,
}

pub fn detect_translation_direction(text: &str) -> Option<DetectedTranslationDirection> {
    if text.trim().is_empty() {
        return None;
    }

    let chinese_count = text.chars().filter(|character| is_cjk(*character)).count();
    let english_count = text
        .chars()
        .filter(|character| character.is_ascii_alphabetic())
        .count();

    if chinese_count > english_count || chinese_count == english_count && chinese_count > 0 {
        Some(DetectedTranslationDirection::ZhToEn)
    } else {
        Some(DetectedTranslationDirection::EnToZh)
    }
}

fn is_cjk(character: char) -> bool {
    matches!(
        character,
        '\u{3400}'..='\u{4DBF}'
            | '\u{4E00}'..='\u{9FFF}'
            | '\u{F900}'..='\u{FAFF}'
            | '\u{20000}'..='\u{2A6DF}'
            | '\u{2A700}'..='\u{2B73F}'
            | '\u{2B740}'..='\u{2B81F}'
            | '\u{2B820}'..='\u{2CEAF}'
            | '\u{2CEB0}'..='\u{2EBEF}'
            | '\u{30000}'..='\u{3134F}'
    )
}

#[cfg(test)]
mod tests {
    use super::{DetectedTranslationDirection, detect_translation_direction};

    #[test]
    fn detects_chinese_majority_as_zh_to_en() {
        assert_eq!(
            detect_translation_direction("你好 a 世界"),
            Some(DetectedTranslationDirection::ZhToEn)
        );
    }

    #[test]
    fn detects_english_majority_as_en_to_zh() {
        assert_eq!(
            detect_translation_direction("hello world 你"),
            Some(DetectedTranslationDirection::EnToZh)
        );
    }

    #[test]
    fn detects_chinese_tie_as_zh_to_en() {
        assert_eq!(
            detect_translation_direction("你a"),
            Some(DetectedTranslationDirection::ZhToEn)
        );
    }

    #[test]
    fn counts_ascii_alphabetic_characters() {
        assert_eq!(
            detect_translation_direction("你abc"),
            Some(DetectedTranslationDirection::EnToZh)
        );
    }

    #[test]
    fn returns_none_for_whitespace() {
        assert_eq!(detect_translation_direction(" \n\t"), None);
    }

    #[test]
    fn defaults_punctuation_and_numbers_to_en_to_zh() {
        assert_eq!(
            detect_translation_direction("123, !?"),
            Some(DetectedTranslationDirection::EnToZh)
        );
    }
}
