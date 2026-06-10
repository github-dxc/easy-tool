//! Local OCR runtime backed by the configured PaddleOCR-VL ONNX model directory.

use std::fs;
use std::path::{Path, PathBuf};
use std::sync::Mutex;
use std::time::Instant;

use image::imageops::FilterType;
use ort::memory::Allocator;
use ort::session::{Session, SessionInputValue, builder::GraphOptimizationLevel};
use ort::value::{Shape, Tensor};
use serde_json::Value;
use tokenizers::Tokenizer;

use crate::infrastructure::idle_model::IdleModel;
use crate::infrastructure::tencent_cloud::client::recognize_image as recognize_image_with_tencent;
use crate::settings::{AiBackend, ImageRecognitionSettings, TencentCloudSettings};

const BOS_TOKEN: &str = "<|begin_of_sentence|>";
const IMAGE_START_TOKEN: &str = "<|IMAGE_START|>";
const IMAGE_TOKEN: &str = "<|IMAGE_PLACEHOLDER|>";
const IMAGE_END_TOKEN: &str = "<|IMAGE_END|>";
const OCR_PROMPT: &str = "OCR:";

const EOS_TOKEN_ID: i64 = 2;
const PAD_TOKEN_ID: i64 = 0;
const IMAGE_TOKEN_ID: i64 = 100_295;
const VISION_START_TOKEN_ID: i64 = 101_305;
const VISION_END_TOKEN_ID: i64 = 101_306;

const HIDDEN_SIZE: usize = 1024;
const NUM_DECODER_LAYERS: usize = 18;
const NUM_KEY_VALUE_HEADS: usize = 2;
const HEAD_DIM: usize = 128;
const PATCH_SIZE: usize = 14;
const MERGE_SIZE: usize = 2;
const RESIZE_FACTOR: usize = PATCH_SIZE * MERGE_SIZE;
const MIN_PIXELS: usize = 112_896;
const MAX_PIXELS: usize = 1_003_520;
const MAX_GENERATED_TOKENS: usize = 512;

/// Owns the loaded OCR model and reloads it when settings change.
pub struct OcrService {
    state: Mutex<OcrState>,
}

struct OcrState {
    enabled: bool,
    ai_backend: AiBackend,
    tencent_cloud: TencentCloudSettings,
    model_path: Option<PathBuf>,
    model: IdleModel<OcrModel>,
}

struct OcrModel {
    tokenizer: Tokenizer,
    vision_encoder: Session,
    embedding: Session,
    decoder: Session,
}

struct PreparedImage {
    pixel_values: Vec<f32>,
    grid_t: usize,
    grid_h: usize,
    grid_w: usize,
}

struct DecoderCache {
    layers: Vec<DecoderLayerCache>,
}

struct DecoderLayerCache {
    key_shape: Vec<usize>,
    key: Vec<f32>,
    value_shape: Vec<usize>,
    value: Vec<f32>,
}

impl OcrService {
    pub fn new(
        settings: &ImageRecognitionSettings,
        ai_backend: AiBackend,
        tencent_cloud: &TencentCloudSettings,
    ) -> Self {
        let service = Self {
            state: Mutex::new(OcrState {
                enabled: false,
                ai_backend,
                tencent_cloud: tencent_cloud.clone(),
                model_path: None,
                model: IdleModel::empty(),
            }),
        };
        service.apply_settings(settings, ai_backend, tencent_cloud);
        service
    }

    pub fn apply_settings(
        &self,
        settings: &ImageRecognitionSettings,
        ai_backend: AiBackend,
        tencent_cloud: &TencentCloudSettings,
    ) {
        let mut state = self.state.lock().unwrap();
        state.ai_backend = ai_backend;
        state.tencent_cloud = tencent_cloud.clone();

        let model_path = settings.model_dir.clone();
        if state.model_path != model_path {
            state.model_path = model_path;
            state.model.unload_now();
        }

        if !settings.enabled {
            state.enabled = false;
            state.model.unload_now();
            return;
        }

        state.enabled = true;
    }

    pub fn recognize(&self, image_path: &Path) -> Result<String, String> {
        self.recognize_streaming(image_path, |_| {})
    }

