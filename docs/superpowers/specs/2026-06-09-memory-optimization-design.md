# Memory Optimization Design

## Goal

Reduce startup resident memory and long-running memory growth without breaking shortcut responsiveness for enabled features.

## Scope

This design covers two optimization targets:

- Startup resident memory: initialize enabled feature entry points at startup, but avoid loading large models until first use.
- Long-running memory: prevent clipboard image history, OCR, translation, and screenshot editing from keeping large data in memory indefinitely.

This design does not change UI layout, feature behavior, model accuracy, shortcut bindings, or the storage location of user settings.

## Startup Initialization

The application reads `AppSettings` during startup and uses the feature flags to decide which feature windows are created immediately.

- Enabled shortcut-style features are initialized at startup so shortcuts stay responsive.
- Disabled feature windows are created lazily when opened from the home window or settings flow.
- `HomeWindow` and tray state are always initialized at startup.
- `SettingsWindow` is created lazily because it is not needed for background feature handling.
- Slint component handles remain owned by the UI thread. Background threads must not store window handles inside `Arc<Mutex<_>>`, `Arc<OnceLock<_>>`, or similar cross-thread containers.

Feature-specific startup behavior:

- `copy_timestamp.enabled = true`: create `TimeTrans` at startup. If disabled, create it lazily when opened from the home window.
- `clipboard_history.enabled = true`: load clipboard history metadata, start the clipboard listener, and create `ClipboardHistoryWindow` at startup. If disabled, do not start the listener or create the window until the feature is enabled from settings/tray or opened manually.
- `screenshot.enabled = true`: create `ScreenshotWindow` at startup. If disabled, create the window only when opened manually or when the feature is enabled.
- `text_translation.enabled = true`: create lightweight `TranslationService` state and `TextTranslationWindow` at startup, but do not load ONNX sessions until translation is called. If disabled, keep only lightweight service state and create the window lazily.
- `image_recognition.enabled = true`: create lightweight `OcrService` state, but do not load ONNX sessions until OCR is called. `FilePreviewWindow` remains lazy unless opened directly by a file preview process argument.

When a feature is enabled at runtime from settings or tray controls, its listener/window is initialized at that time. When a feature is disabled at runtime, expensive runtime state is released, but existing hidden windows may remain allocated until the application exits unless a feature-specific clear/reset hook exists.

## Model Lifecycle

Both translation and OCR use the same lazy model lifecycle:

- Startup creates service state only.
- The first actual call loads the model and runs inference.
- Each call refreshes an idle deadline to one minute after the call.
- A lightweight cleanup timer or background maintenance thread unloads the model when the current time passes the idle deadline.
- A later call reloads the model if it was unloaded.
- Feature settings control whether calls are allowed. Enabling a feature does not load a model by itself.
- Disabling a feature immediately unloads the model by setting the service model state to `None`.

The unload behavior must be race-safe:

- Inference holds the model lock while the model is actively used.
- Cleanup checks the idle deadline while holding the same state lock.
- Cleanup does not interrupt an active inference call.
- The idle deadline is refreshed after every call attempt, whether it succeeds or returns an inference error.

## Clipboard History Persistence

Clipboard history data and clipboard images are persisted under:

`<application root>/temp/screenshot/`

The directory name is fixed by product requirement even though the stored data is clipboard history.
The application root resolves to `std::env::current_exe().parent()` at runtime. If the executable path cannot be resolved, the fallback is the current working directory.

Storage layout:

- `clipboard_history.json`: ordered metadata for the newest 20 clipboard entries.
- `clipboard-image-<timestamp>-<counter>.png`: persisted clipboard images.

In-memory history becomes a lightweight index:

- Text entries keep their text in memory and in the JSON index.
- File entries keep file paths in memory and in the JSON index.
- Image entries keep width, height, image file path, byte size, and preview metadata in memory. Full image bytes are not kept in memory after persistence.

Capture behavior:

- Text and file clipboard entries are inserted into history and saved to the JSON index.
- Image clipboard entries are written to PNG files in the fixed temp directory, then inserted as image-file references.
- The newest 20 entries are retained.
- When entries are evicted, their image files are deleted.
- If saving an image fails, the entry is not added, and the error is logged.

Startup behavior:

- Load `clipboard_history.json` if it exists.
- Keep only the newest 20 valid entries.
- For image entries, verify the referenced image file exists.
- Drop invalid image entries and rewrite the cleaned index.

Display and paste behavior:

- The clipboard history window loads image files only when it needs previews or paste data.
- Previews are generated as small Slint `Image` values and should not store the original image bytes.
- Pasting an image reads the PNG file from disk and writes it back to the system clipboard.
- Opening an image preview can reuse the persisted image path instead of writing a second temp copy.

## Screenshot Editing Memory

Screenshot editing keeps the active image in memory, but undo history is bounded by both count and total bytes.

- Maximum undo snapshots: 8.
- Maximum undo memory: 64 MiB.
- When either limit is exceeded, oldest snapshots are removed first.
- Closing or cancelling a screenshot session clears the session and undo stack.

The existing screenshot behavior remains unchanged from a user perspective.

## File Preview Memory

File preview windows clear large UI-held data when closed or reset:

- Clear `image_content`.
- Clear OCR text and status text.
- Clear stored file path and dimensions.
- Increment the OCR generation token so stale OCR callbacks cannot repopulate a closed preview.

Standalone file preview mode still quits the Slint event loop on close.

## Threading Rules

The implementation must preserve Slint's thread model:

- Slint window handles are created and stored only on the UI thread.
- Background clipboard, OCR, translation, and global input threads may hold `slint::Weak<T>` if the type permits the existing `upgrade_in_event_loop` pattern.
- Background threads may hold shared data models such as `Arc<Mutex<ClipboardHistory>>`.
- Background threads must not hold strong Slint component handles in `Arc`, `Mutex`, `OnceLock`, global statics, or persistent service structs.

## Verification

Automated checks:

- `cargo check`
- Unit tests for clipboard history persistence:
  - loads an empty or missing index as empty history
  - persists and reloads text entries
  - persists image entries as files instead of in-memory bytes
  - keeps only 20 entries
  - deletes image files for evicted entries
  - skips missing image files during load
- Unit tests for model idle lifecycle using a small fake model loader:
  - does not load at service creation
  - loads on first call
  - refreshes the one-minute idle deadline on each call
  - unloads after idle expiry
  - unloads immediately when disabled
- Unit tests for screenshot undo trimming:
  - trims by count
  - trims by total bytes

Manual checks:

- Start the application with translation and OCR enabled and confirm models are not loaded until first use.
- Use translation twice within one minute and confirm the loaded model is reused.
- Wait more than one minute after translation or OCR and confirm the model unloads.
- Copy more than 20 images and confirm only 20 image history files remain.
- Open and close file preview with a large image and confirm the window clears its image state.

## Risks

- Loading a model on first use adds latency to the first OCR or translation call.
- Persisting clipboard images as PNG trades memory for disk I/O and disk space.
- Fixed storage under the application root may fail if the application directory is read-only. If that happens, the implementation should log the failure and skip image history persistence rather than keeping large image bytes in memory.
- Existing garbled UI strings are outside this design's scope and should not be modified as part of the memory work.
