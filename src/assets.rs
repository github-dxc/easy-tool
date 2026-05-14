//! Helpers for decoding embedded image assets into UI and tray icon formats.

use slint::Image;
use tray_icon::Icon;

/// Converts embedded image bytes into a Slint image.
pub fn load_slint_image(img: &[u8]) -> Image {
    let image = image::load_from_memory(img)
        .expect("failed to decode Slint image")
        .into_rgba8();
    let (width, height) = image.dimensions();
    let buffer = slint::SharedPixelBuffer::<slint::Rgba8Pixel>::clone_from_slice(
        image.as_raw(),
        width,
        height,
    );
    Image::from_rgba8(buffer)
}

/// Converts embedded image bytes into a system tray icon.
pub fn load_tray_icon(img: &[u8]) -> Icon {
    let image = image::load_from_memory(img)
        .expect("failed to decode tray icon")
        .into_rgba8();
    let (width, height) = image.dimensions();
    Icon::from_rgba(image.into_raw(), width, height).expect("failed to create tray icon")
}
