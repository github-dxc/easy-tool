# Translation/OCR Backend And Lazy Windows Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Finish the remaining memory optimization tasks while adding local/Tencent backend selection, automatic translation direction detection, and Tencent Cloud translation/OCR support.

**Architecture:** Keep pure settings, language detection, and Tencent signing code testable and independent. Services dispatch between local lazy models and Tencent HTTP clients from existing background threads. Slint strong window handles remain UI-thread-owned through an app window registry, while background shortcuts use weak slots.

**Tech Stack:** Rust 2024, Slint 1.15.1, serde/toml/serde_json, ONNX Runtime (`ort`), tokenizers, blocking HTTPS client, HMAC-SHA256/SHA256/base64/hex, Tencent Cloud API v3, cargo tests.

---

## File Structure

- Modify `Cargo.toml`: add Tencent HTTP/signing dependencies.
- Modify `src/settings.rs`: add `AiBackend`, `TencentCloudSettings`, remove direction/model-directory config from public translation settings, add compatibility tests.
- Create `src/features/text_translation/language.rs`: pure Chinese/English direction detection.
- Modify `src/features/text_translation/translator.rs`: hold two local idle models, dispatch local/Tencent backend, remove user-configured direction.
- Modify `src/features/file_preview/ocr.rs`: dispatch local/Tencent backend.
- Create `src/infrastructure/tencent_cloud/mod.rs`: module exports.
- Create `src/infrastructure/tencent_cloud/signing.rs`: TC3-HMAC-SHA256 signing.
- Create `src/infrastructure/tencent_cloud/client.rs`: blocking Tencent API requests plus response parsing.
- Modify `src/infrastructure/mod.rs`: export `tencent_cloud`.
- Modify `ui/settings.slint`: remove translation direction/model directory controls; add backend and Tencent credential controls.
- Modify `src/features/settings/window.rs`: bind new settings UI and add `on_settings_applied`.
- Modify `src/infrastructure/tray.rs`: replace translation direction submenu with translation enable toggle and backend selection, add `on_settings_applied`.
- Modify `src/features/home/window.rs`: accept lazy-open callbacks.
- Modify `src/app.rs`: add UI-thread window registry, weak shortcut slots, listener start-on-enable, and settings/tray applied hook.
- Modify `src/features/screenshot/window.rs`: bound undo stack and clear screenshot UI image state.
- Modify `src/features/file_preview/window.rs`: clear preview/OCR state on close/reset.
- Modify `src/bin/check_translation.rs`: update for removed translation direction settings.

---

### Task 1: Settings Model And Compatibility

**Files:**
- Modify: `src/settings.rs`
- Modify: `src/bin/check_translation.rs`

- [ ] **Step 1: Add failing settings tests**

Add this test module to the bottom of `src/settings.rs`:

```rust
#[cfg(test)]
mod tests {
    use super::{AiBackend, AppSettings, SettingsStore};

    #[test]
    fn missing_new_backend_fields_default_to_local_and_empty_credentials() {
        let input = r#"
            [text_translation]
            enabled = true
            debounce_seconds = 2
        "#;

        let settings = toml::from_str::<AppSettings>(input).unwrap();

        assert_eq!(settings.ai_backend, AiBackend::Local);
        assert_eq!(settings.tencent_cloud.secret_id, "");
        assert_eq!(settings.tencent_cloud.secret_key, "");
        assert!(settings.text_translation.enabled);
        assert_eq!(settings.text_translation.debounce_seconds, 2);
    }

    #[test]
    fn old_translation_direction_and_model_fields_do_not_break_load() {
        let input = r#"
            [text_translation]
            enabled = true
            direction = "en_to_zh"
            debounce_seconds = 3
            zh_to_en_model_dir = "old-zh-en"
            en_to_zh_model_dir = "old-en-zh"
        "#;

        let settings = toml::from_str::<AppSettings>(input).unwrap();

        assert!(settings.text_translation.enabled);
        assert_eq!(settings.text_translation.debounce_seconds, 3);
    }
}
```

- [ ] **Step 2: Run tests and verify RED**

Run:

```powershell
cargo test settings::tests
```

Expected: compile failure because `AiBackend` and `tencent_cloud` do not exist yet.

- [ ] **Step 3: Implement settings types**

In `src/settings.rs`, add:

```rust
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum AiBackend {
    Local,
    Tencent,
}

impl Default for AiBackend {
    fn default() -> Self {
        Self::Local
    }
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct TencentCloudSettings {
    #[serde(default)]
    pub secret_id: String,
    #[serde(default)]
    pub secret_key: String,
}
```

Update `AppSettings`:

```rust
#[serde(default)]
pub ai_backend: AiBackend,
#[serde(default)]
pub tencent_cloud: TencentCloudSettings,
```

Update `AppSettings::default()` with:

```rust
ai_backend: AiBackend::Local,
tencent_cloud: TencentCloudSettings::default(),
```

Update `TextTranslationSettings` to:

```rust
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TextTranslationSettings {
    pub enabled: bool,
    #[serde(default = "default_text_translation_debounce_seconds")]
    pub debounce_seconds: u64,
}
```

Remove `TranslationDirection`, `zh_to_en_model_dir`, `en_to_zh_model_dir`, and `model_path()` from public settings. Keep these private helpers for local defaults:

```rust
pub fn default_zh_to_en_translation_model_path() -> PathBuf { ... }
pub fn default_en_to_zh_translation_model_path() -> PathBuf { ... }
```

If they are currently private, make them `pub` or `pub(crate)` for `translator.rs`.

- [ ] **Step 4: Update `src/bin/check_translation.rs`**

Replace the settings construction with:

