# Memory Optimization Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Reduce startup resident memory and long-running memory growth by loading feature windows from settings, unloading idle models, and moving clipboard history data/images to disk.

**Architecture:** Keep Slint strong window handles on the UI thread in an app-local window registry. Clipboard history uses an in-memory lightweight index backed by JSON and image files under `<application root>/temp/screenshot/`. Translation and OCR services keep model state behind their existing mutexes, load on first call, and expose an idle cleanup method called by a Slint timer.

**Tech Stack:** Rust 2024, Slint 1.15.1, serde/serde_json, image, arboard, ort, tokenizers, Windows APIs through windows-sys, cargo unit tests.

---

## File Structure

- Create `src/infrastructure/paths.rs`: application-root and clipboard-history storage path helpers.
- Modify `src/infrastructure/mod.rs`: export the new paths module.
- Modify `Cargo.toml`: add `tempfile` as a dev dependency for filesystem tests.
- Modify `src/features/clipboard_history/history.rs`: change image history entries from in-memory RGBA bytes to persisted image references and add trimming helpers.
- Create `src/features/clipboard_history/store.rs`: JSON index load/save, image persistence, image-file cleanup, and recovery from missing files.
- Modify `src/features/clipboard_history/mod.rs`: export the store module.
- Modify `src/features/clipboard_history/clipboard.rs`: return captured images as transient bytes, and paste persisted images by loading from disk.
- Modify `src/features/clipboard_history/window.rs`: render previews from persisted image files and reuse persisted image paths when opening image previews.
- Modify `src/infrastructure/clipboard_listener.rs`: persist captured items before inserting them, save the index after changes, and delete evicted image files.
- Create `src/infrastructure/idle_model.rs`: small testable idle-deadline helper.
- Modify `src/features/text_translation/translator.rs`: do not load the model in `apply_settings`; load on call; refresh one-minute idle deadline; unload on cleanup or disable.
- Modify `src/features/file_preview/ocr.rs`: same lazy/idle lifecycle for OCR.
- Modify `src/app.rs`: add UI-thread window registry, conditional startup initialization, clipboard-listener start-on-enable, and model cleanup timer.
- Modify `src/features/home/window.rs`: open feature windows through UI-thread ensure callbacks instead of requiring all windows to exist at startup.
- Modify `src/features/settings/window.rs`: accept and call an `on_settings_applied` hook after saving settings.
- Modify `src/infrastructure/tray.rs`: accept and call an `on_settings_applied` hook after tray setting changes.
- Modify `src/features/screenshot/window.rs`: add undo count and byte trimming helpers and clear UI image state on finish.
- Modify `src/features/file_preview/window.rs`: clear image/OCR state on close or reset.

---

### Task 1: Application Storage Paths

**Files:**
- Create: `src/infrastructure/paths.rs`
- Modify: `src/infrastructure/mod.rs`
- Modify: `Cargo.toml`

- [ ] **Step 1: Add dev dependency for filesystem tests**

Add this section to `Cargo.toml` after dependencies:

```toml
[dev-dependencies]
tempfile = "3"
```

- [ ] **Step 2: Write path helper tests**

Create `src/infrastructure/paths.rs` with the tests first:

```rust
//! Runtime filesystem paths for app-owned temporary data.

use std::path::{Path, PathBuf};

pub fn application_root() -> PathBuf {
    application_root_from_exe_path(std::env::current_exe().ok())
}

pub fn application_root_from_exe_path(exe_path: Option<PathBuf>) -> PathBuf {
    exe_path
        .and_then(|path| path.parent().map(Path::to_path_buf))
        .unwrap_or_else(|| std::env::current_dir().unwrap_or_else(|_| PathBuf::from(".")))
}

pub fn clipboard_history_dir() -> PathBuf {
    clipboard_history_dir_from_root(&application_root())
}

pub fn clipboard_history_dir_from_root(root: &Path) -> PathBuf {
    root.join("temp").join("screenshot")
}

#[cfg(test)]
mod tests {
    use super::{application_root_from_exe_path, clipboard_history_dir_from_root};
    use std::path::PathBuf;

    #[test]
    fn resolves_application_root_from_executable_parent() {
        let exe = PathBuf::from(r"C:\tools\easy-tool\easy-tool.exe");

        assert_eq!(
            application_root_from_exe_path(Some(exe)),
            PathBuf::from(r"C:\tools\easy-tool")
        );
    }

    #[test]
    fn clipboard_history_dir_is_under_temp_screenshot() {
        let root = PathBuf::from(r"C:\tools\easy-tool");

        assert_eq!(
            clipboard_history_dir_from_root(&root),
            PathBuf::from(r"C:\tools\easy-tool\temp\screenshot")
        );
    }
}
```

- [ ] **Step 3: Export the paths module**

Add this line to `src/infrastructure/mod.rs`:

```rust
pub mod paths;
```

- [ ] **Step 4: Run tests and confirm they pass**

Run:

```powershell
cargo test infrastructure::paths
```

Expected: path helper tests pass.

- [ ] **Step 5: Commit**

