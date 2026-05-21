//! Screenshot overlay window and Windows screen-capture plumbing.

use std::cell::RefCell;
use std::path::PathBuf;
use std::time::{SystemTime, UNIX_EPOCH};

use arboard::{Clipboard, ImageData};
use image::{Rgba, RgbaImage};
use imageproc::drawing::{
    draw_filled_rect_mut, draw_hollow_ellipse_mut, draw_hollow_rect_mut, draw_line_segment_mut,
    draw_text_mut,
};
use imageproc::rect::Rect;
use rusttype::{Font, Scale};
use slint::{ComponentHandle, Image, Rgba8Pixel, SharedPixelBuffer};

use crate::ScreenshotWindow;
use crate::platform::window::{activate_slint_window, hide_taskbar_icon_for, set_window_position};

#[derive(Clone)]
struct ScreenshotSession {
    bounds: ScreenBounds,
    image: RgbaImage,
    scale_factor: f32,
    undo_stack: Vec<RgbaImage>,
}

#[derive(Clone, Copy)]
struct ScreenBounds {
    x: i32,
    y: i32,
    width: u32,
    height: u32,
}

thread_local! {
    static SCREENSHOT_SESSION: RefCell<Option<ScreenshotSession>> = const { RefCell::new(None) };
}

/// Builds the reusable screenshot overlay and wires copy/save/cancel actions.
pub fn init_screenshot_window() -> ScreenshotWindow {
    let window = ScreenshotWindow::new().unwrap();

    let weak = window.as_weak();
    window.on_cancel(move || {
        if let Some(ui) = weak.upgrade() {
            cancel_screenshot_window(&ui);
        }
    });

    let weak = window.as_weak();
    window.on_copy_selection(move |x, y, width, height| {
        match cropped_selection(x, y, width, height).and_then(copy_image) {
            Ok(()) => {
                SCREENSHOT_SESSION.with(|store| {
                    *store.borrow_mut() = None;
                });

                if let Some(ui) = weak.upgrade() {
                    let _ = ui.hide();
                }
            }
            Err(err) => log::error!("copy screenshot failed: {err}"),
        }
    });

    let weak = window.as_weak();
    window.on_save_selection(move |x, y, width, height| {
        match cropped_selection(x, y, width, height) {
            Ok(image) => {
                SCREENSHOT_SESSION.with(|store| {
                    *store.borrow_mut() = None;
                });

                if let Some(ui) = weak.upgrade() {
                    let _ = ui.hide();
                }

                std::thread::spawn(move || {
                    if let Err(err) = save_image(image) {
                        log::error!("save screenshot failed: {err}");
                    }
                });
            }
            Err(err) => log::error!("save screenshot failed: {err}"),
        }
    });

    let weak = window.as_weak();
    window.on_sample_magnifier_pixel(move |x, y| {
        if let Some(ui) = weak.upgrade() {
            update_magnifier_pixel(&ui, x, y);
        }
    });

    let weak = window.as_weak();
    window.on_draw_annotation(move |tool, x1, y1, x2, y2, text, color_index, stroke_width, text_size| {
        if let Some(ui) = weak.upgrade() {
            if let Err(err) = draw_annotation(
                &ui,
                tool.as_str(),
                x1,
                y1,
                x2,
                y2,
                text.as_str(),
                color_index,
                stroke_width,
                text_size,
            ) {
                log::error!("draw screenshot annotation failed: {err}");
            }
        }
    });

    let weak = window.as_weak();
    window.on_undo_annotation(move || {
        if let Some(ui) = weak.upgrade() {
            undo_annotation(&ui);
        }
    });

    window
}

/// Hides the screenshot overlay and drops the captured bitmap for the current session.
pub fn cancel_screenshot_window(window: &ScreenshotWindow) {
    SCREENSHOT_SESSION.with(|store| {
        *store.borrow_mut() = None;
    });
    let _ = window.hide();
}

