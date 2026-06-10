//! Translation runtime backed by ONNX Runtime model loading and execution.

use std::fs;
use std::path::{Path, PathBuf};
use std::sync::Mutex;
use std::time::Instant;

use ort::session::{Session, builder::GraphOptimizationLevel};
use ort::value::Tensor;
use serde_json::Value;
use tokenizers::Tokenizer;

use crate::features::text_translation::language::{
    DetectedTranslationDirection, detect_translation_direction,
};
use crate::infrastructure::idle_model::IdleModel;
use crate::infrastructure::tencent_cloud::client::{
    TencentTranslationDirection, translate_text as translate_text_with_tencent,
};
use crate::settings::{
    AiBackend, TencentCloudSettings, TextTranslationSettings,
    default_en_to_zh_translation_model_path, default_zh_to_en_translation_model_path,
};

const DECODER_START_TOKEN_ID: i64 = 65000;
const EOS_TOKEN_ID: i64 = 0;
const PAD_TOKEN_ID: i64 = 65000;
const SOURCE_SEQUENCE_LEN: usize = 512;
const SOURCE_CHUNK_PAYLOAD_LEN: usize = SOURCE_SEQUENCE_LEN - 1;
const SOURCE_CHUNK_TARGET_LEN: usize = 384;
const MIN_DECODER_SEQUENCE_LEN: usize = 128;
const MAX_DECODER_SEQUENCE_LEN: usize = 512;
const NO_REPEAT_NGRAM_SIZE: usize = 3;
const REPETITION_PENALTY: f32 = 1.2;

/// Owns lightweight translation state and loads the model on first use.
pub struct TranslationService {
    state: Mutex<TranslationState>,
    zh_to_en_model: Mutex<IdleModel<TranslationModel>>,
    en_to_zh_model: Mutex<IdleModel<TranslationModel>>,
}

struct TranslationState {
    enabled: bool,
    ai_backend: AiBackend,
    tencent_cloud: TencentCloudSettings,
}

struct TranslationModel {
    tokenizer: Tokenizer,
    encoder: Session,
    decoder: Session,
}

struct SourceChunk {
    input_ids: Vec<i64>,
    attention_mask: Vec<i64>,
}

impl TranslationService {
    /// Creates the service without loading the model.
    pub fn new(
        settings: &TextTranslationSettings,
        ai_backend: AiBackend,
        tencent_cloud: &TencentCloudSettings,
    ) -> Self {
        let service = Self {
            state: Mutex::new(TranslationState {
                enabled: false,
                ai_backend,
                tencent_cloud: tencent_cloud.clone(),
            }),
            zh_to_en_model: Mutex::new(IdleModel::empty()),
            en_to_zh_model: Mutex::new(IdleModel::empty()),
        };
        service.apply_settings(settings, ai_backend, tencent_cloud);
        service
    }

    /// Applies tray/config changes without loading the ONNX runtime plan.
    pub fn apply_settings(
        &self,
        settings: &TextTranslationSettings,
        ai_backend: AiBackend,
        tencent_cloud: &TencentCloudSettings,
    ) {
        let mut state = self.state.lock().unwrap();
        state.ai_backend = ai_backend;
        state.tencent_cloud = tencent_cloud.clone();

        if !settings.enabled {
            state.enabled = false;
            drop(state);
            self.zh_to_en_model.lock().unwrap().unload_now();
            self.en_to_zh_model.lock().unwrap().unload_now();
            return;
        }

        state.enabled = true;
    }

    /// Runs translation for copied text when the feature is enabled.
    pub fn translate(&self, text: &str) -> Result<String, String> {
        self.translate_streaming(text, |_| {})
    }

    /// Runs translation and reports partial decoded text as tokens are generated.
    pub fn translate_streaming(
        &self,
        text: &str,
        on_partial: impl FnMut(&str),
    ) -> Result<String, String> {
        self.translate_streaming_cancellable(text, on_partial, || false)
    }