    pub fn recognize_streaming(
        &self,
        image_path: &Path,
        mut on_partial: impl FnMut(&str),
    ) -> Result<String, String> {
        let mut state = self.state.lock().unwrap();
        if !state.enabled {
            return Err("图像识别已关闭".into());
        }

        if state.ai_backend == AiBackend::Tencent {
            let tencent_cloud = state.tencent_cloud.clone();
            drop(state);

            let recognized = recognize_image_with_tencent(&tencent_cloud, image_path)?;
            if !recognized.is_empty() {
                on_partial(&recognized);
            }
            return Ok(recognized);
        }

        let model_path = state
            .model_path
            .clone()
            .ok_or_else(|| "请先在设置页配置 OCR 模型目录".to_string())?;
        let result = {
            let model = state
                .model
                .get_or_try_load(|| OcrModel::load(&model_path))?;
            model.recognize_streaming(image_path, on_partial)
        };
        state.model.refresh_idle_deadline(Instant::now());
        result
    }

    pub fn unload_if_idle(&self) {
        let Ok(mut state) = self.state.try_lock() else {
            return;
        };
        if state.model.unload_if_idle(Instant::now()) {
            log::info!("unloaded idle OCR model");
        }
    }
}
impl OcrModel {
    fn load(path: &Path) -> Result<Self, String> {
        let model_dir = resolve_model_dir(path)?;
        let tokenizer = load_tokenizer(&model_dir.join("tokenizer.json"))?;
        let onnx_dir = model_dir.join("onnx");
        let vision_encoder = load_onnx_model(&onnx_dir.join("vision_encoder.onnx"))?;
        let embedding = load_onnx_model(&onnx_dir.join("embedding.onnx"))?;
        let decoder_path = if onnx_dir.join("decoder_q4.onnx").exists() {
            onnx_dir.join("decoder_q4.onnx")
        } else {
            onnx_dir.join("decoder.onnx")
        };
        let decoder = load_onnx_model(&decoder_path)?;

        Ok(Self {
            tokenizer,
            vision_encoder,
            embedding,
            decoder,
        })
    }

    fn recognize_streaming(
        &mut self,
        image_path: &Path,
        on_partial: impl FnMut(&str),
    ) -> Result<String, String> {
        let image = prepare_image(image_path)?;
        let vision_embeddings = self.run_vision_encoder(&image)?;
        let input_ids = self.build_prompt_ids(&image)?;
        let prompt_embeddings = self.run_embedding(&input_ids)?;
        let inputs_embeds =
            inject_image_embeddings(&input_ids, prompt_embeddings, &vision_embeddings)?;
        let generated_ids = self.generate_streaming(&inputs_embeds, input_ids.len(), on_partial)?;

        self.decode_generated_ids(&generated_ids)
    }

    fn run_vision_encoder(&mut self, image: &PreparedImage) -> Result<Vec<f32>, String> {
        let pixel_values = Tensor::from_array((
            [
                1usize,
                image.grid_t * image.grid_h * image.grid_w,
                3,
                PATCH_SIZE,
                PATCH_SIZE,
            ],
            image.pixel_values.clone().into_boxed_slice(),
        ))
        .map_err(|err| format!("构建 OCR 图像张量失败: {err}"))?;
        let image_grid_thw = Tensor::from_array((
            [1usize, 3],
            vec![
                image.grid_t as i64,
                image.grid_h as i64,
                image.grid_w as i64,
            ]
            .into_boxed_slice(),
        ))
        .map_err(|err| format!("构建 OCR 图像网格张量失败: {err}"))?;
        let outputs = self
            .vision_encoder
            .run(ort::inputs! {
                "pixel_values" => pixel_values,
                "image_grid_thw" => image_grid_thw,
            })
            .map_err(|err| format!("运行 OCR vision encoder 失败: {err}"))?;
        let (shape, embeddings) = outputs["image_embeds"]
            .try_extract_tensor::<f32>()
            .map_err(|err| format!("读取 OCR 图像向量失败: {err}"))?;
        let shape = shape_to_usize(shape)?;
        if shape.last().copied() != Some(HIDDEN_SIZE) {
            return Err(format!("OCR 图像向量维度异常: {shape:?}"));
        }

        Ok(embeddings.to_vec())
    }

