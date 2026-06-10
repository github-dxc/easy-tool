# Translation/OCR Backend And Lazy Windows Design

## Goal

Finish the remaining memory-optimization work while changing translation and OCR into a backend-selectable system:

- Keep feature windows and listeners lazy according to settings.
- Remove manual local translation direction/model configuration from settings and tray.
- Choose local translation direction automatically from the copied text.
- Add Tencent Cloud as an alternative backend for text translation and image text recognition.
- Keep screenshot and file-preview memory cleanup from the previous memory plan.

## Current Baseline

The earlier memory work already completed:

- App-owned storage path helpers.
- Disk-backed clipboard history under `<application root>/temp/screenshot/`.
- Clipboard image persistence, index cleanup, and paste/readback from disk.
- Lazy model loading plus one-minute idle unload for local translation and OCR.

The remaining memory tasks are:

- Conditional UI-thread window creation by feature settings.
- Clipboard listener start-on-enable.
- Settings/tray hooks after runtime setting changes.
- Screenshot undo stack count/byte limits.
- File preview large-state cleanup on close/reset.
- Full verification.

## User-Facing Settings

Settings page keeps these feature toggles:

- Show copy timestamp window.
- Clipboard history.
- Screenshot shortcut.
- Text translation shortcut.
- Image recognition.

Settings page removes these controls:

- Manual translation direction switch (`中译英` / `英译中`).
- Local Chinese-to-English translation model directory input.
- Local English-to-Chinese translation model directory input.

Settings page keeps this local model control:

- Image recognition local model directory.

Settings page adds these controls:

- Backend selector for AI features: `本地模型` / `腾讯 API`.
- Tencent credential section:
  - `SecretId` text input.
  - `SecretKey` password-style text input if Slint supports masking in the current controls; otherwise a normal text input with clear labeling.

The Tencent credentials are stored only in the user settings TOML. They must not be committed, embedded as defaults, or written into docs/tests. The temporary credentials provided in chat are for local manual testing only and are intentionally omitted from repo files.

## Tray Behavior

The tray keeps:

- Copy timestamp toggle.
- Clipboard history toggle.
- Screenshot shortcut toggle.
- Text translation shortcut toggle.
- Open home window.
- Quit.

The tray removes:

- `中译英` child item.
- `英译中` child item.
- The translation direction submenu.

The tray adds:

- AI backend toggle or submenu item pair:
  - `本地模型`.
  - `腾讯 API`.

Tray and settings page must stay synchronized through the existing `TrayMenuHandles::sync_from_settings` pattern.

## Settings Model

Add a shared backend enum:

```rust
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum AiBackend {
    Local,
    Tencent,
}
```

Add Tencent credentials:

```rust
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct TencentCloudSettings {
    #[serde(default)]
    pub secret_id: String,
    #[serde(default)]
    pub secret_key: String,
}
```

`AppSettings` gains:

```rust
#[serde(default)]
pub ai_backend: AiBackend,
#[serde(default)]
pub tencent_cloud: TencentCloudSettings,
```

`TextTranslationSettings` changes:

- Keep `enabled`.
- Keep `debounce_seconds`.
- Remove `direction`.
- Remove `zh_to_en_model_dir`.
- Remove `en_to_zh_model_dir`.

The app still has built-in default local model paths:

- `resource/Xenova/opus-mt-zh-en`.
- `resource/Xenova/opus-mt-en-zh`.

Those are no longer configurable in the UI. They can remain as private helper functions.

Existing config compatibility:

- Unknown removed TOML fields are ignored by serde.
- New fields default to `ai_backend = "local"` and empty Tencent credentials.

## Local Translation Direction Detection

Before translation, classify the source text by counting:

- Chinese characters: Unicode CJK Unified Ideographs and common CJK extension ranges that Rust `char` can compare by range.
- English letters: ASCII `A-Z` and `a-z`.

Decision:

- If Chinese count is greater than or equal to English count and Chinese count is nonzero, use Chinese-to-English.
- If English count is greater than Chinese count, use English-to-Chinese.
- If both counts are zero, default to English-to-Chinese for ASCII-ish non-CJK input, but return an empty/no-op result if the source text is empty after trimming.