/// Captures the desktop and shows a full-screen selection overlay.
pub fn show_screenshot_window(window: &ScreenshotWindow) {
    match capture_screen() {
        Ok(session) => {
            let preview = image_from_rgba(&session.image);
            let bounds = session.bounds;

            window
                .window()
                .set_size(slint::PhysicalSize::new(bounds.width, bounds.height));
            window.set_screenshot(preview);
            window.set_has_selection(false);
            window.set_is_dragging(false);
            window.set_is_annotating(false);
            window.set_active_tool("".into());
            window.set_text_editor_visible(false);
            window.set_text_editor_value("".into());
            window.set_magnifier_center_text("Center: 0, 0".into());
            window.set_magnifier_hex_text("HEX: #000000".into());
            window.set_magnifier_rgb_text("RGB: 0, 0, 0".into());
            set_window_position(window, bounds.x as f64, bounds.y as f64);

            let _ = window.show();
            hide_taskbar_icon_for(window);
            activate_slint_window(window);
            let scale_factor = window.window().scale_factor().max(1.0);
            SCREENSHOT_SESSION.with(|store| {
                *store.borrow_mut() = Some(ScreenshotSession {
                    scale_factor,
                    undo_stack: Vec::new(),
                    ..session
                });
            });
            window.invoke_focus_overlay();
        }
        Err(err) => log::error!("capture screen failed: {err}"),
    }
}

fn cropped_selection(x: i32, y: i32, width: i32, height: i32) -> Result<RgbaImage, String> {
    if width <= 0 || height <= 0 {
        return Err("empty screenshot selection".into());
    }

    SCREENSHOT_SESSION.with(|store| {
        let store = store.borrow();
        let session = store
            .as_ref()
            .ok_or_else(|| "missing screenshot session".to_string())?;
        let x = scale_logical_coordinate(x, session.scale_factor)
            .clamp(0, session.bounds.width.saturating_sub(1) as i32) as u32;
        let y = scale_logical_coordinate(y, session.scale_factor)
            .clamp(0, session.bounds.height.saturating_sub(1) as i32) as u32;
        let width = scale_logical_size(width, session.scale_factor).min(session.bounds.width - x);
        let height =
            scale_logical_size(height, session.scale_factor).min(session.bounds.height - y);
        Ok(image::imageops::crop_imm(&session.image, x, y, width, height).to_image())
    })
}

fn update_magnifier_pixel(window: &ScreenshotWindow, x: i32, y: i32) {
    SCREENSHOT_SESSION.with(|store| {
        let store = store.borrow();
        let Some(session) = store.as_ref() else {
            return;
        };

        let pixel_x = scale_logical_coordinate(x, session.scale_factor)
            .clamp(0, session.bounds.width.saturating_sub(1) as i32) as u32;
        let pixel_y = scale_logical_coordinate(y, session.scale_factor)
            .clamp(0, session.bounds.height.saturating_sub(1) as i32) as u32;
        let [r, g, b, _] = session.image.get_pixel(pixel_x, pixel_y).0;

        window.set_magnifier_center_text(
            format!("Center: {pixel_x}, {pixel_y}").into(),
        );
        window.set_magnifier_hex_text(format!("HEX: #{r:02X}{g:02X}{b:02X}").into());
        window.set_magnifier_rgb_text(format!("RGB: {r}, {g}, {b}").into());
    });
}

