use slint::Image;
use tray_icon::Icon;


// 新增辅助函数 - 弹出系统对话框
#[cfg(target_os = "windows")]
pub fn show_message_box(title: &str, message: &str) {
    use windows_sys::Win32::UI::WindowsAndMessaging::*;
    use std::ffi::OsStr;
    use std::os::windows::ffi::OsStrExt;

    // 将字符串转换为宽字符格式
    let title_wide: Vec<u16> = OsStr::new(title)
        .encode_wide()
        .chain(std::iter::once(0))
        .collect();
    
    let message_wide: Vec<u16> = OsStr::new(message)
        .encode_wide()
        .chain(std::iter::once(0))
        .collect();

    unsafe {
        MessageBoxW(
            0,                          // 父窗口句柄，0 表示桌面
            message_wide.as_ptr(),      // 消息文本
            title_wide.as_ptr(),        // 标题文本
            MB_OK | MB_ICONINFORMATION, // 按钮类型和图标
        );
    }
}

// 加载slint图片资源
pub fn load_slint_img(img: &[u8]) -> Image {
    let close_image = image::load_from_memory(img)
        .expect("无法打开关闭按钮图标文件")
        .into_rgba8();
    let (width, height) = close_image.dimensions();
    let close_buffer = slint::SharedPixelBuffer::<slint::Rgba8Pixel>::clone_from_slice(
        close_image.as_raw(),
        width,
        height,
    );
    Image::from_rgba8(close_buffer)
}

// 加载图标文件
pub fn load_icon(img: &[u8]) -> Icon {
    // 打开图片文件 转换为RGBA8格式
    let img = image::load_from_memory(img)
        .expect("无法打开图标文件")
        .into_rgba8();
    // 获取图片宽高
    let (width, height) = img.dimensions();
    // 获取原始像素字节流
    let rgba = img.into_raw();
    Icon::from_rgba(rgba, width, height).expect("创建图标失败")
}