    /// Runs translation and lets the caller stop long-running inference between decoder steps.
    pub fn translate_streaming_cancellable(
        &self,
        text: &str,
        mut on_partial: impl FnMut(&str),
        should_cancel: impl Fn() -> bool,
    ) -> Result<String, String> {
        let Some(direction) = detect_translation_direction(text) else {
            return Ok(String::new());
        };

        let tencent_cloud = {
            let state = self.state.lock().unwrap();
            if !state.enabled {
                return Err("text translation is disabled".into());
            }

            match state.ai_backend {
                AiBackend::Tencent => Some(state.tencent_cloud.clone()),
                AiBackend::Local => None,
            }
        };

        if let Some(tencent_cloud) = tencent_cloud {
            if should_cancel() {
                return Ok(String::new());
            }
            let translated = translate_text_with_tencent(
                &tencent_cloud,
                backend_direction_for(direction),
                text,
            )?;
            if !translated.is_empty() {
                on_partial(&translated);
            }
            return Ok(translated);
        }

        let model_path = local_model_path_for_direction(direction);
        let mut model = self.local_model_for_direction(direction).lock().unwrap();
        let result = model
            .get_or_try_load(|| TranslationModel::load(&model_path))?
            .translate_streaming(text, on_partial, should_cancel);
        model.refresh_idle_deadline(Instant::now());
        result
    }

    pub fn unload_if_idle(&self) {
        let now = Instant::now();
        if let Ok(mut model) = self.zh_to_en_model.try_lock() {
            if model.unload_if_idle(now) {
                log::info!("unloaded idle zh->en translation model");
            }
        }
        if let Ok(mut model) = self.en_to_zh_model.try_lock() {
            if model.unload_if_idle(now) {
                log::info!("unloaded idle en->zh translation model");
            }
        }
    }

    fn local_model_for_direction(
        &self,
        direction: DetectedTranslationDirection,
    ) -> &Mutex<IdleModel<TranslationModel>> {
        match direction {
            DetectedTranslationDirection::ZhToEn => &self.zh_to_en_model,
            DetectedTranslationDirection::EnToZh => &self.en_to_zh_model,
        }
    }
}

fn local_model_path_for_direction(direction: DetectedTranslationDirection) -> PathBuf {
    match direction {
        DetectedTranslationDirection::ZhToEn => default_zh_to_en_translation_model_path(),
        DetectedTranslationDirection::EnToZh => default_en_to_zh_translation_model_path(),
    }
}

fn backend_direction_for(direction: DetectedTranslationDirection) -> TencentTranslationDirection {
    match direction {
        DetectedTranslationDirection::ZhToEn => TencentTranslationDirection::ZhToEn,
        DetectedTranslationDirection::EnToZh => TencentTranslationDirection::EnToZh,
    }
}

impl TranslationModel {
    fn load(path: &Path) -> Result<Self, String> {
        let model_dir = resolve_model_dir(path)?;
        let tokenizer_path = model_dir.join("tokenizer.json");
        let encoder_path = model_dir.join("onnx").join("encoder_model.onnx");
        let decoder_path = model_dir.join("onnx").join("decoder_model.onnx");

        let tokenizer = load_tokenizer(&tokenizer_path)?;
        let encoder = load_onnx_model(&encoder_path)?;
        let decoder = load_onnx_model(&decoder_path)?;

        Ok(Self {
            tokenizer,
            encoder,
            decoder,
        })
    }

    fn translate_streaming(
        &mut self,
        text: &str,
        mut on_partial: impl FnMut(&str),
        should_cancel: impl Fn() -> bool,
    ) -> Result<String, String> {
        let chunks = self.encode_source_chunks(text)?;
        if chunks.is_empty() {
            return Ok(String::new());
        }

        let mut translated_chunks = Vec::with_capacity(chunks.len());
        for chunk in chunks {
            if should_cancel() {
                return Ok(join_translated_chunks(&translated_chunks));
            }

            let translated = self.translate_chunk_streaming(
                &chunk,
                |partial_text| {
                    let mut partial_chunks = translated_chunks.clone();
                    partial_chunks.push(partial_text.to_string());
                    on_partial(&join_translated_chunks(&partial_chunks));
                },
                &should_cancel,
            )?;
            if should_cancel() {
                return Ok(join_translated_chunks(&translated_chunks));
            }

            if !translated.is_empty() {
                translated_chunks.push(translated);
                on_partial(&join_translated_chunks(&translated_chunks));
            }
        }

        Ok(join_translated_chunks(&translated_chunks))
    }

