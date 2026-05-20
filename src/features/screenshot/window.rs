//! Screenshot overlay window and Windows screen-capture plumbing.

use std::cell::RefCell;
use std::path::PathBuf;
use std::time::{SystemTime, UNIX_EPOCH};

use arboard::{Clipboard, ImageData};
use image::RgbaImage;
use slint::{ComponentHandle, Image, Rgba8Pixel, SharedPixelBuffer};

use crate::ScreenshotWindow;
use crate::platform::window::{activate_slint_window, hide_taskbar_icon_for, set_window_position};

#[derive(Clone)]
struct ScreenshotSession {
    bounds: ScreenBounds,
    image: RgbaImage,
    scale_factor: f32,
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
            set_window_position(window, bounds.x as f64, bounds.y as f64);

            let _ = window.show();
            hide_taskbar_icon_for(window);
            activate_slint_window(window);
            let scale_factor = window.window().scale_factor().max(1.0);
            SCREENSHOT_SESSION.with(|store| {
                *store.borrow_mut() = Some(ScreenshotSession {
                    scale_factor,
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
        })
    }
}

#[cfg(not(target_os = "windows"))]
fn capture_screen() -> Result<ScreenshotSession, String> {
    Err("screenshot capture is currently implemented only on Windows".into())
}
