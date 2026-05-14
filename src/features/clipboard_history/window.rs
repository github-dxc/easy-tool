//! Slint window setup and presentation logic for clipboard history.

use std::path::Path;
use std::process::Command;
use std::rc::Rc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use rdev::{EventType, Key, simulate};
use slint::{ComponentHandle, Image, ModelRc, Rgba8Pixel, SharedPixelBuffer, VecModel};

use crate::features::clipboard_history::clipboard::put_clipboard_item;
use crate::features::clipboard_history::history::{ClipboardHistory, ClipboardHistoryItem};
use crate::features::file_preview::window::is_supported_preview_file;
use crate::platform::window::activate_window;
use crate::{ClipboardHistoryRow, ClipboardHistoryWindow};

/// Builds the clipboard-history window and binds actions for paste/delete/open.
pub fn init_clipboard_history_window(
    history: Arc<Mutex<ClipboardHistory>>,
    paste_target: Arc<Mutex<Option<isize>>>,
    suppress_shortcuts: Arc<AtomicBool>,
) -> ClipboardHistoryWindow {
    let window = ClipboardHistoryWindow::new().unwrap();

    let weak = window.as_weak();
    window.on_cancel(move || {
        if let Some(ui) = weak.upgrade() {
            let _ = ui.hide();
        }
    });

    let weak = window.as_weak();
    let history_for_paste = Arc::clone(&history);
    window.on_paste(move |index| {
        let item = history_for_paste.lock().unwrap().get(index as usize);
        if let Some(item) = item {
            if let Err(err) = put_clipboard_item(&item) {
                log::error!("paste history item failed: {err}");
                return;
            }
        }

        if let Some(ui) = weak.upgrade() {
            let _ = ui.hide();
        }

        let target = *paste_target.lock().unwrap();
        let suppress_shortcuts = Arc::clone(&suppress_shortcuts);
        std::thread::spawn(move || {
            suppress_shortcuts.store(true, Ordering::SeqCst);
            std::thread::sleep(Duration::from_millis(120));
            if let Some(hwnd) = target {
                activate_window(hwnd);
                std::thread::sleep(Duration::from_millis(120));
            }
            let _ = simulate(&EventType::KeyPress(Key::ControlLeft));
            let _ = simulate(&EventType::KeyPress(Key::KeyV));
            let _ = simulate(&EventType::KeyRelease(Key::KeyV));
            let _ = simulate(&EventType::KeyRelease(Key::ControlLeft));
            std::thread::sleep(Duration::from_millis(200));
            suppress_shortcuts.store(false, Ordering::SeqCst);
        });
    });

    let history_for_delete = Arc::clone(&history);
    let weak = window.as_weak();
    window.on_delete_entry(move |index| {
        if index < 0 {
            return;
        }

        let removed = history_for_delete
            .lock()
            .unwrap()
            .remove(index as usize)
            .is_some();
        if !removed {
            return;
        }

        if let Some(ui) = weak.upgrade() {
            refresh_clipboard_history_window_with_selection(
                &ui,
                &history_for_delete,
                index as usize,
            );
        }
    });

    let history_for_open = Arc::clone(&history);
    window.on_open_entry(move |index| {
        if index < 0 {
            return;
        }

        let item = history_for_open.lock().unwrap().get(index as usize);
        let Some(ClipboardHistoryItem::Files { paths }) = item else {
            return;
        };
        let Some(path) = paths.first().filter(|_| paths.len() == 1).cloned() else {
            return;
        };
        if !is_supported_preview_file(&path) {
            return;
        }

        if let Err(err) = open_standalone_file_preview(&path) {
            log::error!("open standalone file preview failed: {err}");
        }
    });

    window
}

fn open_standalone_file_preview(path: &Path) -> Result<(), String> {
    let exe_path =
        std::env::current_exe().map_err(|err| format!("get current exe failed: {err}"))?;
    Command::new(exe_path)
        .arg(path)
        .spawn()
        .map_err(|err| format!("spawn file preview failed: {err}"))?;

    Ok(())
}

/// Refreshes history rows before showing the window.
pub fn show_clipboard_history_window(
    window: &ClipboardHistoryWindow,
    history: &Arc<Mutex<ClipboardHistory>>,
) {
    refresh_clipboard_history_window(window, history);
    let _ = window.show();
}