    fn encode_source_chunks(&self, text: &str) -> Result<Vec<SourceChunk>, String> {
        let mut chunks = Vec::new();
        let mut current_ids = Vec::new();

        for segment in split_text_segments(text) {
            let segment_ids = self.encode_source_payload_ids(&segment)?;
            if segment_ids.is_empty() {
                continue;
            }

            if segment_ids.len() > SOURCE_CHUNK_PAYLOAD_LEN {
                push_encoded_chunk(&mut chunks, &mut current_ids);
                for ids in segment_ids.chunks(SOURCE_CHUNK_PAYLOAD_LEN) {
                    push_source_chunk(&mut chunks, ids.to_vec());
                }
                continue;
            }

            let next_len = current_ids.len() + segment_ids.len();
            if !current_ids.is_empty() && next_len > SOURCE_CHUNK_TARGET_LEN {
                push_encoded_chunk(&mut chunks, &mut current_ids);
            }

            current_ids.extend(segment_ids);
        }

        push_encoded_chunk(&mut chunks, &mut current_ids);
        Ok(chunks)
    }

    fn encode_source_payload_ids(&self, text: &str) -> Result<Vec<i64>, String> {
        let encoding = self
            .tokenizer
            .encode(text, true)
            .map_err(|err| format!("encode source text failed: {err}"))?;
        let mut source_ids = encoding
            .get_ids()
            .iter()
            .map(|id| i64::from(*id))
            .collect::<Vec<_>>();

        if source_ids.last() == Some(&EOS_TOKEN_ID) {
            source_ids.pop();
        }

        Ok(source_ids)
    }

    fn translate_chunk_streaming(
        &mut self,
        chunk: &SourceChunk,
        mut on_partial: impl FnMut(&str),
        should_cancel: impl Fn() -> bool,
    ) -> Result<String, String> {
        let input_ids = &chunk.input_ids;
        let attention_mask = &chunk.attention_mask;
        let decoder_sequence_len = decoder_sequence_len_for_source(input_ids.len());

        if should_cancel() {
            return Ok(String::new());
        }

        let encoder_input_ids = tensor_from_i64(input_ids)?;
        let encoder_attention_mask = tensor_from_i64(attention_mask)?;
        let encoder_outputs = self
            .encoder
            .run(ort::inputs! {
                "input_ids" => encoder_input_ids,
                "attention_mask" => encoder_attention_mask,
            })
            .map_err(|err| format!("run translation encoder failed: {err}"))?;
        let (encoder_hidden_shape, encoder_hidden_states) = encoder_outputs[0]
            .try_extract_tensor::<f32>()
            .map_err(|err| format!("read encoder hidden states failed: {err}"))?;
        let encoder_hidden_shape = encoder_hidden_shape
            .iter()
            .map(|dim| {
                usize::try_from(*dim).map_err(|err| format!("invalid hidden state shape: {err}"))
            })
            .collect::<Result<Vec<_>, _>>()?;
        let encoder_hidden_states = encoder_hidden_states.to_vec();
        drop(encoder_outputs);

        let mut generated_ids = vec![DECODER_START_TOKEN_ID];
        let mut last_partial = String::new();
        while generated_ids.len() < decoder_sequence_len {
            if should_cancel() {
                break;
            }

            let position = generated_ids.len() - 1;
            let decoder_attention_mask = tensor_from_i64(attention_mask)?;
            let decoder_input_ids = tensor_from_i64(&generated_ids)?;
            let decoder_encoder_hidden_states = Tensor::from_array((
                encoder_hidden_shape.clone(),
                encoder_hidden_states.clone().into_boxed_slice(),
            ))
            .map_err(|err| format!("build encoder hidden state tensor failed: {err}"))?;
            let decoder_outputs = self
                .decoder
                .run(ort::inputs! {
                    "encoder_attention_mask" => decoder_attention_mask,
                    "input_ids" => decoder_input_ids,
                    "encoder_hidden_states" => decoder_encoder_hidden_states,
                })
                .map_err(|err| format!("run translation decoder failed: {err}"))?;
            let (logits_shape, logits) = decoder_outputs[0]
                .try_extract_tensor::<f32>()
                .map_err(|err| format!("read decoder logits failed: {err}"))?;
            let next_id = next_token_id(logits_shape, logits, position, &generated_ids)?;
            drop(decoder_outputs);

            if should_cancel() {
                break;
            }

            if next_id == EOS_TOKEN_ID {
                break;
            }

            generated_ids.push(next_id);

            let partial_text = self.decode_generated_ids(&generated_ids)?;
            if !partial_text.is_empty() && partial_text != last_partial {
                on_partial(&partial_text);
                last_partial = partial_text;
            }
        }

        self.decode_generated_ids(&generated_ids)
    }