```powershell
git add Cargo.toml src/infrastructure/mod.rs src/infrastructure/paths.rs
git commit -m "feat: add application storage paths"
```

---

### Task 2: Persistent Clipboard History Model And Store

**Files:**
- Modify: `src/features/clipboard_history/history.rs`
- Create: `src/features/clipboard_history/store.rs`
- Modify: `src/features/clipboard_history/mod.rs`

- [ ] **Step 1: Replace image history bytes with image references**

Update `ClipboardHistoryItem` in `src/features/clipboard_history/history.rs`:

```rust
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub enum ClipboardHistoryItem {
    Text {
        text: String,
    },
    Image {
        width: usize,
        height: usize,
        path: PathBuf,
        byte_len: u64,
    },
    Files {
        paths: Vec<PathBuf>,
    },
}
```

Add helpers in the same file:

```rust
impl ClipboardHistory {
    pub fn push(&mut self, item: ClipboardHistoryItem) -> Vec<PathBuf> {
        if self
            .items
            .front()
            .is_some_and(|latest| latest.same_content(&item))
        {
            return Vec::new();
        }

        self.items.push_front(item);
        self.trim_to_limit()
    }

    pub fn retain_valid_images(&mut self) -> Vec<PathBuf> {
        let mut removed = Vec::new();
        self.items.retain(|item| {
            if let ClipboardHistoryItem::Image { path, .. } = item
                && !path.exists()
            {
                removed.push(path.clone());
                return false;
            }
            true
        });
        removed
    }

    pub fn trim_to_limit(&mut self) -> Vec<PathBuf> {
        let mut removed = Vec::new();
        while self.items.len() > MAX_HISTORY_ITEMS {
            if let Some(item) = self.items.pop_back()
                && let Some(path) = item.image_path()
            {
                removed.push(path.to_path_buf());
            }
        }
        removed
    }
}

impl ClipboardHistoryItem {
    pub fn image_path(&self) -> Option<&Path> {
        match self {
            Self::Image { path, .. } => Some(path.as_path()),
            _ => None,
        }
    }

    pub fn image_data(&self) -> Option<ImageData<'static>> {
        let Self::Image {
            width,
            height,
            path,
            ..
        } = self
        else {
            return None;
        };

        let image = image::open(path).ok()?.into_rgba8();
        Some(ImageData {
            width: *width,
            height: *height,
            bytes: image.into_raw().into(),
        })
    }
}
```

Update `detail`, `full_text`, and `same_content` to use image metadata and paths:

```rust
Self::Image {
    width,
    height,
    byte_len,
    ..
} => {
    format!(
        "RGBA 鍥剧墖锛寋} KB锛屽師濮嬪昂瀵?{width} x {height}",
        byte_len / 1024
    )
}
```

For image equality, compare width, height, byte length, and path:

```rust
(
    Self::Image {
        width: left_width,
        height: left_height,
        path: left_path,
        byte_len: left_byte_len,
    },
    Self::Image {
        width: right_width,
        height: right_height,
        path: right_path,
        byte_len: right_byte_len,
    },
) => {
    left_width == right_width
        && left_height == right_height
        && left_byte_len == right_byte_len
        && left_path == right_path
}
```

- [ ] **Step 2: Create clipboard history store tests**

Create `src/features/clipboard_history/store.rs`:

```rust
//! Disk-backed clipboard history storage.

use std::path::{Path, PathBuf};

use arboard::ImageData;
use serde::{Deserialize, Serialize};

use crate::features::clipboard_history::history::{ClipboardHistory, ClipboardHistoryItem};

const HISTORY_INDEX_FILE: &str = "clipboard_history.json";

#[derive(Debug)]
pub enum CapturedClipboardItem {
    Text { text: String },
    Image { width: usize, height: usize, bytes: Vec<u8> },
    Files { paths: Vec<PathBuf> },
}

#[derive(Debug, Serialize, Deserialize)]
struct PersistedClipboardHistory {
    entries: Vec<ClipboardHistoryItem>,
}

pub fn history_index_path(dir: &Path) -> PathBuf {
    dir.join(HISTORY_INDEX_FILE)
}

#[cfg(test)]
mod tests {
    use super::{
        CapturedClipboardItem, delete_image_files, history_index_path, item_from_capture,
        load_history, save_history,
    };
    use crate::features::clipboard_history::history::{ClipboardHistory, ClipboardHistoryItem};
    use tempfile::tempdir;

    #[test]
    fn missing_index_loads_empty_history() {
        let dir = tempdir().unwrap();

        let history = load_history(dir.path()).unwrap();

        assert_eq!(history.items().len(), 0);
    }

    #[test]
    fn persists_and_reloads_text_entries() {
        let dir = tempdir().unwrap();
        let mut history = ClipboardHistory::default();
        history.push(ClipboardHistoryItem::Text {
            text: "hello".to_string(),
        });

        save_history(dir.path(), &history).unwrap();
        let loaded = load_history(dir.path()).unwrap();

        assert!(matches!(
            loaded.get(0),
            Some(ClipboardHistoryItem::Text { text }) if text == "hello"
        ));
    }

    #[test]
    fn persists_image_capture_as_png_reference() {
        let dir = tempdir().unwrap();
        let item = item_from_capture(
            dir.path(),
            CapturedClipboardItem::Image {
                width: 1,
                height: 1,
                bytes: vec![255, 0, 0, 255],
            },
        )
        .unwrap();

        let ClipboardHistoryItem::Image {
            width,
            height,
            path,
            byte_len,
        } = item
        else {
            panic!("expected image item");
        };

        assert_eq!(width, 1);
        assert_eq!(height, 1);
        assert!(byte_len > 0);
        assert!(path.exists());
        assert_eq!(path.extension().and_then(|value| value.to_str()), Some("png"));
    }

    #[test]
    fn load_skips_missing_image_files_and_rewrites_index() {
        let dir = tempdir().unwrap();
        let missing = dir.path().join("missing.png");
        let mut history = ClipboardHistory::default();
        history.push(ClipboardHistoryItem::Image {
            width: 1,
            height: 1,
            path: missing,
            byte_len: 4,
        });
        save_history(dir.path(), &history).unwrap();

        let loaded = load_history(dir.path()).unwrap();

        assert_eq!(loaded.items().len(), 0);
        assert!(history_index_path(dir.path()).exists());
    }

    #[test]
    fn delete_image_files_removes_existing_files() {
        let dir = tempdir().unwrap();
        let path = dir.path().join("image.png");
        std::fs::write(&path, b"png").unwrap();

        delete_image_files(vec![path.clone()]);

        assert!(!path.exists());
    }
}
```