```rust
let service = TranslationService::new(
    &TextTranslationSettings {
        enabled: true,
        debounce_seconds: 1,
    },
    easy_tool::settings::AiBackend::Local,
    &easy_tool::settings::TencentCloudSettings::default(),
);
```

If `TranslationService::new` has not been updated yet, temporarily update this file in Task 4 instead and leave it compiling only after Task 4.

- [ ] **Step 5: Run settings tests**

Run:

```powershell
cargo test settings::tests
```

Expected: both tests pass.

- [ ] **Step 6: Commit**

```powershell
git add src/settings.rs src/bin/check_translation.rs
git commit -m "feat: add AI backend settings"
```

---

### Task 2: Language Direction Detection

**Files:**
- Create: `src/features/text_translation/language.rs`
- Modify: `src/features/text_translation/mod.rs`

- [ ] **Step 1: Add failing language tests**

Create `src/features/text_translation/language.rs`:

```rust
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DetectedTranslationDirection {
    ZhToEn,
    EnToZh,
}

#[cfg(test)]
mod tests {
    use super::{detect_translation_direction, DetectedTranslationDirection};

    #[test]
    fn chinese_majority_selects_zh_to_en() {
        assert_eq!(
            detect_translation_direction("你好 world 世界"),
            Some(DetectedTranslationDirection::ZhToEn)
        );
    }

    #[test]
    fn english_majority_selects_en_to_zh() {
        assert_eq!(
            detect_translation_direction("hello world 测"),
            Some(DetectedTranslationDirection::EnToZh)
        );
    }

    #[test]
    fn chinese_tie_selects_zh_to_en() {
        assert_eq!(
            detect_translation_direction("你a"),
            Some(DetectedTranslationDirection::ZhToEn)
        );
    }

    #[test]
    fn empty_text_returns_none() {
        assert_eq!(detect_translation_direction("   \n\t"), None);
    }
}
```

Add to `src/features/text_translation/mod.rs`:

```rust
pub mod language;
```

- [ ] **Step 2: Run tests and verify RED**

Run:

```powershell
cargo test features::text_translation::language
```

Expected: compile failure because `detect_translation_direction` is missing.

- [ ] **Step 3: Implement detection**

Add above tests in `language.rs`:

```rust
pub fn detect_translation_direction(text: &str) -> Option<DetectedTranslationDirection> {
    if text.trim().is_empty() {
        return None;
    }

    let mut chinese = 0usize;
    let mut english = 0usize;
    for ch in text.chars() {
        if is_cjk(ch) {
            chinese += 1;
        } else if ch.is_ascii_alphabetic() {
            english += 1;
        }
    }

    if chinese == 0 && english == 0 {
        return Some(DetectedTranslationDirection::EnToZh);
    }
    if chinese >= english && chinese > 0 {
        Some(DetectedTranslationDirection::ZhToEn)
    } else {
        Some(DetectedTranslationDirection::EnToZh)
    }
}

fn is_cjk(ch: char) -> bool {
    matches!(
        ch,
        '\u{3400}'..='\u{4DBF}'
            | '\u{4E00}'..='\u{9FFF}'
            | '\u{F900}'..='\u{FAFF}'
            | '\u{20000}'..='\u{2A6DF}'
            | '\u{2A700}'..='\u{2B73F}'
            | '\u{2B740}'..='\u{2B81F}'
            | '\u{2B820}'..='\u{2CEAF}'
    )
}
```

- [ ] **Step 4: Run tests**

Run:

```powershell
cargo test features::text_translation::language
```

Expected: four tests pass.

- [ ] **Step 5: Commit**

```powershell
git add src/features/text_translation/language.rs src/features/text_translation/mod.rs
git commit -m "feat: detect translation direction"
```

---

### Task 3: Tencent Cloud Client

**Files:**
- Modify: `Cargo.toml`
- Create: `src/infrastructure/tencent_cloud/mod.rs`
- Create: `src/infrastructure/tencent_cloud/signing.rs`
- Create: `src/infrastructure/tencent_cloud/client.rs`
- Modify: `src/infrastructure/mod.rs`

- [ ] **Step 1: Add dependencies**

Add to `[dependencies]` in `Cargo.toml`:

```toml
base64 = "0.22"
hex = "0.4"
hmac = "0.12"
sha2 = "0.10"
ureq = { version = "2.12", features = ["json", "tls"] }
```

- [ ] **Step 2: Add signing tests first**

Create `src/infrastructure/tencent_cloud/signing.rs`:

```rust
#[cfg(test)]
mod tests {
    use super::{canonical_headers, hashed_payload};

    #[test]
    fn canonical_headers_lowercase_and_sort_headers() {
        let (headers, signed) = canonical_headers(&[
            ("X-TC-Action", "TextTranslate"),
            ("Host", "tmt.tencentcloudapi.com"),
            ("Content-Type", "application/json; charset=utf-8"),
        ]);

        assert_eq!(
            headers,
            "content-type:application/json; charset=utf-8\nhost:tmt.tencentcloudapi.com\nx-tc-action:TextTranslate\n"
        );
        assert_eq!(signed, "content-type;host;x-tc-action");
    }

    #[test]
    fn hashes_payload_as_lowercase_hex_sha256() {
        assert_eq!(
            hashed_payload("{}"),
            "44136fa355b3678a1146ad16f7e8649e94fb4f7c4dbe2dc8aebe1c9e820d0d5"
        );
    }
}
```

Create `src/infrastructure/tencent_cloud/mod.rs`:

```rust
pub mod client;
pub mod signing;
```

Add to `src/infrastructure/mod.rs`:

```rust
pub mod tencent_cloud;
```

- [ ] **Step 3: Run signing tests and verify RED**

Run:

```powershell
cargo test infrastructure::tencent_cloud::signing
```