    fn decode_generated_ids(&self, generated_ids: &[i64]) -> Result<String, String> {
        let decoded_ids = generated_ids
            .iter()
            .copied()
            .into_iter()
            .filter(|id| *id != DECODER_START_TOKEN_ID && *id != PAD_TOKEN_ID)
            .map(|id| u32::try_from(id).map_err(|err| format!("invalid token id: {err}")))
            .collect::<Result<Vec<_>, _>>()?;

        self.tokenizer
            .decode(&decoded_ids, true)
            .map(|text| text.trim().to_string())
            .map_err(|err| format!("decode translated text failed: {err}"))
    }
}

fn load_onnx_model(path: &Path) -> Result<Session, String> {
    Session::builder()
        .map_err(|err| format!("create onnx session builder failed: {err}"))?
        .with_optimization_level(GraphOptimizationLevel::Level3)
        .map_err(|err| format!("configure onnx optimizer failed: {err}"))?
        .commit_from_file(path)
        .map_err(|err| format!("load onnx model failed: {err}"))
}

fn load_tokenizer(path: &Path) -> Result<Tokenizer, String> {
    let content =
        fs::read_to_string(path).map_err(|err| format!("read tokenizer file failed: {err}"))?;
    let mut json = serde_json::from_str::<Value>(&content)
        .map_err(|err| format!("parse tokenizer json failed: {err}"))?;

    if json
        .get("normalizer")
        .and_then(|normalizer| normalizer.get("type"))
        .and_then(Value::as_str)
        == Some("Precompiled")
        && json
            .get("normalizer")
            .and_then(|normalizer| normalizer.get("precompiled_charsmap"))
            .is_some_and(Value::is_null)
    {
        json["normalizer"] = Value::Null;
    }

    let content = serde_json::to_vec(&json)
        .map_err(|err| format!("serialize tokenizer json failed: {err}"))?;
    Tokenizer::from_bytes(content).map_err(|err| format!("read tokenizer failed: {err}"))
}

fn resolve_model_dir(path: &Path) -> Result<PathBuf, String> {
    if path.is_dir() {
        return Ok(path.to_path_buf());
    }

    if path.ends_with("encoder_model.onnx") || path.ends_with("decoder_model.onnx") {
        return path
            .parent()
            .and_then(Path::parent)
            .map(Path::to_path_buf)
            .ok_or_else(|| format!("invalid translation model path: {}", path.display()));
    }

    Err(format!(
        "translation model directory not found: {}",
        path.display()
    ))
}

fn tensor_from_i64(ids: &[i64]) -> Result<Tensor<i64>, String> {
    Tensor::from_array(([1usize, ids.len()], ids.to_vec().into_boxed_slice()))
        .map_err(|err| format!("build token tensor failed: {err}"))
}

fn push_encoded_chunk(chunks: &mut Vec<SourceChunk>, current_ids: &mut Vec<i64>) {
    if current_ids.is_empty() {
        return;
    }

    push_source_chunk(chunks, std::mem::take(current_ids));
}

fn push_source_chunk(chunks: &mut Vec<SourceChunk>, mut input_ids: Vec<i64>) {
    if input_ids.is_empty() {
        return;
    }

    input_ids.push(EOS_TOKEN_ID);
    let attention_mask = vec![1; input_ids.len()];
    chunks.push(SourceChunk {
        input_ids,
        attention_mask,
    });
}