    fn build_prompt_ids(&self, image: &PreparedImage) -> Result<Vec<i64>, String> {
        let image_tokens = image.grid_t * image.grid_h * image.grid_w / MERGE_SIZE / MERGE_SIZE;
        let prompt = format!(
            "{BOS_TOKEN}User: {IMAGE_START_TOKEN}{}{IMAGE_END_TOKEN}{OCR_PROMPT}\nAssistant:\n",
            IMAGE_TOKEN.repeat(image_tokens)
        );
        let encoding = self
            .tokenizer
            .encode(prompt, false)
            .map_err(|err| format!("编码 OCR prompt 失败: {err}"))?;
        let ids = encoding
            .get_ids()
            .iter()
            .map(|id| i64::from(*id))
            .collect::<Vec<_>>();
        let actual_image_tokens = ids.iter().filter(|id| **id == IMAGE_TOKEN_ID).count();
        if actual_image_tokens != image_tokens {
            return Err(format!(
                "OCR prompt 图像 token 数量异常: 期望 {image_tokens}, 实际 {actual_image_tokens}"
            ));
        }

        Ok(ids)
    }

    fn run_embedding(&mut self, input_ids: &[i64]) -> Result<Vec<f32>, String> {
        let input_ids = tensor_from_i64(input_ids)?;
        let outputs = self
            .embedding
            .run(ort::inputs! {
                "input_ids" => input_ids,
            })
            .map_err(|err| format!("运行 OCR embedding 失败: {err}"))?;
        let (shape, embeddings) = outputs["embeddings"]
            .try_extract_tensor::<f32>()
            .map_err(|err| format!("读取 OCR token 向量失败: {err}"))?;
        let shape = shape_to_usize(shape)?;
        if shape.len() != 3 || shape[2] != HIDDEN_SIZE {
            return Err(format!("OCR token 向量维度异常: {shape:?}"));
        }

        Ok(embeddings.to_vec())
    }

    fn generate_streaming(
        &mut self,
        prompt_embeds: &[f32],
        prompt_len: usize,
        mut on_partial: impl FnMut(&str),
    ) -> Result<Vec<i64>, String> {
        let mut attention_mask = vec![1i64; prompt_len];
        let mut generated_ids = Vec::new();
        let mut inputs_embeds = prompt_embeds.to_vec();
        let mut seq_len = prompt_len;
        let mut cache = DecoderCache::empty();
        let mut last_partial = String::new();

        for _ in 0..MAX_GENERATED_TOKENS {
            let outputs = self.run_decoder(&inputs_embeds, seq_len, &attention_mask, &cache)?;
            let next_id = select_next_token(&outputs, seq_len - 1, &generated_ids)?;
            cache = outputs.cache;

            if next_id == EOS_TOKEN_ID {
                break;
            }

            generated_ids.push(next_id);
            let partial_text = self.decode_generated_ids(&generated_ids)?;
            if !partial_text.is_empty() && partial_text != last_partial {
                on_partial(&partial_text);
                last_partial = partial_text;
            }

            attention_mask.push(1);
            inputs_embeds = self.run_embedding(&[next_id])?;
            seq_len = 1;
        }

        Ok(generated_ids)
    }

