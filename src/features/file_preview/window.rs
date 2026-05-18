//! Slint window setup and file loading logic for the file preview feature.

use std::path::{Path, PathBuf};

use slint::{CloseRequestResponse, ComponentHandle, Image, Rgba8Pixel, SharedPixelBuffer};

use crate::FilePreviewWindow;
use crate::platform::dialog::open_file_dialog;

const MAX_IMAGE_WIDTH: u32 = 760;
const MAX_IMAGE_HEIGHT: u32 = 500;

/// Builds the preview window; standalone preview mode can quit the event loop on close.
pub fn init_file_preview_window(quit_on_close: bool) -> FilePreviewWindow {
    let window = FilePreviewWindow::new().unwrap();

    let weak = window.as_weak();
    window.on_close_preview(move || {
        close_preview_window(&weak, quit_on_close);
    });

    let weak = window.as_weak();
    window.on_open_file(move || {
        let Some(path) = open_file_dialog("Open file") else {
            return;
        };

        if let Some(ui) = weak.upgrade() {
            show_file_preview_window(&ui, path);
        }
    });

    let weak = window.as_weak();
    window.window().on_close_requested(move || {
        close_preview_window(&weak, quit_on_close);
        CloseRequestResponse::HideWindow
    });

    window
}

/// Loads a file and displays the preview window.
pub fn show_file_preview_window(window: &FilePreviewWindow, path: PathBuf) {
    load_file_preview(window, &path);
    let _ = window.show();
}

/// Opens the preview window in an empty state for launchers such as the home page.
pub fn show_empty_file_preview_window(window: &FilePreviewWindow) {
    window.set_file_path("".into());
    window.set_file_name("文件预览".into());
    window.set_status_text("暂无文件内容".into());
    window.set_text_content("".into());
    window.set_image_content(Image::default());
    window.set_is_image(false);
    window.set_has_content(false);
    window.set_image_width(0);
    window.set_image_height(0);
    let _ = window.show();
}

/// Returns true for file types the preview window knows how to render.
pub fn is_supported_preview_file(path: &Path) -> bool {
    is_text_file(path) || is_image_file(path)
}

fn load_file_preview(window: &FilePreviewWindow, path: &Path) {
    window.set_file_path(path.display().to_string().into());
    window.set_file_name(
        path.file_name()
            .and_then(|name| name.to_str())
            .unwrap_or("文件预览")
            .into(),
    );
    window.set_has_content(false);
    window.set_is_image(false);
    window.set_text_content("".into());
    window.set_image_content(Image::default());
    window.set_image_width(0);
    window.set_image_height(0);

    if !path.exists() {
        window.set_status_text("文件不存在或已被移动".into());
        return;
    }

    if is_image_file(path) {
        match load_image_preview(path) {
            Ok((image, width, height)) => {
                window.set_image_content(image);
                window.set_image_width(width);
                window.set_image_height(height);
                window.set_is_image(true);
                window.set_has_content(true);
                window.set_status_text("".into());
            }
            Err(err) => {
                window.set_status_text(format!("图片读取失败：{err}").into());
            }
        }
        return;
    }

    if is_text_file(path) {
        match std::fs::read_to_string(path) {
            Ok(text) => {
                window.set_text_content(text.into());
                window.set_has_content(true);
                window.set_status_text("".into());
            }
            Err(err) => {
                window.set_status_text(format!("文本读取失败：{err}").into());
            }
        }
        return;
    }

    window.set_status_text("暂不支持该文件类型，仅支持 txt 文本和图片".into());
}

fn load_image_preview(path: &Path) -> Result<(Image, i32, i32), String> {
    let source = image::open(path)
        .map_err(|err| err.to_string())?
        .into_rgba8();
    let (source_width, source_height) = source.dimensions();
    let preview = if source_width <= MAX_IMAGE_WIDTH && source_height <= MAX_IMAGE_HEIGHT {
        source
    } else {
        let width_ratio = MAX_IMAGE_WIDTH as f64 / source_width as f64;
        let height_ratio = MAX_IMAGE_HEIGHT as f64 / source_height as f64;
        let scale = width_ratio.min(height_ratio);
        let target_width = ((source_width as f64 * scale).round() as u32).max(1);
        let target_height = ((source_height as f64 * scale).round() as u32).max(1);

        image::imageops::resize(
            &source,
            target_width,
            target_height,
            image::imageops::FilterType::Lanczos3,
        )
    };

    let (width, height) = preview.dimensions();
    let buffer = SharedPixelBuffer::<Rgba8Pixel>::clone_from_slice(preview.as_raw(), width, height);
    Ok((Image::from_rgba8(buffer), width as i32, height as i32))
}

fn is_text_file(path: &Path) -> bool {
    path.extension()
        .and_then(|extension| extension.to_str())
        .map(|extension| extension.eq_ignore_ascii_case("txt"))
        .unwrap_or(false)
}

fn is_image_file(path: &Path) -> bool {
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

fn close_preview_window(window: &slint::Weak<FilePreviewWindow>, quit_on_close: bool) {
    if quit_on_close {
        if let Err(err) = slint::quit_event_loop() {
            log::error!("quit preview event loop failed: {err}");
        }
        return;
    }

    if let Some(ui) = window.upgrade() {
        let _ = ui.hide();
    }
}