- [ ] **Step 3: Implement store functions**

Add the production functions above the tests in `src/features/clipboard_history/store.rs`:

```rust
pub fn load_history(dir: &Path) -> Result<ClipboardHistory, String> {
    let path = history_index_path(dir);
    if !path.exists() {
        return Ok(ClipboardHistory::default());
    }

    let content =
        std::fs::read_to_string(&path).map_err(|err| format!("read history failed: {err}"))?;
    let persisted = serde_json::from_str::<PersistedClipboardHistory>(&content)
        .map_err(|err| format!("parse history failed: {err}"))?;

    let mut history = ClipboardHistory::default();
    for entry in persisted.entries {
        history.push(entry);
    }
    history.retain_valid_images();
    history.trim_to_limit();
    save_history(dir, &history)?;
    Ok(history)
}

pub fn save_history(dir: &Path, history: &ClipboardHistory) -> Result<(), String> {
    std::fs::create_dir_all(dir).map_err(|err| format!("create history dir failed: {err}"))?;
    let persisted = PersistedClipboardHistory {
        entries: history.items(),
    };
    let content = serde_json::to_string_pretty(&persisted)
        .map_err(|err| format!("serialize history failed: {err}"))?;
    std::fs::write(history_index_path(dir), content)
        .map_err(|err| format!("write history failed: {err}"))
}

pub fn item_from_capture(
    dir: &Path,
    captured: CapturedClipboardItem,
) -> Result<ClipboardHistoryItem, String> {
    match captured {
        CapturedClipboardItem::Text { text } => Ok(ClipboardHistoryItem::Text { text }),
        CapturedClipboardItem::Files { paths } => Ok(ClipboardHistoryItem::Files { paths }),
        CapturedClipboardItem::Image {
            width,
            height,
            bytes,
        } => persist_image_capture(dir, width, height, bytes),
    }
}

fn persist_image_capture(
    dir: &Path,
    width: usize,
    height: usize,
    bytes: Vec<u8>,
) -> Result<ClipboardHistoryItem, String> {
    std::fs::create_dir_all(dir).map_err(|err| format!("create history dir failed: {err}"))?;
    let image = image::RgbaImage::from_raw(width as u32, height as u32, bytes)
        .ok_or_else(|| "invalid clipboard image bytes".to_string())?;
    let path = next_image_path(dir)?;
    image
        .save(&path)
        .map_err(|err| format!("save clipboard image failed: {err}"))?;
    let byte_len = std::fs::metadata(&path)
        .map_err(|err| format!("read clipboard image metadata failed: {err}"))?
        .len();

    Ok(ClipboardHistoryItem::Image {
        width,
        height,
        path,
        byte_len,
    })
}

fn next_image_path(dir: &Path) -> Result<PathBuf, String> {
    let timestamp = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_err(|err| format!("read system time failed: {err}"))?
        .as_millis();
    for counter in 0..1000u16 {
        let path = dir.join(format!("clipboard-image-{timestamp}-{counter}.png"));
        if !path.exists() {
            return Ok(path);
        }
    }
    Err("could not allocate clipboard image path".into())
}

pub fn delete_image_files(paths: impl IntoIterator<Item = PathBuf>) {
    for path in paths {
        if let Err(err) = std::fs::remove_file(&path)
            && err.kind() != std::io::ErrorKind::NotFound
        {
            log::error!("delete clipboard image failed: {err}");
        }
    }
}
```

- [ ] **Step 4: Export the store module**

Add this line to `src/features/clipboard_history/mod.rs`:

```rust
pub mod store;
```

- [ ] **Step 5: Run clipboard history tests**

Run:

```powershell
cargo test features::clipboard_history
```

Expected: all clipboard history unit tests pass.

- [ ] **Step 6: Commit**

```powershell
git add Cargo.toml src/features/clipboard_history/history.rs src/features/clipboard_history/mod.rs src/features/clipboard_history/store.rs
git commit -m "feat: persist clipboard history metadata"
```

---

### Task 3: Wire Clipboard Capture, Paste, Window Refresh, And Listener Persistence

**Files:**
- Modify: `src/features/clipboard_history/clipboard.rs`
- Modify: `src/features/clipboard_history/window.rs`
- Modify: `src/infrastructure/clipboard_listener.rs`
- Modify: `src/app.rs`

- [ ] **Step 1: Change clipboard capture to return transient captured items**

In `src/features/clipboard_history/clipboard.rs`, change the import and capture signature:

```rust
use crate::features::clipboard_history::history::ClipboardHistoryItem;
use crate::features::clipboard_history::store::CapturedClipboardItem;

pub fn capture_clipboard_item() -> Option<CapturedClipboardItem> {
    if let Some(paths) = platform::get_clipboard_files() {
        if !paths.is_empty() {
            return Some(CapturedClipboardItem::Files { paths });
        }
    }

    if let Ok(mut clipboard) = Clipboard::new() {
        if let Ok(image) = clipboard.get_image() {
            log::info!(
                "captured image clipboard item: {}x{}, {} bytes",
                image.width,
                image.height,
                image.bytes.len()
            );
            return Some(CapturedClipboardItem::Image {
                width: image.width,
                height: image.height,
                bytes: image.bytes.into_owned(),
            });
        }

        if let Ok(text) = clipboard.get_text() {
            let text = text.trim().to_string();
            if !text.is_empty() {
                return Some(CapturedClipboardItem::Text { text });
            }
        }
    }

    None
}
```

- [ ] **Step 2: Change image paste to load persisted image bytes**

Update the image branch of `put_clipboard_item`:

```rust
ClipboardHistoryItem::Image {
    width,
    height,
    path,
    ..
} => {
    let image = image::open(path)
        .map_err(|err| format!("read clipboard image failed: {err}"))?
        .into_rgba8();
    Clipboard::new()
        .map_err(|err| format!("open clipboard failed: {err}"))?
        .set_image(ImageData {
            width: *width,
            height: *height,
            bytes: image.into_raw().into(),
        })
        .map_err(|err| format!("set image failed: {err}"))
}
```

- [ ] **Step 3: Update history window image preview and open behavior**

In `src/features/clipboard_history/window.rs`, remove `save_history_image_preview` and change image open behavior:

```rust
ClipboardHistoryItem::Image { path, .. } => Ok(Some(path.clone())),
```

Keep `preview_from_item` using `item.image_data()` so it loads from disk only during refresh.

- [ ] **Step 4: Persist captured items in clipboard listener**

Update `start_clipboard_history_listener` and `HistoryClipboardHandler` to accept `history_dir: PathBuf`.

```rust
pub fn start_clipboard_history_listener(
    history: Arc<Mutex<ClipboardHistory>>,
    settings: Arc<Mutex<AppSettings>>,
    history_window: slint::Weak<ClipboardHistoryWindow>,
    suppress_next_clipboard_history: Arc<Mutex<Option<ClipboardHistoryItem>>>,
    history_dir: std::path::PathBuf,
) -> Result<(), String> {
```

In `on_clipboard_change`, persist before pushing:

```rust
let Some(captured) = capture_clipboard_item() else {
    return CallbackResult::Next;
};

let item = match crate::features::clipboard_history::store::item_from_capture(
    &self.history_dir,
    captured,
) {
    Ok(item) => item,
    Err(err) => {
        log::error!("persist clipboard history item failed: {err}");
        return CallbackResult::Next;
    }
};

if self
    .suppress_next_clipboard_history
    .lock()
    .unwrap()
    .take()
    .is_some_and(|expected| expected.same_content(&item))
{
    return CallbackResult::Next;
}

let evicted = {
    let mut history = self.history.lock().unwrap();
    let evicted = history.push(item);
    if let Err(err) = crate::features::clipboard_history::store::save_history(
        &self.history_dir,
        &history,
    ) {
        log::error!("save clipboard history failed: {err}");
    }
    evicted
};
crate::features::clipboard_history::store::delete_image_files(evicted);
```

- [ ] **Step 5: Load persisted history and pass storage dir from app startup**

In `src/app.rs`, replace default history creation:

```rust
let clipboard_history_dir = crate::infrastructure::paths::clipboard_history_dir();
let clipboard_history = Arc::new(Mutex::new(
    crate::features::clipboard_history::store::load_history(&clipboard_history_dir)
        .unwrap_or_else(|err| {
            log::error!("load clipboard history failed: {err}");
            ClipboardHistory::default()
        }),
));
```

Pass `clipboard_history_dir.clone()` into `start_clipboard_history_listener`.

- [ ] **Step 6: Run checks**

Run:

```powershell
cargo test features::clipboard_history
cargo check
```

Expected: tests and check pass.