This classifier is small and pure, with unit tests. It avoids network calls and avoids loading any model before selecting direction.

## Local Translation Runtime

`TranslationService` becomes a dual-local-model service:

- It owns separate idle-loaded model holders for Chinese-to-English and English-to-Chinese.
- `apply_settings` no longer receives or stores a direction.
- On each translation call:
  - Detect direction from the source text.
  - Load only the selected local model if backend is local.
  - Refresh only the selected model's one-minute idle deadline after the call attempt.
  - Cleanup unloads either model whose deadline has expired.
- Disabling text translation unloads both local models.

The current `IdleModel` helper remains the local model lifecycle primitive.

## Tencent Cloud API Integration

Create a focused Tencent Cloud client module. It is responsible for:

- TC3-HMAC-SHA256 request signing.
- JSON POST requests.
- Text translation.
- OCR.
- Error mapping into `Result<T, String>`.

Official documentation used for implementation:

- Tencent Machine Translation `TextTranslate`: `https://cloud.tencent.com/document/product/551/15619`
- Tencent Machine Translation international `TextTranslate`: `https://www.tencentcloud.com/document/product/1161/50435`
- Tencent OCR `GeneralBasicOCR`: `https://www.tencentcloud.com/document/product/1005/37315`
- Tencent Cloud Signature v3: `https://www.tencentcloud.com/document/product/845/32207`

Use API v3 JSON POST with TC3-HMAC-SHA256 because Tencent Cloud recommends Signature v3 and JSON POST for business APIs.

Endpoints:

- Text translation:
  - Host: `tmt.tencentcloudapi.com`.
  - Service: `tmt`.
  - Action: `TextTranslate`.
  - Version: `2018-03-21`.
- OCR:
  - Host: `ocr.tencentcloudapi.com`.
  - Service: `ocr`.
  - Action: `GeneralBasicOCR`.
  - Version: `2018-11-19`.

Region:

- Use `ap-guangzhou` as the default region for both services.
- Keep region constants centralized so future settings can expose them without touching signing code.

Tencent text translation request:

- `SourceText`: input text.
- `Source`: `zh` for Chinese-to-English, `en` for English-to-Chinese.
- `Target`: `en` or `zh`.
- `ProjectId`: `0`.

Tencent OCR request:

- Use `ImageBase64` from the selected image file.
- Join returned text lines in result order with `\n`.

Credential behavior:

- If backend is Tencent and either credential is blank, calls fail with a clear settings error.
- Credentials are loaded from `AppSettings.tencent_cloud`.
- Credentials are cloned into request tasks only when needed.
- Credentials are not logged.

HTTP client:

- Prefer `ureq` or `reqwest` blocking client. Since current OCR/translation work already runs on background threads, blocking HTTP is acceptable and avoids async runtime integration.
- Add only the minimum dependencies required for HTTPS, HMAC-SHA256, SHA256, hex/base64, and time if current dependencies do not already provide them.

## Backend Dispatch

Text translation call flow:

1. Shortcut or translation window requests translation.
2. `TranslationService` checks `text_translation.enabled`.
3. Detect local direction from source text.
4. If backend is `Local`, run selected local model.
5. If backend is `Tencent`, call Tencent `TextTranslate`.
6. UI receives either translated text or error text through the existing translation window flow.

OCR call flow:

1. File preview OCR panel requests recognition.
2. `OcrService` checks `image_recognition.enabled`.
3. If backend is `Local`, use the existing local OCR model with idle unload.
4. If backend is `Tencent`, call Tencent `GeneralBasicOCR`.
5. UI receives recognized text or error text through the existing file preview flow.

## Window And Listener Lazy Initialization

Use a UI-thread window registry in `src/app.rs`:

- Strong Slint handles live only in `Rc<RefCell<AppWindows>>` on the UI thread.
- Background input/listener threads hold only `slint::Weak<T>` through small `Arc<Mutex<Option<Weak<T>>>>` slots.

Startup:

- Always initialize logging, settings, services, tray, home window, and global input listener.
- Create feature windows only when their settings say they are enabled:
  - Copy timestamp window.
  - Clipboard history window.
  - Screenshot window.
  - Text translation window.
- Do not create settings window at startup.
- Do not create normal file preview window at startup.
- Standalone file preview mode still creates only the file preview window.

Runtime:

- Home page buttons call ensure functions on the UI thread.
- Global shortcuts read weak slots. If a feature is enabled but its window does not exist yet, the shortcut uses `slint::invoke_from_event_loop` to run the relevant ensure function.
- Clipboard listener starts only when clipboard history is enabled. Once started, it may remain running but should be inert when settings disable clipboard history.
- Settings and tray changes call an `on_settings_applied` hook. The hook creates newly enabled windows/listeners and updates service settings.

## Screenshot And File Preview Memory Cleanup

Screenshot:

- Limit undo snapshots to 8.
- Limit total undo bytes to 64 MiB.
- Trim oldest snapshots first.
- Clear screenshot image, text preview image, text-preview flag, and brush model when finishing/canceling.

File preview:

- Add `clear_preview_state`.
- Use it when opening empty preview.
- Use it before hiding non-standalone file preview windows.
- Increment OCR generation during cleanup so stale OCR callbacks cannot repopulate a closed preview.

## Error Handling

Backend errors:

- Tencent API errors are returned as user-visible error text in the same places local model errors appear.
- HTTP status, Tencent `Error.Code`, and `Error.Message` should be included in logs.
- User-visible messages should not include `SecretId`, `SecretKey`, signatures, or authorization headers.

Settings errors:

- Blank Tencent credentials with Tencent backend produce a clear message.
- Local backend does not require Tencent credentials.
- Tencent backend does not require local translation model paths.

Threading:

- Network and model inference stay off the UI thread.
- Model cleanup uses non-blocking service locks so Slint timers do not freeze the UI while inference is active.
- Slint strong handles remain UI-thread-owned.

## Verification

Automated tests:

- Language direction detection:
  - Chinese-majority text selects Chinese-to-English.
  - English-majority text selects English-to-Chinese.
  - Mixed tie with Chinese present selects Chinese-to-English.
  - Empty/whitespace text is handled without loading a model.
- Tencent signing:
  - Canonical request and authorization header match a known fixture from Tencent Signature v3 docs or a fixed locally generated fixture.
  - Signing never logs credentials.
- Tencent response parsing:
  - `TextTranslate` extracts `TargetText`.
  - `GeneralBasicOCR` joins detected text lines.
  - Tencent error responses become `Err`.
- Settings migration/defaults:
  - Missing `ai_backend` defaults to local.
  - Missing Tencent credentials default to empty strings.
  - Old removed translation direction/model fields do not break load.
- Window lazy initialization:
  - `cargo check` verifies Slint handle ownership and callback signatures.
- Screenshot undo trimming:
  - Trims by count.
  - Trims by total bytes.
- File preview cleanup:
  - Clears image/OCR fields and bumps generation.

Commands:

```powershell
cargo test
cargo check
```

Manual checks:

1. Start with text translation enabled and local backend. Confirm no translation model loads until translation is triggered.
2. Copy Chinese text and trigger translation. Confirm Chinese-to-English local model is used.
3. Copy English text and trigger translation. Confirm English-to-Chinese local model is used.
4. Switch backend to Tencent API, enter credentials in settings, and trigger Chinese/English translation.
5. Open image preview, switch OCR panel on, and verify local OCR or Tencent OCR according to backend setting.
6. Toggle features from tray and settings. Confirm windows/listeners are created lazily and stay synchronized.
7. Close file preview with a large image and confirm UI-held image/OCR state is cleared.
8. Use screenshot editing enough times to exceed undo limits and confirm old snapshots are trimmed.

## Out Of Scope

- Adding a Tencent region selector UI.
- Encrypting credentials at rest.
- Streaming Tencent translation/OCR partial results.
- Changing clipboard history storage location.
- Fixing unrelated mojibake strings already present in the UI.