fn draw_annotation(
    window: &ScreenshotWindow,
    tool: &str,
    x1: i32,
    y1: i32,
    x2: i32,
    y2: i32,
    text: &str,
    color_index: i32,
    stroke_width: i32,
    text_size: i32,
) -> Result<(), String> {
    SCREENSHOT_SESSION.with(|store| {
        let mut store = store.borrow_mut();
        let session = store
            .as_mut()
            .ok_or_else(|| "missing screenshot session".to_string())?;

        session.undo_stack.push(session.image.clone());
        if session.undo_stack.len() > 20 {
            session.undo_stack.remove(0);
        }

        let x1 = scale_logical_coordinate(x1, session.scale_factor);
        let y1 = scale_logical_coordinate(y1, session.scale_factor);
        let x2 = scale_logical_coordinate(x2, session.scale_factor);
        let y2 = scale_logical_coordinate(y2, session.scale_factor);
        let color = annotation_color(color_index);
        let stroke_width = stroke_width.clamp(2, 12);
        let text_size = text_size.clamp(12, 64);

        match tool {
            "rect" => draw_rect_annotation(&mut session.image, x1, y1, x2, y2, color, stroke_width),
            "circle" => {
                draw_ellipse_annotation(&mut session.image, x1, y1, x2, y2, color, stroke_width)
            }
            "arrow" => draw_arrow_annotation(&mut session.image, x1, y1, x2, y2, color, stroke_width),
            "brush" => draw_thick_line(&mut session.image, x1, y1, x2, y2, color, stroke_width),
            "mosaic" => draw_mosaic_annotation(&mut session.image, x1, y1, x2, y2),
            "text" => draw_text_annotation(&mut session.image, x1, y1, text, color, text_size),
            _ => {}
        }

        window.set_screenshot(image_from_rgba(&session.image));
        Ok(())
    })
}

fn annotation_color(index: i32) -> Rgba<u8> {
    match index {
        1 => Rgba([45, 127, 249, 255]),
        2 => Rgba([255, 77, 79, 255]),
        3 => Rgba([244, 180, 0, 255]),
        4 => Rgba([36, 179, 58, 255]),
        5 => Rgba([154, 160, 166, 255]),
        6 => Rgba([255, 255, 255, 255]),
        _ => Rgba([32, 33, 36, 255]),
    }
}

fn undo_annotation(window: &ScreenshotWindow) {
    SCREENSHOT_SESSION.with(|store| {
        let mut store = store.borrow_mut();
        let Some(session) = store.as_mut() else {
            return;
        };
        let Some(previous) = session.undo_stack.pop() else {
            return;
        };
        session.image = previous;
        window.set_screenshot(image_from_rgba(&session.image));
    });
}

fn draw_rect_annotation(
    image: &mut RgbaImage,
    x1: i32,
    y1: i32,
    x2: i32,
    y2: i32,
    color: Rgba<u8>,
    stroke_width: i32,
) {
    let left = x1.min(x2);
    let top = y1.min(y2);
    let width = (x1 - x2).unsigned_abs().max(1);
    let height = (y1 - y2).unsigned_abs().max(1);
    for offset in 0..stroke_width {
        let rect = Rect::at(left - offset, top - offset).of_size(width + offset as u32 * 2, height + offset as u32 * 2);
        draw_hollow_rect_mut(image, rect, color);
    }
}

fn draw_ellipse_annotation(
    image: &mut RgbaImage,
    x1: i32,
    y1: i32,
    x2: i32,
    y2: i32,
    color: Rgba<u8>,
    stroke_width: i32,
) {
    let center_x = (x1 + x2) / 2;
    let center_y = (y1 + y2) / 2;
    let radius_x = ((x1 - x2).abs() / 2).max(1);
    let radius_y = ((y1 - y2).abs() / 2).max(1);
    for offset in 0..stroke_width {
        draw_hollow_ellipse_mut(image, (center_x, center_y), radius_x + offset, radius_y + offset, color);
    }
}

fn draw_arrow_annotation(
    image: &mut RgbaImage,
    x1: i32,
    y1: i32,
    x2: i32,
    y2: i32,
    color: Rgba<u8>,
    stroke_width: i32,
) {
    draw_thick_line(image, x1, y1, x2, y2, color, stroke_width);

    let angle = ((y2 - y1) as f32).atan2((x2 - x1) as f32);
    let head_len = 18.0;
    let spread = std::f32::consts::PI / 7.0;
    for head_angle in [angle + std::f32::consts::PI - spread, angle + std::f32::consts::PI + spread] {
        let hx = x2 as f32 + head_len * head_angle.cos();
        let hy = y2 as f32 + head_len * head_angle.sin();
        draw_thick_line(image, x2, y2, hx.round() as i32, hy.round() as i32, color, stroke_width);
    }
}