- [ ] **Step 7: Commit**

```powershell
git add src/app.rs src/features/clipboard_history/clipboard.rs src/features/clipboard_history/window.rs src/infrastructure/clipboard_listener.rs
git commit -m "feat: store clipboard images on disk"
```

---

### Task 4: Lazy Model Loading And One-Minute Idle Unload

**Files:**
- Create: `src/infrastructure/idle_model.rs`
- Modify: `src/infrastructure/mod.rs`
- Modify: `src/features/text_translation/translator.rs`
- Modify: `src/features/file_preview/ocr.rs`
- Modify: `src/app.rs`

- [ ] **Step 1: Create a testable idle model helper**

Create `src/infrastructure/idle_model.rs`:

```rust
//! Idle-deadline tracking for heavyweight model state.

use std::time::{Duration, Instant};

pub const MODEL_IDLE_TTL: Duration = Duration::from_secs(60);

#[derive(Debug)]
pub struct IdleModel<T> {
    model: Option<T>,
    idle_deadline: Option<Instant>,
}

impl<T> IdleModel<T> {
    pub fn empty() -> Self {
        Self {
            model: None,
            idle_deadline: None,
        }
    }

    pub fn get_or_try_load(
        &mut self,
        loader: impl FnOnce() -> Result<T, String>,
    ) -> Result<&mut T, String> {
        if self.model.is_none() {
            self.model = Some(loader()?);
        }
        self.model
            .as_mut()
            .ok_or_else(|| "model is not loaded".to_string())
    }

    pub fn refresh_idle_deadline(&mut self, now: Instant) {
        if self.model.is_some() {
            self.idle_deadline = Some(now + MODEL_IDLE_TTL);
        }
    }

    pub fn unload_if_idle(&mut self, now: Instant) -> bool {
        if self
            .idle_deadline
            .is_some_and(|deadline| now >= deadline)
        {
            self.model = None;
            self.idle_deadline = None;
            return true;
        }
        false
    }

    pub fn unload_now(&mut self) {
        self.model = None;
        self.idle_deadline = None;
    }

    pub fn is_loaded(&self) -> bool {
        self.model.is_some()
    }
}

#[cfg(test)]
mod tests {
    use super::{IdleModel, MODEL_IDLE_TTL};
    use std::time::Instant;

    #[test]
    fn starts_without_loaded_model() {
        let model = IdleModel::<u8>::empty();

        assert!(!model.is_loaded());
    }

    #[test]
    fn loads_on_first_access() {
        let mut model = IdleModel::<u8>::empty();

        let value = model.get_or_try_load(|| Ok(7)).unwrap();

        assert_eq!(*value, 7);
        assert!(model.is_loaded());
    }

    #[test]
    fn refreshes_deadline_and_unloads_after_idle_expiry() {
        let mut model = IdleModel::<u8>::empty();
        model.get_or_try_load(|| Ok(7)).unwrap();
        let now = Instant::now();

        model.refresh_idle_deadline(now);

        assert!(!model.unload_if_idle(now + MODEL_IDLE_TTL - std::time::Duration::from_millis(1)));
        assert!(model.unload_if_idle(now + MODEL_IDLE_TTL));
        assert!(!model.is_loaded());
    }

    #[test]
    fn unload_now_drops_loaded_model() {
        let mut model = IdleModel::<u8>::empty();
        model.get_or_try_load(|| Ok(7)).unwrap();

        model.unload_now();

        assert!(!model.is_loaded());
    }
}
```

Export it from `src/infrastructure/mod.rs`:

```rust
pub mod idle_model;
```

- [ ] **Step 2: Refactor translation service state**

In `src/features/text_translation/translator.rs`, import the helper:

```rust
use crate::infrastructure::idle_model::IdleModel;
```

Change `TranslationState`:

```rust
struct TranslationState {
    enabled: bool,
    model_path: PathBuf,
    model: IdleModel<TranslationModel>,
}
```

In `new`, initialize `model: IdleModel::empty()`.

In `apply_settings`, unload on path change or disabled state, and do not load:

```rust
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
```

In `translate_streaming_cancellable`, load lazily and refresh the deadline after the call:

```rust
let model_path = state.model_path.clone();
let result = {
    let model = state
        .model
        .get_or_try_load(|| TranslationModel::load(&model_path))?;
    model.translate_streaming(text, on_partial, should_cancel)
};
state
    .model
    .refresh_idle_deadline(std::time::Instant::now());
result
```

Add public cleanup:

```rust
pub fn unload_if_idle(&self) {
    let mut state = self.state.lock().unwrap();
    if state.model.unload_if_idle(std::time::Instant::now()) {
        log::info!("unloaded idle translation model");
    }
}
```

- [ ] **Step 3: Refactor OCR service state**

Apply the same structure in `src/features/file_preview/ocr.rs`:

```rust
use crate::infrastructure::idle_model::IdleModel;

struct OcrState {
    enabled: bool,
    model_path: Option<PathBuf>,
    model: IdleModel<OcrModel>,
}
```

Use `IdleModel::empty()` in `new`, call `state.model.unload_now()` when disabled or when `model_path` changes, and lazy-load inside `recognize_streaming`:

```rust
let model_path = state
    .model_path
    .clone()
    .ok_or_else(|| "璇峰厛鍦ㄨ缃〉閰嶇疆 OCR 妯″瀷鐩綍".to_string())?;
let result = {
    let model = state.model.get_or_try_load(|| OcrModel::load(&model_path))?;
    model.recognize_streaming(image_path, on_partial)
};
state
    .model
    .refresh_idle_deadline(std::time::Instant::now());
result
```

Add:

```rust
pub fn unload_if_idle(&self) {
    let mut state = self.state.lock().unwrap();
    if state.model.unload_if_idle(std::time::Instant::now()) {
        log::info!("unloaded idle OCR model");
    }
}
```

- [ ] **Step 4: Add cleanup timer in app**

In `src/app.rs`, after services are created:

```rust
let model_cleanup_timer = slint::Timer::default();
model_cleanup_timer.start(
    slint::TimerMode::Repeated,
    Duration::from_secs(5),
    {
        let translation_service = Arc::clone(&translation_service);
        let ocr_service = Arc::clone(&ocr_service);
        move || {
            translation_service.unload_if_idle();
            ocr_service.unload_if_idle();
        }
    },
);
```

Keep the timer alive next to `_tray_timer`:

```rust
let _model_cleanup_timer = model_cleanup_timer;
```

- [ ] **Step 5: Run idle model and existing translator tests**

Run:

```powershell
cargo test infrastructure::idle_model
cargo test features::text_translation::translator
cargo check
```

Expected: tests and check pass.

- [ ] **Step 6: Commit**

```powershell
git add src/app.rs src/infrastructure/mod.rs src/infrastructure/idle_model.rs src/features/text_translation/translator.rs src/features/file_preview/ocr.rs
git commit -m "feat: unload idle OCR and translation models"
```

---

### Task 5: Conditional Window Initialization By Settings

**Files:**
- Modify: `src/app.rs`
- Modify: `src/features/home/window.rs`
- Modify: `src/features/settings/window.rs`
- Modify: `src/infrastructure/tray.rs`
- Modify: `src/infrastructure/clipboard_listener.rs`

- [ ] **Step 1: Add UI-thread app window registry**

In `src/app.rs`, add a struct near `ShortcutState`:

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
```

Store it as `Rc<RefCell<AppWindows>>` inside `run`, not inside any background thread:

```rust
let windows = std::rc::Rc::new(std::cell::RefCell::new(AppWindows::new()));
```

- [ ] **Step 2: Add shared weak slots for background shortcuts**

In `src/app.rs`, create weak slots that background input can read:

```rust
#[derive(Clone)]
struct ShortcutWindows {
    history: Arc<Mutex<Option<slint::Weak<crate::ClipboardHistoryWindow>>>>,
    screenshot: Arc<Mutex<Option<slint::Weak<crate::ScreenshotWindow>>>>,
    translation: Arc<Mutex<Option<slint::Weak<crate::TextTranslationWindow>>>>,
    time_trans: Arc<Mutex<Option<slint::Weak<crate::TimeTrans>>>>,
}

impl ShortcutWindows {
    fn new() -> Self {
        Self {
            history: Arc::new(Mutex::new(None)),
            screenshot: Arc::new(Mutex::new(None)),
            translation: Arc::new(Mutex::new(None)),
            time_trans: Arc::new(Mutex::new(None)),
        }
    }
}
```

Whenever a window is created on the UI thread, write its weak handle into the matching slot.

- [ ] **Step 3: Add ensure functions in app**

Add UI-thread helper functions in `src/app.rs`:

```rust
fn ensure_clipboard_history_window(
    windows: &std::rc::Rc<std::cell::RefCell<AppWindows>>,
    shortcut_windows: &ShortcutWindows,
    clipboard_history: Arc<Mutex<ClipboardHistory>>,
    suppress_shortcuts: Arc<AtomicBool>,
    suppress_next_clipboard_history: Arc<Mutex<Option<crate::features::clipboard_history::history::ClipboardHistoryItem>>>,
) -> slint::Weak<crate::ClipboardHistoryWindow> {
    let mut windows = windows.borrow_mut();
    if windows.clipboard_history.is_none() {
        let window = init_clipboard_history_window(
            clipboard_history,
            suppress_shortcuts,
            suppress_next_clipboard_history,
        );
        *shortcut_windows.history.lock().unwrap() = Some(window.as_weak());
        windows.clipboard_history = Some(window);
    }
    windows.clipboard_history.as_ref().unwrap().as_weak()
}
```

Add these helper functions in `src/app.rs` next to `ensure_clipboard_history_window`:

```rust
fn ensure_time_trans_window(
    windows: &std::rc::Rc<std::cell::RefCell<AppWindows>>,
    shortcut_windows: &ShortcutWindows,
) -> slint::Weak<crate::TimeTrans> {
    let mut windows = windows.borrow_mut();
    if windows.time_trans.is_none() {
        let window = init_time_trans_window();
        *shortcut_windows.time_trans.lock().unwrap() = Some(window.as_weak());
        windows.time_trans = Some(window);
    }
    windows.time_trans.as_ref().unwrap().as_weak()
}

