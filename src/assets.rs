//! Helpers for decoding embedded system tray icon assets.

use tray_icon::Icon;

/// 解码嵌入的图片并创建系统托盘图标。
pub fn load_tray_icon(img: &[u8]) -> Icon {
    let image = image::load_from_memory(img)
        .expect("failed to decode tray icon")
        .into_rgba8();
    let (width, height) = image.dimensions();
    Icon::from_rgba(image.into_raw(), width, height).expect("failed to create tray icon")
}