Expected: compile failure for missing functions.

- [ ] **Step 4: Implement signing helpers**

Add to `signing.rs`:

```rust
use hmac::{Hmac, Mac};
use sha2::{Digest, Sha256};

type HmacSha256 = Hmac<Sha256>;

pub fn hashed_payload(payload: &str) -> String {
    hex::encode(Sha256::digest(payload.as_bytes()))
}

pub fn canonical_headers(headers: &[(&str, &str)]) -> (String, String) {
    let mut normalized = headers
        .iter()
        .map(|(name, value)| (name.to_ascii_lowercase(), value.trim().to_string()))
        .collect::<Vec<_>>();
    normalized.sort_by(|left, right| left.0.cmp(&right.0));

    let canonical = normalized
        .iter()
        .map(|(name, value)| format!("{name}:{value}\n"))
        .collect::<String>();
    let signed = normalized
        .iter()
        .map(|(name, _)| name.as_str())
        .collect::<Vec<_>>()
        .join(";");
    (canonical, signed)
}

fn hmac_sha256(key: &[u8], message: &str) -> Vec<u8> {
    let mut mac = HmacSha256::new_from_slice(key).expect("HMAC accepts any key length");
    mac.update(message.as_bytes());
    mac.finalize().into_bytes().to_vec()
}
```

Then add a public `sign_v3` function:

```rust
pub struct TencentSignatureInput<'a> {
    pub secret_id: &'a str,
    pub secret_key: &'a str,
    pub service: &'a str,
    pub host: &'a str,
    pub action: &'a str,
    pub version: &'a str,
    pub region: &'a str,
    pub timestamp: i64,
    pub date: &'a str,
    pub payload: &'a str,
}

pub fn sign_v3(input: TencentSignatureInput<'_>) -> String {
    let algorithm = "TC3-HMAC-SHA256";
    let (canonical_headers, signed_headers) = canonical_headers(&[
        ("content-type", "application/json; charset=utf-8"),
        ("host", input.host),
    ]);
    let canonical_request = format!(
        "POST\n/\n\n{}{}\n{}",
        canonical_headers,
        signed_headers,
        hashed_payload(input.payload)
    );
    let credential_scope = format!("{}/{}/tc3_request", input.date, input.service);
    let string_to_sign = format!(
        "{algorithm}\n{}\n{}\n{}",
        input.timestamp,
        credential_scope,
        hex::encode(Sha256::digest(canonical_request.as_bytes()))
    );
    let secret_date = hmac_sha256(format!("TC3{}", input.secret_key).as_bytes(), input.date);
    let secret_service = hmac_sha256(&secret_date, input.service);
    let secret_signing = hmac_sha256(&secret_service, "tc3_request");
    let signature = hex::encode(hmac_sha256(&secret_signing, &string_to_sign));
    format!(
        "{algorithm} Credential={}/{}, SignedHeaders={}, Signature={}",
        input.secret_id, credential_scope, signed_headers, signature
    )
}
```

- [ ] **Step 5: Add response parsing tests**

Create `src/infrastructure/tencent_cloud/client.rs`:

```rust
#[cfg(test)]
mod tests {
    use super::{parse_ocr_response, parse_text_translate_response};

    #[test]
    fn parses_text_translate_target_text() {
        let body = r#"{"Response":{"TargetText":"hello","RequestId":"rid"}}"#;
        assert_eq!(parse_text_translate_response(body).unwrap(), "hello");
    }

    #[test]
    fn parses_ocr_text_lines_in_order() {
        let body = r#"{"Response":{"TextDetections":[{"DetectedText":"line1"},{"DetectedText":"line2"}],"RequestId":"rid"}}"#;
        assert_eq!(parse_ocr_response(body).unwrap(), "line1\nline2");
    }

    #[test]
    fn parses_tencent_error_response_as_err() {
        let body = r#"{"Response":{"Error":{"Code":"AuthFailure","Message":"bad"},"RequestId":"rid"}}"#;
        assert!(parse_text_translate_response(body).unwrap_err().contains("AuthFailure"));
    }
}
```

- [ ] **Step 6: Implement Tencent client parsing and request shell**

Add to `client.rs`:

```rust
use base64::Engine;
use serde_json::{json, Value};

use crate::settings::TencentCloudSettings;
use super::signing::{sign_v3, TencentSignatureInput};

const REGION: &str = "ap-guangzhou";
const TMT_HOST: &str = "tmt.tencentcloudapi.com";
const TMT_SERVICE: &str = "tmt";
const TMT_VERSION: &str = "2018-03-21";
const OCR_HOST: &str = "ocr.tencentcloudapi.com";
const OCR_SERVICE: &str = "ocr";
const OCR_VERSION: &str = "2018-11-19";

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TencentTranslationDirection {
    ZhToEn,
    EnToZh,
}

pub fn parse_text_translate_response(body: &str) -> Result<String, String> {
    let value = serde_json::from_str::<Value>(body).map_err(|err| err.to_string())?;
    response_error(&value)?;
    value["Response"]["TargetText"]
        .as_str()
        .map(ToOwned::to_owned)
        .ok_or_else(|| "Tencent translate response missing TargetText".to_string())
}

pub fn parse_ocr_response(body: &str) -> Result<String, String> {
    let value = serde_json::from_str::<Value>(body).map_err(|err| err.to_string())?;
    response_error(&value)?;
    let lines = value["Response"]["TextDetections"]
        .as_array()
        .ok_or_else(|| "Tencent OCR response missing TextDetections".to_string())?
        .iter()
        .filter_map(|item| item["DetectedText"].as_str())
        .collect::<Vec<_>>();
    Ok(lines.join("\n"))
}

fn response_error(value: &Value) -> Result<(), String> {
    if let Some(error) = value["Response"]["Error"].as_object() {
        let code = error.get("Code").and_then(Value::as_str).unwrap_or("Unknown");
        let message = error.get("Message").and_then(Value::as_str).unwrap_or("");
        return Err(format!("Tencent API error {code}: {message}"));
    }
    Ok(())
}
```