fn ensure_text_translation_window(
    windows: &std::rc::Rc<std::cell::RefCell<AppWindows>>,
    shortcut_windows: &ShortcutWindows,
    translation_cancel_generation: Arc<AtomicU64>,
    settings: Arc<Mutex<AppSettings>>,
    translation_service: Arc<TranslationService>,
) -> slint::Weak<crate::TextTranslationWindow> {
    let mut windows = windows.borrow_mut();
    if windows.text_translation.is_none() {
        let window = init_text_translation_window(
            translation_cancel_generation,
            settings,
            translation_service,
        );
        *shortcut_windows.translation.lock().unwrap() = Some(window.as_weak());
        windows.text_translation = Some(window);
    }
    windows.text_translation.as_ref().unwrap().as_weak()
}

fn ensure_file_preview_window(
    windows: &std::rc::Rc<std::cell::RefCell<AppWindows>>,
    ocr_service: Arc<OcrService>,
) -> slint::Weak<crate::FilePreviewWindow> {
    let mut windows = windows.borrow_mut();
    if windows.file_preview.is_none() {
        windows.file_preview = Some(init_file_preview_window(false, ocr_service));
    }
    windows.file_preview.as_ref().unwrap().as_weak()
}

fn ensure_screenshot_window(
    windows: &std::rc::Rc<std::cell::RefCell<AppWindows>>,
    shortcut_windows: &ShortcutWindows,
) -> slint::Weak<crate::ScreenshotWindow> {
    let mut windows = windows.borrow_mut();
    if windows.screenshot.is_none() {
        let window = init_screenshot_window();
        *shortcut_windows.screenshot.lock().unwrap() = Some(window.as_weak());
        windows.screenshot = Some(window);
    }
    windows.screenshot.as_ref().unwrap().as_weak()
}

