//! Disk-backed clipboard history storage.

use std::io::Write;
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

use crate::features::clipboard_history::history::{ClipboardHistory, ClipboardHistoryItem};

const HISTORY_INDEX_FILE: &str = "clipboard_history.json";

#[derive(Debug)]
pub enum CapturedClipboardItem {
    Text {
        text: String,
    },
    Image {
        width: usize,
        height: usize,
        bytes: Vec<u8>,
    },
    Files {
        paths: Vec<PathBuf>,
    },
}

#[derive(Debug, Serialize, Deserialize)]
struct PersistedClipboardHistory {
    entries: Vec<ClipboardHistoryItem>,
}

/// 生成历史索引 JSON 文件的完整路径。
pub fn history_index_path(dir: &Path) -> PathBuf {
    dir.join(HISTORY_INDEX_FILE)
}

/// 加载历史索引，清理重复及失效记录，并安全删除废弃缓存图片。
pub fn load_history(dir: &Path) -> Result<ClipboardHistory, String> {
    let path = history_index_path(dir);
    if !path.exists() {
        return Ok(ClipboardHistory::default());
    }

    let content =
        std::fs::read_to_string(&path).map_err(|err| format!("read history failed: {err}"))?;
    let persisted = serde_json::from_str::<PersistedClipboardHistory>(&content)
        .map_err(|err| format!("parse history failed: {err}"))?;

    let mut cleanup_paths = Vec::new();

    let mut newest_first = Vec::new();
    for entry in persisted.entries {
        if let Some(path) = entry.image_path()
            && !is_managed_existing_clipboard_image_path(dir, path)
        {
            cleanup_paths.push(path.to_path_buf());
            continue;
        }

        if newest_first
            .iter()
            .any(|latest: &ClipboardHistoryItem| latest.same_content(&entry))
        {
            if let Some(path) = entry.image_path() {
                cleanup_paths.push(path.to_path_buf());
            }
            continue;
        }

        newest_first.push(entry);
    }

    let mut history = ClipboardHistory::default();
    for entry in newest_first.into_iter().rev() {
        cleanup_paths.extend(history.push(entry));
    }
    cleanup_paths.extend(history.trim_to_limit());
    match save_history(dir, &history) {
        Ok(()) => delete_image_files(dir, cleanup_paths),
        Err(err) => log::error!("rewrite cleaned clipboard history failed: {err}"),
    }
    Ok(history)
}

/// 序列化历史快照，通过临时文件原子替换索引。
pub fn save_history(dir: &Path, history: &ClipboardHistory) -> Result<(), String> {
    std::fs::create_dir_all(dir).map_err(|err| format!("create history dir failed: {err}"))?;
    let persisted = PersistedClipboardHistory {
        entries: history.items(),
    };
    let content = serde_json::to_string_pretty(&persisted)
        .map_err(|err| format!("serialize history failed: {err}"))?;
    write_index_atomically(dir, &content)
}

/// 将剪贴板采集结果转换为历史条目，图片先持久化为缓存文件。
pub fn item_from_capture(
    dir: &Path,
    captured: CapturedClipboardItem,
) -> Result<ClipboardHistoryItem, String> {
    match captured {
        CapturedClipboardItem::Text { text } => Ok(ClipboardHistoryItem::Text { text }),
        CapturedClipboardItem::Image {
            width,
            height,
            bytes,
        } => persist_image_capture(dir, width, height, bytes),
        CapturedClipboardItem::Files { paths } => Ok(ClipboardHistoryItem::Files { paths }),
    }
}

/// 仅删除历史目录内由应用生成的图片，避免误删用户文件。
pub fn delete_image_files(dir: &Path, paths: impl IntoIterator<Item = PathBuf>) {
    let Ok(history_dir) = dir.canonicalize() else {
        return;
    };

    for path in paths {
        if !is_generated_clipboard_image_path(&path) {
            continue;
        }
        if !is_existing_path_inside_dir(&history_dir, &path) {
            continue;
        }
        if let Err(err) = std::fs::remove_file(path) {
            if err.kind() != std::io::ErrorKind::NotFound {
                log::error!("delete clipboard image failed: {err}");
            }
        }
    }
}