Add request functions:

```rust
pub fn translate_text(
    credentials: &TencentCloudSettings,
    direction: TencentTranslationDirection,
    text: &str,
) -> Result<String, String> {
    let (source, target) = match direction {
        TencentTranslationDirection::ZhToEn => ("zh", "en"),
        TencentTranslationDirection::EnToZh => ("en", "zh"),
    };
    let payload = json!({
        "SourceText": text,
        "Source": source,
        "Target": target,
        "ProjectId": 0
    });
    let body = send_request(credentials, TMT_HOST, TMT_SERVICE, "TextTranslate", TMT_VERSION, payload)?;
    parse_text_translate_response(&body)
}

pub fn recognize_image(
    credentials: &TencentCloudSettings,
    image_path: &std::path::Path,
) -> Result<String, String> {
    let bytes = std::fs::read(image_path).map_err(|err| format!("read image failed: {err}"))?;
    let payload = json!({
        "ImageBase64": base64::engine::general_purpose::STANDARD.encode(bytes)
    });
    let body = send_request(credentials, OCR_HOST, OCR_SERVICE, "GeneralBasicOCR", OCR_VERSION, payload)?;
    parse_ocr_response(&body)
}

fn send_request(
    credentials: &TencentCloudSettings,
    host: &str,
    service: &str,
    action: &str,
    version: &str,
    payload: Value,
) -> Result<String, String> {
    if credentials.secret_id.trim().is_empty() || credentials.secret_key.trim().is_empty() {
        return Err("Tencent SecretId or SecretKey is empty".to_string());
    }
    let payload = serde_json::to_string(&payload).map_err(|err| err.to_string())?;
    let timestamp = chrono::Utc::now().timestamp();
    let date = chrono::Utc::now().format("%Y-%m-%d").to_string();
    let authorization = sign_v3(TencentSignatureInput {
        secret_id: credentials.secret_id.trim(),
        secret_key: credentials.secret_key.trim(),
        service,
        host,
        action,
        version,
        region: REGION,
        timestamp,
        date: &date,
        payload: &payload,
    });
    let response = ureq::post(&format!("https://{host}/"))
        .set("Authorization", &authorization)
        .set("Content-Type", "application/json; charset=utf-8")
        .set("Host", host)
        .set("X-TC-Action", action)
        .set("X-TC-Version", version)
        .set("X-TC-Timestamp", &timestamp.to_string())
        .set("X-TC-Region", REGION)
        .send_string(&payload)
        .map_err(|err| format!("Tencent request failed: {err}"))?;
    response.into_string().map_err(|err| err.to_string())
}
```

- [ ] **Step 7: Run Tencent tests**

Run:

```powershell
cargo test infrastructure::tencent_cloud
```

Expected: signing and parser tests pass without real credentials or network.

- [ ] **Step 8: Commit**

```powershell
git add Cargo.toml src/infrastructure/mod.rs src/infrastructure/tencent_cloud
git commit -m "feat: add Tencent Cloud client"
```

---

### Task 4: Translation And OCR Backend Dispatch

**Files:**
- Modify: `src/features/text_translation/translator.rs`
- Modify: `src/features/file_preview/ocr.rs`
- Modify: `src/app.rs`
- Modify: `src/features/settings/window.rs`
- Modify: `src/infrastructure/tray.rs`
- Modify: `src/bin/check_translation.rs`

- [ ] **Step 1: Add translation service tests for backend-neutral logic**

In `src/features/text_translation/translator.rs` tests, import direction detection:

```rust
use crate::features::text_translation::language::{
    detect_translation_direction, DetectedTranslationDirection,
};
```

Add:

```rust
#[test]
fn detects_direction_before_translation_runtime() {
    assert_eq!(
        detect_translation_direction("你好"),
        Some(DetectedTranslationDirection::ZhToEn)
    );
    assert_eq!(
        detect_translation_direction("hello"),
        Some(DetectedTranslationDirection::EnToZh)
    );
}
```

- [ ] **Step 2: Run translator tests**

Run:

```powershell
cargo test features::text_translation::translator
```

Expected: tests compile after Task 2.

- [ ] **Step 3: Refactor `TranslationService` state**

Replace `TranslationState` with:

```rust
struct TranslationState {
    enabled: bool,
    ai_backend: AiBackend,
    tencent_cloud: TencentCloudSettings,
    zh_to_en_model: IdleModel<TranslationModel>,
    en_to_zh_model: IdleModel<TranslationModel>,
}
```

Change constructor signature:

```rust
pub fn new(
    settings: &TextTranslationSettings,
    ai_backend: AiBackend,
    tencent_cloud: &TencentCloudSettings,
) -> Self
```

Change `apply_settings` signature:

```rust
pub fn apply_settings(
    &self,
    settings: &TextTranslationSettings,
    ai_backend: AiBackend,
    tencent_cloud: &TencentCloudSettings,
)
```

Disable path:

```rust
state.enabled = false;
state.zh_to_en_model.unload_now();
state.en_to_zh_model.unload_now();
```

- [ ] **Step 4: Dispatch translation by backend**

Inside `translate_streaming_cancellable`, detect direction:

```rust
let direction = detect_translation_direction(text)
    .ok_or_else(|| "text translation source is empty".to_string())?;
```

For local backend:

```rust
match direction {
    DetectedTranslationDirection::ZhToEn => {
        let model = state.zh_to_en_model.get_or_try_load(|| {
            TranslationModel::load(&default_zh_to_en_translation_model_path())
        })?;
        let result = model.translate_streaming(text, on_partial, should_cancel);
        state.zh_to_en_model.refresh_idle_deadline(Instant::now());
        result
    }
    DetectedTranslationDirection::EnToZh => {
        let model = state.en_to_zh_model.get_or_try_load(|| {
            TranslationModel::load(&default_en_to_zh_translation_model_path())
        })?;
        let result = model.translate_streaming(text, on_partial, should_cancel);
        state.en_to_zh_model.refresh_idle_deadline(Instant::now());
        result
    }
}
```

For Tencent backend:

```rust
let tencent_direction = match direction {
    DetectedTranslationDirection::ZhToEn => TencentTranslationDirection::ZhToEn,
    DetectedTranslationDirection::EnToZh => TencentTranslationDirection::EnToZh,
};
let credentials = state.tencent_cloud.clone();
drop(state);
crate::infrastructure::tencent_cloud::client::translate_text(
    &credentials,
    tencent_direction,
    text,
)
```

Keep streaming callback behavior for Tencent simple: call `on_partial(&translated)` once after success only if the callback type permits. If the current implementation requires moving `on_partial`, return the final text without partials.

- [ ] **Step 5: Refactor OCR backend dispatch**

Change `OcrService::new` and `apply_settings` to accept:

```rust
ai_backend: AiBackend,
tencent_cloud: &TencentCloudSettings,
```

Add to `OcrState`:

```rust
ai_backend: AiBackend,
tencent_cloud: TencentCloudSettings,
```

In `recognize_streaming`, if backend is Tencent:

```rust
let credentials = state.tencent_cloud.clone();
drop(state);
let result = crate::infrastructure::tencent_cloud::client::recognize_image(
    &credentials,
    image_path,
);
if let Ok(text) = &result {
    on_partial(text);
}
result
```

If backend is local, keep the existing lazy model flow.

- [ ] **Step 6: Update service construction/apply call sites**

In `src/app.rs`, `settings/window.rs`, `tray.rs`, and standalone preview path, pass `settings.ai_backend` and `settings.tencent_cloud`.

Example:

```rust
let snapshot = settings.lock().unwrap().clone();
let translation_service = Arc::new(TranslationService::new(
    &snapshot.text_translation,
    snapshot.ai_backend,
    &snapshot.tencent_cloud,
));
let ocr_service = Arc::new(OcrService::new(
    &snapshot.image_recognition,
    snapshot.ai_backend,
    &snapshot.tencent_cloud,
));
```

Apply settings:

```rust
translation_service.apply_settings(
    &updated_settings.text_translation,
    updated_settings.ai_backend,
    &updated_settings.tencent_cloud,
);
ocr_service.apply_settings(
    &updated_settings.image_recognition,
    updated_settings.ai_backend,
    &updated_settings.tencent_cloud,
);
```

- [ ] **Step 7: Run checks**

Run:

```powershell
cargo test features::text_translation
cargo check
```

Expected: tests and check pass.

- [ ] **Step 8: Commit**

```powershell
git add src/features/text_translation src/features/file_preview/ocr.rs src/app.rs src/features/settings/window.rs src/infrastructure/tray.rs src/bin/check_translation.rs
git commit -m "feat: dispatch translation and OCR backends"
```

---

### Task 5: Settings UI And Tray Redesign

**Files:**
- Modify: `ui/settings.slint`
- Modify: `src/features/settings/window.rs`
- Modify: `src/infrastructure/tray.rs`

- [ ] **Step 1: Update Slint properties and callback signature**

In `ui/settings.slint`, remove:

```slint
in-out property <bool> zh_to_en_enabled: true;
in-out property <string> zh_to_en_model_dir: "";
in-out property <string> en_to_zh_model_dir: "";
callback browse_zh_to_en_model_dir();
callback browse_en_to_zh_model_dir();
```

Add:

```slint
in-out property <bool> tencent_backend_enabled: false;
in-out property <string> tencent_secret_id: "";
in-out property <string> tencent_secret_key: "";
```

Change `apply_settings` parameters to:

```slint
callback apply_settings(
    copy_timestamp_enabled: bool,
    clipboard_history_enabled: bool,
    screenshot_enabled: bool,
    image_recognition_enabled: bool,
    text_translation_enabled: bool,
    tencent_backend_enabled: bool,
    tencent_secret_id: string,
    tencent_secret_key: string,
    image_recognition_model_dir: string,
    text_translation_debounce_seconds: string
);
```

- [ ] **Step 2: Remove direction/local translation model UI**

Delete the direction row and both translation model directory sections. Keep image recognition local model directory. Add a backend row:

```slint
HorizontalLayout {
    spacing: 16px;
    Text {
        text: "AI 后端";
        color: #223041;
        font-size: 14px;
        vertical-alignment: center;
        horizontal-stretch: 1;
    }
    Switch {
        checked <=> root.tencent_backend_enabled;
        text: self.checked ? "腾讯 API" : "本地模型";
        toggled => {
            root.apply_settings(root.copy_timestamp_enabled, root.clipboard_history_enabled, root.screenshot_enabled, root.image_recognition_enabled, root.text_translation_enabled, root.tencent_backend_enabled, root.tencent_secret_id, root.tencent_secret_key, root.image_recognition_model_dir, root.text_translation_debounce_seconds);
        }
    }
}
```

Add Tencent credential section:

```slint
VerticalLayout {
    spacing: 8px;
    Text { text: "腾讯访问密钥"; color: #1A2430; font-size: 18px; font-weight: 800; }
    LineEdit {
        text <=> root.tencent_secret_id;
        placeholder-text: "SecretId";
        edited(_) => {
            root.apply_settings(root.copy_timestamp_enabled, root.clipboard_history_enabled, root.screenshot_enabled, root.image_recognition_enabled, root.text_translation_enabled, root.tencent_backend_enabled, root.tencent_secret_id, root.tencent_secret_key, root.image_recognition_model_dir, root.text_translation_debounce_seconds);
        }
    }
    LineEdit {
        text <=> root.tencent_secret_key;
        placeholder-text: "SecretKey";
        edited(_) => {
            root.apply_settings(root.copy_timestamp_enabled, root.clipboard_history_enabled, root.screenshot_enabled, root.image_recognition_enabled, root.text_translation_enabled, root.tencent_backend_enabled, root.tencent_secret_id, root.tencent_secret_key, root.image_recognition_model_dir, root.text_translation_debounce_seconds);
        }
    }
}
```

- [ ] **Step 3: Update settings window Rust bindings**

In `src/features/settings/window.rs`, remove browse callbacks for translation model dirs and remove imports/usages of `TranslationDirection`.

Add `AiBackend` and `TencentCloudSettings` imports.

Change `init_settings_window` signature:

```rust
pub fn init_settings_window(
    settings: Arc<Mutex<AppSettings>>,
    settings_store: SettingsStore,
    translation_service: Arc<TranslationService>,
    ocr_service: Arc<OcrService>,
    tray_menu_handles: TrayMenuHandles,
    on_settings_applied: std::rc::Rc<dyn Fn(&AppSettings)>,
) -> SettingsWindow
```

Change `apply_settings_snapshot` parameters to match Slint and build:

```rust
let ai_backend = if tencent_backend_enabled {
    AiBackend::Tencent
} else {
    AiBackend::Local
};
let tencent_cloud = TencentCloudSettings {
    secret_id: tencent_secret_id.trim().to_string(),
    secret_key: tencent_secret_key.trim().to_string(),
};
let new_text_translation = TextTranslationSettings {
    enabled: text_translation_enabled,
    debounce_seconds: debounce_seconds_from_string(text_translation_debounce_seconds),
};
```

After service apply calls:

```rust
on_settings_applied(&updated_settings);
```

- [ ] **Step 4: Update `show_settings_window`**

Set:

```rust
window.set_tencent_backend_enabled(settings.ai_backend == AiBackend::Tencent);
window.set_tencent_secret_id(settings.tencent_cloud.secret_id.clone().into());
window.set_tencent_secret_key(settings.tencent_cloud.secret_key.clone().into());
```

Do not set removed direction/model properties.

- [ ] **Step 5: Update tray**

In `src/infrastructure/tray.rs`:

- Remove `TranslationDirection` import.
- Remove `TEXT_TRANSLATION_ZH_EN_MENU_ID` and `TEXT_TRANSLATION_EN_ZH_MENU_ID`.
- Add:

```rust
const TEXT_TRANSLATION_MENU_ID: &str = "text_translation_enabled";
const AI_BACKEND_LOCAL_MENU_ID: &str = "ai_backend_local";
const AI_BACKEND_TENCENT_MENU_ID: &str = "ai_backend_tencent";
```

Add tray items:

```rust
let text_translation_item = CheckMenuItem::with_id(
    TEXT_TRANSLATION_MENU_ID,
    "翻译文本 - Ctrl+C",
    true,
    settings.text_translation.enabled,
    None,
);
let ai_backend_menu = Submenu::new("AI 后端", true);
let ai_backend_local_item = CheckMenuItem::with_id(
    AI_BACKEND_LOCAL_MENU_ID,
    "本地模型",
    true,
    settings.ai_backend == AiBackend::Local,
    None,
);
let ai_backend_tencent_item = CheckMenuItem::with_id(
    AI_BACKEND_TENCENT_MENU_ID,
    "腾讯 API",
    true,
    settings.ai_backend == AiBackend::Tencent,
    None,
);
```

Update `TrayMenuHandles` and `sync_from_settings`.

Change `start_tray_event_pump` signature:

```rust
on_settings_applied: impl Fn(&AppSettings) + 'static,
```

When a tray item changes settings, save settings, sync tray handles, apply services, then call `on_settings_applied(&settings)`.

- [ ] **Step 6: Run UI compile check**

Run:

```powershell
cargo check
```

Expected: Slint generated bindings compile.

- [ ] **Step 7: Commit**

```powershell
git add ui/settings.slint src/features/settings/window.rs src/infrastructure/tray.rs
git commit -m "feat: update AI backend settings UI"
```

---

### Task 6: Lazy Window Initialization And Listener Start

**Files:**
- Modify: `src/app.rs`
- Modify: `src/features/home/window.rs`
- Modify: `src/features/settings/window.rs`
- Modify: `src/infrastructure/tray.rs`

- [ ] **Step 1: Refactor home window to lazy callbacks**

Replace `init_home_window` signature in `src/features/home/window.rs` with:

```rust
pub fn init_home_window(
    open_time_trans: impl Fn() + 'static,
    open_clipboard_history: impl Fn() + 'static,
    open_text_translation: impl Fn() + 'static,
    open_file_preview: impl Fn() + 'static,
    open_screenshot: impl Fn() + 'static,
    open_settings: impl Fn() + 'static,
) -> HomeWindow
```

Inside, bind callbacks directly:

```rust
window.on_open_time_trans(open_time_trans);
window.on_open_clipboard_history(open_clipboard_history);
window.on_open_text_translation(open_text_translation);
window.on_open_file_preview(open_file_preview);
window.on_open_screenshot(open_screenshot);
window.on_open_settings(open_settings);
```

Remove now-unused imports.

- [ ] **Step 2: Add app window registry types**