fn draw_thick_line(
    image: &mut RgbaImage,
    x1: i32,
    y1: i32,
    x2: i32,
    y2: i32,
    color: Rgba<u8>,
    thickness: i32,
) {
    for dx in -thickness..=thickness {
        for dy in -thickness..=thickness {
            if dx * dx + dy * dy <= thickness * thickness {
                draw_line_segment_mut(
                    image,
                    ((x1 + dx) as f32, (y1 + dy) as f32),
                    ((x2 + dx) as f32, (y2 + dy) as f32),
                    color,
                );
            }
        }
    }
}

fn draw_mosaic_annotation(image: &mut RgbaImage, x1: i32, y1: i32, x2: i32, y2: i32) {
    let radius = 14;
    let left = (x1.min(x2) - radius).max(0) as u32;
    let top = (y1.min(y2) - radius).max(0) as u32;
    let right = (x1.max(x2) + radius).clamp(0, image.width().saturating_sub(1) as i32) as u32;
    let bottom = (y1.max(y2) + radius).clamp(0, image.height().saturating_sub(1) as i32) as u32;
    let block = 10;

    let mut by = top;
    while by <= bottom {
        let mut bx = left;
        while bx <= right {
            let sample = *image.get_pixel(bx, by);
            let rect_width = (block as u32).min(right - bx + 1);
            let rect_height = (block as u32).min(bottom - by + 1);
            draw_filled_rect_mut(image, Rect::at(bx as i32, by as i32).of_size(rect_width, rect_height), sample);
            bx = bx.saturating_add(block);
        }
        by = by.saturating_add(block);
    }
}

fn draw_text_annotation(
    image: &mut RgbaImage,
    x: i32,
    y: i32,
    text: &str,
    color: Rgba<u8>,
    text_size: i32,
) {
    if text.trim().is_empty() {
        return;
    }

    let Some(font) = Font::try_from_bytes(include_bytes!("../../../assets/font/AlibabaPuHuiTi-3-55-Regular.ttf") as &[u8]) else {
        return;
    };
    draw_text_mut(image, color, x, y, Scale::uniform(text_size as f32), &font, text);
}

fn scale_logical_coordinate(value: i32, scale_factor: f32) -> i32 {
    ((value as f32) * scale_factor).round() as i32
}

fn scale_logical_size(value: i32, scale_factor: f32) -> u32 {
    (((value as f32) * scale_factor).round() as u32).max(1)
}

fn copy_image(image: RgbaImage) -> Result<(), String> {
    let (width, height) = image.dimensions();
    Clipboard::new()
        .map_err(|err| format!("open clipboard failed: {err}"))?
        .set_image(ImageData {
            width: width as usize,
            height: height as usize,
            bytes: image.into_raw().into(),
        })
        .map_err(|err| format!("set screenshot image failed: {err}"))
}

fn save_image(image: RgbaImage) -> Result<(), String> {
    let default_path = default_screenshot_path()?;
    let Some(path) = rfd::FileDialog::new()
        .set_title("保存截图")
        .add_filter("PNG Image", &["png"])
        .set_file_name(
            default_path
                .file_name()
                .and_then(|name| name.to_str())
                .unwrap_or("screenshot.png"),
        )
        .save_file()
    else {
        return Ok(());
    };

    image
        .save(&path)
        .map_err(|err| format!("save screenshot failed: {err}"))
}

fn default_screenshot_path() -> Result<PathBuf, String> {
    let timestamp = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_err(|err| format!("read system time failed: {err}"))?
        .as_secs();
    Ok(PathBuf::from(format!("screenshot-{timestamp}.png")))
}

fn image_from_rgba(source: &RgbaImage) -> Image {
    let (width, height) = source.dimensions();
    let buffer = SharedPixelBuffer::<Rgba8Pixel>::clone_from_slice(source.as_raw(), width, height);
    Image::from_rgba8(buffer)
}

