//! Clipboard read/write helpers, including Windows file-list clipboard support.

use std::path::PathBuf;

use arboard::{Clipboard, ImageData};

use crate::features::clipboard_history::history::ClipboardHistoryItem;
use crate::features::clipboard_history::store::CapturedClipboardItem;

/// Reads the current clipboard and converts supported formats into a history item.
pub fn capture_clipboard_item() -> Option<CapturedClipboardItem> {
    if let Some(paths) = platform::get_clipboard_files() {
        if !paths.is_empty() {
            return Some(CapturedClipboardItem::Files { paths });
        }
    }

    if let Ok(mut clipboard) = Clipboard::new() {
        if let Ok(image) = clipboard.get_image() {
            log::info!(
                "captured image clipboard item: {}x{}, {} bytes",
                image.width,
                image.height,
                image.bytes.len()
            );
            return Some(CapturedClipboardItem::Image {
                width: image.width,
                height: image.height,
                bytes: image.bytes.into_owned(),
            });
        }

        if let Ok(text) = clipboard.get_text() {
            let text = text.trim().to_string();
            if !text.is_empty() {
                return Some(CapturedClipboardItem::Text { text });
            }
        }
    }

    None
}

/// Writes a history item back to the clipboard so it can be pasted.
pub fn put_clipboard_item(item: &ClipboardHistoryItem) -> Result<(), String> {
    match item {
        ClipboardHistoryItem::Text { text } => Clipboard::new()
            .map_err(|err| format!("open clipboard failed: {err}"))?
            .set_text(text.clone())
            .map_err(|err| format!("set text failed: {err}")),
        ClipboardHistoryItem::Image {
            width,
            height,
            path,
            ..
        } => {
            let image = image::open(path)
                .map_err(|err| format!("read clipboard image failed: {err}"))?
                .into_rgba8();
            Clipboard::new()
                .map_err(|err| format!("open clipboard failed: {err}"))?
                .set_image(ImageData {
                    width: *width,
                    height: *height,
                    bytes: image.into_raw().into(),
                })
                .map_err(|err| format!("set image failed: {err}"))
        }
        ClipboardHistoryItem::Files { paths } => platform::set_clipboard_files(paths),
    }
}

mod platform {
    use super::*;

    #[cfg(target_os = "windows")]
    /// Reads Windows CF_HDROP file paths from the clipboard.
    pub fn get_clipboard_files() -> Option<Vec<PathBuf>> {
        use std::os::windows::ffi::OsStringExt;
        use windows_sys::Win32::System::DataExchange::{
            CloseClipboard, GetClipboardData, IsClipboardFormatAvailable, OpenClipboard,
        };
        use windows_sys::Win32::System::Ole::CF_HDROP;
        use windows_sys::Win32::UI::Shell::{DragQueryFileW, HDROP};

        unsafe {
            if IsClipboardFormatAvailable(CF_HDROP as u32) == 0 || OpenClipboard(0) == 0 {
                return None;
            }

            let handle = GetClipboardData(CF_HDROP as u32);
            if handle == 0 {
                CloseClipboard();
                return None;
            }

            let hdrop = handle as HDROP;
            let count = DragQueryFileW(hdrop, u32::MAX, std::ptr::null_mut(), 0);
            let mut paths = Vec::new();

            for index in 0..count {
                let len = DragQueryFileW(hdrop, index, std::ptr::null_mut(), 0);
                if len == 0 {
                    continue;
                }

                let mut buffer = vec![0u16; len as usize + 1];
                let written =
                    DragQueryFileW(hdrop, index, buffer.as_mut_ptr(), buffer.len() as u32);
                if written > 0 {
                    buffer.truncate(written as usize);
                    paths.push(PathBuf::from(std::ffi::OsString::from_wide(&buffer)));
                }
            }

            CloseClipboard();
            Some(paths)
        }
    }

    #[cfg(not(target_os = "windows"))]
    /// File-list clipboard capture is currently implemented only for Windows.
    pub fn get_clipboard_files() -> Option<Vec<PathBuf>> {
        None
    }

    #[cfg(target_os = "windows")]
    /// Writes Windows CF_HDROP file paths to the clipboard.
    pub fn set_clipboard_files(paths: &[PathBuf]) -> Result<(), String> {
        use std::mem::size_of;
        use std::os::windows::ffi::OsStrExt;
        use windows_sys::Win32::System::DataExchange::{
            CloseClipboard, EmptyClipboard, OpenClipboard, SetClipboardData,
        };
        use windows_sys::Win32::System::Memory::{
            GMEM_MOVEABLE, GlobalAlloc, GlobalLock, GlobalUnlock,
        };
        use windows_sys::Win32::System::Ole::CF_HDROP;
        use windows_sys::Win32::UI::Shell::DROPFILES;

        let mut encoded_paths = Vec::<u16>::new();
        for path in paths {
            encoded_paths.extend(path.as_os_str().encode_wide());
            encoded_paths.push(0);
        }
        encoded_paths.push(0);

        let header_size = size_of::<DROPFILES>();
        let data_size = header_size + encoded_paths.len() * size_of::<u16>();

        unsafe {
            if OpenClipboard(0) == 0 {
                return Err("open clipboard failed".into());
            }

            if EmptyClipboard() == 0 {
                CloseClipboard();
                return Err("empty clipboard failed".into());
            }

            let handle = GlobalAlloc(GMEM_MOVEABLE, data_size);
            if handle.is_null() {
                CloseClipboard();
                return Err("allocate clipboard memory failed".into());
            }

            let memory = GlobalLock(handle);
            if memory.is_null() {
                CloseClipboard();
                return Err("lock clipboard memory failed".into());
            }

            let dropfiles = memory as *mut DROPFILES;
            (*dropfiles).pFiles = header_size as u32;
            (*dropfiles).pt.x = 0;
            (*dropfiles).pt.y = 0;
            (*dropfiles).fNC = 0;
            (*dropfiles).fWide = 1;

            let target = (memory as *mut u8).add(header_size) as *mut u16;
            std::ptr::copy_nonoverlapping(encoded_paths.as_ptr(), target, encoded_paths.len());
            GlobalUnlock(handle);

            if SetClipboardData(CF_HDROP as u32, handle as isize) == 0 {
                CloseClipboard();
                return Err("set file clipboard data failed".into());
            }

            CloseClipboard();
            Ok(())
        }
    }

    #[cfg(not(target_os = "windows"))]
    /// Falls back to newline-separated file paths on platforms without CF_HDROP.
    pub fn set_clipboard_files(paths: &[PathBuf]) -> Result<(), String> {
        Clipboard::new()
            .map_err(|err| format!("open clipboard failed: {err}"))?
            .set_text(
                paths
                    .iter()
                    .map(|path| path.display().to_string())
                    .collect::<Vec<_>>()
                    .join("\n"),
            )
            .map_err(|err| format!("set file paths as text failed: {err}"))
    }
}
