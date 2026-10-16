//! 历史窗口绑定与数据展示，持久化操作统一交给历史服务。

use super::history::ClipboardHistoryItem;
use super::service::ClipboardHistoryService;
use crate::platform::window::{activate_slint_window, show_centered_window};
use crate::{ClipboardHistoryRow, ClipboardHistoryWindow};
use rdev::{EventType, Key, simulate};
use slint::{ComponentHandle, Image, ModelRc, Rgba8Pixel, SharedPixelBuffer, VecModel};
use std::rc::Rc;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::Duration;

/// 创建窗口并绑定取消、粘贴和删除操作。
pub fn init_clipboard_history_window(
    service: Arc<ClipboardHistoryService>,
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
    let paste_service = Arc::clone(&service);
    window.on_paste(move |index| {
        if index < 0 {
            return;
        }
        match paste_service.prepare_paste(index as usize) {
            Ok(true) => {}
            Ok(false) => return,
            Err(err) => {
                log::error!("paste history item failed: {err}");
                return;
            }
        }
        if let Some(ui) = weak.upgrade() {
            let _ = ui.hide();
        }
        simulate_paste(Arc::clone(&suppress_shortcuts));
    });

    let weak = window.as_weak();
    window.on_delete_entry(move |index| {
        if index < 0 {
            return;
        }
        match service.delete(index as usize) {
            Ok(true) => {
                if let Some(ui) = weak.upgrade() {
                    refresh_with_selection(&ui, &service, index as usize);
                }
            }
            Ok(false) => {}
            Err(err) => log::error!("delete clipboard history failed: {err}"),
        }
    });
    window
}

/// 等窗口隐藏和快捷键松开后模拟粘贴；所有等待均发生在后台线程。
fn simulate_paste(suppress_shortcuts: Arc<AtomicBool>) {
    suppress_shortcuts.store(true, Ordering::SeqCst);
    std::thread::spawn(move || {
        std::thread::sleep(Duration::from_millis(120));
        let _ = simulate(&EventType::KeyPress(Key::ControlLeft));
        let _ = simulate(&EventType::KeyPress(Key::KeyV));
        let _ = simulate(&EventType::KeyRelease(Key::KeyV));
        let _ = simulate(&EventType::KeyRelease(Key::ControlLeft));
        std::thread::sleep(Duration::from_millis(200));
        suppress_shortcuts.store(false, Ordering::SeqCst);
    });
}

/// 打开窗口前刷新内容，已显示时仅激活窗口。
pub fn show_clipboard_history_window(
    window: &ClipboardHistoryWindow,
    service: &ClipboardHistoryService,
) {
    if window.window().is_visible() {
        activate_slint_window(window);
        return;
    }
    refresh_clipboard_history_window(window, service);
    let _ = window.show();
    activate_slint_window(window);
}

/// 从主页打开时刷新历史内容，并在显示前完成居中定位。
pub fn show_centered_clipboard_history_window(
    window: &ClipboardHistoryWindow,
    service: &ClipboardHistoryService,
) {
    refresh_clipboard_history_window(window, service);
    show_centered_window(
        window,
        slint::LogicalSize::new(window.get_initial_width(), window.get_initial_height()),
    );
}

/// 使用历史快照重新构建窗口列表。
pub fn refresh_clipboard_history_window(
    window: &ClipboardHistoryWindow,
    service: &ClipboardHistoryService,
) {
    refresh_with_selection(window, service, 0);
}

/// 更新列表并保留指定选择位置，先释放历史锁再解码缩略图。
fn refresh_with_selection(
    window: &ClipboardHistoryWindow,
    service: &ClipboardHistoryService,
    selected_index: usize,
) {
    let rows = service
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

/// 将历史条目转换为 Slint 列表行，并准备内容预览。
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

/// 为图片条目或单个图片文件生成窗口内缩略图。
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

/// 将剪贴板像素数据转换为可缩放的 RGBA 图像。
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

/// 按预览区域等比缩放图片，并转换为 Slint 图像缓冲。
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

/// 依据扩展名判断文件是否属于可尝试预览的图片类型。
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
