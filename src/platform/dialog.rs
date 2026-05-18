//! Cross-platform message dialog helper.

use std::path::PathBuf;

#[cfg(target_os = "windows")]
/// Shows a native Windows information message box.
pub fn show_message_box(title: &str, message: &str) {
    use std::ffi::OsStr;
    use std::os::windows::ffi::OsStrExt;
    use windows_sys::Win32::UI::WindowsAndMessaging::{MB_ICONINFORMATION, MB_OK, MessageBoxW};

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
            0,
            message_wide.as_ptr(),
            title_wide.as_ptr(),
            MB_OK | MB_ICONINFORMATION,
        );
    }
}

#[cfg(target_os = "windows")]
/// Opens a native Windows file picker and returns the selected path.
pub fn open_file_dialog(title: &str) -> Option<PathBuf> {
    use std::ffi::OsStr;
    use std::mem::{size_of, zeroed};
    use std::os::windows::ffi::OsStrExt;
    use windows_sys::Win32::UI::Controls::Dialogs::{
        GetOpenFileNameW, OFN_EXPLORER, OFN_FILEMUSTEXIST, OFN_PATHMUSTEXIST, OPENFILENAMEW,
    };

    let title_wide: Vec<u16> = OsStr::new(title)
        .encode_wide()
        .chain(std::iter::once(0))
        .collect();
    let filter_wide: Vec<u16> = OsStr::new("Supported Files\0*.txt;*.png;*.jpg;*.jpeg;*.bmp;*.gif;*.webp;*.tif;*.tiff\0All Files\0*.*\0")
        .encode_wide()
        .chain(std::iter::once(0))
        .collect();
    let mut file_buffer = [0u16; 1024];

    let mut dialog: OPENFILENAMEW = unsafe { zeroed() };
    dialog.lStructSize = size_of::<OPENFILENAMEW>() as u32;
    dialog.lpstrTitle = title_wide.as_ptr();
    dialog.lpstrFilter = filter_wide.as_ptr();
    dialog.lpstrFile = file_buffer.as_mut_ptr();
    dialog.nMaxFile = file_buffer.len() as u32;
    dialog.Flags = OFN_EXPLORER | OFN_FILEMUSTEXIST | OFN_PATHMUSTEXIST;

    let selected = unsafe { GetOpenFileNameW(&mut dialog) != 0 };
    if !selected {
        return None;
    }

    let length = file_buffer
        .iter()
        .position(|character| *character == 0)
        .unwrap_or(file_buffer.len());
    if length == 0 {
        return None;
    }

    Some(PathBuf::from(String::from_utf16_lossy(
        &file_buffer[..length],
    )))
}

#[cfg(not(target_os = "windows"))]
/// Falls back to stderr when a native dialog is not implemented.
pub fn show_message_box(title: &str, message: &str) {
    eprintln!("{title}: {message}");
}

#[cfg(not(target_os = "windows"))]
/// File picker fallback for platforms without a native implementation here.
pub fn open_file_dialog(_title: &str) -> Option<PathBuf> {
    None
}