    fn run_decoder(
        &mut self,
        inputs_embeds: &[f32],
        seq_len: usize,
        attention_mask: &[i64],
        cache: &DecoderCache,
    ) -> Result<DecoderOutputs, String> {
        let inputs_embeds = Tensor::from_array((
            [1usize, seq_len, HIDDEN_SIZE],
            inputs_embeds.to_vec().into_boxed_slice(),
        ))
        .map_err(|err| format!("构建 OCR decoder 输入向量失败: {err}"))?;
        let attention_mask = Tensor::from_array((
            [1usize, attention_mask.len()],
            attention_mask.to_vec().into_boxed_slice(),
        ))
        .map_err(|err| format!("构建 OCR attention mask 失败: {err}"))?;

        let mut inputs: Vec<(String, SessionInputValue<'static>)> = vec![
            ("inputs_embeds".to_string(), inputs_embeds.into()),
            ("attention_mask".to_string(), attention_mask.into()),
        ];

        for (index, layer) in cache.layers.iter().enumerate() {
            let key = tensor_from_f32_cache(&layer.key_shape, &layer.key)
                .map_err(|err| format!("构建 OCR key cache 失败: {err}"))?;
            let value = tensor_from_f32_cache(&layer.value_shape, &layer.value)
                .map_err(|err| format!("构建 OCR value cache 失败: {err}"))?;
            inputs.push((format!("past_key_values.{index}.key"), key.into()));
            inputs.push((format!("past_key_values.{index}.value"), value.into()));
        }

        let outputs = self
            .decoder
            .run(inputs)
            .map_err(|err| format!("运行 OCR decoder 失败: {err}"))?;
        let (logits_shape, logits) = outputs["logits"]
            .try_extract_tensor::<f32>()
            .map_err(|err| format!("读取 OCR logits 失败: {err}"))?;
        let logits_shape = shape_to_usize(logits_shape)?;
        let logits = logits.to_vec();
        let cache = DecoderCache::from_outputs(&outputs)?;

        Ok(DecoderOutputs {
            logits_shape,
            logits,
            cache,
        })
    }

    fn decode_generated_ids(&self, generated_ids: &[i64]) -> Result<String, String> {
        let ids = generated_ids
            .iter()
            .copied()
            .filter(|id| !is_filtered_token(*id))
            .map(|id| u32::try_from(id).map_err(|err| format!("OCR token id 异常: {err}")))
            .collect::<Result<Vec<_>, _>>()?;

        self.tokenizer
            .decode(&ids, true)
            .map(|text| text.trim().to_string())
            .map_err(|err| format!("解码 OCR 文本失败: {err}"))
    }
}

struct DecoderOutputs {
    logits_shape: Vec<usize>,
    logits: Vec<f32>,
    cache: DecoderCache,
}

impl DecoderCache {
    fn empty() -> Self {
        Self {
            layers: (0..NUM_DECODER_LAYERS)
                .map(|_| DecoderLayerCache {
                    key_shape: vec![1, NUM_KEY_VALUE_HEADS, 0, HEAD_DIM],
                    key: Vec::new(),
                    value_shape: vec![1, NUM_KEY_VALUE_HEADS, 0, HEAD_DIM],
                    value: Vec::new(),
                })
                .collect(),
        }
    }

    fn from_outputs(outputs: &ort::session::SessionOutputs<'_>) -> Result<Self, String> {
        let mut layers = Vec::with_capacity(NUM_DECODER_LAYERS);
        for index in 0..NUM_DECODER_LAYERS {
            let key_name = format!("present.{index}.key");
            let value_name = format!("present.{index}.value");
            let (key_shape, key) = outputs[key_name]
                .try_extract_tensor::<f32>()
                .map_err(|err| format!("读取 OCR key cache 失败: {err}"))?;
            let (value_shape, value) = outputs[value_name]
                .try_extract_tensor::<f32>()
                .map_err(|err| format!("读取 OCR value cache 失败: {err}"))?;
            layers.push(DecoderLayerCache {
                key_shape: shape_to_usize(key_shape)?,
                key: key.to_vec(),
                value_shape: shape_to_usize(value_shape)?,
                value: value.to_vec(),
            });
        }

        Ok(Self { layers })
    }
}

fn prepare_image(path: &Path) -> Result<PreparedImage, String> {
    let image = image::open(path).map_err(|err| format!("读取图片失败: {err}"))?;
    let rgb = image.to_rgb8();
    let (width, height) = rgb.dimensions();
    let (resized_h, resized_w) = smart_resize(height as usize, width as usize)?;
    let resized = image::imageops::resize(
        &rgb,
        resized_w as u32,
        resized_h as u32,
        FilterType::CatmullRom,
    );

    let grid_h = resized_h / PATCH_SIZE;
    let grid_w = resized_w / PATCH_SIZE;
    let grid_t = 1;
    let mut pixel_values =
        Vec::with_capacity(grid_t * grid_h * grid_w * 3 * PATCH_SIZE * PATCH_SIZE);

    for patch_y in 0..grid_h {
        for patch_x in 0..grid_w {
            for channel in 0..3 {
                for y in 0..PATCH_SIZE {
                    for x in 0..PATCH_SIZE {
                        let px = resized.get_pixel(
                            (patch_x * PATCH_SIZE + x) as u32,
                            (patch_y * PATCH_SIZE + y) as u32,
                        );
                        let value = f32::from(px[channel]) / 255.0;
                        pixel_values.push((value - 0.5) / 0.5);
                    }
                }
            }
        }
    }

    Ok(PreparedImage {
        pixel_values,
        grid_t,
        grid_h,
        grid_w,
    })
}