fn split_text_segments(text: &str) -> Vec<String> {
    let chars = text.chars().collect::<Vec<_>>();
    let mut segments = Vec::new();
    let mut segment = String::new();
    let mut index = 0;

    while index < chars.len() {
        let ch = chars[index];
        segment.push(ch);

        if is_line_break(ch) {
            push_text_segment(&mut segments, &mut segment);
            index += 1;
            continue;
        }

        if is_sentence_terminator(ch) {
            while index + 1 < chars.len() && is_closing_punctuation(chars[index + 1]) {
                index += 1;
                segment.push(chars[index]);
            }

            if is_cjk_sentence_terminator(ch)
                || index + 1 == chars.len()
                || chars[index + 1].is_whitespace()
            {
                push_text_segment(&mut segments, &mut segment);
            }
        }

        index += 1;
    }

    push_text_segment(&mut segments, &mut segment);
    segments
}

fn push_text_segment(segments: &mut Vec<String>, segment: &mut String) {
    let trimmed = segment.trim();
    if !trimmed.is_empty() {
        segments.push(trimmed.to_string());
    }
    segment.clear();
}

fn is_line_break(ch: char) -> bool {
    ch == '\n' || ch == '\r'
}

fn is_sentence_terminator(ch: char) -> bool {
    is_cjk_sentence_terminator(ch) || matches!(ch, '.' | '!' | '?' | ';')
}

fn is_cjk_sentence_terminator(ch: char) -> bool {
    matches!(ch, '。' | '！' | '？' | '；' | '…')
}

fn is_closing_punctuation(ch: char) -> bool {
    matches!(
        ch,
        '"' | '\'' | ')' | ']' | '}' | '）' | '】' | '》' | '」' | '』' | '”' | '’'
    )
}

fn decoder_sequence_len_for_source(source_len: usize) -> usize {
    (source_len.saturating_mul(2) + 16).clamp(MIN_DECODER_SEQUENCE_LEN, MAX_DECODER_SEQUENCE_LEN)
}

fn join_translated_chunks(chunks: &[String]) -> String {
    chunks
        .iter()
        .map(String::as_str)
        .filter(|chunk| !chunk.trim().is_empty())
        .collect::<Vec<_>>()
        .join("\n")
}

fn next_token_id(
    logits_shape: &[i64],
    logits: &[f32],
    position: usize,
    generated_ids: &[i64],
) -> Result<i64, String> {
    let vocab_size = logits_shape
        .last()
        .ok_or_else(|| "decoder logits have no dimensions".to_string())
        .and_then(|dim| {
            usize::try_from(*dim).map_err(|err| format!("invalid decoder vocab size: {err}"))
        })?;
    let start = position
        .checked_mul(vocab_size)
        .ok_or_else(|| "decoder logits position overflowed".to_string())?;
    let token_logits = logits
        .get(start..start + vocab_size)
        .ok_or_else(|| "decoder logits are empty".to_string())?;
    let banned_tokens = banned_repeated_ngram_tokens(generated_ids, NO_REPEAT_NGRAM_SIZE);

    token_logits
        .iter()
        .enumerate()
        .filter_map(|(id, score)| {
            let id = i64::try_from(id).ok()?;
            if id == PAD_TOKEN_ID || id == DECODER_START_TOKEN_ID || banned_tokens.contains(&id) {
                return None;
            }

            Some((id, repetition_adjusted_score(id, *score, generated_ids)))
        })
        .max_by(|(_, left), (_, right)| left.total_cmp(right))
        .map(|(id, _)| id as i64)
        .ok_or_else(|| "decoder logits did not contain any token".to_string())
}

fn repetition_adjusted_score(token_id: i64, score: f32, generated_ids: &[i64]) -> f32 {
    if generated_ids.contains(&token_id) {
        if score < 0.0 {
            score * REPETITION_PENALTY
        } else {
            score / REPETITION_PENALTY
        }
    } else {
        score
    }
}

