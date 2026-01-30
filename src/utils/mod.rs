
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