fn smart_resize(mut height: usize, mut width: usize) -> Result<(usize, usize), String> {
    if height == 0 || width == 0 {
        return Err("图片尺寸无效".into());
    }

    if height < RESIZE_FACTOR {
        width = ((width * RESIZE_FACTOR) as f64 / height as f64).round() as usize;
        height = RESIZE_FACTOR;
    }
    if width < RESIZE_FACTOR {
        height = ((height * RESIZE_FACTOR) as f64 / width as f64).round() as usize;
        width = RESIZE_FACTOR;
    }

    let ratio = height.max(width) as f64 / height.min(width) as f64;
    if ratio > 200.0 {
        return Err(format!("图片宽高比过大: {ratio:.2}"));
    }

    let mut h_bar = round_to_factor(height, RESIZE_FACTOR);
    let mut w_bar = round_to_factor(width, RESIZE_FACTOR);
    if h_bar * w_bar > MAX_PIXELS {
        let beta = ((height * width) as f64 / MAX_PIXELS as f64).sqrt();
        h_bar = floor_to_factor((height as f64 / beta) as usize, RESIZE_FACTOR);
        w_bar = floor_to_factor((width as f64 / beta) as usize, RESIZE_FACTOR);
    } else if h_bar * w_bar < MIN_PIXELS {
        let beta = (MIN_PIXELS as f64 / (height * width) as f64).sqrt();
        h_bar = ceil_to_factor((height as f64 * beta) as usize, RESIZE_FACTOR);
        w_bar = ceil_to_factor((width as f64 * beta) as usize, RESIZE_FACTOR);
    }

    Ok((h_bar.max(RESIZE_FACTOR), w_bar.max(RESIZE_FACTOR)))
}

fn round_to_factor(value: usize, factor: usize) -> usize {
    ((value as f64 / factor as f64).round() as usize) * factor
}

fn floor_to_factor(value: usize, factor: usize) -> usize {
    (value / factor) * factor
}

fn ceil_to_factor(value: usize, factor: usize) -> usize {
    value.div_ceil(factor) * factor
}

fn inject_image_embeddings(
    input_ids: &[i64],
    mut prompt_embeddings: Vec<f32>,
    image_embeddings: &[f32],
) -> Result<Vec<f32>, String> {
    let image_positions = input_ids
        .iter()
        .enumerate()
        .filter_map(|(index, id)| (*id == IMAGE_TOKEN_ID).then_some(index))
        .collect::<Vec<_>>();
    let expected_values = image_positions.len() * HIDDEN_SIZE;
    if image_embeddings.len() != expected_values {
        return Err(format!(
            "OCR 图像向量数量异常: 期望 {expected_values}, 实际 {}",
            image_embeddings.len()
        ));
    }

    for (image_index, token_index) in image_positions.iter().copied().enumerate() {
        let token_start = token_index * HIDDEN_SIZE;
        let image_start = image_index * HIDDEN_SIZE;
        prompt_embeddings[token_start..token_start + HIDDEN_SIZE]
            .copy_from_slice(&image_embeddings[image_start..image_start + HIDDEN_SIZE]);
    }

    Ok(prompt_embeddings)
}