/// 检查文件名与扩展名是否符合应用缓存图片的命名约定。
fn is_generated_clipboard_image_path(path: &Path) -> bool {
    path.file_name()
        .and_then(|file_name| file_name.to_str())
        .is_some_and(|file_name| file_name.starts_with("clipboard-image-"))
        && path.extension().and_then(|extension| extension.to_str()) == Some("png")
}

/// 规范化路径后判断文件是否确实位于指定目录内。
fn is_existing_path_inside_dir(canonical_dir: &Path, path: &Path) -> bool {
    path.canonicalize()
        .is_ok_and(|candidate| candidate.starts_with(canonical_dir))
}

/// 验证图片存在且属于当前历史目录管理的缓存文件。
fn is_managed_existing_clipboard_image_path(dir: &Path, path: &Path) -> bool {
    if !is_generated_clipboard_image_path(path) {
        return false;
    }

    dir.canonicalize()
        .is_ok_and(|history_dir| is_existing_path_inside_dir(&history_dir, path))
}

/// 写入临时索引并同步磁盘后替换旧索引，失败时保留原索引。
fn write_index_atomically(dir: &Path, content: &str) -> Result<(), String> {
    std::fs::create_dir_all(dir).map_err(|err| format!("create history dir failed: {err}"))?;
    let index_path = history_index_path(dir);
    let temp_path = next_index_temp_path(dir)?;

    let write_result = (|| -> Result<(), String> {
        let mut file = std::fs::OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(&temp_path)
            .map_err(|err| format!("create temporary history index failed: {err}"))?;
        file.write_all(content.as_bytes())
            .map_err(|err| format!("write temporary history index failed: {err}"))?;
        file.sync_all()
            .map_err(|err| format!("sync temporary history index failed: {err}"))
    })();

    if let Err(err) = write_result {
        let _ = std::fs::remove_file(&temp_path);
        return Err(err);
    }

    replace_file(&temp_path, &index_path).map_err(|err| {
        let _ = std::fs::remove_file(&temp_path);
        format!("replace history index failed: {err}")
    })
}

#[cfg(target_os = "windows")]
/// 替换索引文件；Windows 使用允许覆盖目标的原生移动操作。
fn replace_file(from: &Path, to: &Path) -> std::io::Result<()> {
    use std::os::windows::ffi::OsStrExt;
    use windows_sys::Win32::Storage::FileSystem::{
        MOVEFILE_REPLACE_EXISTING, MOVEFILE_WRITE_THROUGH, MoveFileExW,
    };

    /// 将文件系统路径转换为以空字符结束的 Windows UTF-16 字符串。
    fn wide(path: &Path) -> Vec<u16> {
        path.as_os_str().encode_wide().chain(Some(0)).collect()
    }

    let from = wide(from);
    let to = wide(to);
    let result = unsafe {
        MoveFileExW(
            from.as_ptr(),
            to.as_ptr(),
            MOVEFILE_REPLACE_EXISTING | MOVEFILE_WRITE_THROUGH,
        )
    };

    if result == 0 {
        Err(std::io::Error::last_os_error())
    } else {
        Ok(())
    }
}

#[cfg(not(target_os = "windows"))]
/// 替换索引文件；Windows 使用允许覆盖目标的原生移动操作。
fn replace_file(from: &Path, to: &Path) -> std::io::Result<()> {
    std::fs::rename(from, to)
}

/// 生成包含进程和时间信息的唯一临时索引路径。
fn next_index_temp_path(dir: &Path) -> Result<PathBuf, String> {
    let timestamp = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_err(|err| format!("read system time failed: {err}"))?
        .as_nanos();
    for counter in 0..1000u16 {
        let path = dir.join(format!(
            "{HISTORY_INDEX_FILE}.tmp-{}-{timestamp}-{counter}",
            std::process::id()
        ));
        if !path.exists() {
            return Ok(path);
        }
    }
    Err("could not allocate temporary history index path".to_string())
}