fn ensure_settings_window(
    windows: &std::rc::Rc<std::cell::RefCell<AppWindows>>,
    settings: Arc<Mutex<AppSettings>>,
    settings_store: SettingsStore,
    translation_service: Arc<TranslationService>,
    ocr_service: Arc<OcrService>,
    tray_menu_handles: crate::infrastructure::tray::TrayMenuHandles,
    on_settings_applied: std::rc::Rc<dyn Fn(&AppSettings)>,
) -> slint::Weak<crate::SettingsWindow> {
    let mut windows = windows.borrow_mut();
    if windows.settings.is_none() {
        windows.settings = Some(init_settings_window(
            settings,
            settings_store,
            translation_service,
            ocr_service,
            tray_menu_handles,
            on_settings_applied,
        ));
    }
    windows.settings.as_ref().unwrap().as_weak()
}
```

- [ ] **Step 4: Update startup preloading according to settings**

After loading settings in `src/app.rs`, create only enabled windows:

```rust
let settings_snapshot = settings.lock().unwrap().clone();
if settings_snapshot.copy_timestamp.enabled {
    ensure_time_trans_window(&windows, &shortcut_windows);
}
if settings_snapshot.clipboard_history.enabled {
    ensure_clipboard_history_window(
        &windows,
        &shortcut_windows,
        Arc::clone(&clipboard_history),
        Arc::clone(&suppress_shortcuts),
        Arc::clone(&suppress_next_clipboard_history),
    );
}
if settings_snapshot.screenshot.enabled {
    ensure_screenshot_window(&windows, &shortcut_windows);
}
if settings_snapshot.text_translation.enabled {
    ensure_text_translation_window(
        &windows,
        &shortcut_windows,
        Arc::clone(&translation_cancel_generation),
        Arc::clone(&settings),
        Arc::clone(&translation_service),
    );
}
```

Do not create `FilePreviewWindow` or `SettingsWindow` at startup unless the standalone file-preview argument path is used.

- [ ] **Step 5: Make home callbacks create windows lazily**

Change `init_home_window` in `src/features/home/window.rs` to accept closures:

```rust
pub fn init_home_window(
    open_time_trans: impl Fn() + 'static,
    open_clipboard_history: impl Fn() + 'static,
    open_text_translation: impl Fn() + 'static,
    open_file_preview: impl Fn() + 'static,
    open_screenshot: impl Fn() + 'static,
    open_settings: impl Fn() + 'static,
) -> HomeWindow {
    let window = HomeWindow::new().unwrap();
    window
        .window()
        .on_close_requested(|| CloseRequestResponse::HideWindow);
    window.on_open_time_trans(open_time_trans);
    window.on_open_clipboard_history(open_clipboard_history);
    window.on_open_text_translation(open_text_translation);
    window.on_open_file_preview(open_file_preview);
    window.on_open_screenshot(open_screenshot);
    window.on_open_settings(open_settings);
    window
}
```

In `src/app.rs`, pass closures that call the ensure helpers and then show/activate the windows on the UI thread.

- [ ] **Step 6: Make global input read weak slots**

Replace direct captured weak handles in `start_global_input_listener` with reads from `shortcut_windows`.

For screenshot:

```rust
if let Some(weak_screenshot_window) = shortcut_windows
    .screenshot
    .lock()
    .unwrap()
    .as_ref()
    .cloned()
{
    let _ = weak_screenshot_window.upgrade_in_event_loop(move |window| {
        if window.window().is_visible() {
            cancel_screenshot_window(&window);
        }
    });
}
```

For shortcuts where the feature is enabled but the weak slot is empty, use `slint::invoke_from_event_loop` to create the window via an app callback. Do not create Slint windows directly inside the global input thread.

- [ ] **Step 7: Start clipboard listener only when enabled**

Add an `Arc<std::sync::atomic::AtomicBool>` named `clipboard_listener_started`.

Create a helper in `src/app.rs`:

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

Call it at startup only when `clipboard_history.enabled` is true, and call it from runtime settings/tray hooks when the feature changes from disabled to enabled.

- [ ] **Step 8: Add settings-applied hooks**

In `src/features/settings/window.rs`, add a parameter to `init_settings_window`:

```rust
on_settings_applied: std::rc::Rc<dyn Fn(&AppSettings)>,
```

Call it at the end of `apply_settings_snapshot` after services are updated:

```rust
on_settings_applied(&updated_settings);
```

In `src/infrastructure/tray.rs`, add a parameter to `start_tray_event_pump`:

```rust
on_settings_applied: impl Fn(&AppSettings) + 'static,
```

After each successful save and service setting application, call:

```rust
on_settings_applied(&settings);
```

The hook runs on the Slint timer/UI thread and can safely call window ensure helpers.

- [ ] **Step 9: Run checks**

Run:

```powershell
cargo check
cargo test
```

Expected: project compiles and existing tests pass.

- [ ] **Step 10: Commit**

```powershell
git add src/app.rs src/features/home/window.rs src/features/settings/window.rs src/infrastructure/tray.rs src/infrastructure/clipboard_listener.rs
git commit -m "feat: initialize feature windows from settings"
```

---

### Task 6: Screenshot Undo Limits And File Preview Cleanup

**Files:**
- Modify: `src/features/screenshot/window.rs`
- Modify: `src/features/file_preview/window.rs`

- [ ] **Step 1: Add screenshot undo trimming tests**

In `src/features/screenshot/window.rs`, add helper functions near `push_undo_snapshot`:

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

Add tests at the bottom of the file:

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

- [ ] **Step 2: Use trimming in push_undo_snapshot**

Replace `push_undo_snapshot`:

```rust
fn push_undo_snapshot(session: &mut ScreenshotSession) {
    session.undo_stack.push(session.image.clone());
    trim_undo_stack(&mut session.undo_stack);
}
```

- [ ] **Step 3: Clear screenshot UI image state when finishing**

In `finish_screenshot_window`, before hiding:

```rust
window.set_screenshot(Image::default());
window.set_text_preview(Image::default());
window.set_has_text_preview(false);
window.set_brush_segments(ModelRc::from(Rc::new(VecModel::from(Vec::<BrushSegment>::new()))));
```

- [ ] **Step 4: Add file preview clear helper**

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

Use it inside `show_empty_file_preview_window` and `close_preview_window` before hiding non-standalone windows.

- [ ] **Step 5: Run tests and checks**

Run:

```powershell
cargo test features::screenshot
cargo check
```

Expected: screenshot tests pass and project compiles.

- [ ] **Step 6: Commit**

```powershell
git add src/features/screenshot/window.rs src/features/file_preview/window.rs
git commit -m "feat: bound screenshot and preview memory"
```

---

### Task 7: Full Verification And Manual Memory Checks

**Files:**
- Modify only if a verification failure points to a specific file.

- [ ] **Step 1: Run full automated verification**

Run:

```powershell
cargo test
cargo check
```

Expected: all tests pass and `cargo check` finishes successfully.

- [ ] **Step 2: Run startup manual check**

Run the application:

```powershell
cargo run
```

Manual expectations:

- With translation enabled, the translation window may be initialized according to settings, but the model is not loaded until a translation call.
- With OCR enabled, the service exists, but the model is not loaded until OCR is invoked.
- Disabled feature windows are not created at startup and are created from the home window when opened.

- [ ] **Step 3: Run clipboard persistence manual check**

Manual actions:

1. Copy more than 20 images.
2. Open `<application root>\temp\screenshot`.
3. Confirm `clipboard_history.json` exists.
4. Confirm no more than 20 `clipboard-image-*.png` files remain.
5. Open clipboard history and paste one image entry.

Expected: image paste works and evicted image files are gone.

- [ ] **Step 4: Run model idle manual check**

Manual actions:

1. Trigger translation or OCR.
2. Trigger the same feature again within one minute.
3. Wait longer than one minute.
4. Trigger it again.

Expected:

- The second call within one minute reuses the loaded model.
- After one minute of idleness, logs show the idle model unload.
- The next call reloads and runs successfully.

- [ ] **Step 5: Commit verification fixes if needed**

If verification required fixes:

```powershell
git status --short
git add Cargo.toml src docs
git commit -m "fix: complete memory optimization verification"
```

If no fixes were required, do not create an empty commit.