#[cfg(target_os = "windows")]
fn capture_screen() -> Result<ScreenshotSession, String> {
    use std::mem::{size_of, zeroed};
    use windows_sys::Win32::Graphics::Gdi::{
        BI_RGB, BITMAPINFO, BITMAPINFOHEADER, BitBlt, CAPTUREBLT, CreateCompatibleBitmap,
        CreateCompatibleDC, DIB_RGB_COLORS, DeleteDC, DeleteObject, GetDC, GetDIBits, ReleaseDC,
        SRCCOPY, SelectObject,
    };
    use windows_sys::Win32::UI::WindowsAndMessaging::{
        GetSystemMetrics, SM_CXVIRTUALSCREEN, SM_CYVIRTUALSCREEN, SM_XVIRTUALSCREEN,
        SM_YVIRTUALSCREEN,
    };

    unsafe {
        let bounds = ScreenBounds {
            x: GetSystemMetrics(SM_XVIRTUALSCREEN),
            y: GetSystemMetrics(SM_YVIRTUALSCREEN),
            width: GetSystemMetrics(SM_CXVIRTUALSCREEN) as u32,
            height: GetSystemMetrics(SM_CYVIRTUALSCREEN) as u32,
        };

        if bounds.width == 0 || bounds.height == 0 {
            return Err("virtual screen has no size".into());
        }

        let screen_dc = GetDC(0);
        if screen_dc == 0 {
            return Err("GetDC failed".into());
        }

        let memory_dc = CreateCompatibleDC(screen_dc);
        if memory_dc == 0 {
            ReleaseDC(0, screen_dc);
            return Err("CreateCompatibleDC failed".into());
        }

        let bitmap = CreateCompatibleBitmap(screen_dc, bounds.width as i32, bounds.height as i32);
        if bitmap == 0 {
            DeleteDC(memory_dc);
            ReleaseDC(0, screen_dc);
            return Err("CreateCompatibleBitmap failed".into());
        }

        let old_object = SelectObject(memory_dc, bitmap);
        let copied = BitBlt(
            memory_dc,
            0,
            0,
            bounds.width as i32,
            bounds.height as i32,
            screen_dc,
            bounds.x,
            bounds.y,
            SRCCOPY | CAPTUREBLT,
        );

        if copied == 0 {
            SelectObject(memory_dc, old_object);
            DeleteObject(bitmap);
            DeleteDC(memory_dc);
            ReleaseDC(0, screen_dc);
            return Err("BitBlt failed".into());
        }

        let mut bitmap_info: BITMAPINFO = zeroed();
        bitmap_info.bmiHeader = BITMAPINFOHEADER {
            biSize: size_of::<BITMAPINFOHEADER>() as u32,
            biWidth: bounds.width as i32,
            biHeight: -(bounds.height as i32),
            biPlanes: 1,
            biBitCount: 32,
            biCompression: BI_RGB,
            biSizeImage: bounds.width * bounds.height * 4,
            biXPelsPerMeter: 0,
            biYPelsPerMeter: 0,
            biClrUsed: 0,
            biClrImportant: 0,
        };

        let mut bgra = vec![0u8; (bounds.width * bounds.height * 4) as usize];
        let scan_lines = GetDIBits(
            memory_dc,
            bitmap,
            0,
            bounds.height,
            bgra.as_mut_ptr().cast(),
            &mut bitmap_info,
            DIB_RGB_COLORS,
        );

        SelectObject(memory_dc, old_object);
        DeleteObject(bitmap);
        DeleteDC(memory_dc);
        ReleaseDC(0, screen_dc);

        if scan_lines == 0 {
            return Err("GetDIBits failed".into());
        }

        for pixel in bgra.chunks_exact_mut(4) {
            pixel.swap(0, 2);
            pixel[3] = 255;
        }

        let image = RgbaImage::from_raw(bounds.width, bounds.height, bgra)
            .ok_or_else(|| "invalid captured image buffer".to_string())?;
        Ok(ScreenshotSession {
            bounds,
            image,
            scale_factor: 1.0,
            undo_stack: Vec::new(),
        })
    }
}

#[cfg(not(target_os = "windows"))]
fn capture_screen() -> Result<ScreenshotSession, String> {
    Err("screenshot capture is currently implemented only on Windows".into())
}