fn banned_repeated_ngram_tokens(generated_ids: &[i64], ngram_size: usize) -> Vec<i64> {
    if ngram_size < 2 || generated_ids.len() + 1 < ngram_size {
        return Vec::new();
    }

    let context_size = ngram_size - 1;
    let context = &generated_ids[generated_ids.len() - context_size..];
    generated_ids
        .windows(ngram_size)
        .filter_map(|ngram| {
            let (prefix, next) = ngram.split_at(context_size);
            (prefix == context).then_some(next[0])
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::{
        MAX_DECODER_SEQUENCE_LEN, MIN_DECODER_SEQUENCE_LEN, TranslationService,
        backend_direction_for, banned_repeated_ngram_tokens, decoder_sequence_len_for_source,
        join_translated_chunks, local_model_path_for_direction, repetition_adjusted_score,
        split_text_segments,
    };
    use crate::features::text_translation::language::DetectedTranslationDirection;
    use crate::infrastructure::tencent_cloud::client::TencentTranslationDirection;
    use crate::settings::{
        AiBackend, TencentCloudSettings, TextTranslationSettings,
        default_en_to_zh_translation_model_path, default_zh_to_en_translation_model_path,
    };

    #[test]
    fn decoder_len_grows_with_source_but_stays_bounded() {
        assert_eq!(decoder_sequence_len_for_source(1), MIN_DECODER_SEQUENCE_LEN);
        assert_eq!(
            decoder_sequence_len_for_source(10_000),
            MAX_DECODER_SEQUENCE_LEN
        );
        assert!(decoder_sequence_len_for_source(200) > MIN_DECODER_SEQUENCE_LEN);
    }

    #[test]
    fn joins_non_empty_translated_chunks() {
        let chunks = vec!["first".to_string(), "".to_string(), "second".to_string()];

        assert_eq!(join_translated_chunks(&chunks), "first\nsecond");
    }

    #[test]
    fn maps_detected_direction_to_local_model_path() {
        assert_eq!(
            local_model_path_for_direction(DetectedTranslationDirection::ZhToEn),
            default_zh_to_en_translation_model_path()
        );
        assert_eq!(
            local_model_path_for_direction(DetectedTranslationDirection::EnToZh),
            default_en_to_zh_translation_model_path()
        );
    }

    #[test]
    fn maps_detected_direction_to_tencent_direction() {
        assert!(matches!(
            backend_direction_for(DetectedTranslationDirection::ZhToEn),
            TencentTranslationDirection::ZhToEn
        ));
        assert!(matches!(
            backend_direction_for(DetectedTranslationDirection::EnToZh),
            TencentTranslationDirection::EnToZh
        ));
    }

    #[test]
    fn returns_empty_for_text_without_detected_direction() {
        let service = TranslationService::new(
            &TextTranslationSettings {
                enabled: true,
                debounce_seconds: 1,
            },
            AiBackend::Local,
            &TencentCloudSettings::default(),
        );

        assert_eq!(service.translate("123, !?"), Ok(String::new()));
    }

    #[test]
    fn repeated_token_scores_are_penalized() {
        assert!(repetition_adjusted_score(7, 12.0, &[7]) < 12.0);
        assert!(repetition_adjusted_score(8, -12.0, &[8]) < -12.0);
        assert_eq!(repetition_adjusted_score(9, 12.0, &[7]), 12.0);
    }

    #[test]
    fn repeated_ngram_next_tokens_are_banned() {
        let generated_ids = vec![1, 2, 3, 1, 2];

        assert_eq!(banned_repeated_ngram_tokens(&generated_ids, 3), vec![3]);
    }

    #[test]
    fn splits_chinese_sentences() {
        let segments = split_text_segments("第一句。第二句！第三句？第四句；");

        assert_eq!(
            segments,
            vec!["第一句。", "第二句！", "第三句？", "第四句；"]
        );
    }

    #[test]
    fn splits_english_sentences_on_whitespace_boundaries() {
        let segments = split_text_segments("First sentence. Second sentence! Version 1.2 works?");

        assert_eq!(
            segments,
            vec!["First sentence.", "Second sentence!", "Version 1.2 works?"]
        );
    }

    #[test]
    fn keeps_closing_quotes_with_the_sentence() {
        let segments = split_text_segments("他说：“可以。” 然后离开。");

        assert_eq!(segments, vec!["他说：“可以。”", "然后离开。"]);
    }

    #[test]
    fn splits_on_line_breaks() {
        let segments = split_text_segments("Title\nFirst paragraph.\n第二段。");

        assert_eq!(segments, vec!["Title", "First paragraph.", "第二段。"]);
    }
}