/// 校验 RGBA 像素数据并保存 PNG，返回基于文件引用的历史条目。
fn persist_image_capture(
    dir: &Path,
    width: usize,
    height: usize,
    bytes: Vec<u8>,
) -> Result<ClipboardHistoryItem, String> {
    std::fs::create_dir_all(dir).map_err(|err| format!("create history dir failed: {err}"))?;
    let width_u32 =
        u32::try_from(width).map_err(|_| "clipboard image width is too large".to_string())?;
    let height_u32 =
        u32::try_from(height).map_err(|_| "clipboard image height is too large".to_string())?;
    let image = image::RgbaImage::from_raw(width_u32, height_u32, bytes)
        .ok_or_else(|| "invalid clipboard image bytes".to_string())?;
    let path = next_image_path(dir)?;
    image
        .save(&path)
        .map_err(|err| format!("save clipboard image failed: {err}"))?;
    let byte_len = std::fs::metadata(&path)
        .map_err(|err| format!("read clipboard image metadata failed: {err}"))?
        .len();

    Ok(ClipboardHistoryItem::Image {
        width,
        height,
        path,
        byte_len,
    })
}

/// 生成不与现有缓存冲突的历史图片路径。
fn next_image_path(dir: &Path) -> Result<PathBuf, String> {
    let timestamp = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_err(|err| format!("read system time failed: {err}"))?
        .as_millis();
    for counter in 0..1000u16 {
        let path = dir.join(format!("clipboard-image-{timestamp}-{counter}.png"));
        if !path.exists() {
            return Ok(path);
        }
    }
    Err("could not allocate clipboard image path".to_string())
}

#[cfg(test)]
mod tests {
    use super::{
        CapturedClipboardItem, delete_image_files, history_index_path, item_from_capture,
        load_history, save_history, write_index_atomically,
    };
    use crate::features::clipboard_history::history::{ClipboardHistory, ClipboardHistoryItem};
    use tempfile::tempdir;

    #[test]
    /// 验证首次运行缺少索引时恢复为空历史。
    fn missing_index_loads_empty_history() {
        let dir = tempdir().unwrap();

        let history = load_history(dir.path()).unwrap();

        assert_eq!(history.items().len(), 0);
    }

    #[test]
    /// 验证文本条目写入磁盘后可按原顺序恢复。
    fn persists_and_reloads_text_entries() {
        let dir = tempdir().unwrap();
        let mut history = ClipboardHistory::default();
        history.push(ClipboardHistoryItem::Text {
            text: "older".to_string(),
        });
        history.push(ClipboardHistoryItem::Text {
            text: "newer".to_string(),
        });

        save_history(dir.path(), &history).unwrap();
        let loaded = load_history(dir.path()).unwrap();

        assert!(matches!(
            loaded.get(0),
            Some(ClipboardHistoryItem::Text { text }) if text == "newer"
        ));
        assert!(matches!(
            loaded.get(1),
            Some(ClipboardHistoryItem::Text { text }) if text == "older"
        ));
    }