Add near `ShortcutState` in `src/app.rs`:

```rust
struct AppWindows {
    time_trans: Option<crate::TimeTrans>,
    clipboard_history: Option<crate::ClipboardHistoryWindow>,
    text_translation: Option<crate::TextTranslationWindow>,
    file_preview: Option<crate::FilePreviewWindow>,
    screenshot: Option<crate::ScreenshotWindow>,
    settings: Option<crate::SettingsWindow>,
}

impl AppWindows {
    fn new() -> Self {
        Self {
            time_trans: None,
            clipboard_history: None,
            text_translation: None,
            file_preview: None,
            screenshot: None,
            settings: None,
        }
    }
}

#[derive(Clone)]
struct ShortcutWindows {
    history: Arc<Mutex<Option<slint::Weak<crate::ClipboardHistoryWindow>>>>,
    screenshot: Arc<Mutex<Option<slint::Weak<crate::ScreenshotWindow>>>>,
    translation: Arc<Mutex<Option<slint::Weak<crate::TextTranslationWindow>>>>,
    time_trans: Arc<Mutex<Option<slint::Weak<crate::TimeTrans>>>>,
}
```

Implement `ShortcutWindows::new()`.

- [ ] **Step 3: Add ensure helper functions**

Add `ensure_time_trans_window`, `ensure_clipboard_history_window`, `ensure_text_translation_window`, `ensure_file_preview_window`, `ensure_screenshot_window`, and `ensure_settings_window` in `src/app.rs`. Each helper:

- Borrows `Rc<RefCell<AppWindows>>`.
- Creates the window only when the slot is `None`.
- Writes weak handle into `ShortcutWindows` where needed.
- Returns a `slint::Weak<T>`.

Use existing window init functions and pass `clipboard_history_dir` to `init_clipboard_history_window`.

- [ ] **Step 4: Add clipboard listener start helper**

Add in `src/app.rs`:

```rust
fn ensure_clipboard_listener_started(
    started: &Arc<AtomicBool>,
    history: Arc<Mutex<ClipboardHistory>>,
    settings: Arc<Mutex<AppSettings>>,
    history_window: slint::Weak<crate::ClipboardHistoryWindow>,
    suppress_next_clipboard_history: Arc<Mutex<Option<crate::features::clipboard_history::history::ClipboardHistoryItem>>>,
    history_dir: std::path::PathBuf,
) {
    if started.swap(true, Ordering::SeqCst) {
        return;
    }
    if let Err(err) = start_clipboard_history_listener(
        history,
        settings,
        history_window,
        suppress_next_clipboard_history,
        history_dir,
    ) {
        started.store(false, Ordering::SeqCst);
        log::error!("failed to start clipboard history listener: {err}");
    }
}
```

- [ ] **Step 5: Rewrite startup window creation**

In `run`, replace eager creation of all feature windows with:

```rust
let windows = Rc::new(RefCell::new(AppWindows::new()));
let shortcut_windows = ShortcutWindows::new();
let settings_snapshot = settings.lock().unwrap().clone();
if settings_snapshot.copy_timestamp.enabled { ensure_time_trans_window(...); }
if settings_snapshot.clipboard_history.enabled {
    let weak = ensure_clipboard_history_window(...);
    ensure_clipboard_listener_started(..., weak, ...);
}
if settings_snapshot.screenshot.enabled { ensure_screenshot_window(...); }
if settings_snapshot.text_translation.enabled { ensure_text_translation_window(...); }
```

Do not create settings or normal file preview at startup.

- [ ] **Step 6: Update home callbacks**

Construct `home_window` with closures that call ensure helpers then show the relevant window:

- Time window: `show_without_taskbar_icon` and `activate_slint_window`.
- Clipboard history: `show_clipboard_history_window`.
- Translation: `show_translation_pending("", false)` pattern from current code.
- File preview: `show_empty_file_preview_window`.
- Screenshot: `show_screenshot_window`.
- Settings: `show_settings_window`.

- [ ] **Step 7: Update global input listener**

Use `ShortcutWindows` slots in the global input closure. If a required weak slot is missing and setting is enabled, call `slint::invoke_from_event_loop` to create/show the window from the UI thread. Do not create Slint windows directly in the input thread.

- [ ] **Step 8: Wire settings/tray applied hook**

Create:

```rust
let on_settings_applied: Rc<dyn Fn(&AppSettings)> = Rc::new({
    // clones of windows, shortcut_windows, services, listener state
    move |settings| {
        if settings.copy_timestamp.enabled { ensure_time_trans_window(...); }
        if settings.clipboard_history.enabled {
            let weak = ensure_clipboard_history_window(...);
            ensure_clipboard_listener_started(..., weak, ...);
        }
        if settings.screenshot.enabled { ensure_screenshot_window(...); }
        if settings.text_translation.enabled { ensure_text_translation_window(...); }
    }
});
```

Pass this hook to settings window creation and tray event pump.

- [ ] **Step 9: Run checks**

Run:

```powershell
cargo check
```

Expected: project compiles.

- [ ] **Step 10: Commit**

```powershell
git add src/app.rs src/features/home/window.rs src/features/settings/window.rs src/infrastructure/tray.rs
git commit -m "feat: lazily initialize feature windows"
```

---

### Task 7: Screenshot Undo Limits And File Preview Cleanup

**Files:**
- Modify: `src/features/screenshot/window.rs`
- Modify: `src/features/file_preview/window.rs`

- [ ] **Step 1: Add screenshot undo tests**

In `src/features/screenshot/window.rs`, add tests:

```rust
#[cfg(test)]
mod tests {
    use super::{trim_undo_stack, undo_stack_byte_len, MAX_UNDO_BYTES, MAX_UNDO_SNAPSHOTS};
    use image::RgbaImage;

    #[test]
    fn trim_undo_stack_limits_snapshot_count() {
        let mut stack = (0..MAX_UNDO_SNAPSHOTS + 2)
            .map(|_| RgbaImage::new(1, 1))
            .collect::<Vec<_>>();

        trim_undo_stack(&mut stack);

        assert_eq!(stack.len(), MAX_UNDO_SNAPSHOTS);
    }

    #[test]
    fn trim_undo_stack_limits_total_bytes() {
        let side = 4096;
        let mut stack = vec![RgbaImage::new(side, side), RgbaImage::new(side, side)];

        trim_undo_stack(&mut stack);

        assert!(undo_stack_byte_len(&stack) <= MAX_UNDO_BYTES);
    }
}
```

- [ ] **Step 2: Run screenshot tests and verify RED**

Run:

```powershell
cargo test features::screenshot
```

Expected: compile failure because trim helpers are missing.

- [ ] **Step 3: Implement undo trimming**

Add near `push_undo_snapshot`:

```rust
const MAX_UNDO_SNAPSHOTS: usize = 8;
const MAX_UNDO_BYTES: usize = 64 * 1024 * 1024;

fn image_byte_len(image: &RgbaImage) -> usize {
    image.as_raw().len()
}

fn undo_stack_byte_len(stack: &[RgbaImage]) -> usize {
    stack.iter().map(image_byte_len).sum()
}

fn trim_undo_stack(stack: &mut Vec<RgbaImage>) {
    while stack.len() > MAX_UNDO_SNAPSHOTS || undo_stack_byte_len(stack) > MAX_UNDO_BYTES {
        if stack.is_empty() {
            break;
        }
        stack.remove(0);
    }
}
```

Update:

```rust
fn push_undo_snapshot(session: &mut ScreenshotSession) {
    session.undo_stack.push(session.image.clone());
    trim_undo_stack(&mut session.undo_stack);
}
```

- [ ] **Step 4: Clear screenshot UI image state**

In finish/cancel cleanup paths, before hiding:

```rust
window.set_screenshot(Image::default());
window.set_text_preview(Image::default());
window.set_has_text_preview(false);
window.set_brush_segments(ModelRc::from(Rc::new(VecModel::from(Vec::<BrushSegment>::new()))));
```

- [ ] **Step 5: Add file preview clear helper**

In `src/features/file_preview/window.rs`, add:

```rust
fn clear_preview_state(window: &FilePreviewWindow) {
    window.set_file_path("".into());
    window.set_file_name("".into());
    window.set_status_text("".into());
    window.set_image_content(Image::default());
    window.set_has_content(false);
    window.set_image_width(0);
    window.set_image_height(0);
    window.set_ocr_panel_visible(false);
    window.set_ocr_text("".into());
    window.set_ocr_status_text("".into());
    OCR_GENERATION.fetch_add(1, Ordering::SeqCst);
}
```

Use it in `show_empty_file_preview_window` and in `close_preview_window` before hiding non-standalone windows.

- [ ] **Step 6: Run tests and check**

Run:

```powershell
cargo test features::screenshot
cargo check
```

Expected: tests and check pass.

- [ ] **Step 7: Commit**

```powershell
git add src/features/screenshot/window.rs src/features/file_preview/window.rs
git commit -m "feat: bound screenshot and preview memory"
```

---

### Task 8: Full Verification And Manual Checks

**Files:**
- Modify only files required by failing verification.

- [ ] **Step 1: Run full automated verification**

Run:

```powershell
cargo test
cargo check
```

Expected: all tests pass and `cargo check` exits 0.

- [ ] **Step 2: Manual local backend checks**

Run:

```powershell
cargo run
```

Manual expectations:

- With local backend and translation enabled, no translation model loads at startup.
- Chinese copied text triggers Chinese-to-English local model.
- English copied text triggers English-to-Chinese local model.
- Repeated use within one minute reuses the model.
- After more than one minute idle, logs show idle unload.

- [ ] **Step 3: Manual Tencent backend checks**

In settings, select Tencent API backend and enter credentials manually. Do not commit credentials.

Manual expectations:

- Chinese copied text returns English translation.
- English copied text returns Chinese translation.
- File preview OCR panel returns text for an image.
- Blank credentials produce a clear user-visible error and do not log secrets.

- [ ] **Step 4: Manual lazy initialization checks**

Manual expectations:

- Disabled feature windows are not created at startup.
- Home window buttons create windows on demand.
- Enabling a feature in settings or tray creates required shortcut windows/listeners.
- Clipboard listener captures history only when enabled.

- [ ] **Step 5: Manual memory cleanup checks**

Manual expectations:

- Clipboard history directory contains `clipboard_history.json` and at most 20 `clipboard-image-*.png` files after copying more than 20 images.
- Screenshot undo does not grow beyond count/byte limits.
- Closing file preview clears image/OCR UI state.

- [ ] **Step 6: Commit verification fixes if needed**

If fixes were needed:

```powershell
git status --short
git add Cargo.toml src ui docs
git commit -m "fix: complete translation OCR backend verification"
```

If no fixes were needed, do not create an empty commit.

---

## Self-Review

- Spec coverage: settings/tray removal, backend selector, Tencent credentials, language detection, Tencent translation/OCR, lazy windows/listener, screenshot undo limits, file preview cleanup, and final verification are all mapped to tasks.
- Placeholder scan: no `TBD`, `TODO`, or open-ended implementation placeholders remain.
- Type consistency: `AiBackend`, `TencentCloudSettings`, `DetectedTranslationDirection`, and `TencentTranslationDirection` are introduced before use in later tasks.
- Secret handling: no Tencent credential values are included in this plan.