/// Rebuilds the Slint row model from the shared history store.
pub fn refresh_clipboard_history_window(
    window: &ClipboardHistoryWindow,
    history: &Arc<Mutex<ClipboardHistory>>,
) {
    refresh_clipboard_history_window_with_selection(window, history, 0);
}

fn refresh_clipboard_history_window_with_selection(
    window: &ClipboardHistoryWindow,
    history: &Arc<Mutex<ClipboardHistory>>,
    selected_index: usize,
) {
    let rows = history
        .lock()
        .unwrap()
        .items()
        .into_iter()
        .map(row_from_item)
        .collect::<Vec<_>>();

    let selected_index = selected_index.min(rows.len().saturating_sub(1));

    if let Some(selected) = rows.get(selected_index) {
        window.set_current_kind(selected.kind.clone());
        window.set_current_detail(selected.detail.clone());
        window.set_current_full_text(selected.full_text.clone());
        window.set_current_preview(selected.preview.clone());
        window.set_current_has_preview(selected.has_preview);
        window.set_current_preview_width(selected.preview_width);
        window.set_current_preview_height(selected.preview_height);
    } else {
        window.set_current_kind("".into());
        window.set_current_detail("".into());
        window.set_current_full_text("".into());
        window.set_current_preview(Image::default());
        window.set_current_has_preview(false);
        window.set_current_preview_width(0);
        window.set_current_preview_height(0);
    }

    window.set_entries(ModelRc::from(Rc::new(VecModel::from(rows))));
    window.set_selected_index(selected_index as i32);
}

fn row_from_item(item: ClipboardHistoryItem) -> ClipboardHistoryRow {
    let preview = preview_from_item(&item);
    let has_preview = preview.is_some();
    let (preview, preview_width, preview_height) =
        preview.unwrap_or_else(|| (Image::default(), 0, 0));

    ClipboardHistoryRow {
        title: item.title().into(),
        detail: item.detail().into(),
        full_text: item.full_text().into(),
        kind: item.kind().into(),
        preview,
        has_preview,
        preview_width,
        preview_height,
    }
}

fn preview_from_item(item: &ClipboardHistoryItem) -> Option<(Image, i32, i32)> {
    if let Some(image) = item.image_data() {
        return Some(image_preview(image));
    }

    if let ClipboardHistoryItem::Files { paths } = item
        && paths.len() == 1
        && is_image_file(&paths[0])
    {
        if let Ok(image) = image::open(&paths[0]) {
            let rgba = image.into_rgba8();
            return Some(rgba_preview(&rgba));
        }
    }

    None
}

fn image_preview(image: arboard::ImageData<'static>) -> (Image, i32, i32) {
    let Some(source) = image::RgbaImage::from_raw(
        image.width as u32,
        image.height as u32,
        image.bytes.into_owned(),
    ) else {
        return (Image::default(), 0, 0);
    };

    rgba_preview(&source)
}

fn rgba_preview(source: &image::RgbaImage) -> (Image, i32, i32) {
    const MAX_PREVIEW_WIDTH: u32 = 400;
    const MAX_PREVIEW_HEIGHT: u32 = 300;

    let (source_width, source_height) = source.dimensions();
    let preview = if source_width <= MAX_PREVIEW_WIDTH && source_height <= MAX_PREVIEW_HEIGHT {
        source.clone()
    } else {
        let width_ratio = MAX_PREVIEW_WIDTH as f64 / source_width as f64;
        let height_ratio = MAX_PREVIEW_HEIGHT as f64 / source_height as f64;
        let scale = width_ratio.min(height_ratio);
        let target_width = ((source_width as f64 * scale).round() as u32).max(1);
        let target_height = ((source_height as f64 * scale).round() as u32).max(1);

        image::imageops::resize(
            source,
            target_width,
            target_height,
            image::imageops::FilterType::Lanczos3,
        )
    };

    let (width, height) = preview.dimensions();
    log::info!("created image history preview: {width}x{height}");
    let buffer = SharedPixelBuffer::<Rgba8Pixel>::clone_from_slice(preview.as_raw(), width, height);
    (Image::from_rgba8(buffer), width as i32, height as i32)
}

fn is_image_file(path: &std::path::Path) -> bool {
    path.extension()
        .and_then(|extension| extension.to_str())
        .map(|extension| {
            matches!(
                extension.to_ascii_lowercase().as_str(),
                "png" | "jpg" | "jpeg" | "bmp" | "gif" | "webp" | "tif" | "tiff"
            )
        })
        .unwrap_or(false)
}