    #[test]
    /// 验证原子保存可以替换已存在的索引文件。
    fn write_index_atomically_replaces_existing_index() {
        let dir = tempdir().unwrap();
        let index_path = history_index_path(dir.path());
        std::fs::create_dir_all(dir.path()).unwrap();
        std::fs::write(&index_path, r#"{"entries":[{"Text":{"text":"old"}}]}"#).unwrap();

        write_index_atomically(dir.path(), r#"{"entries":[]}"#).unwrap();

        assert_eq!(
            std::fs::read_to_string(index_path).unwrap(),
            r#"{"entries":[]}"#
        );
        let temp_files = std::fs::read_dir(dir.path())
            .unwrap()
            .filter_map(Result::ok)
            .filter(|entry| {
                entry
                    .file_name()
                    .to_str()
                    .is_some_and(|name| name.starts_with("clipboard_history.json."))
            })
            .count();
        assert_eq!(temp_files, 0);
    }

    #[test]
    /// 验证采集图片保存为 PNG，历史条目仅保留缓存引用。
    fn persists_image_capture_as_png_reference() {
        let dir = tempdir().unwrap();
        let rgba = vec![255, 0, 0, 255];

        let item = item_from_capture(
            dir.path(),
            CapturedClipboardItem::Image {
                width: 1,
                height: 1,
                bytes: rgba.clone(),
            },
        )
        .unwrap();

        let ClipboardHistoryItem::Image {
            width,
            height,
            path,
            byte_len,
        } = item
        else {
            panic!("expected image item");
        };

        assert_eq!(width, 1);
        assert_eq!(height, 1);
        assert!(path.starts_with(dir.path()));
        assert_eq!(
            path.extension().and_then(|value| value.to_str()),
            Some("png")
        );
        assert!(path.exists());
        assert_eq!(byte_len, std::fs::metadata(&path).unwrap().len());

        let decoded = image::open(&path).unwrap().into_rgba8();
        assert_eq!(decoded.as_raw(), &rgba);
    }

    #[test]
    /// 验证加载时移除失效图片并重写清理后的索引。
    fn load_skips_missing_image_files_and_rewrites_index() {
        let dir = tempdir().unwrap();
        let missing = dir.path().join("missing.png");
        let mut history = ClipboardHistory::default();
        history.push(ClipboardHistoryItem::Image {
            width: 1,
            height: 1,
            path: missing,
            byte_len: 4,
        });
        save_history(dir.path(), &history).unwrap();

        let loaded = load_history(dir.path()).unwrap();

        assert_eq!(loaded.items().len(), 0);
        let content = std::fs::read_to_string(history_index_path(dir.path())).unwrap();
        let json = serde_json::from_str::<serde_json::Value>(&content).unwrap();
        assert_eq!(json["entries"].as_array().unwrap().len(), 0);
    }

    #[test]
    /// 验证恢复时重复条目的废弃图片缓存被删除。
    fn load_deletes_duplicate_image_files_removed_during_recovery() {
        let dir = tempdir().unwrap();
        let newest_path = dir.path().join("clipboard-image-1-0.png");
        let older_path = dir.path().join("clipboard-image-1-1.png");
        let bytes = b"same png bytes";
        std::fs::write(&newest_path, bytes).unwrap();
        std::fs::write(&older_path, bytes).unwrap();
        let mut history = ClipboardHistory::default();
        history.push(ClipboardHistoryItem::Image {
            width: 2,
            height: 2,
            path: older_path.clone(),
            byte_len: bytes.len() as u64,
        });
        history.push(ClipboardHistoryItem::Image {
            width: 2,
            height: 2,
            path: newest_path.clone(),
            byte_len: bytes.len() as u64,
        });
        let index = serde_json::json!({
            "entries": [
                {
                    "Image": {
                        "width": 2,
                        "height": 2,
                        "path": newest_path,
                        "byte_len": bytes.len()
                    }
                },
                {
                    "Image": {
                        "width": 2,
                        "height": 2,
                        "path": older_path,
                        "byte_len": bytes.len()
                    }
                }
            ]
        });
        std::fs::write(
            history_index_path(dir.path()),
            serde_json::to_string(&index).unwrap(),
        )
        .unwrap();

        let loaded = load_history(dir.path()).unwrap();

        assert_eq!(loaded.items().len(), 1);
        let kept_path = loaded
            .get(0)
            .and_then(|item| item.image_path().map(std::path::Path::to_path_buf));
        assert_eq!(kept_path, Some(newest_path.clone()));
        assert!(newest_path.exists());
        assert!(!older_path.exists());
    }

    #[test]
    /// 验证混合历史去重时保留较新的图片记录。
    fn load_keeps_newer_duplicate_image_when_non_image_entry_is_newest() {
        let dir = tempdir().unwrap();
        let newest_path = dir.path().join("clipboard-image-2-0.png");
        let older_path = dir.path().join("clipboard-image-2-1.png");
        let bytes = b"same png bytes";
        std::fs::write(&newest_path, bytes).unwrap();
        std::fs::write(&older_path, bytes).unwrap();
        let index = serde_json::json!({
            "entries": [
                { "Text": { "text": "newer text" } },
                {
                    "Image": {
                        "width": 2,
                        "height": 2,
                        "path": newest_path,
                        "byte_len": bytes.len()
                    }
                },
                {
                    "Image": {
                        "width": 2,
                        "height": 2,
                        "path": older_path,
                        "byte_len": bytes.len()
                    }
                }
            ]
        });
        std::fs::write(
            history_index_path(dir.path()),
            serde_json::to_string(&index).unwrap(),
        )
        .unwrap();

        let loaded = load_history(dir.path()).unwrap();

        assert_eq!(loaded.items().len(), 2);
        let kept_path = loaded
            .get(1)
            .and_then(|item| item.image_path().map(std::path::Path::to_path_buf));
        assert_eq!(kept_path, Some(newest_path.clone()));
        assert!(newest_path.exists());
        assert!(!older_path.exists());
    }

    #[test]
    /// 验证恢复时删除超出历史上限条目的图片缓存。
    fn load_deletes_over_limit_image_files_removed_during_recovery() {
        let dir = tempdir().unwrap();
        let entries = (0..21)
            .map(|index| {
                let path = dir.path().join(format!("clipboard-image-{index}-0.png"));
                std::fs::write(&path, format!("png {index}")).unwrap();
                serde_json::json!({
                    "Image": {
                        "width": 1,
                        "height": 1,
                        "path": path,
                        "byte_len": format!("png {index}").len()
                    }
                })
            })
            .collect::<Vec<_>>();
        std::fs::write(
            history_index_path(dir.path()),
            serde_json::to_string(&serde_json::json!({ "entries": entries })).unwrap(),
        )
        .unwrap();

        let loaded = load_history(dir.path()).unwrap();

        assert_eq!(loaded.items().len(), 20);
        let remaining_files = std::fs::read_dir(dir.path())
            .unwrap()
            .filter_map(Result::ok)
            .filter(|entry| {
                entry
                    .file_name()
                    .to_str()
                    .is_some_and(|name| name.starts_with("clipboard-image-"))
            })
            .count();
        assert_eq!(remaining_files, 20);
    }

    #[test]
    /// 验证能够删除属于应用管理范围的缓存图片。
    fn delete_image_files_removes_existing_files() {
        let dir = tempdir().unwrap();
        let path = dir.path().join("clipboard-image-1-0.png");
        std::fs::write(&path, b"png").unwrap();

        delete_image_files(dir.path(), vec![path.clone()]);

        assert!(!path.exists());
    }

    #[test]
    /// 验证目录外即使命名相同的文件也不会被删除。
    fn delete_image_files_ignores_outside_paths_with_generated_names() {
        let dir = tempdir().unwrap();
        let outside = tempdir().unwrap();
        let outside_path = outside.path().join("clipboard-image-1-0.png");
        std::fs::write(&outside_path, b"png").unwrap();

        delete_image_files(dir.path(), vec![outside_path.clone()]);

        assert!(outside_path.exists());
    }

    #[test]
    /// 验证目录外同名文件不会影响应用内的删除边界。
    fn delete_image_files_ignores_outside_paths_even_when_filename_collides() {
        let dir = tempdir().unwrap();
        let outside = tempdir().unwrap();
        let inside_path = dir.path().join("clipboard-image-1-0.png");
        let outside_path = outside.path().join("clipboard-image-1-0.png");
        std::fs::write(&inside_path, b"inside").unwrap();
        std::fs::write(&outside_path, b"outside").unwrap();

        delete_image_files(dir.path(), vec![outside_path.clone()]);

        assert!(inside_path.exists());
        assert!(outside_path.exists());

        delete_image_files(dir.path(), vec![inside_path.clone()]);

        assert!(!inside_path.exists());
        assert!(outside_path.exists());
    }

    #[test]
    /// 验证历史目录中的非应用生成文件不会被删除。
    fn delete_image_files_ignores_non_generated_names_inside_history_dir() {
        let dir = tempdir().unwrap();
        let path = dir.path().join("image.png");
        std::fs::write(&path, b"png").unwrap();

        delete_image_files(dir.path(), vec![path.clone()]);

        assert!(path.exists());
    }

    #[test]
    /// 验证恢复时拒绝将目录外图片作为应用缓存记录。
    fn load_skips_existing_image_paths_outside_history_dir() {
        let dir = tempdir().unwrap();
        let outside = tempdir().unwrap();
        let outside_path = outside.path().join("clipboard-image-1-0.png");
        std::fs::write(&outside_path, b"png").unwrap();
        let index = serde_json::json!({
            "entries": [
                {
                    "Image": {
                        "width": 1,
                        "height": 1,
                        "path": outside_path,
                        "byte_len": 3
                    }
                }
            ]
        });
        std::fs::write(
            history_index_path(dir.path()),
            serde_json::to_string(&index).unwrap(),
        )
        .unwrap();

        let loaded = load_history(dir.path()).unwrap();

        assert_eq!(loaded.items().len(), 0);
        assert!(outside_path.exists());
    }
}
