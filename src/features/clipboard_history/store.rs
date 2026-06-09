//! Disk-backed clipboard history storage.

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

pub fn history_index_path(dir: &Path) -> PathBuf {
    dir.join(HISTORY_INDEX_FILE)
}

pub fn load_history(dir: &Path) -> Result<ClipboardHistory, String> {
    let path = history_index_path(dir);
    if !path.exists() {
        return Ok(ClipboardHistory::default());
    }

    let content =
        std::fs::read_to_string(&path).map_err(|err| format!("read history failed: {err}"))?;
    let persisted = serde_json::from_str::<PersistedClipboardHistory>(&content)
        .map_err(|err| format!("parse history failed: {err}"))?;

    let mut history = ClipboardHistory::default();
    for entry in persisted.entries.into_iter().rev() {
        history.push(entry);
    }
    history.retain_valid_images();
    history.trim_to_limit();
    save_history(dir, &history)?;
    Ok(history)
}

pub fn save_history(dir: &Path, history: &ClipboardHistory) -> Result<(), String> {
    std::fs::create_dir_all(dir).map_err(|err| format!("create history dir failed: {err}"))?;
    let persisted = PersistedClipboardHistory {
        entries: history.items(),
    };
    let content = serde_json::to_string_pretty(&persisted)
        .map_err(|err| format!("serialize history failed: {err}"))?;
    std::fs::write(history_index_path(dir), content)
        .map_err(|err| format!("write history failed: {err}"))
}

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

pub fn delete_image_files(paths: impl IntoIterator<Item = PathBuf>) {
    for path in paths {
        if let Err(err) = std::fs::remove_file(&path) {
            if err.kind() != std::io::ErrorKind::NotFound {
                log::error!("delete clipboard image failed: {err}");
            }
        }
    }
}

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
        load_history, save_history,
    };
    use crate::features::clipboard_history::history::{ClipboardHistory, ClipboardHistoryItem};
    use tempfile::tempdir;

    #[test]
    fn missing_index_loads_empty_history() {
        let dir = tempdir().unwrap();

        let history = load_history(dir.path()).unwrap();

        assert_eq!(history.items().len(), 0);
    }

    #[test]
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
    fn delete_image_files_removes_existing_files() {
        let dir = tempdir().unwrap();
        let path = dir.path().join("image.png");
        std::fs::write(&path, b"png").unwrap();

        delete_image_files(vec![path.clone()]);

        assert!(!path.exists());
    }
}