fn select_next_token(
    outputs: &DecoderOutputs,
    position: usize,
    generated_ids: &[i64],
) -> Result<i64, String> {
    let vocab_size = outputs
        .logits_shape
        .last()
        .copied()
        .ok_or_else(|| "OCR logits 维度为空".to_string())?;
    let start = position
        .checked_mul(vocab_size)
        .ok_or_else(|| "OCR logits 索引溢出".to_string())?;
    let logits = outputs
        .logits
        .get(start..start + vocab_size)
        .ok_or_else(|| "OCR logits 数据为空".to_string())?;

    logits
        .iter()
        .enumerate()
        .filter_map(|(id, score)| {
            let id = i64::try_from(id).ok()?;
            (!is_filtered_token(id))
                .then_some((id, repetition_adjusted_score(id, *score, generated_ids)))
        })
        .max_by(|(_, left), (_, right)| left.total_cmp(right))
        .map(|(id, _)| id)
        .ok_or_else(|| "OCR decoder 未生成有效 token".to_string())
}

fn repetition_adjusted_score(token_id: i64, score: f32, generated_ids: &[i64]) -> f32 {
    if generated_ids.contains(&token_id) {
        if score < 0.0 {
            score * 1.05
        } else {
            score / 1.05
        }
    } else {
        score
    }
}

fn is_filtered_token(token_id: i64) -> bool {
    matches!(
        token_id,
        PAD_TOKEN_ID | IMAGE_TOKEN_ID | VISION_START_TOKEN_ID | VISION_END_TOKEN_ID
    )
}

fn resolve_model_dir(path: &Path) -> Result<PathBuf, String> {
    if !path.exists() {
        return Err(format!("OCR 模型目录不存在: {}", path.display()));
    }

    if !path.is_dir() {
        return Err(format!("OCR 模型路径不是文件夹: {}", path.display()));
    }

    for required in [
        "tokenizer.json",
        "onnx/vision_encoder.onnx",
        "onnx/embedding.onnx",
    ] {
        let required_path = path.join(required);
        if !required_path.exists() {
            return Err(format!("OCR 模型缺少文件: {}", required_path.display()));
        }
    }

    if !path.join("onnx/decoder.onnx").exists() && !path.join("onnx/decoder_q4.onnx").exists() {
        return Err(format!(
            "OCR 模型缺少 decoder.onnx 或 decoder_q4.onnx: {}",
            path.join("onnx").display()
        ));
    }

    Ok(path.to_path_buf())
}

fn load_onnx_model(path: &Path) -> Result<Session, String> {
    Session::builder()
        .map_err(|err| format!("create OCR onnx session builder failed: {err}"))?
        .with_optimization_level(GraphOptimizationLevel::Level3)
        .map_err(|err| format!("configure OCR onnx optimizer failed: {err}"))?
        .commit_from_file(path)
        .map_err(|err| format!("load OCR onnx model failed: {err}"))
}

fn load_tokenizer(path: &Path) -> Result<Tokenizer, String> {
    let content =
        fs::read_to_string(path).map_err(|err| format!("read OCR tokenizer failed: {err}"))?;
    let mut json = serde_json::from_str::<Value>(&content)
        .map_err(|err| format!("parse OCR tokenizer json failed: {err}"))?;

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
        .map_err(|err| format!("serialize OCR tokenizer json failed: {err}"))?;
    Tokenizer::from_bytes(content).map_err(|err| format!("load OCR tokenizer failed: {err}"))
}

fn tensor_from_i64(ids: &[i64]) -> Result<Tensor<i64>, String> {
    Tensor::from_array(([1usize, ids.len()], ids.to_vec().into_boxed_slice()))
        .map_err(|err| format!("构建 OCR token 张量失败: {err}"))
}

fn tensor_from_f32_cache(shape: &[usize], data: &[f32]) -> Result<Tensor<f32>, String> {
    if data.is_empty() && shape.contains(&0) {
        let shape = Shape::new(shape.iter().map(|dim| *dim as i64));
        return Tensor::new(&Allocator::default(), shape).map_err(|err| err.to_string());
    }

    Tensor::from_array((shape.to_vec(), data.to_vec().into_boxed_slice()))
        .map_err(|err| err.to_string())
}

fn shape_to_usize(shape: &[i64]) -> Result<Vec<usize>, String> {
    shape
        .iter()
        .map(|dim| usize::try_from(*dim).map_err(|err| format!("OCR 张量维度异常: {err}")))
        .collect()
}
