use slint::Image;
use tray_icon::Icon;

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

pub fn load_tray_icon(img: &[u8]) -> Icon {
    let image = image::load_from_memory(img)
        .expect("failed to decode tray icon")
        .into_rgba8();
    let (width, height) = image.dimensions();
    Icon::from_rgba(image.into_raw(), width, height).expect("failed to create tray icon")
}
